"""Compare offline export_cross_reference outputs; never train production models.

Usage: python scripts/compare-research.py PREFIX_A PREFIX_B --output NEW_REPORT.json
Each prefix names PREFIX.jsonl and PREFIX-audit.json. Private exports stay ignored.
This strict version requires identical complete ordered attack streams. A shorter
or divergent stream needs a separately reviewed alignment; clocks/amount equality
must not silently select convenient witnesses.
"""
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import statistics
import struct


def read_view(prefix):
    rows = defaultdict(list)
    with Path(str(prefix) + '.jsonl').open(encoding='utf-8') as source:
        for line in source:
            row = json.loads(line)
            rows[row['kind']].append(row)
    audit = json.loads(Path(str(prefix) + '-audit.json').read_text(encoding='utf-8'))
    if audit['semantic_error'] is not None:
        raise ValueError('replay semantic failure')
    packets = {p['at']['frame']: p for p in rows['packet']}
    attacks = [p for p in rows['packet'] if p['wire'] == 20526 and p['inbound']]
    for p in attacks:
        if not p['decoded'] or p['truncated'] or not p['payload']:
            raise ValueError('opaque/truncated attack stream cannot be aligned')
    return dict(rows=rows, audit=audit, packets=packets, attacks=attacks)


def summarize(cases):
    return dict(records=len(cases), raw=sum(c['raw'] for c in cases),
                impact_candidate=sum(c['impact'] for c in cases),
                estimated=sum(c['estimated'] for c in cases),
                exact=sum(c['impact'] == c['estimated'] for c in cases),
                hp_capped_exact=sum(min(c['impact'], c['hp_before']) == c['estimated'] for c in cases),
                error_records=sum(c['impact'] != c['estimated'] for c in cases),
                absolute_error=sum(abs(c['impact'] - c['estimated']) for c in cases))


