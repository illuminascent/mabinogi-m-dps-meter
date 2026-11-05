//! Thin Packet Monitor bridge. No third-party crates, injected code, or Python callbacks.
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const SNAPLEN: usize = 9000;
const QUEUE_LIMIT: usize = 2048;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Flow {
    pub family: u32,
    pub local: [u8; 16],
    pub remote: [u8; 16],
    pub local_port: u16,
    pub remote_port: u16,
    pub adapter_guid: [u8; 16],
    pub adapter_mac: [u8; 8],
}

#[repr(C)]
#[derive(Clone)]
pub struct Packet {
    pub timestamp: u64,
    pub group: u64,
    pub packet_count: u16,
    pub appearance: u16,
    pub component: u16,
    pub edge: u16,
    pub direction: u16,
    pub kind: u16,
    pub length: u32,
    pub missed_write: u32,
    pub missed_read: u32,
    pub data: [u8; SNAPLEN],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    data: *const u8,
    size: u32,
    metadata_offset: u32,
    packet_offset: u32,
    packet_length: u32,
    missed_write: u32,
    missed_read: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Metadata {
    group: u64,
    packet_count: u16,
    appearance: u16,
    direction: u16,
    kind: u16,
    component: u16,
    edge: u16,
    reserved: u16,
    drop_reason: u32,
    drop_location: u32,
    processor: u16,
    timestamp: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
union Address {
    bytes: [u8; 16],
    alignment: [u64; 2],
}

#[repr(C)]
struct Source {
    kind: u32,
    name: [u16; 64],
    description: [u16; 128],
    id: u32,
    secondary: u32,
    parent: u32,
    present: u32,
    detail: Address,
}

#[repr(C)]
struct SourceList {
    count: u32,
    first: *const Source,
}

#[repr(C)]
struct Constraint {
    name: [u16; 64],
    present: u32,
    mac1: [u8; 6],
    mac2: [u8; 6],
    vlan: u16,
    ether_type: u16,
    dscp: u16,
    protocol: u8,
    ip1: Address,
    ip2: Address,
    prefix1: u8,
    prefix2: u8,
    port1: u16,
    port2: u16,
    flags: u8,
    encapsulation: u32,
    vxlan: u16,
    packets: u64,
    bytes: u64,
}

fn constraint(flow: &Flow, reverse: bool) -> Constraint {
    let mut result: Constraint = unsafe { std::mem::zeroed() };
    result.name[..8].copy_from_slice(&[109, 97, 98, 105, 45, 100, 112, 115]);
    result.present = (1 << 5) | (1 << 6) | (1 << 7) | (1 << 11) | (1 << 12);
    if flow.family == 6 { result.present |= 1 << 8; }
    result.protocol = 6;
    let (a, b, ap, bp) = if reverse {
        (flow.remote, flow.local, flow.remote_port, flow.local_port)
    } else { (flow.local, flow.remote, flow.local_port, flow.remote_port) };
    result.ip1 = Address { bytes: a };
    result.ip2 = Address { bytes: b };
    // The API constraint's port fields use native numeric port values. Packet
    // headers still use network byte order; that conversion is in the parser.
    result.port1 = ap;
    result.port2 = bp;
    result
}

fn tcp_constraint() -> Constraint {
    let mut result: Constraint = unsafe { std::mem::zeroed() };
    result.present = 1 << 5;
    result.protocol = 6;
    result
}

struct QueuedPacket {
    timestamp:u64,group:u64,packet_count:u16,appearance:u16,component:u16,edge:u16,
    direction:u16,kind:u16,length:u32,missed_write:u32,missed_read:u32,data:Vec<u8>,
}

struct Context {
    queue: Mutex<VecDeque<QueuedPacket>>,
    lost: AtomicU64,
    events: AtomicU64,
    fault: AtomicU64,
    oversized: AtomicU64,
    contended:AtomicU64,
    full:AtomicU64,
}

impl Context {
    fn new() -> Self {
        Self { queue: Mutex::new(VecDeque::with_capacity(QUEUE_LIMIT)),
               lost: AtomicU64::new(0), events: AtomicU64::new(0), fault: AtomicU64::new(0),oversized:AtomicU64::new(0),
               contended:AtomicU64::new(0),full:AtomicU64::new(0) }
    }
}

fn bounded_range(offset: u32, length: usize, size: u32) -> bool {
    (offset as usize).checked_add(length).is_some_and(|end| end <= size as usize)
}

unsafe extern "system" fn packet_callback(context: *mut c_void, descriptor: *const Descriptor) {
    if context.is_null() || descriptor.is_null() { return; }
    let context = &*(context as *const Context);
    let d = std::ptr::read_unaligned(descriptor);
    if d.data.is_null() || d.size > 2 * 1024 * 1024 || d.packet_length as usize > SNAPLEN
        || !bounded_range(d.packet_offset, d.packet_length as usize, d.size)
        || !bounded_range(d.metadata_offset, std::mem::size_of::<Metadata>(), d.size) {
        context.lost.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let m = std::ptr::read_unaligned(d.data.add(d.metadata_offset as usize) as *const Metadata);
    // Windows 11 26100 serializes the metadata packed (40 bytes), despite the
    // public prose showing an unpacked C declaration. Accept only the two
    // explicitly understood layouts; never read Ethernet bytes as a timestamp.
    let metadata_span = d.packet_offset.checked_sub(d.metadata_offset);
    let timestamp_offset = match metadata_span { Some(40) => 32, Some(48) => 40,
        _ => { context.lost.fetch_add(1, Ordering::Relaxed); return; } };
    if !bounded_range(d.metadata_offset, timestamp_offset + 8, d.size) {
        context.lost.fetch_add(1, Ordering::Relaxed); return;
    }
    let timestamp = std::ptr::read_unaligned(d.data.add(d.metadata_offset as usize + timestamp_offset) as *const u64);
    let packet = QueuedPacket { timestamp, group: m.group,
        packet_count: m.packet_count, appearance: m.appearance, component: m.component,
        edge: m.edge, direction: m.direction, kind: m.kind, length: d.packet_length,
        missed_write: d.missed_write, missed_read: d.missed_read,
        data:std::slice::from_raw_parts(d.data.add(d.packet_offset as usize),d.packet_length as usize).to_vec() };
    // The callback never blocks on a consumer or calls Python. Contention is
    // treated as measurable loss, just like a full queue.
    match context.queue.try_lock() {
        Ok(mut queue) if queue.len() < QUEUE_LIMIT => queue.push_back(packet),
        Ok(_) => { context.full.fetch_add(1,Ordering::Relaxed);
            context.lost.fetch_add(1 + d.missed_write as u64 + d.missed_read as u64,Ordering::Relaxed); }
        Err(_) => { context.contended.fetch_add(1,Ordering::Relaxed);
            context.lost.fetch_add(1 + d.missed_write as u64 + d.missed_read as u64,
                                    Ordering::Relaxed); }
    }
}

unsafe extern "system" fn event_callback(context: *mut c_void, info: *const c_void, kind: u32) {
    if !context.is_null() {
        let context = &*(context as *const Context);
        context.events.fetch_add(1, Ordering::Relaxed);
        if kind == 1 || kind == 2 {
            let reason = if info.is_null() { 1 } else {
                std::ptr::read_unaligned((info as *const u8).add(4) as *const u32).max(1)
            };
            context.fault.store(reason as u64, Ordering::Relaxed);
        } else if kind == 3 {
            context.lost.fetch_add(1, Ordering::Relaxed);
            if !info.is_null() && std::ptr::read_unaligned((info as *const u8).add(4) as *const u32)==122 {
                context.oversized.fetch_add(1,Ordering::Relaxed);
            }
        }
    }
}

#[repr(C)]
struct StreamConfig {
    context: *mut c_void,
    event: unsafe extern "system" fn(*mut c_void, *const c_void, u32),
    data: unsafe extern "system" fn(*mut c_void, *const Descriptor),
    multiplier: u16,
    truncation: u16,
}

#[no_mangle]
pub extern "C" fn mabi_capture_abi_version() -> u32 { 2 }

#[no_mangle]
pub extern "C" fn mabi_capture_packet_size() -> u32 { std::mem::size_of::<Packet>() as u32 }

#[cfg(windows)]
pub mod win {
    use super::*;
    type Handle = *mut c_void;
    type Hr = i32;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(name: *const u16, file: Handle, flags: u32) -> Handle;
        fn GetProcAddress(module: Handle, name: *const u8) -> *mut c_void;
        fn FreeLibrary(module: Handle) -> i32;
        fn GetLastError() -> u32;
    }

    struct Api {
        module: Handle,
        initialize: unsafe extern "system" fn(u32, Handle, *mut Handle) -> Hr,
        uninitialize: unsafe extern "system" fn(Handle),
        create_session: unsafe extern "system" fn(Handle, *const u16, *mut Handle) -> Hr,
        close_session: unsafe extern "system" fn(Handle),
        sources: unsafe extern "system" fn(Handle, u32, u8, usize, *mut usize, *mut SourceList) -> Hr,
        add_source: unsafe extern "system" fn(Handle, *const Source) -> Hr,
        add_constraint: unsafe extern "system" fn(Handle, *const Constraint) -> Hr,
        create_stream: unsafe extern "system" fn(Handle, *const StreamConfig, *mut Handle) -> Hr,
        close_stream: unsafe extern "system" fn(Handle),
        attach: unsafe extern "system" fn(Handle, Handle) -> Hr,
        active: unsafe extern "system" fn(Handle, u8) -> Hr,
    }

    impl Api {
        unsafe fn load() -> Result<Self, String> {
            let name: Vec<u16> = "pktmonapi.dll\0".encode_utf16().collect();
            let module = LoadLibraryExW(name.as_ptr(), std::ptr::null_mut(), 0x800);
            if module.is_null() { return Err(format!("LoadLibraryExW: {}", GetLastError())); }
            macro_rules! bind { ($name:literal) => {{
                let p = GetProcAddress(module, concat!($name, "\0").as_ptr());
                if p.is_null() { FreeLibrary(module); return Err(concat!("Missing ", $name).into()); }
                std::mem::transmute(p)
            }}; }
            Ok(Self { module,
                initialize: bind!("PacketMonitorInitialize"), uninitialize: bind!("PacketMonitorUninitialize"),
                create_session: bind!("PacketMonitorCreateLiveSession"), close_session: bind!("PacketMonitorCloseSessionHandle"),
                sources: bind!("PacketMonitorEnumDataSources"), add_source: bind!("PacketMonitorAddSingleDataSourceToSession"),
                add_constraint: bind!("PacketMonitorAddCaptureConstraint"), create_stream: bind!("PacketMonitorCreateRealtimeStream"),
                close_stream: bind!("PacketMonitorCloseRealtimeStream"), attach: bind!("PacketMonitorAttachOutputToSession"),
                active: bind!("PacketMonitorSetSessionActive") })
        }
    }

    impl Drop for Api { fn drop(&mut self) { unsafe { FreeLibrary(self.module); } } }
    /// Probe system exports without initializing a driver or capture session.
    pub fn capability_check()->Result<(),String> { unsafe {Api::load().map(drop)} }

    pub struct Capture {
        api: Api,
        monitor: Handle,
        session: Handle,
        stream: Handle,
        context: Box<Context>,
    }

    fn check(hr: Hr, stage: &str) -> Result<(), String> {
        if hr < 0 { Err(format!("{stage}: HRESULT 0x{:08X}", hr as u32)) } else { Ok(()) }
    }

    impl Capture {
        unsafe fn open(flows: &[Flow], broad_tcp: bool) -> Result<Box<Self>, String> {
            if flows.is_empty() || flows.len() > 16 || flows.iter().any(|f|
                !matches!(f.family, 4 | 6) || f.local_port == 0 || f.remote_port == 0
                || f.adapter_guid == [0; 16]) {
                return Err("Expected 1..16 approved TCP tuples with adapter GUIDs".into());
            }
            let mut c = Box::new(Self { api: Api::load()?, monitor: std::ptr::null_mut(),
                session: std::ptr::null_mut(), stream: std::ptr::null_mut(), context: Box::new(Context::new()) });
            check((c.api.initialize)(0x00010000, std::ptr::null_mut(), &mut c.monitor), "PacketMonitorInitialize")?;
            let name: Vec<u16> = format!("MabiDps-{}\0", std::process::id()).encode_utf16().collect();
            check((c.api.create_session)(c.monitor, name.as_ptr(), &mut c.session), "CreateLiveSession")?;
            let mut size = 0;
            (c.api.sources)(c.monitor, 1, 0, 0, &mut size, std::ptr::null_mut());
            if size < std::mem::size_of::<SourceList>() || size > 4 * 1024 * 1024 {
                return Err(format!("Invalid data source buffer size {size}"));
            }
            // u64 storage supplies the native alignment required by the API.
            let mut storage = vec![0u64; size.div_ceil(8)];
            let base = storage.as_mut_ptr() as usize;
            let capacity = storage.len() * 8;
            check((c.api.sources)(c.monitor, 1, 0, capacity, &mut size, base as *mut SourceList), "EnumDataSources")?;
            let list = &*(base as *const SourceList);
            if list.count > 1024 || 8 + list.count as usize * 8 > capacity { return Err("Invalid source list".into()); }
            let mut added = std::collections::HashSet::new();
            let mut diagnostics = Vec::new();
            let pointers = std::slice::from_raw_parts((base + 8) as *const *const Source, list.count as usize);
            for &pointer in pointers {
                let address = pointer as usize;
                if address < base || address.checked_add(std::mem::size_of::<Source>()).is_none_or(|v| v > base + capacity) {
                    return Err("Data source pointer outside enumeration buffer".into());
                }
                let source = &*pointer;
                let detail = source.detail.bytes;
                let end = source.name.iter().position(|v| *v == 0).unwrap_or(source.name.len());
                diagnostics.push(format!("id={} kind={} flags={:x} detail={:02x?} name={}",
                    source.id, source.kind, source.present, detail, String::from_utf16_lossy(&source.name[..end])));
            }
            for flow in flows {
                let candidates: Vec<_> = pointers.iter().copied().filter(|pointer| {
                    let source = &**pointer;
                    let detail = source.detail.bytes;
                    source.kind == 1 &&
                        ((source.present & 8 != 0 && flow.adapter_mac[..6] != [0; 6]
                          && detail[..6] == flow.adapter_mac[..6])
                         || (source.present & 9 == 1 && detail == flow.adapter_guid))
                }).collect();
                if candidates.len() != 1 {
                    return Err(format!("No unique Packet Monitor NIC identity ({} matches); sources: {}",
                        candidates.len(), diagnostics.join("; ")));
                }
                if added.insert((*candidates[0]).id) {
                    check((c.api.add_source)(c.session, candidates[0]), "AddSingleDataSourceToSession")?;
                }
            }
            if broad_tcp {
                check((c.api.add_constraint)(c.session, &tcp_constraint()), "AddTcpCaptureConstraint")?;
            } else {
                for f in flows {
                    for reverse in [false, true] {
                        check((c.api.add_constraint)(c.session, &constraint(f, reverse)), "AddCaptureConstraint")?;
                    }
                }
            }
            let config = StreamConfig { context: &mut *c.context as *mut Context as Handle,
                event: event_callback, data: packet_callback, multiplier: 8, truncation: SNAPLEN as u16 };
            check((c.api.create_stream)(c.monitor, &config, &mut c.stream), "CreateRealtimeStream")?;
            check((c.api.attach)(c.session, c.stream), "AttachOutputToSession")?;
            check((c.api.active)(c.session, 1), "SetSessionActive")?;
            Ok(c)
        }
    }

    impl Drop for Capture {
        fn drop(&mut self) { unsafe {
            if !self.session.is_null() { (self.api.active)(self.session, 0); }
            if !self.stream.is_null() { (self.api.close_stream)(self.stream); }
            if !self.session.is_null() { (self.api.close_session)(self.session); }
            if !self.monitor.is_null() { (self.api.uninitialize)(self.monitor); }
        } }
    }

    unsafe fn error_text(buffer: *mut u8, capacity: u32, message: &str) {
        if !buffer.is_null() && capacity > 0 {
            let length = message.len().min(capacity as usize - 1);
            std::ptr::copy_nonoverlapping(message.as_ptr(), buffer, length);
            *buffer.add(length) = 0;
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn mabi_capture_open(flows: *const Flow, count: u32,
        output: *mut *mut Capture, error: *mut u8, error_capacity: u32) -> i32 {
        if flows.is_null() || output.is_null() || count == 0 || count > 16 { return -1; }
        *output = std::ptr::null_mut();
        match Capture::open(std::slice::from_raw_parts(flows, count as usize), false) {
            Ok(capture) => { *output = Box::into_raw(capture); 0 }
            Err(message) => { error_text(error, error_capacity, &message); -1 }
        }
    }

    /// Broad TCP on selected NICs. The private worker performs delayed exact
    /// process-ownership checks before any payload can leave that worker.
    #[no_mangle]
    pub unsafe extern "C" fn mabi_capture_open_tcp_nics(flows: *const Flow, count: u32,
        output: *mut *mut Capture, error: *mut u8, error_capacity: u32) -> i32 {
        if flows.is_null() || output.is_null() || count == 0 || count > 16 { return -1; }
        *output = std::ptr::null_mut();
        match Capture::open(std::slice::from_raw_parts(flows, count as usize), true) {
            Ok(capture) => { *output = Box::into_raw(capture); 0 }
            Err(message) => { error_text(error, error_capacity, &message); -1 }
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn mabi_capture_poll(capture: *mut Capture, output: *mut Packet) -> i32 {
        if capture.is_null() || output.is_null() { return -1; }
        let c = &*capture;
        // Only compact metadata/vector moves under the lock. Filling the 9KB
        // public ABI record happens after releasing it, away from callbacks.
        let next=match c.context.queue.lock() { Ok(mut queue)=>queue.pop_front(),Err(_)=>return -1 };
        if let Some(p)=next {
            let mut packet=Packet {timestamp:p.timestamp,group:p.group,packet_count:p.packet_count,appearance:p.appearance,
                component:p.component,edge:p.edge,direction:p.direction,kind:p.kind,length:p.length,
                missed_write:p.missed_write,missed_read:p.missed_read,data:[0;SNAPLEN]};
            packet.data[..p.data.len()].copy_from_slice(&p.data);
            std::ptr::write(output,packet);1
        } else {0}
    }

    #[no_mangle]
    pub unsafe extern "C" fn mabi_capture_loss(capture: *mut Capture) -> u64 {
        if capture.is_null() { return 0; }
        (*capture).context.lost.swap(0, Ordering::Relaxed)
    }

    #[no_mangle]
    pub unsafe extern "C" fn mabi_capture_fault(capture: *mut Capture) -> u64 {
        if capture.is_null() { return 0; }
        (*capture).context.fault.load(Ordering::Relaxed)
    }

    #[no_mangle]
    pub unsafe extern "C" fn mabi_capture_oversized(capture: *mut Capture) -> u64 {
        if capture.is_null() { return 0; }
        (*capture).context.oversized.swap(0,Ordering::Relaxed)
    }

    #[no_mangle]
    pub unsafe extern "C" fn mabi_capture_queue_reports(capture:*mut Capture,output:*mut u64,count:u32)->i32 {
        if capture.is_null() || output.is_null() || count!=2 {return -1;}
        *output=(*capture).context.contended.swap(0,Ordering::Relaxed);
        *output.add(1)=(*capture).context.full.swap(0,Ordering::Relaxed);0
    }

    #[no_mangle]
    pub unsafe extern "C" fn mabi_capture_close(capture: *mut Capture) {
        if !capture.is_null() { drop(Box::from_raw(capture)); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn abi_layouts() {
        assert_eq!(std::mem::size_of::<Flow>(), 64);
        assert_eq!(std::mem::size_of::<Packet>(), 9040);
        assert_eq!(std::mem::size_of::<Metadata>(), 40);
        assert_eq!(std::mem::offset_of!(Metadata, timestamp), 32);
        assert_eq!(std::mem::size_of::<Source>(), 424);
        assert_eq!(std::mem::offset_of!(Source, detail), 408);
        assert_eq!(std::mem::size_of::<Constraint>(), 216);
    }
    #[test] fn broad_tcp_constraint_has_no_address_or_port_filter() {
        let c=tcp_constraint();
        assert_eq!(c.present,1<<5);assert_eq!(c.protocol,6);
        assert_eq!(c.port1,0);assert_eq!(c.port2,0);
        unsafe { assert_eq!(c.ip1.bytes,[0;16]);assert_eq!(c.ip2.bytes,[0;16]); }
    }
    #[test] fn bidirectional_constraints() {
        let flow = Flow { family: 4, local: [1; 16], remote: [2; 16],
            local_port: 45678, remote_port: 23456, adapter_guid: [3; 16], adapter_mac: [4; 8] };
        let a = constraint(&flow, false);
        let b = constraint(&flow, true);
        assert_eq!((a.port1, a.port2), (45678, 23456));
        assert_eq!((b.port1, b.port2), (23456, 45678));
        assert_eq!(unsafe { b.ip1.bytes }, flow.remote);
        assert_eq!(a.present, (1 << 5) | (1 << 6) | (1 << 7) | (1 << 11) | (1 << 12));
    }
    #[test] fn descriptor_bounds() {
        assert!(bounded_range(0, 48, 48));
        assert!(!bounded_range(u32::MAX, 48, 48));
        assert!(!bounded_range(20, usize::MAX, 48));
    }
    #[test] fn callback_copy_and_invalid_descriptor_loss() {
        let context = Context::new();
        let mut bytes = [0u8; 64];
        let meta: Metadata = unsafe { std::mem::zeroed() };
        unsafe { std::ptr::write_unaligned(bytes.as_mut_ptr() as *mut Metadata, meta); }
        bytes[40..44].copy_from_slice(b"test");
        let mut descriptor = Descriptor { data: bytes.as_ptr(), size: 64, metadata_offset: 0,
            packet_offset: 40, packet_length: 4, missed_write: 2, missed_read: 0 };
        unsafe { packet_callback(&context as *const Context as *mut c_void, &descriptor); }
        let packet = context.queue.lock().unwrap().pop_front().unwrap();
        assert_eq!(&packet.data[..4], b"test");
        assert_eq!(packet.missed_write, 2);
        descriptor.packet_offset = 100;
        unsafe { packet_callback(&context as *const Context as *mut c_void, &descriptor); }
        assert_eq!(context.lost.load(Ordering::Relaxed), 1);
    }
    #[test] fn packed_and_aligned_timestamp_layouts() {
        for (span, offset) in [(40, 32), (48, 40)] {
            let context = Context::new();
            let mut bytes = [0u8; 64];
            let timestamp = 134_000_000_000_000_000u64;
            bytes[offset..offset + 8].copy_from_slice(&timestamp.to_le_bytes());
            let descriptor = Descriptor { data: bytes.as_ptr(), size: 64, metadata_offset: 0,
                packet_offset: span, packet_length: 1, missed_write: 0, missed_read: 0 };
            unsafe { packet_callback(&context as *const Context as *mut c_void, &descriptor); }
            let packet = context.queue.lock().unwrap().pop_front().unwrap();
            assert_eq!(packet.timestamp, timestamp);
        }
    }
    #[test] fn fatal_stream_event_is_reported() {
        let context = Context::new();
        let info = [0u32, 13u32];
        unsafe { event_callback(&context as *const Context as *mut c_void,
                                 info.as_ptr() as *const c_void, 2); }
        assert_eq!(context.fault.load(Ordering::Relaxed), 13);
    }
}
