"""Offline, bounded observation of hit-count hypotheses; never allocates damage.

Pipe export_cross_reference CAPTURE - NEW_AUDIT.json into this tool with
--events - --audit NEW_AUDIT.json --baseline-audit BASELINE.json --output NEW.json.
Optional --rows NEW.jsonl writes interval annotations, never read as an index.
Packet/notification counts are hypotheses, not established physical hit counts.
"""
import argparse
from collections import Counter, OrderedDict
from dataclasses import dataclass
import json
from pathlib import Path
import struct
import sys


FRAME_CAP = 4096
MAX_PACKET_BYTES = 256
MAX_LINE_BYTES = 1024 * 1024
MAX_INTERVAL_ITEMS = 4096
MAX_ALLOCATIONS = 1024
MAX_EXAMPLES = 12


def entity(key):
    return tuple(key[k] for k in ('process', 'binding', 'actor', 'generation'))


def scope(at):
    return tuple(at[k] for k in ('process', 'context', 'continuity'))


@dataclass(slots=True)
class Packet:
    frame: int
    order: int
    scope: tuple
    wire: int
    body: bytes
    source: tuple | None
    target: tuple | None
    eligible: bool | None
    explicit_overlap: bool = False
    first_interval: int | None = None


class Audit:
    def __init__(self, capacity=FRAME_CAP):
        if not 1 <= capacity <= FRAME_CAP:
            raise ValueError('invalid frame capacity')
        self.capacity = capacity
        self.packets = OrderedDict()
        self.counts = Counter()
        self.shapes = Counter()
        self.damage = Counter()
        self.examples = []
        self.last_frame = 0
        self.last_interval = 0

    def packet(self, row):
        frame = row['at']['frame']
        if frame <= self.last_frame:
            raise ValueError('packet frames must be strictly increasing')
        self.last_frame = frame
        wire = row['wire']
        if not row['inbound'] or wire not in (20526, 21196):
            return
        self.counts['presentation_records' if wire == 20526 else 'explicit_records'] += 1
        expected = 35 if wire == 20526 else 53
        if (not row['decoded'] or row['truncated'] or not row['payload']
                or row['length'] != expected):
            self.counts['opaque_truncated_or_extended_records'] += 1
            return
        body = bytes.fromhex(row['payload'])
        if len(body) != expected or len(body) > MAX_PACKET_BYTES:
            raise ValueError('packet length disagrees with export')
        resolved = row.get('resolved_attack') or {}
        p = Packet(frame, row['at']['order'], scope(row['at']), wire, body,
                   entity(resolved['source']) if 'source' in resolved else None,
                   entity(resolved['target']) if 'target' in resolved else None,
                   resolved.get('eligible_at_export'))
        # Immediate concordant successor is the witnessed pair family. No time
        # window, amount equality or convenient later match selects a witness.
        if wire == 20526:
            previous = self.packets.get(frame - 1)
            if previous and previous.wire == 21196 and previous.scope == p.scope and previous.order + 1 == p.order:
                if previous.body[:16] == body[:16] and previous.body[32:39] == body[24:31]:
                    self.counts['concordant_explicit_presentation_pairs'] += 1
                    raw, aux = struct.unpack_from('<QQ', previous.body, 16)
                    source_prefix, target_prefix = struct.unpack_from('<QQ', body)
                    p.explicit_overlap = (raw != 2**64 - 1 and p.eligible is True and
                                          0 < source_prefix <= 2**32 - 1 and
                                          0 < target_prefix <= 2**32 - 1)
                    if raw >= aux and raw != 2**64 - 1:
                        self.counts['zero_impact_pairs' if raw == aux else 'positive_impact_pairs'] += 1
                    else:
                        self.counts['unsupported_impact_pairs'] += 1
                else:
                    self.counts['adjacent_explicit_role_or_flag_disagreement'] += 1
            if p.source is None or p.target is None:
                self.counts['unresolved_presentation_identity'] += 1
            if p.eligible is False:
                self.counts['forbidden_presentations'] += 1
        if len(self.packets) == self.capacity:
            self.packets.popitem(last=False)
            self.counts['packet_cache_evictions'] += 1
        self.packets[frame] = p
        self.counts['peak_packet_rows'] = max(self.counts['peak_packet_rows'], len(self.packets))

    def interval(self, row):
        # IDs are unique, but resolution order need not be ID order.
        interval_id = row['id']
        if len(row['evidence']) > MAX_INTERVAL_ITEMS or len(row['allocations']) > MAX_ALLOCATIONS:
            raise ValueError('interval exceeds declared exporter bounds')
        self.last_interval = max(self.last_interval, interval_id)
        self.counts['hp_intervals'] += 1
        self.damage['hp_loss'] += row['loss']
        self.damage['matched_explicit_impact'] += row['explicit']
        self.damage['unassigned_hp'] += row['unassigned']
        allocations = row['allocations']
        self.damage['inferred_hp'] += sum(a['amount'] for a in allocations)
        if row['loss'] != row['explicit'] + row['unassigned'] + sum(a['amount'] for a in allocations):
            raise ValueError('exported HP interval does not conserve')
        target = entity(row['target'])
        at_scope = scope(row['at'])
        evidence = set(row['evidence'])
        annotations = []
        for allocation in allocations:
            if len(allocation['frames']) > MAX_INTERVAL_ITEMS:
                raise ValueError('allocation exceeds declared evidence bound')
            source = entity(allocation['source'])
            selected = {}
            missing = wrong_scope = raw_only = ineligible = 0
            seen_frames = set(allocation['frames'])
            self.counts['repeated_frame_references_within_allocation'] += len(allocation['frames']) - len(seen_frames)
            for frame in seen_frames:
                packet = self.packets.get(frame)
                if packet is None:
                    missing += 1
                elif packet.scope != at_scope or packet.source != source or packet.target != target:
                    # Explicit-only frames have no presentation identity; keep
                    # them separate from missing/contested presentation roles.
                    if packet.wire == 21196:
                        raw_only += 1
                    else:
                        wrong_scope += 1
                elif packet.wire != 20526:
                    raw_only += 1
                elif packet.eligible is False:
                    ineligible += 1
                elif packet.eligible is None:
                    wrong_scope += 1
                elif frame not in evidence:
                    # Recent activity can carry old frame provenance. It does
                    # not establish a new hit in this interval.
                    wrong_scope += 1
                else:
                    selected[frame] = packet
            all_presentations = list(selected.values())
            # Match explicit+presentation as one observation bundle. It cannot
            # add a second hit merely because this actor also has HP residual.
            uncovered = [p for p in all_presentations if not p.explicit_overlap]
            signatures = {p.body for p in uncovered}
            overlap = len(all_presentations) - len(uncovered)
            reused = shared = fresh = 0
            for p in uncovered:
                if p.first_interval is None:
                    p.first_interval = interval_id
                    fresh += 1
                elif p.first_interval != interval_id:
                    reused += 1
                else:
                    shared += 1
            self.counts['unique_uncovered_presentation_observations'] += fresh
            self.counts['shared_evidence_references_within_interval'] += shared
            ambiguous = bool(missing or wrong_scope or ineligible or reused or shared or len(signatures) != len(uncovered))
            shape = ('no_current_presentation' if not all_presentations else
                     'explicit_overlap_only' if not uncovered else
                     'ambiguous_presentation_bundle' if ambiguous else
                     'single_uncovered_presentation' if len(uncovered) == 1 else
                     'multiple_distinct_presentations')
            self.shapes[shape] += 1
            self.damage[shape] += allocation['amount']
            self.counts['inferred_allocations'] += 1
            self.counts['explicit_overlap_references'] += overlap
            self.counts['missing_packet_references'] += missing
            self.counts['scope_identity_or_interval_disagreements'] += wrong_scope
            self.counts['forbidden_presentation_references'] += ineligible
            self.counts['cross_interval_frame_reuses_detected'] += reused
            self.counts['presentation_reference_hypothesis_sum'] += len(uncovered)
            self.counts['signature_reference_collapse_hypothesis_sum'] += len(signatures)
            annotation = dict(source=allocation['source'], target=row['target'], amount=allocation['amount'],
                              skill=allocation['skill'], classification=shape,
                              physical_hit_count=None,
                              presentation_hypothesis=len(uncovered) if uncovered else None,
                              identical_signature_collapse_hypothesis=len(signatures) if uncovered else None,
                              presentation_frames=sorted(selected), explicit_overlap_references=overlap,
                              missing_references=missing, disagreements=wrong_scope,
                              forbidden_references=ineligible, reused_references=reused,
                              shared_references=shared, newly_referenced_presentations=fresh,
                              explicit_only_references=raw_only)
            annotations.append(annotation)
            if (len(self.examples) < MAX_EXAMPLES and shape != 'single_uncovered_presentation'
                    and sum(e['classification'] == shape for e in self.examples) < 3):
                self.examples.append(dict(interval=interval_id, hp_frame=row['at']['frame'], **annotation))
        return dict(kind='hit_count_annotation', interval=interval_id, at=row['at'],
                    loss=row['loss'], matched_explicit=row['explicit'], unassigned=row['unassigned'],
                    allocations=annotations)

    def observe(self, row):
        if row['kind'] == 'packet':
            self.packet(row)
        elif row['kind'] == 'interval':
            return self.interval(row)
        return None

    def summary(self):
        return dict(format='offline-hit-count-audit-v1',
                    scope='read-only hypotheses over resolver evidence; no production hit estimate',
                    counts=dict(self.counts), allocation_shapes=dict(self.shapes), damage=dict(self.damage),
                    examples=self.examples,
                    bounds=dict(packet_rows=self.capacity,
                                packet_bytes=MAX_PACKET_BYTES, examples=MAX_EXAMPLES,
                                input_line_bytes=MAX_LINE_BYTES,
                                history='fixed FIFO caches; no disk lookup or unbounded assignment history'),
                    limits=['One presentation need not equal one physical hit.',
                            'Identical payloads can be distinct legitimate hits; collapse is only a competing hypothesis.',
                            'No adjacent explicit record does not establish absence of explicit coverage.',
                            'Per-allocation hypotheses can share evidence and must not be summed as unique hits.',
                            'Counts are not deduced from damage amount, allocation count or actor power.',
                            'Cache eviction and export-time roles can leave observations unresolved.',
                            'Physical multiplicity and critical counts remain unestablished.'])


