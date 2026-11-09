"""Synthetic competing interpretations and noninterference checks for the audit."""
import copy
import importlib.util
import json
from pathlib import Path
import struct
import sys
import tracemalloc
import unittest

SPEC = importlib.util.spec_from_file_location('hit_count_audit', Path(__file__).parents[1] / 'audit-hit-counts.py')
audit = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = audit
SPEC.loader.exec_module(audit)


def key(actor, generation=1):
    return dict(process=0, binding=1, actor=actor, generation=generation)


def stamp(frame, context=1, continuity=0):
    return dict(process=0, context=context, continuity=continuity, frame=frame, order=frame, capture_ns=frame)


def packet(frame, wire=20526, *, source=101, target=999, context=1,
           continuity=0, generation=1, raw=4, aux=0, component=77, eligible=True):
    if wire == 20526:
        body = struct.pack('<QQII', source, target, component, 0) + bytes(11)
    else:
        body = struct.pack('<QQQQ', source, target, raw, aux) + bytes(21)
    resolved = dict(source=key(source, generation), target=key(target), eligible_at_export=eligible)
    return dict(kind='packet', at=stamp(frame, context, continuity), wire=wire, inbound=True,
                length=len(body), decoded=True, truncated=False, payload=body.hex(),
                resolved_attack=resolved if wire == 20526 else None)


def interval(number=1, frames=(2,), *, source=101, amount=10, explicit=0, generation=1):
    allocation = dict(source=key(source, generation), amount=amount, skill=77, frames=list(frames))
    return dict(kind='interval', id=number, at=stamp(10), target=key(999), evidence=list(frames),
                loss=amount + explicit, explicit=explicit, unassigned=0, allocations=[allocation])


