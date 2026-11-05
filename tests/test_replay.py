"""Synthetic protocol/transport regression tests. No original binaries needed."""
from dataclasses import replace
import json
from pathlib import Path
import random
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
from mabi_dps.capture import CaptureIssue, json_segments, pcap_segments, tcp_segment
from mabi_dps import compression as compression_module
from mabi_dps.compression import brotli_decoder
from mabi_dps.protocol import JP_MAP, parse_attack, parse_damage, read_frame
from mabi_dps.replay import Replay, chart_points, demo_replay, load_replay, summarize, write_demo
from mabi_dps.stream import Framer, Segment, TcpStream


def frame(wire=21196, payload=None, compression=0):
    if payload is None:
        payload = bytearray(53)
        struct.pack_into('<I', payload, 0, 101)
        struct.pack_into('<I', payload, 8, 999)
        struct.pack_into('<Q', payload, 16, 1200)
    return struct.pack('<iiB', wire, len(payload), compression) + payload


def segment(seq=1000, payload=b'', timestamp_ns=0, **flags):
    return Segment(timestamp_ns, '192.0.2.1', 23456, '192.0.2.2', 34567,
                   seq % 2**32, payload, **flags)


def ipv4_tcp(payload=b'', *, seq=1000, flags=16, vlan=False):
    tcp = struct.pack('!HHIIBBHHH', 23456, 34567, seq, 0, 5 << 4, flags, 65535, 0, 0) + payload
    ip = struct.pack('!BBHHHBBH4s4s', 0x45, 0, 20 + len(tcp), 0, 0, 64, 6, 0,
                     b'\xc0\x00\x02\x01', b'\xc0\x00\x02\x02') + tcp
    ethernet = b'\x00' * 12 + (b'\x81\x00\x00\x01\x08\x00' if vlan else b'\x08\x00')
    return ethernet + ip


def pcap_bytes(packets, endian='<', nano=False, linktype=1):
    result = struct.pack(endian + 'IHHiIII', 0xA1B23C4D if nano else 0xA1B2C3D4,
                         2, 4, 0, 0, 262144, linktype)
    for index, packet in enumerate(packets):
        result += struct.pack(endian + 'IIII', index + 1, 123, len(packet), len(packet)) + packet
    return result


class ProtocolTests(unittest.TestCase):
    def test_mapping_and_damage_flags(self):
        self.assertEqual(len(JP_MAP), 43)
        payload = bytearray(frame()[9:])
        payload[32], payload[33], payload[35], payload[36] = 1, 6, 8, 16
        payload[4:8] = b'\xff' * 4  # Upper half of wire IDs is deliberately ignored.
        hit = parse_damage(payload)
        self.assertEqual((hit.actor, hit.target, hit.amount), (101, 999, 1200))
        self.assertTrue(all((hit.critical, hit.power, hit.fast, hit.additional, hit.is_dot)))

    def test_invalid_damage_and_zero_damage(self):
        for offset, fmt, value in ((0, '<I', 0), (8, '<I', 101), (16, '<Q', 10**12 + 1)):
            payload = bytearray(frame()[9:])
            struct.pack_into(fmt, payload, offset, value)
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                parse_damage(payload)
        payload = bytearray(frame()[9:])
        struct.pack_into('<Q', payload, 16, 0)
        self.assertEqual(parse_damage(payload).amount, 0)

    def test_attack_fallback_no_damage(self):
        payload = bytearray(35)
        struct.pack_into('<II', payload, 16, 0, 789)
        self.assertEqual(parse_attack(payload).skill_key, 789)
        replay = Replay(assume_aligned=True)
        replay.feed(segment(payload=frame(20526, payload)))
        self.assertFalse(replay.hits)
        self.assertEqual(replay.frames[0]['decoded']['key_source'], 'Key2')

    def test_read_frame_all_partial_boundaries(self):
        data = frame()
        for size in range(len(data)):
            self.assertIsNone(read_frame(data[:size]))
        self.assertEqual(read_frame(data).consumed, len(data))


