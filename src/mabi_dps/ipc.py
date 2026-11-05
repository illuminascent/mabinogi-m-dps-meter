"""Private local named pipe + UAC launch for the capture-only worker.

Only JSON byte messages cross the pipe: never unpickle an elevated process's
input. The pipe ACL admits the current user and SYSTEM, rejects remote clients,
and verifies the worker PID before the mutual authentication handshake.
"""
import ctypes as C
import json
from multiprocessing.connection import PipeConnection, answer_challenge, deliver_challenge
import os
from pathlib import Path
import secrets
import subprocess
import sys
import time
import uuid

from .windows import _bind, _dll

MAX_MESSAGE = 256 * 1024


def send_json(connection, message):
    data = json.dumps(message, separators=(',', ':'), allow_nan=False).encode('utf-8')
    if len(data) > MAX_MESSAGE:
        raise ValueError('Capture IPC message exceeds limit')
    connection.send_bytes(data)


def receive_json(connection):
    value = json.loads(connection.recv_bytes(MAX_MESSAGE))
    if not isinstance(value, dict):
        raise ValueError('Invalid capture IPC message')
    return value


def _user_sid():
    kernel, advapi = _dll('kernel32.dll'), _dll('advapi32.dll')
    token = C.c_void_p()
    current = _bind(kernel, 'GetCurrentProcess', C.c_void_p, [])()
    if not _bind(advapi, 'OpenProcessToken', C.c_int,
                 [C.c_void_p, C.c_uint32, C.POINTER(C.c_void_p)])(current, 8, C.byref(token)):
        raise C.WinError(C.get_last_error())
    close = _bind(kernel, 'CloseHandle', C.c_int, [C.c_void_p])
    try:
        function = _bind(advapi, 'GetTokenInformation', C.c_int,
                         [C.c_void_p, C.c_int, C.c_void_p, C.c_uint32, C.POINTER(C.c_uint32)])
        size = C.c_uint32()
        function(token, 1, None, 0, C.byref(size))
        buffer = C.create_string_buffer(size.value)
        if not function(token, 1, buffer, size.value, C.byref(size)):
            raise C.WinError(C.get_last_error())
        sid = C.cast(buffer, C.POINTER(C.c_void_p)).contents
        text = C.c_void_p()
        if not _bind(advapi, 'ConvertSidToStringSidW', C.c_int,
                     [C.c_void_p, C.POINTER(C.c_void_p)])(sid, C.byref(text)):
            raise C.WinError(C.get_last_error())
        try:
            return C.wstring_at(text)
        finally:
            _bind(kernel, 'LocalFree', C.c_void_p, [C.c_void_p])(text)
    finally:
        close(token)


class PrivatePipe:
    def __init__(self):
        self.name = '\\\\.\\pipe\\mabi-dps-' + uuid.uuid4().hex
        self.key = secrets.token_bytes(32)
        self.kernel = _dll('kernel32.dll')
        advapi = _dll('advapi32.dll')
        descriptor = C.c_void_p()
        sddl = f'D:P(A;;GA;;;SY)(A;;GA;;;{_user_sid()})'
        if not _bind(advapi, 'ConvertStringSecurityDescriptorToSecurityDescriptorW', C.c_int,
                     [C.c_wchar_p, C.c_uint32, C.POINTER(C.c_void_p), C.c_void_p])(
                         sddl, 1, C.byref(descriptor), None):
            raise C.WinError(C.get_last_error())

        class Security(C.Structure):
            _fields_ = [('length', C.c_uint32), ('descriptor', C.c_void_p), ('inherit', C.c_int)]

        security = Security(C.sizeof(Security), descriptor, False)
        create = _bind(self.kernel, 'CreateNamedPipeW', C.c_void_p,
                       [C.c_wchar_p, C.c_uint32, C.c_uint32, C.c_uint32,
                        C.c_uint32, C.c_uint32, C.c_uint32, C.c_void_p])
        try:
            self.handle = create(self.name, 3 | 0x40000000 | 0x80000,
                                 4 | 2 | 8, 1, MAX_MESSAGE, MAX_MESSAGE, 0, C.byref(security))
        finally:
            _bind(self.kernel, 'LocalFree', C.c_void_p, [C.c_void_p])(descriptor)
        if self.handle == C.c_void_p(-1).value:
            raise C.WinError(C.get_last_error())

    def accept(self, worker_pid, stop, timeout=90):
        # Reuse CPython's supported Windows overlapped connector rather than
        # implementing its pointer-sized OVERLAPPED structure by hand.
        import _winapi
        overlapped = _winapi.ConnectNamedPipe(self.handle, overlapped=True)
        deadline = time.monotonic() + timeout
        try:
            while _winapi.WaitForSingleObject(overlapped.event, 100) == 258:
                if stop.is_set() or time.monotonic() >= deadline:
                    overlapped.cancel()
                    raise TimeoutError('Capture worker did not connect')
            overlapped.GetOverlappedResult(True)
        except BaseException:
            self.close()
            raise
        actual = C.c_uint32()
        if not _bind(self.kernel, 'GetNamedPipeClientProcessId', C.c_int,
                     [C.c_void_p, C.POINTER(C.c_uint32)])(self.handle, C.byref(actual)) or actual.value != worker_pid:
            self.close()
            raise OSError('Capture worker identity mismatch')
        connection = PipeConnection(self.handle)
        self.handle = None  # connection now owns it
        try:
            deliver_challenge(connection, self.key)
            answer_challenge(connection, self.key)
        except BaseException:
            connection.close()
            raise
        return connection

    def close(self):
        if self.handle is not None:
            _bind(self.kernel, 'CloseHandle', C.c_int, [C.c_void_p])(self.handle)
            self.handle = None


