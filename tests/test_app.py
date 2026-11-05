"""Target rankings and live row refreshes, including open actor menus."""
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace
import sys
import tkinter as tk
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
from mabi_dps.app import MeterApp
from mabi_dps.live import LiveReplay,apply_live_commands
from mabi_dps.identity import Identity
from mabi_dps.replay import demo_segments


class ActorMenuTests(unittest.TestCase):
    def setUp(self):
        try:
            self.root = tk.Tk()
        except tk.TclError as exc:
            self.skipTest(f'Tk display unavailable: {exc}')
        self.root.withdraw()
        self.addCleanup(self.root.destroy)
        self.app = MeterApp(self.root)
        self.engine = LiveReplay()
        for segment in demo_segments():
            self.engine.feed(segment)
        self.update_snapshot()

    def update_snapshot(self):
        self.app.emit_live('snapshot', self.engine.snapshot(), 'Live test')
        self.app.poll()

    def actor_row(self, actor):
        return next(item for item, key in self.app.actor_rows.items() if key[1] == actor)

    def test_live_refresh_preserves_actor_row_and_selection(self):
        row = self.actor_row(101)
        self.app.table.selection_set(row)
        self.update_snapshot()
        self.assertEqual(self.actor_row(101), row)
        self.assertEqual(self.app.table.selection(), (row,))

    def test_menu_keeps_clicked_actor_through_refresh_and_selection_change(self):
        clicked = self.actor_row(101)
        intended_key = self.app.actor_rows[clicked]
        damage = self.app.metrics['Damage'].get()

        def while_menu_open(*_):
            # A queued capture snapshot arrives while the user is choosing.
            self.update_snapshot()
            self.app.table.selection_set(self.actor_row(202))
            self.app.actor_menu.invoke(0)

        with patch.object(self.app.table, 'identify_row', return_value=clicked), \
                patch.object(self.app.actor_menu, 'tk_popup', side_effect=while_menu_open):
            self.app.show_actor_menu(SimpleNamespace(y=50, x_root=0, y_root=0))

        # A click is a pending command; only the engine acknowledgement marks it.
        self.assertIn('Name pending',self.app.table.item(self.actor_row(101),'values')[1])
        self.assertIn('Name pending', self.app.table.item(self.actor_row(202), 'values')[1])
        stale=self.engine.snapshot()
        replies=apply_live_commands(self.engine,self.app.live_commands,self.app.live_acks.put)
        self.assertEqual(replies[0]['id'],1);self.assertTrue(replies[0]['applied'])
        self.assertEqual(replies[0]['snapshot'].self_key,intended_key)
        self.app.poll()
        self.assertEqual(self.app.table.item(self.actor_row(101), 'values')[1], '<ME> [101]')
        self.app.emit_live('snapshot',stale,'stale snapshot')
        self.app.poll()
        self.assertEqual(self.app.table.item(self.actor_row(101), 'values')[1], '<ME> [101]')
        self.update_snapshot()
        self.assertEqual(self.app.table.item(self.actor_row(101), 'values')[1], '<ME> [101]')
        self.assertEqual(self.app.metrics['Damage'].get(), damage)

    def test_clear_marker_updates_row_and_survives_next_snapshot(self):
        key = self.app.actor_rows[self.actor_row(101)]
        self.engine.set_self(key, evidence='user_selection')
        self.update_snapshot()
        self.app.actor_menu.invoke(1)
        apply_live_commands(self.engine,self.app.live_commands,self.app.live_acks.put)
        self.app.poll()
        self.assertIn('Name pending', self.app.table.item(self.actor_row(101), 'values')[1])
        self.update_snapshot()
        self.assertIsNone(self.app.replay.self_key)

    def test_hp_estimates_are_visible_optional_and_reset_is_acknowledged(self):
        first=self.engine.hits[0]
        source=self.engine.entities.observe(first.scope,303,self.engine.last_timestamp)
        self.engine.add_identity(first.scope,Identity(303,'Test ally','player',0,uid=123))
        self.engine.add_estimate({'timestamp_ns':first.timestamp_ns+1_000_000_000,
            'source':source.key,'target':first.target_key,'amount':500,'context':(1,1),
            'hp_frame':1,'attack_frame':2,'reason':'test','evidence':'HP inference'})
        self.update_snapshot()
        self.assertEqual(self.app.metrics['Damage'].get(),'9,400')
        row=self.app.table.item(self.actor_row(303),'values')
        self.assertEqual(row[6],'\u2014');self.assertEqual(row[7],'500')
        self.app.include_estimates.set(False);self.app.refresh()
        self.assertEqual(self.app.metrics['Damage'].get(),'8,900')
        self.assertNotIn(303,self.displayed_actors())
        self.app.reset_measurement()
        apply_live_commands(self.engine,self.app.live_commands,self.app.live_acks.put)
        self.app.poll()
        self.assertEqual(self.app.metrics['Damage'].get(),'0')
        self.assertFalse(self.engine.finished)
        self.assertEqual(self.engine.snapshot().summary()['damage'],8900)

    def populate_rankings(self):
        seed = self.engine.hits[0]
        # Deliberately evict nearly all raw hits: the table must use full totals.
        self.engine = LiveReplay(hit_limit=4)
        for second in (1, 2):
            for actor in range(1, 21):
                for target, amount in ((9001, actor * 100), (9002, (21 - actor) * 100)):
                    self.engine.add_hit(replace(seed, timestamp_ns=second * 1_000_000_000,
                                               actor=actor, target=target, amount=amount))
        self.update_snapshot()

    def displayed_actors(self):
        return [self.app.actor_rows[item][1] for item in self.app.table.get_children()]

    def select_target(self, actor):
        key = next(key for key in self.app.targets[1:] if key[1] == actor)
        self.app.target.current(self.app.targets.index(key))
        self.app.refresh()

    def test_top_sixteen_are_target_specific_and_keep_full_totals(self):
        self.populate_rankings()
        self.select_target(9001)
        self.assertEqual(self.displayed_actors(), list(range(20, 4, -1)))
        self.assertIn('Showing 16 of 20', self.app.source_status.get())
        self.assertEqual(self.app.metrics['Damage'].get(), '42,000')
        self.assertEqual(self.app.metrics['Hits'].get(), '40')
        first = self.app.table.get_children()[0]
        self.assertEqual(self.app.table.item(first, 'values')[0], '1')
        self.assertEqual(self.app.table.item(first, 'values')[4], '9.5%')
        self.select_target(9002)
        self.assertEqual(self.displayed_actors(), list(range(1, 17)))
        self.assertEqual(self.app.metrics['Damage'].get(), '42,000')
        self.assertEqual(len(self.app.replay.summary(self.app.selected_target())['actors']), 20)
        self.app.target.current(0)
        self.app.refresh()
        self.assertEqual(self.app.metrics['Damage'].get(), '84,000')
        self.assertEqual(len(self.app.table.get_children()), 16)

    def test_live_rank_changes_and_late_names_refresh_selected_target(self):
        self.populate_rankings()
        self.select_target(9001)
        stable_row = self.actor_row(20)
        self.app.table.selection_set(stable_row)
        scope = self.app.selected_target()[0]
        self.engine.add_identity(scope, Identity(1, 'Source one', 'player', 0, uid=123))
        self.engine.add_hit(replace(self.engine.hits[-1], timestamp_ns=3_000_000_000,
                                    actor=1, target=9001, amount=100_000))
        self.update_snapshot()
        self.assertEqual(self.app.selected_target()[1], 9001)
        self.assertEqual(self.displayed_actors(), [1] + list(range(20, 5, -1)))
        self.assertEqual(self.actor_row(20), stable_row)
        self.assertEqual(self.app.table.selection(), (stable_row,))
        self.assertEqual(self.app.table.item(self.actor_row(1), 'values')[1], 'Source one [1]')
        self.assertEqual(self.app.metrics['Damage'].get(), '142,000')


if __name__ == '__main__':
    unittest.main()
