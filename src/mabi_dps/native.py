"""C ABI client for our independently built Rust DLL, never a bundled helper."""
import ctypes as C
import ipaddress
from pathlib import Path
import socket

from .windows import _bind, _dll, system_directory

ROOT = Path(__file__).resolve().parents[2]
DLL_PATH = ROOT / 'dist' / 'mabi_capture.dll'


class NativeFlow(C.Structure):
    _fields_ = [('family', C.c_uint32), ('local', C.c_ubyte * 16),
                ('remote', C.c_ubyte * 16), ('local_port', C.c_uint16),
                ('remote_port', C.c_uint16), ('adapter_guid', C.c_ubyte * 16),
                ('adapter_mac', C.c_ubyte * 8)]


class NativePacket(C.Structure):
    _fields_ = [('timestamp', C.c_uint64), ('group', C.c_uint64),
                ('packet_count', C.c_uint16), ('appearance', C.c_uint16),
                ('component', C.c_uint16), ('edge', C.c_uint16),
                ('direction', C.c_uint16), ('kind', C.c_uint16),
                ('length', C.c_uint32), ('missed_write', C.c_uint32),
                ('missed_read', C.c_uint32), ('data', C.c_ubyte * 9000)]


def adapter_identities():
    """Map actual local unicast addresses to their OS adapter GUIDs.

    Parse only the documented stable prefix of IP_ADAPTER_ADDRESSES and
    IP_ADAPTER_UNICAST_ADDRESS. No routing guess or fixed interface index.
    """
    class SockAddr(C.Structure):
        _fields_ = [('address', C.c_void_p), ('length', C.c_int)]

    class Unicast(C.Structure):
        pass

    Unicast._fields_ = [('length_flags', C.c_uint64), ('next', C.POINTER(Unicast)),
                       ('socket', SockAddr)]

    class Adapter(C.Structure):
        pass

    Adapter._fields_ = [('length_index', C.c_uint64), ('next', C.POINTER(Adapter)),
                       ('name', C.c_char_p), ('unicast', C.POINTER(Unicast)),
                       ('anycast', C.c_void_p), ('multicast', C.c_void_p),
                       ('dns_server', C.c_void_p), ('dns_suffix', C.c_wchar_p),
                       ('description', C.c_wchar_p), ('friendly', C.c_wchar_p),
                       ('physical', C.c_ubyte * 8), ('physical_length', C.c_uint32)]
    function = _bind(_dll('iphlpapi.dll'), 'GetAdaptersAddresses', C.c_uint32,
                     [C.c_uint32, C.c_uint32, C.c_void_p, C.c_void_p, C.POINTER(C.c_uint32)])
    size = C.c_uint32(16384)
    for _ in range(5):
        if size.value > 4 * 1024 * 1024:
            raise OSError('Adapter table exceeds limit')
        buffer = C.create_string_buffer(size.value)
        status = function(0, 2 | 4 | 8, None, buffer, C.byref(size))
        if status == 111:  # ERROR_BUFFER_OVERFLOW
            continue
        if status:
            raise C.WinError(status)
        break
    else:
        raise OSError('Adapter table kept changing')
    import uuid
    found = {}
    current = C.cast(buffer, C.POINTER(Adapter))
    seen = set()
    while current:
        address = C.addressof(current.contents)
        if address in seen or len(seen) > 1024:
            raise OSError('Invalid adapter list')
        seen.add(address)
        adapter = current.contents
        try:
            guid = uuid.UUID(adapter.name.decode('ascii').strip('{}')).bytes_le
        except (ValueError, UnicodeError):
            current = adapter.next
            continue
        unicast = adapter.unicast
        unicast_seen = set()
        while unicast:
            item_address = C.addressof(unicast.contents)
            if item_address in unicast_seen or len(unicast_seen) > 4096:
                raise OSError('Invalid unicast list')
            unicast_seen.add(item_address)
            item = unicast.contents
            if item.socket.address and item.socket.length >= 16:
                raw = C.string_at(item.socket.address, item.socket.length)
                family = int.from_bytes(raw[:2], 'little')
                packed = raw[4:8] if family == 2 else raw[8:24] if family == 23 and len(raw) >= 28 else None
                if packed is not None:
                    ip = str(ipaddress.ip_address(packed))
                    mac = bytes(adapter.physical[:adapter.physical_length])
                    found.setdefault(ip, set()).add((guid, mac))
            unicast = item.next
        current = adapter.next
    return found


def adapter_guids():
    return {address: {guid for guid, _ in values} for address, values in adapter_identities().items()}