class HitCountAuditTests(unittest.TestCase):
    def test_observation_is_read_only_and_preserves_damage(self):
        observer = audit.Audit()
        events = [packet(2), interval()]
        before = copy.deepcopy(events)
        for event in events:
            observer.observe(event)
        self.assertEqual(events, before)
        self.assertEqual(observer.damage['inferred_hp'], 10)
        self.assertEqual(observer.shapes['single_uncovered_presentation'], 1)

    def test_explicit_and_presentation_are_one_bundle_not_two_hits(self):
        observer = audit.Audit()
        observer.observe(packet(1, 21196))
        observer.observe(packet(2))
        row = observer.observe(interval(explicit=4))['allocations'][0]
        self.assertEqual(row['explicit_overlap_references'], 1)
        self.assertIsNone(row['presentation_hypothesis'])
        self.assertEqual(row['classification'], 'explicit_overlap_only')
        self.assertEqual(observer.damage['inferred_hp'], 10)

    def test_identical_payloads_are_competing_hypotheses_not_deduplicated_truth(self):
        observer = audit.Audit()
        for frame in (2, 3):
            observer.observe(packet(frame))
        row = observer.observe(interval(frames=(2, 3)))['allocations'][0]
        self.assertEqual(row['presentation_hypothesis'], 2)
        self.assertEqual(row['identical_signature_collapse_hypothesis'], 1)
        self.assertIsNone(row['physical_hit_count'])
        self.assertEqual(row['classification'], 'ambiguous_presentation_bundle')

    def test_distinct_components_retain_two_observations_with_unknown_physical_count(self):
        observer = audit.Audit()
        observer.observe(packet(2, component=77))
        observer.observe(packet(3, component=78))
        row = observer.observe(interval(frames=(2, 3)))['allocations'][0]
        self.assertEqual(row['classification'], 'multiple_distinct_presentations')
        self.assertEqual(row['presentation_hypothesis'], 2)
        self.assertIsNone(row['physical_hit_count'])

    def test_fallback_without_current_presentation_keeps_all_damage(self):
        observer = audit.Audit()
        row = observer.observe(interval(frames=()))['allocations'][0]
        self.assertEqual(row['classification'], 'no_current_presentation')
        self.assertIsNone(row['physical_hit_count'])
        self.assertIsNone(row['presentation_hypothesis'])
        self.assertEqual(observer.damage['inferred_hp'], 10)

    def test_scope_incarnation_and_authoritative_eligibility_are_required(self):
        for overrides in (dict(context=2), dict(continuity=1), dict(generation=2), dict(eligible=False)):
            observer = audit.Audit()
            observer.observe(packet(2, **overrides))
            row = observer.observe(interval())['allocations'][0]
            self.assertEqual(row['classification'], 'no_current_presentation')
            self.assertEqual(row['amount'], 10)

    def test_source_or_flag_disagreement_cannot_hide_an_uncovered_presentation(self):
        observer = audit.Audit()
        observer.observe(packet(1, 21196, source=202))
        observer.observe(packet(2))
        row = observer.observe(interval())['allocations'][0]
        self.assertEqual(row['explicit_overlap_references'], 0)
        self.assertEqual(observer.counts['adjacent_explicit_role_or_flag_disagreement'], 1)

    def test_repeated_evidence_references_do_not_add_new_events(self):
        observer = audit.Audit()
        observer.observe(packet(2))
        first = interval(frames=(2, 2))
        first['allocations'].append(copy.deepcopy(first['allocations'][0]))
        first['loss'] = 20
        a = observer.observe(first)
        b = observer.observe(interval(2))
        self.assertEqual(observer.counts['unique_uncovered_presentation_observations'], 1)
        self.assertEqual(a['allocations'][1]['shared_references'], 1)
        self.assertEqual(b['allocations'][0]['reused_references'], 1)
        self.assertEqual(observer.damage['inferred_hp'], 30)

    def test_capacity_loss_is_unknown_count_not_lost_damage(self):
        observer = audit.Audit(capacity=2)
        for frame in (2, 3, 4):
            observer.observe(packet(frame))
        row = observer.observe(interval())['allocations'][0]
        self.assertEqual(len(observer.packets), 2)
        self.assertEqual(observer.counts['packet_cache_evictions'], 1)
        self.assertEqual(row['missing_references'], 1)
        self.assertIsNone(row['physical_hit_count'])
        self.assertEqual(row['amount'], 10)

    def test_zero_impact_and_unsupported_explicit_are_not_positive_hits(self):
        for aux, counter in ((4, 'zero_impact_pairs'), (5, 'unsupported_impact_pairs')):
            observer = audit.Audit()
            observer.observe(packet(1, 21196, raw=4, aux=aux))
            observer.observe(packet(2))
            self.assertEqual(observer.counts[counter], 1)

    def test_sentinel_does_not_cover_a_presentation_but_large_finite_raw_can(self):
        for raw, covered in ((2**64 - 1, False), (10**12 + 1, True)):
            observer = audit.Audit()
            observer.observe(packet(1, 21196, raw=raw))
            observer.observe(packet(2))
            row = observer.observe(interval())['allocations'][0]
            self.assertEqual(row['explicit_overlap_references'], int(covered))
            self.assertEqual(row['amount'], 10)

    def test_old_activity_evidence_does_not_establish_a_new_hit(self):
        observer = audit.Audit()
        observer.observe(packet(2))
        event = interval()
        event['evidence'] = []
        row = observer.observe(event)['allocations'][0]
        self.assertEqual(row['classification'], 'no_current_presentation')
        self.assertEqual(row['disagreements'], 1)

    def test_equal_totals_cannot_hide_changed_actors_or_skills(self):
        baseline = {key: [] for key in ('capture_totals', 'encounters', 'skill_amounts', 'hp_accounting',
                    'attribution_obligations', 'damage_model', 'numeric_observations', 'hits', 'estimated_allocations')}
        baseline['skill_amounts'] = [dict(source=key(101), skill=77, amount='10')]
        changed = copy.deepcopy(baseline)
        changed['skill_amounts'][0]['source'] = key(202)
        self.assertNotEqual(audit.damage_projection(baseline), audit.damage_projection(changed))

    def test_observer_storage_plateaus_while_input_history_keeps_growing(self):
        tracemalloc.start()
        try:
            baseline = tracemalloc.get_traced_memory()[0]
            observer = audit.Audit()
            for frame in range(1, 2 * audit.FRAME_CAP + 1):
                observer.observe(packet(frame))
                observer.observe(interval(frame, frames=(frame,)))
            first_peak = tracemalloc.get_traced_memory()[1]
            for frame in range(2 * audit.FRAME_CAP + 1, 4 * audit.FRAME_CAP + 1):
                observer.observe(packet(frame))
                observer.observe(interval(frame, frames=(frame,)))
            peak = tracemalloc.get_traced_memory()[1]
            self.assertEqual(len(observer.packets), audit.FRAME_CAP)
            self.assertLess(peak - first_peak, 512 * 1024)
            self.assertLess(peak - baseline, 8 * 1024 * 1024)
            print(json.dumps(dict(observer_traced_peak_bytes=peak-baseline,
                                  second_half_peak_growth_bytes=peak-first_peak,
                                  input_presentations=4*audit.FRAME_CAP,
                                  retained_packets=len(observer.packets),
                                  scope='Python observer allocations under synthetic streaming pressure; excludes Replay and final comparison JSON, not process RSS')))
        finally:
            tracemalloc.stop()


if __name__ == '__main__':
    unittest.main()
