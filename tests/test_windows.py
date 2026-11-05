"""Native table layouts and capture policy, without loading a capture driver."""
from dataclasses import replace
import ipaddress
from pathlib import Path
import struct
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
from mabi_dps.windows import GameProcess, TcpConnection, bpf_filter, build_plan, parse_tcp_table


def row(**kwargs):
    return replace(TcpConnection(4, '192.0.2.1', 43123, '198.51.100.2', 28765, 5, 77), **kwargs)


class WindowsDiscoveryTests(unittest.TestCase):
    def test_ipv4_table_port_byte_order(self):
        record = (struct.pack('<I4s', 5, b'\xc0\x00\x02\x01') +
                  struct.pack('!H', 43123) + b'\0\0' + b'\xc6\x33\x64\x02' +
                  struct.pack('!H', 28765) + b'\0\0' + struct.pack('<I', 77))
        self.assertEqual(parse_tcp_table(struct.pack('<I', 1) + record, 4), [row()])

    def test_ipv6_table_and_raw_scope_preservation(self):
        local = ipaddress.ip_address('fe80::1').packed
        remote = ipaddress.ip_address('fe80::2').packed
        record = (local + struct.pack('<I', 0x05000000) + struct.pack('!H', 43123) + b'\0\0' +
                  remote + struct.pack('<I', 0x05000000) + struct.pack('!H', 28765) + b'\0\0' +
                  struct.pack('<II', 5, 77))
        connection = parse_tcp_table(struct.pack('<I', 1) + record, 6)[0]
        self.assertEqual(connection.local_address, 'fe80::1')
        self.assertEqual(connection.remote_port, 28765)
        self.assertEqual(connection.local_scope_raw, 0x05000000)

    def test_table_validation(self):
        self.assertEqual(parse_tcp_table(b'\0' * 4, 4), [])
        for data, family in ((b'', 4), (b'\0' * 4, 5), (b'\1\0\0\0', 4),
                             (b'\xff' * 4 + b'\0' * 56, 6)):
            with self.subTest(data=data, family=family), self.assertRaises(ValueError):
                parse_tcp_table(data, family)

    def test_exact_filter_both_directions_without_hardcoded_port(self):
        text = bpf_filter([row()])
        self.assertIn('src host 192.0.2.1 and src port 43123', text)
        self.assertIn('src host 198.51.100.2 and src port 28765', text)
        self.assertNotIn('16000', text)
        self.assertEqual(text, bpf_filter([row(), row()]))

    def test_no_candidates_never_widens_capture(self):
        self.assertIsNone(bpf_filter([]))
        self.assertIsNone(bpf_filter([row(state=8), row(remote_address='127.0.0.1')]))
        plan = build_plan([], [row()], {})
        self.assertFalse(plan['capture_started'])
        self.assertFalse(plan['candidate_flows'])
        self.assertFalse(plan['identity_verified'])
        self.assertIsNone(plan['npcap_bpf'])

    def test_ownership_states_loopback_and_soft_service_preferences(self):
        processes = [GameProcess(77, 'MabinogiMobile.exe', 123456)]
        connections = [row(), row(remote_port=443), row(remote_port=8084), row(pid=88),
                       row(state=8), row(local_address='127.0.0.1'), row(remote_address='0.0.0.0')]
        plan = build_plan(processes, connections, {'pktmon_streaming_exports_present': True})
        self.assertEqual(len(plan['candidate_flows']), 3)
        self.assertEqual(len(plan['excluded_rows']), 3)
        self.assertEqual(plan['owned_tcp_rows'], 6)
        self.assertEqual(plan['candidate_flows'][0]['priority'], 'game_candidate')
        self.assertEqual(plan['preferred_backend'], 'pktmon_streaming')
        self.assertTrue(plan['identity_verified'])
        self.assertFalse(plan['backend_runtime_verified'])
        self.assertTrue(all(not flow['combat_protocol_confirmed'] for flow in plan['candidate_flows']))

    def test_missing_process_identity_and_capture_backend(self):
        processes = [GameProcess(77, 'MabinogiMobile.exe', None, 'access denied')]
        plan = build_plan(processes, [row()], {'npcap_wpcap_present': True})
        self.assertFalse(plan['identity_verified'])
        self.assertEqual(plan['preferred_backend'], 'npcap')
        plan = build_plan(processes, [row()], {'legacy_wpcap_present': True})
        self.assertIsNone(plan['preferred_backend'])


if __name__ == '__main__':
    unittest.main()