class StreamTests(unittest.TestCase):
    def make_stream(self, **kwargs):
        issues = []
        return TcpStream(lambda code, detail: issues.append(code), **kwargs), issues

    def test_every_split_retransmission_and_wrap(self):
        data = frame()
        for start in (1000, 2**32 - 25):
            for split in range(1, len(data)):
                with self.subTest(start=start, split=split):
                    stream, issues = self.make_stream()
                    stream.feed(segment(start - 1, syn=True, ack=False))
                    self.assertEqual(stream.feed(segment(start + split, data[split:])), [])
                    frames = stream.feed(segment(start, data[:split]))
                    self.assertEqual(len(frames), 1)
                    self.assertEqual(parse_damage(frames[0].payload).amount, 1200)
                    self.assertEqual(stream.feed(segment(start, data)), [])
                    self.assertIn('retransmission', issues)
                    stream.finish()
                    self.assertNotIn('tcp_gap_at_eof', issues)

    def test_random_segment_orders_preserve_stream(self):
        data = frame() * 20
        for seed in range(20):
            rng = random.Random(seed)
            pieces = [(i, data[i:i + 17]) for i in range(0, len(data), 17)]
            rng.shuffle(pieces)
            stream, _ = self.make_stream()
            stream.feed(segment(999, syn=True, ack=False))
            output = []
            for offset, part in pieces + pieces:
                output.extend(stream.feed(segment(1000 + offset, part)))
            self.assertEqual(len(output), 20)

    def test_overlap_preserves_first_arrival_bytes(self):
        data = frame()
        stream, _ = self.make_stream()
        stream.feed(segment(999, syn=True, ack=False))
        stream.feed(segment(1030, data[30:]))
        corrupted = data[:30] + b'X' * (len(data) - 30)
        output = stream.feed(segment(1000, corrupted))
        self.assertEqual(output[0].payload, data[9:])

    def test_gap_timeout_does_not_join_unrelated_bytes(self):
        stream, issues = self.make_stream()
        stream.feed(segment(999, syn=True, ack=False))
        stream.feed(segment(1000, frame()[:20]))
        stream.feed(segment(1040, frame()[40:], 1))
        self.assertEqual(stream.feed(segment(2000, frame(), 3_000_000_002)), [])
        self.assertIn('tcp_gap_timeout', issues)
        self.assertFalse(stream.framer.aligned)

    def test_pending_and_framer_limits(self):
        stream, issues = self.make_stream()
        stream.feed(segment(999, syn=True, ack=False))
        with patch('mabi_dps.stream.MAX_PENDING', 2):
            for offset in (20, 40, 60):
                stream.feed(segment(1000 + offset, b'x'))
        self.assertIn('tcp_pending_overflow', issues)
        framer = Framer(lambda code, detail: issues.append(code))
        with patch('mabi_dps.stream.MAX_BUFFER', 4):
            framer.feed(b'12345')
        self.assertIn('framing_overflow', issues)

    def test_midstream_damage_needs_anchor(self):
        stream, _ = self.make_stream()
        self.assertEqual(stream.feed(segment(payload=frame())), [])
        # 20132 normalizes to empty anchor 20110.
        output = stream.feed(segment(1062, frame(20132, b'') + frame()))
        self.assertEqual([item.normalized_opcode for item in output], [20110, 21045])

    def test_control_anchor_unknown_and_multiple_frames(self):
        stream, _ = self.make_stream()
        data = b'noise' + b'MMCG' + struct.pack('<i', 3) + b'abc' + frame(29999, b'hello') + frame()
        output = stream.feed(segment(payload=data))
        self.assertEqual(len(output), 3)
        self.assertIsNone(output[0].wire_opcode)
        self.assertEqual(output[1].wire_opcode, 29999)
        self.assertEqual(output[1].payload, b'hello')

    def test_loss_and_unfinished_tail_reported(self):
        stream, issues = self.make_stream(assume_aligned=True)
        stream.feed(segment(payload=frame()[:20]))
        stream.feed(segment(1040, b'gap'))
        stream.finish()
        self.assertIn('tcp_gap_at_eof', issues)
        self.assertIn('frame_tail_at_eof', issues)

    def test_syn_retransmit_and_fin(self):
        stream, _ = self.make_stream()
        stream.feed(segment(999, syn=True, ack=False))
        stream.feed(segment(999, syn=True, ack=False))
        self.assertEqual(len(stream.feed(segment(payload=frame(), fin=True))), 1)
        self.assertTrue(stream.closed)
        self.assertEqual(stream.expected, 1063)

    def test_out_of_order_fin_waits_for_gap_and_reports_loss(self):
        stream, issues = self.make_stream()
        stream.feed(segment(999, syn=True, ack=False))
        stream.feed(segment(1062, fin=True))
        self.assertFalse(stream.closed)
        self.assertEqual(len(stream.feed(segment(payload=frame()))), 1)
        self.assertTrue(stream.closed)
        self.assertEqual(stream.expected, 1063)
        stream.finish()
        self.assertNotIn('tcp_gap_at_eof', issues)
        stream, issues = self.make_stream()
        stream.feed(segment(999, syn=True, ack=False))
        stream.feed(segment(1062, fin=True))
        stream.finish()
        self.assertIn('tcp_gap_at_eof', issues)

    def test_compression_unavailable_preserves_wire_bytes(self):
        replay = Replay(assume_aligned=True)
        replay.feed(segment(payload=frame(payload=b'compressed', compression=1) + frame()))
        replay.finish()
        self.assertTrue(replay.partial)
        self.assertEqual(len(replay.hits), 1)
        self.assertEqual(replay.frames[0]['payload_hex'], b'compressed'.hex())
        self.assertEqual(replay.frames[0]['payload_encoding'], 'wire')

    def test_compression_dispatch_and_bad_frame_boundary(self):
        replay = Replay(assume_aligned=True, decompress=lambda data: frame()[9:])
        replay.feed(segment(payload=frame(payload=b'compressed', compression=1)))
        self.assertEqual(len(replay.hits), 1)
        issues = []
        stream = Framer(lambda code, detail: issues.append(code), aligned=True,
                        decompress=lambda data: (_ for _ in ()).throw(ValueError('bad brotli')))
        frames = stream.feed(frame(payload=b'bad', compression=1) + frame())
        self.assertFalse(frames[0].decoded)
        self.assertTrue(frames[1].decoded)
        self.assertIn('frame_rejected', issues)