def compare(prefixes):
    views = [read_view(p) for p in prefixes]
    if not views[0]['attacks'] or [p['payload'] for p in views[0]['attacks']] != [p['payload'] for p in views[1]['attacks']]:
        raise ValueError('complete ordered attack streams differ; do not align by clock or damage equality')
    paired = [{}, {}]
    offsets = []
    for a, b in zip(views[0]['attacks'], views[1]['attacks']):
        paired[0][a['at']['frame']] = b['at']['frame']
        paired[1][b['at']['frame']] = a['at']['frame']
        offsets.append(b['at']['capture_ns'] - a['at']['capture_ns'])
    result = dict(format='mabi-paired-hp-validation-v1',
                  scope='one paired run; observed HP impact and raw amount remain separate; no model training',
                  alignment=dict(method='identical complete ordered 20526 payload streams; amounts not used',
                                 attacks=len(offsets), offset_median_ns=int(statistics.median(offsets)),
                                 offset_min_ns=min(offsets), offset_max_ns=max(offsets)), views=[])
    for i, view in enumerate(views):
        peer = views[1-i]
        allocations = defaultdict(list)
        for interval in peer['rows']['interval']:
            for allocation in interval['allocations']:
                for frame in allocation['frames']:
                    allocations[frame].append((interval, allocation))
        cases, excluded = [], []
        zero, flags, components = Counter(), Counter(), set()
        for p in view['rows']['packet']:
            if p['wire'] != 21196 or not p['inbound']:
                continue
            if not p['decoded'] or p['truncated'] or not p['payload']:
                excluded.append(dict(frame=p['at']['frame'], reason='opaque/truncated explicit'))
                continue
            body = bytes.fromhex(p['payload'])
            if len(body) != 53:
                excluded.append(dict(frame=p['at']['frame'], reason='unsupported explicit length', length=len(body)))
                continue
            source, target, raw, aux = struct.unpack_from('<QQQQ', body)
            evidence = dict(frame=p['at']['frame'], source=source, target=target, raw=raw, auxiliary=aux)
            if raw > 10**12 or not (source & 0xffffffff) or not (target & 0xffffffff) or source == target or aux > raw:
                excluded.append(dict(**evidence, reason='outside finite supported explicit scope'))
                continue
            # Structural immediate successor, selected before examining amount.
            attack = view['packets'].get(p['at']['frame'] + 1)
            if attack is None or attack['wire'] != 20526 or not attack['payload'] or attack['at']['context'] != p['at']['context']:
                excluded.append(dict(**evidence, reason='no immediate presentation'))
                continue
            attack_body = bytes.fromhex(attack['payload'])
            if len(attack_body) < 35 or attack_body[:16] != body[:16] or attack_body[24:31] != body[32:39]:
                excluded.append(dict(**evidence, reason='presentation role/flags disagreement'))
                continue
            flags[body[32:39].hex()] += 1
            components.add((struct.unpack_from('<II', attack_body, 16), body[32:39].hex()))
            peer_frame = paired[i].get(attack['at']['frame'])
            candidates = [(interval, a) for interval, a in allocations[peer_frame]
                          if interval['target']['actor'] == target and a['source']['actor'] == source]
            impact = raw - aux
            if impact == 0:
                zero['records'] += 1
                zero['raw'] += raw
                zero['unexpected_allocated'] += sum(a['amount'] for _, a in candidates)
                continue
            if len(candidates) != 1:
                excluded.append(dict(**evidence, reason='no unique peer allocation', candidates=len(candidates)))
                continue
            interval, allocation = candidates[0]
            if interval['explicit'] or interval['unassigned'] or len(interval['allocations']) != 1 or allocation['frames'] != [peer_frame]:
                excluded.append(dict(**evidence, reason='mixed or coalesced interval; separate allocation study required'))
                continue
            cases.append(dict(**evidence, impact=impact, peer_attack_frame=peer_frame,
                              peer_hp_frame=interval['at']['frame'], peer_interval=interval['id'],
                              hp_before=interval['hp_before'], hp_after=interval['hp_after'],
                              estimated=allocation['amount'], method=allocation['method'],
                              skill=struct.unpack_from('<I', attack_body, 16)[0], flags=body[32:39].hex()))
        result['views'].append(dict(prefix=str(prefixes[i]), all=summarize(cases),
            nonterminal=summarize([c for c in cases if c['hp_after'] > 0]),
            terminal=summarize([c for c in cases if c['hp_after'] == 0]),
            zero_impact=dict(zero), excluded=excluded, cases=cases,
            component_templates=len(components), flag_distribution=dict(flags),
            multiple_candidate_intervals=sum(len(p['candidates']) > 1 for p in peer['rows']['interval'])))
    result['assessment'] = dict(
        nonterminal_error_records=sum(v['nonterminal']['error_records'] for v in result['views']),
        terminal_error_records=sum(v['terminal']['error_records'] for v in result['views']),
        unexplained_error_records=sum(v['all']['records'] - v['all']['hp_capped_exact'] for v in result['views']),
        missing_or_contested_reference_records=sum(
            e['reason'] != 'outside finite supported explicit scope' for v in result['views'] for e in v['excluded']),
        zero_impact_inferred_amount=sum(v['zero_impact'].get('unexpected_allocated', 0) for v in result['views']),
        limits='Only structurally joined finite records are scored. Terminal HP caps raw impact; '
               'zero-impact records are separate. Matching single-source cells cannot validate mixed-source weights.')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('prefix_a', type=Path)
    parser.add_argument('prefix_b', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = compare([args.prefix_a, args.prefix_b])
    with args.output.open('x', encoding='utf-8') as output:
        json.dump(result, output, ensure_ascii=False, indent=2)
        output.write('\n')
    print(json.dumps(dict(alignment=result['alignment'], views=[
        {k: v[k] for k in ['all', 'nonterminal', 'terminal', 'zero_impact', 'multiple_candidate_intervals']}
        for v in result['views']]), ensure_ascii=True))


if __name__ == '__main__':
    main()
