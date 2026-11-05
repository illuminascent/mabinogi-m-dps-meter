"""Read-only Windows process/connection discovery and capture planning.

Uses trusted Windows DLLs only. Does not initialize a capture driver, create
capture sessions, request elevation, or read game memory. Native functions are
bound lazily so the table parsers and planner can also be tested off Windows.
"""
import ctypes
from dataclasses import asdict, dataclass
import ipaddress
import os
from pathlib import Path
import socket
import struct
import time

ESTABLISHED = 5
ERROR_INSUFFICIENT_BUFFER = 122
ERROR_NO_MORE_FILES = 18
PKTMON_EXPORTS = (
    'PacketMonitorInitialize', 'PacketMonitorUninitialize',
    'PacketMonitorCreateLiveSession', 'PacketMonitorCloseSessionHandle',
    'PacketMonitorEnumDataSources', 'PacketMonitorAddSingleDataSourceToSession',
    'PacketMonitorAddCaptureConstraint', 'PacketMonitorCreateRealtimeStream',
    'PacketMonitorCloseRealtimeStream', 'PacketMonitorAttachOutputToSession',
    'PacketMonitorSetSessionActive',
)


@dataclass(frozen=True)
class GameProcess:
    pid: int
    name: str
    created_filetime: int | None
    identity_error: str | None = None


@dataclass(frozen=True)
class TcpConnection:
    family: int
    local_address: str
    local_port: int
    remote_address: str
    remote_port: int
    state: int
    pid: int
    local_scope_raw: int = 0
    remote_scope_raw: int = 0

    @property
    def eligible(self):
        local, remote = ipaddress.ip_address(self.local_address), ipaddress.ip_address(self.remote_address)
        return (self.state == ESTABLISHED and self.local_port > 0 and self.remote_port > 0
                and not local.is_loopback and not remote.is_loopback
                and not local.is_unspecified and not remote.is_unspecified)

    @property
    def priority(self):
        # Soft preferences inherited from static evidence, never proof of which
        # socket carries combat. Keep service-port tuples in the capture plan.
        return 'service' if self.remote_port <= 1024 or self.remote_port == 8084 else 'game_candidate'

    @property
    def identity(self):
        return (self.pid, self.family, self.local_address, self.local_scope_raw, self.local_port,
                self.remote_address, self.remote_scope_raw, self.remote_port)


def parse_tcp_table(data: bytes, family: int) -> list[TcpConnection]:
    if family not in (4, 6) or len(data) < 4:
        raise ValueError('Invalid TCP table family/header')
    count, = struct.unpack_from('<I', data)
    row_size = 24 if family == 4 else 56
    if count > (len(data) - 4) // row_size:
        raise ValueError('Truncated owner-PID TCP table')
    rows = []
    for index in range(count):
        offset = 4 + index * row_size
        if family == 4:
            state, local, _, remote, _, pid = struct.unpack_from('<I4sI4sII', data, offset)
            local_port = struct.unpack_from('!H', data, offset + 8)[0]
            remote_port = struct.unpack_from('!H', data, offset + 16)[0]
            local_scope, remote_scope = 0, 0
        else:
            local, local_scope, _, remote, remote_scope, _, state, pid = struct.unpack_from(
                '<16sII16sIIII', data, offset)
            local_port = struct.unpack_from('!H', data, offset + 20)[0]
            remote_port = struct.unpack_from('!H', data, offset + 44)[0]
        rows.append(TcpConnection(family, str(ipaddress.ip_address(local)), local_port,
                                  str(ipaddress.ip_address(remote)), remote_port, state, pid,
                                  local_scope, remote_scope))
    return rows


def _dll(name):
    if os.name != 'nt':
        raise OSError('Live process discovery requires native Windows Python')
    # LOAD_LIBRARY_SEARCH_SYSTEM32: never search the original distribution or cwd.
    return ctypes.WinDLL(name, use_last_error=True, winmode=0x800)


def _bind(dll, name, result, arguments):
    function = getattr(dll, name)
    function.restype = result
    function.argtypes = arguments
    return function


def system_directory() -> Path:
    kernel = _dll('kernel32.dll')
    function = _bind(kernel, 'GetSystemDirectoryW', ctypes.c_uint32,
                     [ctypes.c_wchar_p, ctypes.c_uint32])
    buffer = ctypes.create_unicode_buffer(32768)
    length = function(buffer, len(buffer))
    if not length or length >= len(buffer):
        raise ctypes.WinError(ctypes.get_last_error())
    return Path(buffer.value)


