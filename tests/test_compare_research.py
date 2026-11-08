"""Adversarial checks for reference selection in the offline paired evaluator."""
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('compare_research', Path(__file__).resolve().parents[1] / 'scripts/compare-research.py')
compare_research = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compare_research)


class PairedReferenceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.prefixes = [Path(self.directory.name) / f'view{i}' for i in range(2)]
        self.rows = []
        for i, receiver in enumerate([101, 202]):
            rows = []
            frame = 0
            for source, raw, aux, before, after in [(101, 10, 0, 100, 90), (202, 5, 5, 90, 90), (202, 100, 0, 90, 0)]:
                def packet(wire, body):
                    nonlocal frame
                    frame += 1
                    return dict(kind='packet', wire=wire, inbound=True, decoded=True, truncated=False,
                                payload=body.hex(), at=dict(frame=frame, context=1, capture_ns=frame*1000+i*100))
                if source == receiver:
                    body = bytearray(53)
                    struct.pack_into('<QQQQ', body, 0, source, 999, raw, aux)
                    body[32] = 7
                    rows.append(packet(21196, body))
                body = bytearray(35)
                struct.pack_into('<QQII', body, 0, source, 999, 7000001, 0)
                body[24] = 7
                attack = packet(20526, body)
                rows.append(attack)
                if source != receiver and before > after:
                    rows.append(dict(kind='interval', id=frame, at=dict(frame=frame), target=dict(actor=999),
                        hp_before=before, hp_after=after, explicit=0, unassigned=0, candidates=[dict(actor=source)],
                        allocations=[dict(source=dict(actor=source), amount=before-after,
                                          frames=[frame], method='single packet-supported actor')]))
            self.rows.append(rows)
        self.save()

    def save(self):
        for prefix, rows in zip(self.prefixes, self.rows):
            Path(str(prefix)+'.jsonl').write_text(''.join(json.dumps(r)+'\n' for r in rows), encoding='utf-8')
            Path(str(prefix)+'-audit.json').write_text(json.dumps(dict(semantic_error=None)), encoding='utf-8')

    def test_separate_terminal_capping_and_zero_impact_from_nonterminal_errors(self):
        result = compare_research.compare(self.prefixes)
        self.assertEqual(result['alignment']['attacks'], 3)
        self.assertEqual(result['assessment']['nonterminal_error_records'], 0)
        self.assertEqual(result['assessment']['terminal_error_records'], 1)
        self.assertEqual(result['assessment']['unexplained_error_records'], 0)
        self.assertEqual(result['views'][1]['zero_impact']['records'], 1)

    def test_bad_amount_is_scored_without_changing_witness_selection(self):
        explicit = next(r for r in self.rows[0] if r['kind']=='packet' and r['wire']==21196)
        body = bytearray.fromhex(explicit['payload'])
        struct.pack_into('<Q', body, 16, 11)
        explicit['payload'] = body.hex()
        self.save()
        result = compare_research.compare(self.prefixes)
        self.assertEqual(result['alignment']['attacks'], 3)
        self.assertEqual(result['assessment']['nonterminal_error_records'], 1)
        self.assertEqual(result['assessment']['unexplained_error_records'], 1)

    def test_divergent_attack_stream_requires_explicit_alignment_work(self):
        attack = next(r for r in self.rows[1] if r['kind']=='packet' and r['wire']==20526)
        body = bytearray.fromhex(attack['payload'])
        body[16] ^= 1
        attack['payload'] = body.hex()
        self.save()
        with self.assertRaisesRegex(ValueError, 'ordered attack streams differ'):
            compare_research.compare(self.prefixes)

    def test_wrong_actor_is_a_missing_reference_and_cannot_be_scored_as_correct(self):
        interval = next(r for r in self.rows[1] if r['kind']=='interval')
        interval['allocations'][0]['source']['actor'] = 303
        self.save()
        result = compare_research.compare(self.prefixes)
        self.assertEqual(result['assessment']['missing_or_contested_reference_records'], 1)
        self.assertEqual(result['views'][0]['all']['records'], 0)


if __name__ == '__main__':
    unittest.main()
