//! Inbox Windows discovery and handles; never reads game memory.
use mabi_meter_core::{Result,transport::Flow};
use std::{ffi::c_void,net::{IpAddr,Ipv4Addr,Ipv6Addr},collections::{BTreeMap,BTreeSet},ptr::null_mut};
pub type Handle=*mut c_void;
pub fn wide(s:&str)->Vec<u16>{s.encode_utf16().chain(Some(0)).collect()}
pub fn error(stage:&str)->String {format!("{stage}: Windows error {}",unsafe{GetLastError()})}
pub struct Owned(pub Handle);
impl Drop for Owned {fn drop(&mut self){unsafe{if !self.0.is_null() && self.0 as isize != -1 {CloseHandle(self.0);}}}}
#[repr(C)] struct ProcessEntry {size:u32,usage:u32,pid:u32,heap:usize,module:u32,threads:u32,parent:u32,priority:i32,flags:u32,name:[u16;260]}
#[repr(C)] pub struct Security {pub length:u32,pub descriptor:Handle,pub inherit:i32}
#[link(name="kernel32")] extern "system" {
    fn GetLocalTime(time:*mut [u16;8]);
    pub fn CloseHandle(h:Handle)->i32;
    pub fn GetLastError()->u32;
    fn CreateToolhelp32Snapshot(flags:u32,pid:u32)->Handle;
    fn Process32FirstW(h:Handle,p:*mut ProcessEntry)->i32;
    fn Process32NextW(h:Handle,p:*mut ProcessEntry)->i32;
    pub fn OpenProcess(access:u32,inherit:i32,pid:u32)->Handle;
    pub fn GetProcessTimes(h:Handle,created:*mut u64,exit:*mut u64,kernel:*mut u64,user:*mut u64)->i32;
    pub fn GetCurrentProcess()->Handle;
    pub fn LocalFree(p:Handle)->Handle;
    pub fn WaitForSingleObject(h:Handle,millis:u32)->u32;
    pub fn CreateNamedPipeW(name:*const u16,open:u32,mode:u32,instances:u32,out_size:u32,in_size:u32,timeout:u32,security:*const Security)->Handle;
    pub fn ConnectNamedPipe(h:Handle,overlapped:Handle)->i32;
    pub fn GetNamedPipeClientProcessId(h:Handle,pid:*mut u32)->i32;
    pub fn GetNamedPipeServerProcessId(h:Handle,pid:*mut u32)->i32;
    pub fn CreateFileW(name:*const u16,access:u32,share:u32,security:Handle,disposition:u32,flags:u32,template:Handle)->Handle;
    pub fn SetNamedPipeHandleState(h:Handle,mode:*const u32,collect:*const u32,timeout:*const u32)->i32;
    pub fn PeekNamedPipe(h:Handle,buffer:Handle,size:u32,read:*mut u32,available:*mut u32,left:*mut u32)->i32;
    pub fn ReadFile(h:Handle,buffer:*mut u8,size:u32,read:*mut u32,overlapped:Handle)->i32;
    pub fn WriteFile(h:Handle,buffer:*const u8,size:u32,written:*mut u32,overlapped:Handle)->i32;
}
/// Windows local wall-clock time for user-facing filenames; absolute Unix time
/// is also retained in the recording. Neither clock is a server event timestamp.
pub fn local_timestamp()->String {
    let mut time=[0u16;8];unsafe{GetLocalTime(&mut time);}
    format!("{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.{:03}",time[0],time[1],time[3],time[4],time[5],time[6],time[7])
}
#[link(name="iphlpapi")] extern "system" {
    fn GetExtendedTcpTable(table:Handle,size:*mut u32,order:i32,family:u32,class:u32,reserved:u32)->u32;
    fn GetAdaptersAddresses(family:u32,flags:u32,reserved:Handle,addresses:Handle,size:*mut u32)->u32;
}
#[link(name="advapi32")] extern "system" {
    pub fn OpenProcessToken(h:Handle,access:u32,token:*mut Handle)->i32;
    pub fn GetTokenInformation(h:Handle,class:u32,data:Handle,size:u32,needed:*mut u32)->i32;
    pub fn ConvertSidToStringSidW(sid:Handle,text:*mut *mut u16)->i32;
    pub fn ConvertStringSecurityDescriptorToSecurityDescriptorW(text:*const u16,revision:u32,out:*mut Handle,size:*mut u32)->i32;
}
#[link(name="bcrypt")] extern "system" {fn BCryptGenRandom(algorithm:Handle,buffer:*mut u8,size:u32,flags:u32)->i32;}
pub fn random()->Result<[u8;32]> {let mut bytes=[0;32];if unsafe{BCryptGenRandom(null_mut(),bytes.as_mut_ptr(),32,2)}<0 {Err("Windows random generator failed".into())}else{Ok(bytes)}}
pub fn created(pid:u32)->Result<u64> {unsafe {
    let p=Owned(OpenProcess(0x1000,0,pid));if p.0.is_null(){return Err(error("OpenProcess"));}
    let (mut c,mut e,mut k,mut u)=(0,0,0,0);
    if GetProcessTimes(p.0,&mut c,&mut e,&mut k,&mut u)==0{return Err(error("GetProcessTimes"));}Ok(c)
}}
fn processes()->Result<Vec<(u32,u64)>> {unsafe {
    let h=Owned(CreateToolhelp32Snapshot(2,0));if h.0 as isize == -1{return Err(error("Process snapshot"));}
    let mut p:ProcessEntry=std::mem::zeroed();p.size=std::mem::size_of::<ProcessEntry>() as u32;
    let mut more=Process32FirstW(h.0,&mut p);let mut found=Vec::new();
    while more!=0 {let n=p.name.iter().position(|v|*v==0).unwrap_or(260);
        if String::from_utf16_lossy(&p.name[..n]).eq_ignore_ascii_case("MabinogiMobile.exe") {found.push((p.pid,created(p.pid)?));}
        more=Process32NextW(h.0,&mut p);
    }Ok(found)
}}
fn u32at(p:&[u8],i:usize)->u32 {u32::from_le_bytes(p[i..i+4].try_into().unwrap())}
#[derive(Clone,Debug)] pub struct Plan {pub owners:Vec<(u32,u64)>,pub flows:Vec<Flow>}
pub fn discover()->Result<Plan> {unsafe {
    let owners=processes()?;let pids:BTreeSet<_>=owners.iter().map(|(pid,_)|*pid).collect();let mut flows=Vec::new();
    for (family,row_size) in [(2u32,24usize),(23,56)] {
        let mut size=0;GetExtendedTcpTable(null_mut(),&mut size,0,family,5,0);
        let mut data=Vec::new();let mut ready=false;
        for _ in 0..5 {if size>16*1024*1024{return Err("TCP owner table exceeds limit".into());}data.resize(size as usize,0);
            let status=GetExtendedTcpTable(data.as_mut_ptr() as Handle,&mut size,0,family,5,0);
            if status==122{continue;}if status!=0{return Err(format!("TCP owner table: Windows error {status}"));}ready=true;break;
        }if !ready || data.len()<4{return Err("TCP owner table unavailable".into());}
        let count=u32at(&data,0) as usize;if count>(data.len()-4)/row_size{return Err("TCP owner table truncated".into());}
        for p in data[4..4+count*row_size].chunks_exact(row_size) {
            let (state,pid,src,dst,sp,dp)=if family==2 {(u32at(p,0),u32at(p,20),IpAddr::V4(Ipv4Addr::from(<[u8;4]>::try_from(&p[4..8]).unwrap())),IpAddr::V4(Ipv4Addr::from(<[u8;4]>::try_from(&p[12..16]).unwrap())),8,16)}
            else {(u32at(p,48),u32at(p,52),IpAddr::V6(Ipv6Addr::from(<[u8;16]>::try_from(&p[..16]).unwrap())),IpAddr::V6(Ipv6Addr::from(<[u8;16]>::try_from(&p[24..40]).unwrap())),20,44)};
            if state!=5 || !pids.contains(&pid) || src.is_loopback() || dst.is_loopback() || src.is_unspecified() || dst.is_unspecified(){continue;}
            if matches!(src,IpAddr::V6(ip) if ip.is_unicast_link_local()) || matches!(dst,IpAddr::V6(ip) if ip.is_unicast_link_local()){continue;}
            let (sport,dport)=(u16::from_be_bytes(p[sp..sp+2].try_into().unwrap()),u16::from_be_bytes(p[dp..dp+2].try_into().unwrap()));
            if sport!=0 && dport!=0{flows.push(Flow{src:src.to_string(),sport,dst:dst.to_string(),dport});}
        }
    }
    if owners.iter().any(|(pid,c)|created(*pid).ok()!=Some(*c)){return Err("Game process changed during discovery".into());}
    Ok(Plan{owners,flows})
}}
#[repr(C)] struct SocketAddress {address:*const u8,length:i32}
#[repr(C)] struct Unicast {length_flags:u64,next:*const Unicast,socket:SocketAddress}
#[repr(C)] struct Adapter {length_index:u64,next:*const Adapter,name:*const i8,unicast:*const Unicast,anycast:Handle,multicast:Handle,dns:Handle,suffix:*const u16,description:*const u16,friendly:*const u16,mac:[u8;8],mac_len:u32}
fn guid(text:&str)->Option<[u8;16]> {
    let clean:String=text.chars().filter(|c|!"{}-".contains(*c)).collect();if clean.len()!=32{return None;}
    let mut b=[0;16];for (i,v) in b.iter_mut().enumerate(){*v=u8::from_str_radix(&clean[2*i..2*i+2],16).ok()?;}
    b[..4].reverse();b[4..6].reverse();b[6..8].reverse();Some(b)
}
pub fn native_flows(flows:&[Flow])->Result<Vec<mabi_capture::Flow>> {unsafe {
    let mut size=16384;let mut data=Vec::<u64>::new();let mut ready=false;
    for _ in 0..5 {if size>4*1024*1024{return Err("Adapter table exceeds limit".into());}data.resize((size as usize).div_ceil(8),0);
        let status=GetAdaptersAddresses(0,14,null_mut(),data.as_mut_ptr() as Handle,&mut size);
        if status==111{continue;}if status!=0{return Err(format!("Adapter enumeration: Windows error {status}"));}ready=true;break;
    }if !ready{return Err("Adapter table kept changing".into());}
    let base=data.as_ptr() as usize;let end=base+data.len()*8;
    let valid=|ptr:usize,len:usize|ptr>=base && ptr.checked_add(len).is_some_and(|v|v<=end);
    let mut a=data.as_ptr() as *const Adapter;let mut adapters:BTreeMap<IpAddr,BTreeSet<([u8;16],[u8;8])>>=BTreeMap::new();let mut seen=BTreeSet::new();
    while !a.is_null() {if !valid(a as usize,std::mem::size_of::<Adapter>()) || !seen.insert(a as usize) || seen.len()>1024{return Err("Invalid adapter list".into());}
        let item=&*a;
        if !item.name.is_null() {
            let mut name=Vec::new();for i in 0..80{let p=item.name.add(i);if !valid(p as usize,1){return Err("Invalid adapter name".into());}if *p==0{break;}name.push(*p as u8);}
            if let Some(g)=std::str::from_utf8(&name).ok().and_then(guid) {
                let mut u=item.unicast;let mut visited=BTreeSet::new();
                while !u.is_null(){if !valid(u as usize,std::mem::size_of::<Unicast>()) || !visited.insert(u as usize) || visited.len()>4096{return Err("Invalid unicast list".into());}
                    let socket=&(*u).socket;
                    if socket.length>=16 && valid(socket.address as usize,socket.length as usize){let bytes=std::slice::from_raw_parts(socket.address,socket.length as usize);
                        let ip=match u16::from_le_bytes(bytes[..2].try_into().unwrap()){2=>Some(IpAddr::V4(Ipv4Addr::from(<[u8;4]>::try_from(&bytes[4..8]).unwrap()))),23 if bytes.len()>=28=>Some(IpAddr::V6(Ipv6Addr::from(<[u8;16]>::try_from(&bytes[8..24]).unwrap()))),_=>None};
                        if let Some(ip)=ip {let mac=if item.mac_len==6{item.mac}else{[0;8]};adapters.entry(ip).or_default().insert((g,mac));}
                    }u=(*u).next;
                }
            }
        }a=item.next;
    }
    let mut result=Vec::new();for f in flows {
        let local:IpAddr=f.src.parse().map_err(|_|"Invalid local IP")?;let remote:IpAddr=f.dst.parse().map_err(|_|"Invalid remote IP")?;
        let candidates=adapters.get(&local).filter(|c|c.len()==1).ok_or("No unique adapter for game socket")?;let (g,mac)=candidates.first().unwrap();
        let mut native=mabi_capture::Flow{family:4,local:[0;16],remote:[0;16],local_port:f.sport,remote_port:f.dport,adapter_guid:*g,adapter_mac:*mac};
        match (local,remote){(IpAddr::V4(a),IpAddr::V4(b))=>{native.local[..4].copy_from_slice(&a.octets());native.remote[..4].copy_from_slice(&b.octets());},(IpAddr::V6(a),IpAddr::V6(b))=>{native.family=6;native.local=a.octets();native.remote=b.octets();},_=>return Err("Mixed IP families".into())}
        result.push(native);
    }Ok(result)
}}
pub fn user_sid()->Result<String> {unsafe {
    let mut token=null_mut();if OpenProcessToken(GetCurrentProcess(),8,&mut token)==0{return Err(error("OpenProcessToken"));}let token=Owned(token);
    let mut size=0;GetTokenInformation(token.0,1,null_mut(),0,&mut size);if size==0||size>65536{return Err("Invalid token size".into());}
    let mut data=vec![0usize;(size as usize).div_ceil(std::mem::size_of::<usize>())];
    if GetTokenInformation(token.0,1,data.as_mut_ptr() as Handle,size,&mut size)==0{return Err(error("Token user"));}
    let mut text=null_mut();if ConvertSidToStringSidW(data[0] as Handle,&mut text)==0{return Err(error("User SID"));}
    let mut n=0;while n<256 && *text.add(n)!=0{n+=1;}let result=String::from_utf16_lossy(std::slice::from_raw_parts(text,n));LocalFree(text as Handle);Ok(result)
}}