def damage_projection(audit):
    # Covers all retained actor/target/encounter/skill/origin views and exact
    # unattributed causes. Timing/resource diagnostics are deliberately separate.
    keys = ('capture_totals', 'encounters', 'skill_amounts', 'hp_accounting',
            'attribution_obligations', 'damage_model', 'numeric_observations',
            'hits', 'estimated_allocations')
    return {k: audit[k] for k in keys}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--events', required=True, help='export JSONL path, or - for stdin')
    parser.add_argument('--audit', required=True, type=Path)
    parser.add_argument('--baseline-audit', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--rows', type=Path)
    args = parser.parse_args()
    observer = Audit()
    source = sys.stdin.buffer if args.events == '-' else open(args.events, 'rb')
    rows = args.rows.open('x', encoding='utf-8') if args.rows else None
    try:
        while True:
            line = source.readline(MAX_LINE_BYTES + 1)
            if not line:
                break
            if len(line) > MAX_LINE_BYTES:
                raise ValueError('export line exceeds audit bound')
            annotation = observer.observe(json.loads(line))
            if annotation is not None and rows is not None:
                rows.write(json.dumps(annotation, ensure_ascii=False) + '\n')
    finally:
        if source is not sys.stdin.buffer:
            source.close()
        if rows:
            rows.close()
    audit = json.loads(args.audit.read_text(encoding='utf-8-sig'))
    baseline = json.loads(args.baseline_audit.read_text(encoding='utf-8-sig'))
    if audit['semantic_error'] is not None:
        raise ValueError('export replay has a semantic error')
    result = observer.summary()
    before, after = damage_projection(baseline), damage_projection(audit)
    differences = [key for key in before if before[key] != after[key]]
    result['damage_regression'] = dict(unchanged=not differences, differing_sections=differences,
                                     complete_exported_audit_unchanged=baseline == audit,
                                     baseline=str(args.baseline_audit), observed=str(args.audit))
    result['lifetime_hp_tally_matches'] = (
        observer.damage['hp_loss'] == int(audit['hp_accounting']['loss']) and
        observer.damage['matched_explicit_impact'] == int(audit['hp_accounting']['matched_explicit']) and
        observer.damage['inferred_hp'] == int(audit['hp_accounting']['allocated']) and
        observer.damage['unassigned_hp'] == int(audit['hp_accounting']['unassigned']))
    with args.output.open('x', encoding='utf-8') as output:
        json.dump(result, output, ensure_ascii=False, indent=2)
        output.write('\n')
    if differences or not result['lifetime_hp_tally_matches']:
        raise ValueError('damage regression or incomplete interval observation')
    print(json.dumps({k: result[k] for k in ('counts', 'allocation_shapes', 'damage_regression')}))


if __name__ == '__main__':
    main()