def native_flows(rows, adapters):
    flows = []
    for row in rows:
        ip = ipaddress.ip_address(row['local_address'])
        remote = ipaddress.ip_address(row['remote_address'])
        if ip.version != row['family'] or remote.version != ip.version:
            raise ValueError('Mixed IP families in capture tuple')
        if ip.is_link_local or remote.is_link_local:
            raise ValueError('Scoped/link-local capture is not supported by this prototype')
        guids = adapters.get(str(ip), set())
        if len(guids) != 1:
            raise ValueError(f'No unique adapter GUID for {ip}')
        f = NativeFlow()
        f.family = ip.version
        f.local[:len(ip.packed)] = ip.packed
        f.remote[:len(remote.packed)] = remote.packed
        f.local_port = row['local_port']
        f.remote_port = row['remote_port']
        identity = next(iter(guids))
        guid, mac = identity if isinstance(identity, tuple) else (identity, b'')
        f.adapter_guid[:] = guid
        if len(mac) == 6:
            f.adapter_mac[:6] = mac
        flows.append(f)
    if not 1 <= len(flows) <= 16:
        raise ValueError('Capture requires 1..16 game-owned TCP connections')
    return (NativeFlow * len(flows))(*flows)


class NativeCapture:
    def __init__(self, rows, adapters=None, *, broad_tcp=False):
        if not DLL_PATH.is_file():
            raise OSError('Build our capture DLL first: scripts/build-capture.ps1')
        self.dll = C.CDLL(str(DLL_PATH), winmode=0x100 | 0x800)
        self.handle = C.c_void_p()
        d = self.dll
        d.mabi_capture_abi_version.restype = C.c_uint32
        d.mabi_capture_packet_size.restype = C.c_uint32
        if d.mabi_capture_abi_version() != 2 or d.mabi_capture_packet_size() != C.sizeof(NativePacket):
            raise OSError('Capture DLL ABI mismatch; rebuild it')
        d.mabi_capture_open.argtypes = [C.POINTER(NativeFlow), C.c_uint32, C.POINTER(C.c_void_p),
                                       C.c_void_p, C.c_uint32]
        d.mabi_capture_open.restype = C.c_int32
        d.mabi_capture_poll.argtypes = [C.c_void_p, C.POINTER(NativePacket)]
        d.mabi_capture_poll.restype = C.c_int32
        for name in ('mabi_capture_loss', 'mabi_capture_fault'):
            getattr(d, name).argtypes = [C.c_void_p]
            getattr(d, name).restype = C.c_uint64
        if hasattr(d,'mabi_capture_oversized'):
            d.mabi_capture_oversized.argtypes=[C.c_void_p]
            d.mabi_capture_oversized.restype=C.c_uint64
        if hasattr(d,'mabi_capture_queue_reports'):
            d.mabi_capture_queue_reports.argtypes=[C.c_void_p,C.POINTER(C.c_uint64),C.c_uint32]
            d.mabi_capture_queue_reports.restype=C.c_int32
        d.mabi_capture_close.argtypes = [C.c_void_p]
        d.mabi_capture_close.restype = None
        if broad_tcp:
            try:
                open_capture=d.mabi_capture_open_tcp_nics
            except AttributeError as exc:
                raise OSError('Capture DLL needs rebuilding: scripts/build-capture.ps1') from exc
            open_capture.argtypes=d.mabi_capture_open.argtypes
            open_capture.restype=C.c_int32
        else:open_capture=d.mabi_capture_open
        flows = native_flows(rows, adapters if adapters is not None else adapter_identities())
        error = C.create_string_buffer(1024)
        if open_capture(flows, len(flows), C.byref(self.handle), error, len(error)):
            raise OSError(error.value.decode('utf-8', errors='replace') or 'Native capture startup failed')

    def poll(self):
        packet = NativePacket()
        result = self.dll.mabi_capture_poll(self.handle, C.byref(packet))
        if result < 0:
            raise OSError('Native capture queue failed')
        return packet if result else None

    def loss(self):
        fault = self.dll.mabi_capture_fault(self.handle)
        if fault:
            raise OSError(f'Packet Monitor stream stopped: Windows error {fault}')
        return self.dll.mabi_capture_loss(self.handle)

    def close(self):
        if self.handle.value:
            self.dll.mabi_capture_close(self.handle)
            self.handle = C.c_void_p()

    def oversized(self):
        function=getattr(self.dll,'mabi_capture_oversized',None)
        return function(self.handle) if function is not None else 0

    def queue_reports(self):
        function=getattr(self.dll,'mabi_capture_queue_reports',None)
        values=(C.c_uint64*2)()
        if function is not None:function(self.handle,values,2)
        return values[0],values[1]

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
