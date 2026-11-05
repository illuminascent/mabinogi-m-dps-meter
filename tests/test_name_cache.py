"""Session matching, metadata reuse and replacement without damage mutation."""
from dataclasses import replace
import json
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
from mabi_dps.identity import Identity
from mabi_dps.live import LiveReplay
from mabi_dps.name_cache import load_names, save_names
from mabi_dps.replay import demo_segments


class CacheTests(unittest.TestCase):
    def engine(self, created=100):
        engine = LiveReplay()
        engine.process_identity = ((77, created),)
        return engine

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name) / 'names.json'
        self.original = self.engine()
        self.identity = Identity(101, 'Observed name', 'player', 32, uid=12345)
        self.original.add_identity('E1', self.identity)
        save_names(self.original, self.path)

    def test_same_process_loads_names_before_combat_without_restoring_totals(self):
        engine = self.engine()
        self.assertEqual(load_names(engine, self.path), 1)
        self.assertEqual(load_names(engine, self.path), 0)
        for segment in demo_segments():
            engine.feed(segment)
        key = engine.hits[0].actor_key
        self.assertEqual(engine.actor_names[key].name, 'Observed name')
        ref = engine.entities.active[key[:2]]
        self.assertEqual(engine.entities.states[ref].metadata_status, 'cached')
        self.assertEqual(engine.snapshot().summary()['damage'], 8900)
        self.assertEqual(engine.snapshot().summary()['hits'], 6)

    def test_same_pid_new_creation_or_missing_identity_rejects_cache(self):
        self.assertEqual(load_names(self.engine(created=200), self.path), 0)
        unknown = LiveReplay()
        self.assertEqual(load_names(unknown, self.path), 0)
        unknown.process_identity = ((77, None),)
        self.assertEqual(load_names(unknown, self.path), 0)

    def test_confirmed_uid_change_replaces_cached_identity(self):
        engine = self.engine()
        load_names(engine, self.path)
        for item in demo_segments():
            engine.feed(item)
        old = engine.hits[0].actor_key
        fresh = engine.add_identity('E1', replace(self.identity, name='Replacement', uid=67890))
        self.assertEqual((old[2], fresh.generation), (1, 2))
        self.assertEqual(engine.actor_names[old].name, 'Observed name')
        save_names(engine, self.path)
        restarted = self.engine()
        self.assertEqual(load_names(restarted, self.path), 1)
        self.assertEqual(restarted.actor_names[('E1', 101, 1)].name, 'Replacement')

    def test_conflicting_or_retired_names_are_not_exported_as_current(self):
        self.original.add_identity('E1', replace(self.identity, name='Conflict'))
        save_names(self.original, self.path)
        self.assertEqual(load_names(self.engine(), self.path), 0)

    def test_invalid_document_rejected_before_partial_import(self):
        data = json.loads(self.path.read_text(encoding='utf-8'))
        data['identities'].append(dict(data['identities'][0], actor=0))
        self.path.write_text(json.dumps(data), encoding='utf-8')
        engine = self.engine()
        with self.assertRaises(ValueError):
            load_names(engine, self.path)
        self.assertFalse(engine.actor_names)

    def test_self_assignment_survives_meter_restart_without_fabricated_name(self):
        ref = self.original.entities.observe('E1', 202, 0)
        self.original.set_self(ref.key, evidence='user_selection')
        save_names(self.original, self.path)
        engine = self.engine()
        load_names(engine, self.path)
        self.assertEqual(engine.self_key, ('E1', 202, 1))
        self.assertNotIn(engine.self_key, engine.actor_names)
        restarted = self.engine(created=200)
        load_names(restarted, self.path)
        self.assertIsNone(restarted.self_key)

    def test_retired_self_assignment_is_not_reassigned_to_new_occupant(self):
        ref = self.original.entities.active[('E1', 101)]
        self.original.set_self(ref.key, evidence='user_selection')
        self.original.add_identity('E1', replace(self.identity, uid=98765, name='Other'))
        save_names(self.original, self.path)
        engine = self.engine()
        load_names(engine, self.path)
        self.assertIsNone(engine.self_key)

    def test_clear_self_assignment_is_persisted(self):
        ref = self.original.entities.active[('E1', 101)]
        self.original.set_self(ref.key, evidence='user_selection')
        self.original.set_self(None, evidence='user_selection')
        save_names(self.original, self.path)
        engine = self.engine()
        load_names(engine, self.path)
        self.assertIsNone(engine.self_key)


if __name__ == '__main__':
    unittest.main()