class CaptureTests(unittest.TestCase):
    def test_pcap_endianness_timestamps_and_vlan(self):
        for endian in ('<', '>'):
            for nano in (False, True):
                with tempfile.TemporaryDirectory() as directory:
                    path = Path(directory) / 'capture.pcap'
                    path.write_bytes(pcap_bytes([ipv4_tcp(frame(), vlan=True)], endian, nano))
                    items = list(pcap_segments(path, 23456))
                    self.assertEqual(len(items), 1)
                    self.assertEqual(items[0].timestamp_ns, 1_000_000_000 + 123 * (1 if nano else 1000))
                    self.assertEqual(items[0].payload, frame())
                    self.assertEqual(list(pcap_segments(path, 33333)), [])

    def test_ipv4_fragments_and_header_lengths(self):
        packet = bytearray(ipv4_tcp(frame()))
        packet[20:22] = b'\x20\x00'
        with self.assertRaisesRegex(ValueError, 'fragmentation'):
            tcp_segment(packet, 1, 0)
        packet = bytearray(ipv4_tcp(frame()))
        packet[46] = 0x10
        with self.assertRaisesRegex(ValueError, 'TCP header'):
            tcp_segment(packet, 1, 0)

    def test_ipv6_and_extension_header_diagnostic(self):
        tcp = ipv4_tcp(frame())[34:]
        ip = struct.pack('!IHBB16s16s', 6 << 28, len(tcp), 6, 64,
                         b'\0' * 15 + b'\1', b'\0' * 15 + b'\2') + tcp
        self.assertEqual(tcp_segment(ip, 101, 0).payload, frame())
        ext = bytearray(ip)
        ext[6] = 44
        with self.assertRaisesRegex(ValueError, 'extension'):
            tcp_segment(ext, 101, 0)

    def test_bad_pcap_and_truncated_capture_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'capture.pcap'
            path.write_bytes(b'\x0a\x0d\x0d\x0a' * 6)
            with self.assertRaisesRegex(ValueError, 'PCAPNG'):
                list(pcap_segments(path, 23456))
            data = bytearray(pcap_bytes([ipv4_tcp(frame())]))
            struct.pack_into('<I', data, 36, 10000)
            path.write_bytes(data)
            self.assertIsInstance(list(pcap_segments(path, 23456))[0], CaptureIssue)
            path.write_bytes(pcap_bytes([ipv4_tcp(frame())])[:-1])
            with self.assertRaisesRegex(ValueError, 'Truncated'):
                list(pcap_segments(path, 23456))

    def test_json_demo_roundtrip_and_schema_error(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'demo.jsonl'
            write_demo(path)
            self.assertEqual(summarize(load_replay(path).hits)['damage'], 8900)
            rows = path.read_text().splitlines()
            row = json.loads(rows[0])
            row['timestamp_ns'] = 1.2
            path.write_text(json.dumps(row))
            with self.assertRaisesRegex(ValueError, 'line 1'):
                list(json_segments(path))

    def test_full_pcap_replay_with_out_of_order_segments(self):
        data = frame()
        packets = [ipv4_tcp(seq=999, flags=2), ipv4_tcp(data[20:], seq=1020),
                   ipv4_tcp(data[:20]), ipv4_tcp(data), ipv4_tcp(frame(), seq=1062)]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'demo.pcap'
            path.write_bytes(pcap_bytes(packets))
            result = load_replay(path, port=23456)
            self.assertFalse(result.partial)
            self.assertEqual(summarize(result.hits)['damage'], 2400)
            self.assertEqual(result.hits[0].timestamp_ns, 3_000_123_000)


class ReplayTests(unittest.TestCase):
    def test_demo_totals_and_common_denominator(self):
        result = demo_replay()
        summary = summarize(result.hits)
        self.assertFalse(result.partial)
        self.assertEqual((summary['damage'], summary['hits'], summary['duration_seconds']), (8900, 6, 5))
        self.assertEqual(summary['dps'], 1780)
        self.assertEqual([row['damage'] for row in summary['actors']], [5400, 3500])
        self.assertEqual([row['dps'] for row in summary['actors']], [1080, 700])

    def test_selected_target_and_insufficient_duration(self):
        hits = demo_replay().hits
        hits[1] = replace(hits[1], target=888)
        result = summarize(hits, (hits[1].scope, 888))
        self.assertEqual(result['damage'], 900)
        self.assertIsNone(result['dps'])
        result = summarize(hits, (hits[0].scope, 999))
        self.assertEqual(result['duration_seconds'], 5)
        self.assertEqual(result['damage'], 8000)

    def test_chart_five_second_denominator_and_batching(self):
        hits = demo_replay().hits
        points = chart_points(hits)
        self.assertEqual(points[0], (0, None, 240))
        self.assertEqual(points[-1], (5, 1780, (8900 - 1200) / 5))
        points = chart_points([hits[0], replace(hits[1], timestamp_ns=hits[0].timestamp_ns)])
        self.assertEqual(points, [(0, None, 420)])

    def test_connection_and_scene_scopes_and_unknown_ids(self):
        result = Replay(assume_aligned=True)
        data = frame() + frame(20344, b'state') + frame() + frame(29999, b'unknown')
        result.feed(segment(payload=data))
        result.feed(replace(segment(payload=frame()), src='192.0.2.3'))
        self.assertEqual(len(summarize(result.hits)['actors']), 2)
        self.assertEqual(result.hits[0].actor_key, result.hits[1].actor_key)
        self.assertNotEqual(result.hits[0].actor_key, result.hits[2].actor_key)
        self.assertEqual(result.frames[3]['kind'], 'unknown')
        self.assertEqual(result.frames[3]['wire_opcode'], 29999)
        self.assertEqual(result.frames[3]['payload_hex'], b'unknown'.hex())

    def test_reconnect_resets_both_directions(self):
        result = Replay()
        result.feed(segment(999, syn=True, ack=False))
        result.feed(segment(payload=frame()))
        result.feed(segment(8999, timestamp_ns=1, syn=True, ack=False))
        result.feed(segment(9000, frame(), timestamp_ns=2))
        self.assertEqual(len(summarize(result.hits)['actors']), 1)
        self.assertEqual(result.hits[1].scope, 'C1')
        self.assertEqual(result.frames[-1]['scope'], 'C1.2.S0')

    def test_rst_closes_connection_and_preserves_scope_separation(self):
        result = Replay(assume_aligned=True)
        result.feed(segment(payload=frame()))
        result.feed(segment(1062, timestamp_ns=1, rst=True))
        result.feed(segment(5000, timestamp_ns=2, syn=True, ack=False))
        result.feed(segment(5001, frame(), timestamp_ns=3))
        self.assertEqual([hit.scope for hit in result.hits], ['C1', 'C1'])
        self.assertEqual([frame['scope'] for frame in result.frames], ['C1.1.S0', 'C1.2.S0'])

    def test_invalid_payload_does_not_drop_valid_damage(self):
        result = Replay(assume_aligned=True)
        result.feed(segment(payload=frame(payload=b'short') + frame()))
        self.assertEqual(len(result.hits), 1)
        self.assertTrue(result.partial)
        self.assertEqual(result.frames[0]['kind'], 'rejected')

    def test_timestamp_and_resource_limits(self):
        result = Replay(assume_aligned=True)
        result.feed(segment(payload=frame(), timestamp_ns=1))
        with self.assertRaisesRegex(ValueError, 'backwards'):
            result.feed(segment(1062, frame(), timestamp_ns=0))
        with patch('mabi_dps.replay.MAX_FRAMES', 1), self.assertRaisesRegex(ValueError, 'audit limit'):
            result.feed(segment(1062, frame(), timestamp_ns=2))

    def test_audit_serializable_and_finish_idempotent(self):
        result = Replay(assume_aligned=True)
        result.feed(segment(payload=frame()[:20]))
        result.finish()
        result.finish()
        self.assertEqual(result.issues['frame_tail_at_eof'], 1)
        json.dumps(result.export(), allow_nan=False)

    def test_optional_brotli_missing_or_old(self):
        with patch('mabi_dps.compression.importlib.import_module', side_effect=ImportError):
            self.assertIsNone(brotli_decoder())
        class OldBinding:
            class Decompressor:
                def process(self, data):
                    return b''
        with patch('mabi_dps.compression.importlib.import_module', return_value=OldBinding):
            self.assertIsNone(brotli_decoder())

    def test_brotli_adapter_limits_and_completion(self):
        class Binding:
            error = RuntimeError

            class Decompressor:
                def __init__(self):
                    self.done = False

                def process(self, data, *, output_buffer_limit):
                    if data == b'invalid':
                        raise RuntimeError('invalid stream')
                    if data == b'large':
                        return b'x' * output_buffer_limit
                    if data == b'incomplete' or not data:
                        return b''
                    self.done = True
                    return b'decoded'

                def is_finished(self):
                    return self.done

        with patch('mabi_dps.compression.importlib.import_module', return_value=Binding):
            decode = brotli_decoder()
            self.assertEqual(decode(b'valid'), b'decoded')
            with self.assertRaisesRegex(ValueError, 'Incomplete'):
                decode(b'incomplete')
            with self.assertRaisesRegex(ValueError, 'Invalid'):
                decode(b'invalid')
            with patch.object(compression_module, 'MAX_OUTPUT', 20):
                with self.assertRaisesRegex(ValueError, 'exceeds'):
                    decode(b'large')


if __name__ == '__main__':
    unittest.main()