def game_processes(name='MabinogiMobile.exe', pid=None):
    kernel = _dll('kernel32.dll')
    u32, handle = ctypes.c_uint32, ctypes.c_void_p

    class ProcessEntry(ctypes.Structure):
        _fields_ = [('size', u32), ('usage', u32), ('pid', u32),
                    ('heap', ctypes.c_size_t), ('module', u32), ('threads', u32),
                    ('parent', u32), ('priority', ctypes.c_int32), ('flags', u32),
                    ('exe', ctypes.c_wchar * 260)]

    snapshot = _bind(kernel, 'CreateToolhelp32Snapshot', handle, [u32, u32])
    first = _bind(kernel, 'Process32FirstW', ctypes.c_int, [handle, ctypes.POINTER(ProcessEntry)])
    next_entry = _bind(kernel, 'Process32NextW', ctypes.c_int, [handle, ctypes.POINTER(ProcessEntry)])
    close = _bind(kernel, 'CloseHandle', ctypes.c_int, [handle])
    open_process = _bind(kernel, 'OpenProcess', handle, [u32, ctypes.c_int, u32])
    get_times = _bind(kernel, 'GetProcessTimes', ctypes.c_int,
                      [handle] + [ctypes.POINTER(ctypes.c_uint64)] * 4)
    current = snapshot(2, 0)  # TH32CS_SNAPPROCESS, not module/heap snapshots.
    if current == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    matches = []
    try:
        entry = ProcessEntry()
        entry.size = ctypes.sizeof(entry)
        available = first(current, ctypes.byref(entry))
        while available:
            if entry.exe.casefold() == name.casefold() and (pid is None or entry.pid == pid):
                created, error = None, None
                process = open_process(0x1000, False, entry.pid)  # QUERY_LIMITED_INFORMATION only
                if process:
                    try:
                        values = [ctypes.c_uint64() for _ in range(4)]
                        if get_times(process, *(ctypes.byref(value) for value in values)):
                            created = values[0].value
                        else:
                            error = str(ctypes.WinError(ctypes.get_last_error()))
                    finally:
                        close(process)
                else:
                    error = str(ctypes.WinError(ctypes.get_last_error()))
                matches.append(GameProcess(entry.pid, entry.exe, created, error))
            available = next_entry(current, ctypes.byref(entry))
        if ctypes.get_last_error() != ERROR_NO_MORE_FILES:
            raise ctypes.WinError(ctypes.get_last_error())
    finally:
        close(current)
    return matches


def tcp_connections(family):
    if family not in (4, 6):
        raise ValueError('Expected IP family 4 or 6')
    library = _dll('iphlpapi.dll')
    function = _bind(library, 'GetExtendedTcpTable', ctypes.c_uint32,
                     [ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint32), ctypes.c_int,
                      ctypes.c_uint32, ctypes.c_int, ctypes.c_uint32])
    size = ctypes.c_uint32(0)
    address_family = 2 if family == 4 else 23
    for _ in range(5):
        if size.value > 16 * 1024 * 1024:
            raise OSError('Windows TCP table exceeded 16 MiB')
        buffer = ctypes.create_string_buffer(size.value) if size.value else None
        result = function(buffer, ctypes.byref(size), False, address_family, 5, 0)
        if result == ERROR_INSUFFICIENT_BUFFER:
            continue
        if result:
            raise ctypes.WinError(result)
        return parse_tcp_table(buffer.raw[:size.value] if buffer is not None else b'\0' * 4, family)
    raise OSError('Windows TCP table kept changing; retry discovery')


def capability_probe():
    directory = system_directory()
    result = {'system_directory': str(directory), 'pktmon_exports': {},
              'pktmon_streaming_exports_present': False, 'capture_tested': False,
              'npcap_wpcap_present': (directory / 'Npcap' / 'wpcap.dll').is_file(),
              'legacy_wpcap_present': (directory / 'wpcap.dll').is_file()}
    shell = _dll('shell32.dll')
    result['administrator_token'] = bool(_bind(shell, 'IsUserAnAdmin', ctypes.c_int, [])())
    try:
        module = _dll('pktmonapi.dll')
    except OSError as exc:
        result['pktmon_load_error'] = str(exc)
    else:
        result['pktmon_exports'] = {name: hasattr(module, name) for name in PKTMON_EXPORTS}
        result['pktmon_streaming_exports_present'] = all(result['pktmon_exports'].values())
    return result


def bpf_filter(connections):
    """Exact endpoint pairs in both directions; never just `tcp port N`."""
    terms = []
    seen = set()
    for row in connections:
        if not row.eligible or row.identity in seen:
            continue
        seen.add(row.identity)
        for source, sport, target, dport in (
                (row.local_address, row.local_port, row.remote_address, row.remote_port),
                (row.remote_address, row.remote_port, row.local_address, row.local_port)):
            terms.append(f'(src host {source} and src port {sport} and '
                         f'dst host {target} and dst port {dport})')
    # An empty candidate set must match nothing, never widen to all TCP.
    return 'tcp and (' + ' or '.join(terms) + ')' if terms else None


def build_plan(processes, connections, capabilities):
    pids = {process.pid for process in processes}
    owned = [row for row in connections if row.pid in pids]
    candidates = sorted((row for row in owned if row.eligible),
                        key=lambda row: (row.priority != 'game_candidate', row.identity))
    flows = [{**asdict(row), 'priority': row.priority, 'combat_protocol_confirmed': False}
             for row in candidates]
    return {'format': 'mabi-dps-capture-plan-v1', 'observed_utc_ns': time.time_ns(),
            'capture_started': False, 'processes': [asdict(process) for process in processes],
            'identity_verified': bool(processes) and all(process.created_filetime is not None for process in processes),
            'owned_tcp_rows': len(owned), 'candidate_flows': flows,
            'excluded_rows': [asdict(row) for row in owned if not row.eligible],
            'capabilities': capabilities,
            'preferred_backend': ('pktmon_streaming' if capabilities.get('pktmon_streaming_exports_present')
                                  else 'npcap' if capabilities.get('npcap_wpcap_present') else None),
            'backend_runtime_verified': False,
            'refresh_seconds': 2, 'npcap_bpf': bpf_filter(candidates),
            'pktmon_plan': {'own_session_only': True, 'network_interface_sources_only': True,
                            'exact_tuple_constraints_both_directions': True,
                            'truncation_size': 9000, 'buffer_size_multiplier': 8,
                            'administrator_required': True}}


def discover(pid=None):
    processes = game_processes(pid=pid)
    connections = tcp_connections(4) + tcp_connections(6)
    # Avoid presenting a stale PID snapshot as a stable identity.
    later = {(process.pid, process.created_filetime) for process in game_processes(pid=pid)}
    processes = [process for process in processes if (process.pid, process.created_filetime) in later]
    return build_plan(processes, connections, capability_probe())
