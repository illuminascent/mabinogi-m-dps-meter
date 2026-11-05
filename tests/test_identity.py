"""Synthetic identity fixtures, with no real player names committed."""
from dataclasses import replace
from pathlib import Path
import struct
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
from mabi_dps.identity import Identity, entity_label, parse_jp_npc_name, parse_npc_name, parse_player_name
from mabi_dps.live import LiveReplay
from mabi_dps.protocol import normalize_jp
from mabi_dps.replay import Replay
from mabi_dps.stream import Segment


def appearance(name='検証太郎', actor=101, offset=32, uid=12345):
    encoded = name.encode('utf-16-le')
    data = bytearray(offset + len(encoded) + 70)
    struct.pack_into('<I', data, 0, actor)
    struct.pack_into('<Qii', data, offset, uid, 1, len(encoded))
    data[offset + 16:offset + 16 + len(encoded)] = encoded
    struct.pack_into('<q', data, offset + 16 + len(encoded), 1_790_000_000_000)
    struct.pack_into('<I', data, offset + 49 + len(encoded), 999)
    return bytes(data)


def frame(wire, payload):
    return struct.pack('<iiB', wire, len(payload), 0) + payload


def segment(payload):
    return Segment(0, '192.0.2.1', 12345, '192.0.2.2', 23456, 1000, payload)


