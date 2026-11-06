"""Capture DLL ABI and Python wrapper tests; no meter logic or live capture."""
import ctypes as C
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
from mabi_dps import native


GUID = bytes(range(1, 17))
MAC = bytes.fromhex('021122334455')
ROW = {'family': 4, 'local_address': '192.0.2.1', 'remote_address': '198.51.100.2',
       'local_port': 23456, 'remote_port': 34567}
ADAPTERS = {ROW['local_address']: {(GUID, MAC)}}


def fake_dll():
    functions = {'abi_version': 2, 'packet_size': 9040, 'open': 0, 'open_tcp_nics': 0,
                 'poll': 0, 'loss': 0, 'fault': 0, 'oversized': 0, 'queue_reports': 0,
                 'close': None}
    dll = SimpleNamespace(**{f'mabi_capture_{name}': Mock(return_value=value)
                             for name, value in functions.items()})

    def opened(flows, count, handle, error, capacity):
        handle._obj.value = 123
        return 0

    dll.mabi_capture_open.side_effect = opened
    dll.mabi_capture_open_tcp_nics.side_effect = opened
    return dll


class NativeFlowTests(unittest.TestCase):
    def test_abi_sizes_and_offsets(self):
        self.assertEqual(C.sizeof(native.NativeFlow), 64)
        self.assertEqual(C.sizeof(native.NativePacket), 9040)
        self.assertEqual(native.NativeFlow.local_port.offset, 36)
        self.assertEqual(native.NativeFlow.adapter_guid.offset, 40)
        self.assertEqual(native.NativePacket.kind.offset, 26)
        self.assertEqual(native.NativePacket.data.offset, 40)

    def test_ipv4_addresses_numeric_ports_and_adapter_identity(self):
        flows = native.native_flows([ROW], ADAPTERS)
        self.assertEqual(len(flows), 1)
        self.assertEqual(flows[0].family, 4)
        self.assertEqual(bytes(flows[0].local), bytes.fromhex('c0000201') + bytes(12))
        self.assertEqual(bytes(flows[0].remote[:4]), bytes.fromhex('c6336402'))
        self.assertEqual((flows[0].local_port, flows[0].remote_port), (23456, 34567))
        self.assertEqual(bytes(flows[0].adapter_guid), GUID)
        self.assertEqual(bytes(flows[0].adapter_mac), MAC + bytes(2))

    def test_ipv6_and_guid_only_identity(self):
        row = dict(ROW, family=6, local_address='2001:db8::1', remote_address='2001:db8::2')
        flow = native.native_flows([row], {row['local_address']: {GUID}})[0]
        self.assertEqual(flow.family, 6)
        self.assertEqual(bytes(flow.local), bytes.fromhex('20010db8000000000000000000000001'))
        self.assertEqual(bytes(flow.remote), bytes.fromhex('20010db8000000000000000000000002'))
        self.assertEqual(bytes(flow.adapter_guid), GUID)
        self.assertEqual(bytes(flow.adapter_mac), bytes(8))

    def test_missing_and_ambiguous_adapters_reject(self):
        for adapters in ({}, {ROW['local_address']: {GUID, bytes(16)}}):
            with self.subTest(adapters=adapters), self.assertRaisesRegex(ValueError, 'unique adapter'):
                native.native_flows([ROW], adapters)

    def test_mixed_families_and_scoped_addresses_reject(self):
        rows = [dict(ROW, family=6), dict(ROW, remote_address='2001:db8::2'),
                dict(ROW, family=6, local_address='fe80::1', remote_address='2001:db8::2')]
        for row in rows:
            with self.subTest(row=row), self.assertRaises(ValueError):
                native.native_flows([row], ADAPTERS)

    def test_flow_count_bounds(self):
        for rows in ([], [ROW] * 17):
            with self.subTest(count=len(rows)), self.assertRaisesRegex(ValueError, '1..16'):
                native.native_flows(rows, ADAPTERS)
        self.assertEqual(len(native.native_flows([ROW] * 16, ADAPTERS)), 16)

    def test_adapter_guid_view_preserves_each_candidate(self):
        with patch.object(native, 'adapter_identities', return_value=ADAPTERS):
            self.assertEqual(native.adapter_guids(), {ROW['local_address']: {GUID}})


class NativeCaptureTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / 'mabi_capture.dll'
        path.touch()
        self.dll = fake_dll()
        self.addCleanup(patch.stopall)
        patch.object(native, 'DLL_PATH', path).start()
        self.load = patch.object(native.C, 'CDLL', return_value=self.dll).start()

    def capture(self, **kwargs):
        return native.NativeCapture([ROW], ADAPTERS, **kwargs)

    def test_missing_dll_fails_without_loading(self):
        native.DLL_PATH.unlink()
        with self.assertRaisesRegex(OSError, 'Build our capture DLL'):
            self.capture()
        self.load.assert_not_called()

    def test_abi_mismatch_fails_before_opening(self):
        for name, value in [('abi_version', 1), ('packet_size', 9000)]:
            function = getattr(self.dll, f'mabi_capture_{name}')
            original = function.return_value
            function.return_value = value
            with self.subTest(name=name), self.assertRaisesRegex(OSError, 'ABI mismatch'):
                self.capture()
            function.return_value = original
        self.dll.mabi_capture_open.assert_not_called()

    def test_exact_open_context_manager_and_idempotent_close(self):
        with self.capture() as capture:
            self.assertEqual(capture.handle.value, 123)
            args = self.dll.mabi_capture_open.call_args.args
            self.assertEqual(args[1], 1)
            self.assertEqual(bytes(args[0][0].adapter_guid), GUID)
            self.dll.mabi_capture_open_tcp_nics.assert_not_called()
        capture.close()
        self.dll.mabi_capture_close.assert_called_once()
        self.assertIsNone(capture.handle.value)

    def test_broad_open_is_explicit_and_requires_export(self):
        with self.capture(broad_tcp=True):
            self.dll.mabi_capture_open_tcp_nics.assert_called_once()
            self.dll.mabi_capture_open.assert_not_called()
        del self.dll.mabi_capture_open_tcp_nics
        with self.assertRaisesRegex(OSError, 'needs rebuilding'):
            self.capture(broad_tcp=True)

    def test_default_adapter_lookup(self):
        with patch.object(native, 'adapter_identities', return_value=ADAPTERS) as adapters:
            with native.NativeCapture([ROW]):
                adapters.assert_called_once_with()

    def test_startup_error_preserves_native_message(self):
        def fail(flows, count, handle, error, capacity):
            error.value = b'No unique Packet Monitor NIC identity'
            return -1

        self.dll.mabi_capture_open.side_effect = fail
        with self.assertRaisesRegex(OSError, 'No unique Packet Monitor NIC identity'):
            self.capture()

    def test_poll_preserves_native_wifi_bytes_and_metadata(self):
        def packet(handle, output):
            p = output._obj
            p.kind = 2
            p.timestamp = 134000000000000000
            p.component = 17
            p.length = 4
            p.missed_read = 3
            p.data[:4] = b'\x88\x02\x00\x00'
            return 1

        with self.capture() as capture:
            self.assertIsNone(capture.poll())
            self.dll.mabi_capture_poll.side_effect = packet
            p = capture.poll()
            self.assertEqual((p.kind, p.component, p.missed_read), (2, 17, 3))
            self.assertEqual(p.timestamp, 134000000000000000)
            self.assertEqual(bytes(p.data[:p.length]), b'\x88\x02\x00\x00')
            self.dll.mabi_capture_poll.side_effect = None
            self.dll.mabi_capture_poll.return_value = -1
            with self.assertRaisesRegex(OSError, 'queue failed'):
                capture.poll()

    def test_loss_and_stream_fault(self):
        with self.capture() as capture:
            self.dll.mabi_capture_loss.return_value = 7
            self.assertEqual(capture.loss(), 7)
            self.dll.mabi_capture_fault.return_value = 13
            with self.assertRaisesRegex(OSError, 'Windows error 13'):
                capture.loss()

    def test_optional_counters_and_context_cleanup_on_exception(self):
        def counters(handle, values, count):
            values[0], values[1] = 11, 22
            return 0

        with self.assertRaisesRegex(RuntimeError, 'caller failed'):
            with self.capture() as capture:
                self.dll.mabi_capture_oversized.return_value = 3
                self.dll.mabi_capture_queue_reports.side_effect = counters
                self.assertEqual(capture.oversized(), 3)
                self.assertEqual(capture.queue_reports(), (11, 22))
                del self.dll.mabi_capture_oversized
                del self.dll.mabi_capture_queue_reports
                self.assertEqual(capture.oversized(), 0)
                self.assertEqual(capture.queue_reports(), (0, 0))
                raise RuntimeError('caller failed')
        self.dll.mabi_capture_close.assert_called_once()


@unittest.skipUnless(os.name == 'nt' and native.DLL_PATH.is_file(), 'Requires built Windows capture DLL')
class BuiltDllTests(unittest.TestCase):
    def test_real_abi_and_invalid_open_without_starting_capture(self):
        dll = C.CDLL(str(native.DLL_PATH), winmode=0x900)
        dll.mabi_capture_abi_version.argtypes = []
        dll.mabi_capture_abi_version.restype = C.c_uint32
        dll.mabi_capture_packet_size.argtypes = []
        dll.mabi_capture_packet_size.restype = C.c_uint32
        self.assertEqual(dll.mabi_capture_abi_version(), 2)
        self.assertEqual(dll.mabi_capture_packet_size(), C.sizeof(native.NativePacket))
        for name in ('mabi_capture_open', 'mabi_capture_open_tcp_nics'):
            function = getattr(dll, name)
            function.argtypes = [C.POINTER(native.NativeFlow), C.c_uint32, C.POINTER(C.c_void_p),
                                 C.c_void_p, C.c_uint32]
            function.restype = C.c_int32
            handle, error = C.c_void_p(), C.create_string_buffer(1024)
            self.assertEqual(function(None, 0, C.byref(handle), error, len(error)), -1)
            self.assertIsNone(handle.value)


if __name__ == '__main__':
    unittest.main()