def launch_worker(pipe, seconds):
    class ShellInfo(C.Structure):
        _fields_ = [('size', C.c_uint32), ('mask', C.c_uint32), ('window', C.c_void_p),
                    ('verb', C.c_wchar_p), ('file', C.c_wchar_p), ('parameters', C.c_wchar_p),
                    ('directory', C.c_wchar_p), ('show', C.c_int), ('instance', C.c_void_p),
                    ('id_list', C.c_void_p), ('class_name', C.c_wchar_p), ('class_key', C.c_void_p),
                    ('hotkey', C.c_uint32), ('icon', C.c_void_p), ('process', C.c_void_p)]

    root = Path(__file__).resolve().parents[2]
    arguments = [str(root / 'run_meter.py'), '_capture-worker', '--pipe', pipe.name,
                 '--key', pipe.key.hex(), '--parent-pid', str(os.getpid()), '--seconds', str(seconds)]
    info = ShellInfo()
    info.size, info.mask = C.sizeof(info), 0x40 | 0x100
    # Windows venv python.exe is a redirector that launches a child. Use the
    # base interpreter for this capture-only worker so the ShellExecute PID is
    # the pipe client PID. Brotli stays in the unprivileged venv controller.
    info.verb, info.file = 'runas', getattr(sys, '_base_executable', sys.executable)
    info.parameters = subprocess.list2cmdline(arguments)
    info.directory, info.show = str(root), 0
    if not _bind(_dll('shell32.dll'), 'ShellExecuteExW', C.c_int, [C.POINTER(ShellInfo)])(C.byref(info)):
        raise C.WinError(C.get_last_error())
    kernel = _dll('kernel32.dll')
    try:
        pid = _bind(kernel, 'GetProcessId', C.c_uint32, [C.c_void_p])(info.process)
        if not pid:
            raise C.WinError(C.get_last_error())
        return pid
    finally:
        _bind(kernel, 'CloseHandle', C.c_int, [C.c_void_p])(info.process)


def worker_connection(name, key, parent_pid):
    from multiprocessing.connection import Client
    if not name.startswith('\\\\.\\pipe\\mabi-dps-') or len(key) != 64:
        raise ValueError('Invalid capture pipe parameters')
    connection = Client(name, family='AF_PIPE', authkey=bytes.fromhex(key))
    actual = C.c_uint32()
    if not _bind(_dll('kernel32.dll'), 'GetNamedPipeServerProcessId', C.c_int,
                 [C.c_void_p, C.POINTER(C.c_uint32)])(connection.fileno(), C.byref(actual)) or actual.value != parent_pid:
        connection.close()
        raise OSError('Capture controller identity mismatch')
    return connection