class IdentityTests(unittest.TestCase):
    def live_packets(self, *, appearances=False, late=False):
        damage = bytearray(53)
        struct.pack_into('<I', damage, 0, 101)
        struct.pack_into('<I', damage, 8, 999)
        struct.pack_into('<Q', damage, 16, 1200)
        npc = bytearray(40)
        struct.pack_into('<I', npc, 0, 999)
        npc[16:28] = b'\x03\0\0\0' + bytes.fromhex('6A891148') + b'\0' * 4
        names = frame(20054, appearance()) + frame(20517, struct.pack('<Q', 5544) + npc) if appearances else b''
        data = (frame(21196, damage) if late else b'') + names + frame(21196, damage)
        live = LiveReplay()
        live.feed(replace(segment(b''), seq=999, syn=True, ack=False))
        live.feed(segment(data))
        return live

    def test_wire_to_live_snapshot_labels_without_injecting_names(self):
        live = self.live_packets(appearances=True)
        view = live.snapshot()
        hit = view.hits[0]
        self.assertEqual(entity_label(view.actor_names, hit.actor_key), '検証太郎 [101]')
        self.assertEqual(entity_label(view.actor_names, hit.target_key, role='target'), '修練用稻草人 [999]')
        coverage = view.name_coverage()
        self.assertEqual((coverage['actors']['resolved'], coverage['targets']['resolved']), (1, 1))
        self.assertEqual(view.identity_records, {'player_received': 1, 'player_resolved': 1,
                                               'npc_received': 1, 'npc_resolved': 1})
        self.assertEqual(view.summary()['damage'], 1200)

    def test_combat_only_startup_reports_pending_names_not_guessed_labels(self):
        view = self.live_packets().snapshot()
        hit = view.hits[0]
        self.assertEqual(entity_label(view.actor_names, hit.actor_key), 'Name pending (actor 101)')
        self.assertEqual(entity_label(view.actor_names, hit.target_key, role='target'), 'Name pending (target 999)')
        self.assertEqual(view.identity_records, {})
        self.assertEqual(view.name_coverage()['actors']['resolved'], 0)
        self.assertEqual(view.name_coverage()['targets']['resolved'], 0)

    def test_late_names_report_coverage_for_selected_incarnation(self):
        view = self.live_packets(appearances=True, late=True).snapshot()
        old, new = view.hits
        self.assertEqual(view.name_coverage(old.target_key)['actors']['resolved'], 1)
        self.assertEqual(view.name_coverage(new.target_key)['actors']['resolved'], 1)
        self.assertEqual(view.name_coverage()['targets']['total'], 1)
        self.assertEqual(view.name_coverage()['targets']['resolved'], 1)
        self.assertEqual(old.actor_key, new.actor_key)
        self.assertEqual(old.target_key, new.target_key)
        self.assertEqual(len(view.summary()['actors']), 1)
        self.assertIn('検証太郎', entity_label(view.actor_names, new.actor_key))
        self.assertEqual(view.summary()['damage'], 2400)

    def test_other_scope_or_generation_names_do_not_count_as_displayed_names(self):
        live = self.live_packets()
        live.add_identity('other scene', parse_player_name(appearance()))
        view = live.snapshot()
        self.assertEqual(len(view.actor_names), 1)
        self.assertEqual(view.name_coverage()['actors']['resolved'], 0)

    def test_compressed_appearances_can_align_midstream_at_every_split(self):
        try:
            import brotli
        except ImportError:
            self.skipTest('Optional Brotli binding unavailable')
        from mabi_dps.compression import brotli_decoder
        player = appearance()
        npc = bytearray(160)
        struct.pack_into('<I', npc, 0, 999)
        npc[16:28] = b'\x03\0\0\0' + bytes.fromhex('6A891148') + b'\0' * 4
        for wire, payload, actor in ((20054, player, 101), (20517, struct.pack('<Q', 5544) + npc, 999)):
            body = brotli.compress(payload)
            data = b'partial' + struct.pack('<iiB', wire, len(body), 1) + body
            for split in range(1, len(data)):
                replay = Replay(decompress=brotli_decoder())
                replay.feed(segment(data[:split]))
                replay.feed(replace(segment(data[split:]), seq=1000 + split, timestamp_ns=1))
                self.assertEqual(replay.frames[-1]['kind'], 'identity')
                self.assertEqual(replay.frames[-1]['decoded']['actor'], actor)
                self.assertEqual(replay.issues['identity_alignment'], 1)

    def test_identity_header_without_valid_payload_cannot_establish_alignment(self):
        replay = Replay()
        replay.feed(segment(frame(20054, b'\0' * 160)))
        self.assertFalse(replay.actor_names)
        self.assertFalse(replay.frames)
        self.assertFalse(replay.issues['identity_alignment'])

    def test_identity_evidence_survives_regular_audit_eviction(self):
        live = self.live_packets(appearances=True)
        for index in range(1100):
            live.record_frame({'scope': 'C1.1.S0', 'wire_opcode': 29999,
                               'payload_hex': '', 'kind': 'unknown'})
        view = live.snapshot()
        self.assertEqual(len(view.identity_evidence), 2)
        self.assertFalse(any(frame['wire_opcode'] == 20054 for frame in view.frames))
        self.assertEqual(view.identity_records['player_resolved'], 1)
        self.assertEqual(len(view.export()['identity_evidence']), 2)

    def test_player_fields_and_low_uint32_id(self):
        payload = bytearray(appearance())
        payload[4:8] = b'\xff' * 4
        result = parse_player_name(payload)
        self.assertEqual((result.actor, result.name, result.uid, result.job_key), (101, '検証太郎', 12345, 999))
        self.assertEqual(result.source_offset, 32)

    def test_no_name_ambiguity_or_invalid_unicode(self):
        self.assertIsNone(parse_player_name(b'\0' * 200))
        self.assertIsNone(parse_player_name(appearance(' \t ')))
        self.assertIsNone(parse_player_name(appearance('a\nb')))
        self.assertIsNone(parse_player_name(appearance(actor=0)))
        payload = bytearray(appearance())
        payload[48:50] = b'\x00\xd8'  # Unpaired UTF-16 surrogate.
        self.assertIsNone(parse_player_name(payload))
        first = appearance('first')
        second = appearance('second', offset=16)
        self.assertIsNone(parse_player_name(first + second))

    def test_timestamp_length_bounds_and_uid(self):
        for field, fmt, value in ((32, '<Q', 0), (40, '<i', 129), (44, '<i', 3),
                                  (44, '<i', 66), (56, '<q', 0)):
            data = bytearray(appearance())
            struct.pack_into(fmt, data, field, value)
            self.assertIsNone(parse_player_name(data))
        self.assertIsNone(parse_player_name(appearance()[:60]))

    def test_npc_key_is_raw_bytes_not_actor_id(self):
        data = bytearray(40)
        struct.pack_into('<I', data, 0, 999)
        data[16:28] = b'\x03\0\0\0' + bytes.fromhex('6A891148') + b'\0' * 4
        result = parse_npc_name(data, {'6A891148': 'inherited dummy'})
        self.assertEqual((result.actor, result.template_key, result.name), (999, '6A891148', 'inherited dummy'))
        self.assertEqual(result.kind, 'npc_lookup')
        self.assertIsNone(parse_npc_name(data, {'999': 'wrong actor lookup'}))
        data[24] = 1
        self.assertIsNone(parse_npc_name(data, {'6A891148': 'inherited dummy'}))

    def test_npc_conflicting_templates_unresolved(self):
        data = bytearray(48)
        struct.pack_into('<I', data, 0, 999)
        data[8:20] = b'\x03\0\0\0AAAA\0\0\0\0'
        data[24:36] = b'\x03\0\0\0BBBB\0\0\0\0'
        self.assertIsNone(parse_npc_name(data, {'41414141': 'a', '42424242': 'b'}))

    def test_jp_npc_prefix_is_reference_not_runtime_entity(self):
        body = bytearray(40)
        struct.pack_into('<I', body, 0, 2540693)
        body[16:28] = b'\x03\0\0\0' + bytes.fromhex('59FE4755') + b'\0' * 4
        payload = struct.pack('<Q', 5544) + body
        result = parse_jp_npc_name(payload, {'59FE4755': 'dummy'})
        self.assertEqual((result.actor, result.record_reference_id, result.source_offset), (2540693, 5544, 24))
        self.assertEqual(parse_npc_name(payload, {'59FE4755': 'dummy'}).actor, 5544)
        self.assertIsNone(parse_jp_npc_name(payload[:20], {'59FE4755': 'dummy'}))

    def test_unresolved_jp_npc_uses_nested_runtime_id(self):
        live = LiveReplay()
        live.feed(replace(segment(b''), seq=999, syn=True, ack=False))
        payload = struct.pack('<QQ', 5544, 2540707) + b'\0' * 24
        live.feed(segment(frame(20517, payload)))
        self.assertIn(('E1', 2540707), live.entities.active)
        self.assertNotIn(('E1', 5544), live.entities.active)

    def test_scoped_name_records_preserve_raw_frame_and_damage_independence(self):
        replay = Replay(assume_aligned=True)
        replay.feed(segment(frame(20054, appearance())))
        self.assertEqual(replay.actor_names[('C1', 101, 1)].name, '検証太郎')
        self.assertEqual(replay.frames[0]['kind'], 'identity')
        self.assertEqual(replay.frames[0]['wire_opcode'], 20054)
        self.assertFalse(replay.hits)
        self.assertEqual(replay.export()['actor_names'][0]['actor'], 101)

    def test_conflicting_names_fall_back_to_raw_id(self):
        replay = Replay()
        first = parse_player_name(appearance())
        replay.add_identity('C1.1.S0', first)
        replay.add_identity('C1.1.S0', replace(first, name='different'))
        self.assertNotIn(('C1.1.S0', 101, 1), replay.actor_names)
        replay.add_identity('C1.1.S0', first)
        self.assertNotIn(('C1.1.S0', 101, 1), replay.actor_names)
        replay.add_identity('C1.1.S1', first)
        self.assertIn(('C1.1.S1', 101, 1), replay.actor_names)

    def test_live_names_follow_transport_epochs_and_snapshot_export(self):
        replay = LiveReplay()
        identity = parse_player_name(appearance())
        replay.add_identity(replay.identity_scope('C1.1.S0'), identity)
        replay.new_transport('restart')
        self.assertNotIn((replay.identity_scope('C1.1.S0'), 101, 1), replay.actor_names)
        replay.add_identity(replay.identity_scope('C1.1.S0'), replace(identity, name='new'))
        snapshot = replay.snapshot()
        self.assertEqual(len(snapshot.export()['actor_names']), 2)
        self.assertEqual(snapshot.actor_names[('E2', 101, 1)].name, 'new')

    def test_npc_wire_observation_does_not_rewrite_static_map(self):
        self.assertEqual(normalize_jp(20517), 220517)

    def test_self_placeholder_is_separate_from_packet_name_and_counts(self):
        live = self.live_packets()
        key = live.hits[0].actor_key
        live.set_self(key, evidence='user_selection')
        view = live.snapshot()
        self.assertEqual(entity_label(view.actor_names, key, self_key=view.self_key), '<ME> [101]')
        self.assertEqual(view.name_coverage()['actors']['resolved'], 0)
        self.assertFalse(view.actor_names)
        self.assertEqual(view.export()['self_identity']['evidence'], 'user_selection')
        self.assertEqual(view.summary()['damage'], 1200)

    def test_packet_name_wins_and_self_marker_does_not_follow_reused_id(self):
        live = self.live_packets()
        key = live.hits[0].actor_key
        live.set_self(key, evidence='user_selection')
        live.add_identity(key[0], Identity(101, 'Observed name', 'player', 0, uid=123))
        self.assertEqual(entity_label(live.actor_names, key, self_key=key), 'Observed name [101]')
        fresh = live.add_identity(key[0], Identity(101, 'Other actor', 'player', 0, uid=456))
        self.assertNotEqual(fresh.key, key)
        self.assertIn('Name pending', entity_label({}, fresh.key, self_key=key))
        live.new_transport('game restarted')
        self.assertIsNone(live.self_key)


if __name__ == '__main__':
    unittest.main()
