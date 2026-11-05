"""Live/replay parity, bounded history, loss epochs, IPC, and native ABI."""
import ctypes
from dataclasses import asdict
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
from mabi_dps.capture import CaptureIssue, json_segments
from mabi_dps.live import LiveReplay, scope_signature
from mabi_dps.native import DLL_PATH, NativeFlow, NativePacket, adapter_guids, native_flows
from mabi_dps.replay import Replay, demo_replay, demo_segments, summarize


class LiveTests(unittest.TestCase):
    def test_live_replay_numeric_parity_with_bounded_audit(self):
        live = LiveReplay(frame_limit=2, hit_limit=3)
        for segment in demo_segments():
            live.feed(segment)
        view = live.snapshot()
        expected = summarize(demo_replay().hits)
        for field in ('damage', 'hits', 'dps', 'duration_seconds'):
            self.assertEqual(view.summary()[field], expected[field])
        self.assertEqual(len(view.frames), 2)
        self.assertEqual(len(view.hits), 3)
        self.assertEqual(view.frame_count, 6)
        self.assertEqual(view.summary((view.hits[-1].scope, 999))['damage'], 8900)
        self.assertEqual(view.chart_points()[-1][1], 1780)
        json.dumps(view.export(), allow_nan=False)

    def test_snapshot_immutable_when_capture_continues(self):
        live = LiveReplay()
        items = list(demo_segments())
        for segment in items[:4]:
            live.feed(segment)
        view = live.snapshot()
        for segment in items[4:]:
            live.feed(segment)
        self.assertEqual(view.summary()['damage'], 1200)
        self.assertEqual(live.snapshot().summary()['damage'], 8900)

    def test_capture_restart_and_loss_preserve_numeric_history(self):
        live = LiveReplay()
        for segment in demo_segments():
            live.feed(segment)
        old_scope = live.hits[0].scope
        live.new_transport('game restarted')
        # The next transport has a fresh epoch and a fresh capture timeline.
        from dataclasses import replace
        for segment in demo_segments():
            live.feed(replace(segment, timestamp_ns=segment.timestamp_ns + 10_000_000_000))
        view = live.snapshot()
        self.assertEqual(view.summary()['damage'], 17800)
        self.assertNotEqual(old_scope, view.hits[-1].scope)
        self.assertTrue(view.partial)
        live.feed(CaptureIssue('queue_loss', 'one packet', True))
        self.assertFalse(live.streams)

    def test_loss_fixture_roundtrip_invalidates_alignment(self):
        rows = []
        items = list(demo_segments())
        for segment in items[:4]:
            row = asdict(segment)
            row['payload_hex'] = row.pop('payload').hex()
            rows.append(row)
        rows.append({'type': 'loss', 'code': 'queue_loss', 'detail': 'one packet'})
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'capture.jsonl'
            path.write_text('\n'.join(json.dumps(row) for row in rows))
            replay = Replay()
            for item in json_segments(path):
                replay.feed(item)
            self.assertEqual(len(replay.hits), 1)
            self.assertFalse(replay.streams)
            self.assertTrue(replay.partial)

    def test_creation_time_and_endpoint_changes_change_signature(self):
        plan = {'processes': [{'pid': 77, 'created_filetime': 100}], 'candidate_flows': []}
        original = scope_signature(plan)
        plan['processes'][0]['created_filetime'] = 200
        self.assertNotEqual(scope_signature(plan), original)

    def test_service_update_preserves_combat_stream_and_restart_clears_it(self):
        live = LiveReplay()
        items = list(demo_segments())
        first = items[0]
        flow = {'pid': 77, 'family': 4, 'local_address': first.src, 'local_port': first.sport,
                'remote_address': first.dst, 'remote_port': first.dport}
        plan = {'processes': [{'pid': 77, 'created_filetime': 100}], 'candidate_flows': [flow]}
        self.assertTrue(live.attach_plan(plan))
        for segment in items[:4]:
            live.feed(segment)
        stream = next(iter(live.streams.values()))
        plan['candidate_flows'].append(dict(flow, local_port=55555, remote_port=443))
        self.assertFalse(live.attach_plan(plan))
        self.assertIs(next(iter(live.streams.values())), stream)
        plan['processes'][0]['created_filetime'] = 200
        self.assertTrue(live.attach_plan(plan))
        self.assertFalse(live.streams)
        self.assertEqual(live.snapshot().summary()['damage'], 1200)

    def test_worker_rediscovers_restarted_process(self):
        from unittest.mock import patch
        from mabi_dps.live import capture_worker
        flow = {'pid': 77, 'family': 4, 'local_address': '192.0.2.1', 'local_port': 23456,
                'remote_address': '198.51.100.2', 'remote_port': 34567}
        plans = [{'processes': [{'pid': 77, 'created_filetime': created}],
                  'candidate_flows': [flow], 'identity_verified': True,
                  'capabilities': {'administrator_token': True}} for created in (100, 200)]
        class Connection:
            def __init__(self):
                self.messages = []
            def send_bytes(self, data):
                self.messages.append(json.loads(data))
            def poll(self):
                return sum(m['type'] == 'attachment' for m in self.messages) >= 2
            def recv_bytes(self, *_):
                return b'{"type":"stop"}'
        captures = []
        class FakeCapture:
            def __init__(self, rows, **kwargs):
                self.closed = False
                captures.append(self)
            def loss(self):
                return 0
            def poll(self):
                return None
            def close(self):
                self.closed = True
        clock = [0]
        def monotonic():
            clock[0] += 0.5
            return clock[0]
        connection = Connection()
        with patch('mabi_dps.windows.discover', side_effect=plans), \
                patch('mabi_dps.native.adapter_identities', return_value={'192.0.2.1':{(bytes([1])*16,bytes([2])*6)}}), \
                patch('mabi_dps.native.NativeCapture', FakeCapture), \
                patch('mabi_dps.live.time.monotonic', side_effect=monotonic), \
                patch('mabi_dps.live.time.sleep'):
            capture_worker(connection, 30)
        self.assertEqual(len(captures), 2)
        self.assertTrue(all(c.closed for c in captures))
        self.assertEqual(connection.messages[-1]['type'], 'stopped')

    def test_same_game_socket_change_preserves_entity_directory(self):
        from dataclasses import replace
        from mabi_dps.identity import Identity
        live = LiveReplay()
        items = list(demo_segments())
        for item in items:
            live.feed(item)
        old = live.hits[0].actor_key
        initial_hits = len(live.hits)
        live.add_identity(old[0], Identity(101, 'Known actor', 'player', 0, uid=123))
        # A new socket in the same attached game must use its existing names.
        for item in items:
            live.feed(replace(item, sport=item.sport + 1,
                              timestamp_ns=item.timestamp_ns + 10_000_000_000))
        new = list(live.hits)[initial_hits].actor_key
        self.assertEqual(old, new)
        self.assertEqual(live.snapshot().actor_names[new].name, 'Known actor')
        self.assertEqual(len(live.snapshot().summary()['actors']), 2)

    def test_real_brotli_decoder_if_installed(self):
        try:
            import brotli
        except ImportError:
            self.skipTest('Optional Brotli binding not installed in this interpreter')
        from mabi_dps.compression import brotli_decoder
        decode = brotli_decoder()
        self.assertIsNotNone(decode)
        self.assertEqual(decode(brotli.compress(b'newly generated frame bytes' * 1000)),
                         b'newly generated frame bytes' * 1000)
        with self.assertRaises(ValueError):
            decode(brotli.compress(b'frame')[:-1])
        from unittest.mock import patch
        with patch('mabi_dps.compression.MAX_OUTPUT', 32), self.assertRaises(ValueError):
            decode(brotli.compress(b'x' * 1000))

    def test_native_abi_and_filter_validation(self):
        self.assertEqual(ctypes.sizeof(NativePacket), 9040)
        self.assertEqual(ctypes.sizeof(NativeFlow), 64)
        row = {'family': 4, 'local_address': '192.0.2.1', 'remote_address': '198.51.100.2',
               'local_port': 23456, 'remote_port': 34567}
        flows = native_flows([row], {'192.0.2.1': {b'x' * 16}})
        self.assertEqual(bytes(flows[0].local[:4]), b'\xc0\x00\x02\x01')
        self.assertEqual(flows[0].local_port, 23456)
        with self.assertRaises(ValueError):
            native_flows([row], {})
        with self.assertRaises(ValueError):
            native_flows([], {})

    @unittest.skipUnless(os.name == 'nt', 'Windows named pipe')
    def test_private_pipe_json_and_mutual_pid_checks(self):
        from mabi_dps.ipc import PrivatePipe, receive_json, send_json, worker_connection
        pipe = PrivatePipe()
        result = []

        def client():
            try:
                connection = worker_connection(pipe.name, pipe.key.hex(), os.getpid())
                send_json(connection, {'type': 'test', 'payload': 'copy bytes only'})
                result.append(receive_json(connection))
                connection.close()
            except BaseException as exc:
                result.append(exc)

        thread = threading.Thread(target=client, daemon=True)
        thread.start()
        try:
            connection = pipe.accept(os.getpid(), threading.Event(), timeout=5)
            self.assertEqual(receive_json(connection)['type'], 'test')
            send_json(connection, {'type': 'ack'})
            connection.close()
        finally:
            pipe.close()
        thread.join(5)
        self.assertEqual(result, [{'type': 'ack'}])

    @unittest.skipUnless(os.name == 'nt' and DLL_PATH.exists(), 'Local Rust DLL build')
    def test_built_dll_abi_loads_without_capture(self):
        dll = ctypes.CDLL(str(DLL_PATH), winmode=0x900)
        self.assertEqual(dll.mabi_capture_abi_version(), 2)
        self.assertEqual(dll.mabi_capture_packet_size(), ctypes.sizeof(NativePacket))


if __name__ == '__main__':
    unittest.main()
