"""Incarnation isolation and history preservation, including ID reuse."""
from dataclasses import replace
from pathlib import Path
import struct
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
from mabi_dps.capture import CaptureIssue
from mabi_dps.identity import Identity
from mabi_dps.lifecycle import EntityRegistry, LifecycleLimitError
from mabi_dps.live import LiveReplay
from mabi_dps.replay import Replay, chart_points, summarize
from mabi_dps.stream import Segment


def name(actor=101, uid=12345, text='first'):
    return Identity(actor, text, 'player', 0, uid=uid)


def damage_frame(amount=100):
    data = bytearray(53)
    struct.pack_into('<I', data, 0, 101)
    struct.pack_into('<I', data, 8, 999)
    struct.pack_into('<Q', data, 16, amount)
    return struct.pack('<iiB', 21196, 53, 0) + data


def segment(time=0, seq=1000, payload=b'', **flags):
    return Segment(time, '192.0.2.1', 12345, '192.0.2.2', 23456, seq, payload, **flags)


class RegistryTests(unittest.TestCase):
    def setUp(self):
        self.reports = []
        self.registry = EntityRegistry(lambda *args: self.reports.append(args))

    def test_repeated_appearance_is_not_spawn(self):
        a = self.registry.appearance('scene', name(), 1)
        b = self.registry.appearance('scene', replace(name(), source_offset=40, job_key=77), 2)
        self.assertEqual(a, b)
        self.assertEqual(len(self.registry.states), 1)

    def test_changed_uid_same_name_does_not_merge_characters(self):
        a = self.registry.appearance('scene', name(), 1)
        self.registry.observe('scene', 101, 2, damage=True)
        b = self.registry.appearance('scene', replace(name(), uid=67890), 3)
        self.assertEqual((a.actor, b.actor, a.generation, b.generation), (101, 101, 1, 2))
        self.assertEqual(self.registry.states[a].retired_ns, 3)
        self.assertEqual(self.registry.names[a.key].uid, 12345)
        self.assertEqual(self.registry.names[b.key].uid, 67890)

    def test_despawn_then_same_uid_spawn_creates_new_life(self):
        a = self.registry.spawn('scene', 101, 1, evidence='confirmed fixture birth 1', identity=name())
        self.registry.despawn('scene', 101, 2, evidence='confirmed fixture removal')
        duplicate = self.registry.despawn('scene', 101, 2, evidence='duplicate removal')
        self.assertEqual(duplicate, a)
        b = self.registry.spawn('scene', 101, 3, evidence='confirmed fixture birth 2', identity=name())
        self.assertEqual(b.generation, 2)
        self.assertIn(a.key, self.registry.names)
        self.assertEqual(self.registry.states[a].retirement_reason, 'despawn: confirmed fixture removal')

    def test_despawn_unknown_actor_leaves_tombstone(self):
        dead = self.registry.despawn('scene', 777, 1, evidence='fixture removal')
        fresh = self.registry.observe('scene', 777, 2)
        self.assertEqual((dead.generation, fresh.generation), (1, 2))

    def test_same_fingerprint_name_conflict_does_not_invent_spawn(self):
        a = self.registry.appearance('scene', name(), 1)
        b = self.registry.appearance('scene', replace(name(), name='other'), 2)
        self.assertEqual(a, b)
        self.assertNotIn(a.key, self.registry.names)
        self.registry.appearance('scene', name(), 3)
        self.assertNotIn(a.key, self.registry.names)
        fresh = self.registry.appearance('scene', replace(name(), uid=67890, name='other'), 4)
        self.assertEqual(fresh.generation, 2)
        self.assertEqual(self.registry.names[fresh.key].name, 'other')

    def test_first_identity_enriches_existing_damage_reference(self):
        old = self.registry.observe('scene', 101, 1, damage=True)
        new = self.registry.appearance('scene', name(), 2)
        self.assertEqual(old, new)
        self.assertIn(new.key, self.registry.names)

    def test_unidentified_attack_placeholder_can_be_enriched(self):
        old = self.registry.observe('scene', 101, 1)
        new = self.registry.appearance('scene', name(), 2)
        self.assertEqual(old, new)

    def test_unreadable_appearance_marks_cached_without_inventing_replacement(self):
        old = self.registry.appearance('scene', name(), 1)
        current = self.registry.unresolved_appearance('scene', 101, 2, 'fixture')
        self.assertEqual(old, current)
        self.assertIn(old.key, self.registry.names)
        self.assertEqual(self.registry.states[current].metadata_status, 'cached')
        repeated = self.registry.unresolved_appearance('scene', 101, 3, 'fixture update')
        self.assertEqual(current, repeated)

    def test_template_change_is_a_conservative_boundary_not_claimed_spawn(self):
        npc = Identity(999, 'form a', 'npc_lookup', 10, template_key='AAAAAAAA')
        a = self.registry.appearance('scene', npc, 1)
        b = self.registry.appearance('scene', replace(npc, name='form b', template_key='BBBBBBBB'), 2)
        self.assertNotEqual(a, b)
        self.assertEqual(self.registry.states[b].birth_reason, 'identity_type_or_template_changed')
        self.assertFalse(any(event['event'] == 'spawn' for event in self.registry.events))

    def test_scene_retirement_preserves_names_but_blocks_old_scope_reuse(self):
        old = self.registry.appearance('old scene', name(), 1)
        self.registry.retire_scope('old scene', 2, 'scene exit')
        self.assertIn(old.key, self.registry.names)
        self.assertNotIn(('old scene', 101), self.registry.active)
        with self.assertRaises(ValueError):
            self.registry.observe('old scene', 101, 3)
        new = self.registry.appearance('new scene', name(uid=67890), 3)
        self.assertNotEqual(old, new)

    def test_explicit_events_require_evidence_and_matching_ids(self):
        with self.assertRaises(ValueError):
            self.registry.spawn('scene', 101, 1, evidence='')
        with self.assertRaises(ValueError):
            self.registry.despawn('scene', 101, 1, evidence='')
        with self.assertRaises(ValueError):
            self.registry.spawn('scene', 999, 1, evidence='fixture', identity=name())

    def test_out_of_order_events_and_limits_fail_explicitly(self):
        self.registry.appearance('scene', name(), 2)
        with self.assertRaises(ValueError):
            self.registry.despawn('scene', 101, 1, evidence='late fixture')
        registry = EntityRegistry(lambda *_: None, limit=1)
        registry.observe('scene', 101, 1)
        with self.assertRaises(LifecycleLimitError):
            registry.observe('scene', 999, 2)


class ReplayLifecycleTests(unittest.TestCase):
    def test_uid_reuse_splits_actor_rows_without_changing_damage(self):
        for factory in (lambda: Replay(assume_aligned=True), LiveReplay):
            replay = factory()
            scope = replay.identity_scope('C1.1.S0')
            replay.add_identity(scope, name(), 0)
            # LiveReplay normally aligns at SYN; use it for both implementations.
            replay.feed(segment(seq=999, syn=True, ack=False))
            replay.feed(segment(payload=damage_frame()))
            replay.add_identity(scope, name(uid=67890, text='second'), 1)
            replay.feed(segment(time=2, seq=1062, payload=damage_frame(300)))
            summary = replay.snapshot().summary() if isinstance(replay, LiveReplay) else summarize(replay.hits)
            self.assertEqual(summary['damage'], 400)
            self.assertEqual({row['actor_generation'] for row in summary['actors']}, {1, 2})
            self.assertEqual(replay.actor_names[(scope, 101, 1)].name, 'first')
            self.assertEqual(replay.actor_names[(scope, 101, 2)].name, 'second')

    def test_target_reuse_splits_filters_and_charts(self):
        replay = LiveReplay()
        replay.feed(segment(seq=999, syn=True, ack=False))
        replay.feed(segment(payload=damage_frame()))
        old = replay.hits[-1].target_key
        scope = old[0]
        replay.entities.despawn(scope, 999, 1, evidence='fixture despawn')
        replay.entities.spawn(scope, 999, 1, evidence='fixture spawn')
        replay.feed(segment(time=2_000_000_000, seq=1062, payload=damage_frame(300)))
        new = replay.hits[-1].target_key
        self.assertNotEqual(old, new)
        self.assertEqual(summarize(replay.hits, old)['damage'], 100)
        self.assertEqual(summarize(replay.hits, new)['damage'], 300)
        view = replay.snapshot()
        self.assertEqual(view.summary(old)['damage'], 100)
        self.assertEqual(view.summary(new)['damage'], 300)
        self.assertEqual(view.chart_points(old)[0][2], 20)
        self.assertEqual(view.chart_points(new)[0][2], 60)
        with self.assertRaises(ValueError):
            summarize(replay.hits, old[:2])
        with self.assertRaises(ValueError):
            view.summary(old[:2])

    def test_loss_preserves_identity_but_marks_metadata_cached(self):
        replay = Replay(assume_aligned=True)
        replay.add_identity(replay.identity_scope('C1.1.S0'), name(), 0)
        replay.feed(segment(payload=damage_frame()))
        old = replay.hits[-1].actor_key
        replay.feed(CaptureIssue('fixture_loss', 'missing data', True))
        self.assertIn(old, replay.actor_names)
        ref = replay.entities.active[old[:2]]
        self.assertIsNone(replay.entities.states[ref].retired_ns)
        self.assertEqual(replay.entities.states[ref].metadata_status, 'cached')

    def test_both_fin_directions_close_transport_without_killing_entities(self):
        replay = Replay(assume_aligned=True)
        replay.feed(segment(seq=999, syn=True, ack=False))
        replay.feed(segment(payload=damage_frame()))
        old = replay.hits[-1].actor_key
        replay.feed(segment(time=1, seq=1062, fin=True))
        self.assertIn(old[:2], replay.entities.active)  # One FIN is half-close.
        reverse = Segment(2, '192.0.2.2', 23456, '192.0.2.1', 12345, 5000, fin=True)
        replay.feed(reverse)
        self.assertIn(old[:2], replay.entities.active)
        self.assertFalse(replay.streams)
        replay.feed(segment(time=3, seq=999, syn=True, ack=False))
        replay.feed(segment(time=4, payload=damage_frame()))
        self.assertEqual(replay.hits[-1].actor_key, old)
        self.assertEqual(len(replay.hits), 2)

    def test_unrecognized_wire_message_does_not_invent_despawn(self):
        replay = Replay(assume_aligned=True)
        replay.feed(segment(payload=damage_frame()))
        old = replay.hits[-1].actor_key
        body = struct.pack('<I', 101)
        unknown = struct.pack('<iiB', 20518, len(body), 0) + body
        replay.feed(segment(time=1, seq=1062, payload=unknown))
        self.assertIn(old[:2], replay.entities.active)
        self.assertEqual(replay.frames[-1]['kind'], 'unknown')

    def test_tcp_loss_keeps_named_entity_and_total_but_records_delivery_gap(self):
        replay = Replay(assume_aligned=True)
        replay.add_identity(replay.identity_scope('C1.1.S0'), name(), 0)
        replay.feed(segment(payload=damage_frame()))
        old = replay.hits[-1].actor_key
        replay.feed(segment(time=1, seq=1080, payload=b'gap'))
        anchor = struct.pack('<iiB', 20132, 0, 0)
        replay.feed(segment(time=4_000_000_002, seq=2000, payload=anchor + damage_frame(300)))
        new = replay.hits[-1].actor_key
        self.assertIn(old, replay.actor_names)
        self.assertEqual(new, old)
        self.assertIn(new, replay.actor_names)
        self.assertEqual(replay.frames[-1]['scope'], 'C1.1.S0.L1')
        self.assertEqual(summarize(replay.hits)['damage'], 400)

    def test_scene_refresh_retains_names_and_late_identity_enriches_old_row(self):
        replay = Replay(assume_aligned=True)
        replay.feed(segment(payload=damage_frame()))
        old = replay.hits[-1].actor_key
        replay.add_identity(old[0], name(), 1)
        marker = struct.pack('<iiB', 20344, 5, 0) + b'state'
        replay.feed(segment(time=2, seq=1062, payload=marker + damage_frame(300)))
        self.assertEqual(old, replay.hits[-1].actor_key)
        self.assertEqual(len(summarize(replay.hits)['actors']), 1)
        self.assertEqual(summarize(replay.hits)['damage'], 400)
        state = replay.entities.states[replay.entities.active[old[:2]]]
        self.assertEqual(state.metadata_status, 'cached')
        replay.add_identity(old[0], name(), 3)
        self.assertEqual(state.metadata_status, 'observed')


if __name__ == '__main__':
    unittest.main()
