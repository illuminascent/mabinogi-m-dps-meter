"""Identity, encounter and inferred-HP behavior in the live Python core."""
from dataclasses import replace
import json
from pathlib import Path
import queue
import struct
import sys
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'src'))
from mabi_dps.appearance import parse_jp_player
from mabi_dps.capture import read_capture
from mabi_dps.capture import CaptureIssue
from mabi_dps.capture_buffer import OwnershipBuffer
from mabi_dps.compression import brotli_decoder
from mabi_dps.health_inference import HealthInference,parse_health
from mabi_dps.identity import Identity
from mabi_dps.live import LiveReplay,apply_live_commands
from mabi_dps.self_tracking import SelfTracker,personal_prefix
from mabi_dps.stream import Segment


def fixture(actor=123,uid=0x1122334455660000,realm=1):
    row=json.loads((ROOT/'native/core/tests/fixtures/jp-appearance-synthetic.json').read_text(encoding='utf-8'))['records'][0]
    data=bytearray.fromhex(row['body_hex']);offset=row['uid_offset']
    struct.pack_into('<I',data,0,actor);struct.pack_into('<QI',data,offset,uid,realm)
    return bytes(data)


def wire(op,payload):return struct.pack('<iiB',op,len(payload),0)+payload
def segment(payload=b'',timestamp_ns=0,seq=1,**flags):
    return Segment(timestamp_ns,'192.0.2.1',1234,'192.0.2.2',2345,seq,payload,**flags)
def damage(actor=123,target=999,amount=100):
    data=bytearray(53);struct.pack_into('<I',data,0,actor);struct.pack_into('<I',data,8,target);struct.pack_into('<Q',data,16,amount)
    return wire(21196,data)


class IdentityExperimentTests(unittest.TestCase):
    def test_full_grammar_server_variants_and_truncated_records(self):
        for realm in (1,2,3,128):self.assertEqual(parse_jp_player(fixture(realm=realm)).realm,realm)
        self.assertIsNone(parse_jp_player(fixture(realm=129)))
        data=fixture()
        for end in range(len(data)):self.assertIsNone(parse_jp_player(data[:end]))

    def test_both_packet_arrival_orders_and_latest_context(self):
        uid=0x1122334455660000
        for first in ('personal','appearance'):
            live=LiveReplay();prefix=uid.to_bytes(8,'little')[:6]+bytes(58)
            bodies=[wire(20590,prefix),wire(20054,fixture())]
            if first=='appearance':bodies.reverse()
            seq=1
            for n,body in enumerate(bodies):live.feed(segment(body,n,seq));seq+=len(body)
            self.assertEqual(live.self_key[1],123)
            self.assertEqual(live.self_tracker.known_uid,uid)
            live.self_tracker.personal_record((2,1),prefix[:6],100,100)
            key=('E1',234,1)
            live.self_tracker.appearance((2,1),key,replace(parse_jp_player(fixture()),actor=234),101,101)
            live.self_tracker.personal_record((1,1),prefix[:6],1,1)
            self.assertEqual(live.self_tracker.binding[0],key)

    def test_manual_reset_keeps_verified_self_on_unrelated_appearance(self):
        t=SelfTracker();identity=parse_jp_player(fixture());key=('E1',123,1)
        t.personal_record((1,1),identity.uid.to_bytes(8,'little')[:6],1,1)
        t.appearance((1,1),key,identity,2,2);t.reset_observations()
        t.appearance((1,1),('E1',234,1),replace(identity,actor=234,uid=0x9999888877770001),3,3)
        self.assertEqual(t.binding[0],key)

    def test_full_uid_not_weakened_by_same_code_prefix(self):
        t=SelfTracker();a=parse_jp_player(fixture());key=('E1',123,1)
        t.personal_record((1,1),a.uid.to_bytes(8,'little')[:6],1,1);t.appearance((1,1),key,a,2,2)
        t.personal_record((2,1),a.uid.to_bytes(8,'little')[:6],3,3)
        t.appearance((2,1),('E1',234,1),replace(a,actor=234,uid=a.uid^(1<<56)),4,4)
        self.assertIsNone(t.binding[0])

    def test_packet_evidence_cache_never_becomes_operator_override(self):
        from mabi_dps.name_cache import save_names,load_names
        live=LiveReplay();live.process_identity=((77,100),)
        identity=parse_jp_player(fixture());ref=live.add_identity('E1',identity,0)
        live.self_tracker.personal_record((1,1),identity.uid.to_bytes(8,'little')[:6],1,1)
        live.self_tracker.appearance((1,1),ref.key,identity,2,2);live.sync_self()
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'names.json';save_names(live,path)
            other=LiveReplay();other.process_identity=live.process_identity;load_names(other,path)
            self.assertIsNone(other.self_key);self.assertIsNone(other.self_tracker.operator_key)


class InferenceTests(unittest.TestCase):
    def test_explicit_and_overkill_suppress_inference(self):
        rows=[];h=HealthInference(rows.append);ctx=(1,1);target=('E1',9,1);source=('E1',1,1)
        h.health(ctx,target,1000,0,1);h.health(ctx,target,900,1,2);h.attack(ctx,source,target,3)
        h.health(ctx,target,800,2,4)
        self.assertEqual([r['amount'] for r in rows],[100])
        self.assertEqual(rows[0]['timestamp_ns'],1)
        h.attack(ctx,source,target,5);h.damage(ctx,target);h.health(ctx,target,0,3,6)
        h.attack(ctx,source,target,7);h.damage(ctx,target);h.flush();h.flush()
        self.assertEqual(len(rows),1)

    def test_ambiguity_heal_and_gap_do_not_invent_damage(self):
        for action in ('duplicate','heal','gap'):
            rows=[];h=HealthInference(rows.append);target=('E1',9,1);source=('E1',1,1)
            h.health((1,1),target,1000,0,1);h.health((1,1),target,900,1,2);h.attack((1,1),source,target,3)
            if action=='duplicate':h.attack((1,1),source,target,4)
            elif action=='heal':h.health((1,1),target,1100,2,4)
            else:h.discard('gap')
            h.flush();self.assertFalse(rows)

    def test_counted_health_layout(self):
        self.assertEqual(parse_health(struct.pack('<QiqB',123,8,1000,1)),(123,1000,1))
        self.assertIsNone(parse_health(struct.pack('<QiqB',123,7,1000,1)))

    def test_global_capture_warning_preserves_contiguous_game_evidence(self):
        live=LiveReplay();target=('E1',9,1)
        live.health_inference.health((1,1),target,1000,0,1)
        live.health_inference.health((1,1),target,900,1,2)
        live.feed(CaptureIssue('capture_oversized_packet','global warning',False))
        self.assertIn(((1,1),target),live.health_inference.targets)
        live.feed(CaptureIssue('confirmed_game_gap','gap',True))
        self.assertFalse(live.health_inference.targets)


class EncounterTests(unittest.TestCase):
    def test_idle_end_capture_continues_and_next_self_hit_starts_fresh(self):
        live=LiveReplay(assume_aligned=True);ref=live.entities.observe('E1',123,0);live.set_self(ref.key,evidence='user_selection')
        first=damage(amount=100);live.feed(segment(first,0));live.tick(15_000_000_000)
        self.assertEqual(live.encounters.current.state,'ended');self.assertFalse(live.finished)
        live.feed(segment(damage(amount=7),20_000_000_000,1+len(first)))
        view=live.snapshot();self.assertEqual(view.summary()['damage'],107)
        self.assertEqual(view.view_summary()['damage'],7)
        live.reset_measurement();self.assertEqual(live.snapshot().view_summary()['damage'],0)
        self.assertEqual(live.self_key,ref.key)

    def test_queue_ack_rejects_stale_key_without_aborting_capture(self):
        live=LiveReplay(assume_aligned=True);ref=live.entities.observe('E1',123,0);commands=queue.SimpleQueue();replies=[]
        commands.put({'id':1,'type':'set_self','key':ref.key})
        apply_live_commands(live,commands,replies.append)
        self.assertTrue(replies[-1]['applied']);self.assertEqual(replies[-1]['snapshot'].self_key,ref.key)
        commands.put({'id':2,'type':'set_self','key':('E1',123,999)})
        apply_live_commands(live,commands,replies.append)
        self.assertFalse(replies[-1]['applied']);self.assertEqual(live.self_key,ref.key)
        live.feed(segment(damage(),1));self.assertEqual(live.snapshot().summary()['damage'],100)

    def test_manual_reset_preserves_all_warmed_identity_and_partitions_only_combat(self):
        live=LiveReplay();self_ref=live.entities.observe('E1',123,0)
        live.set_self(self_ref.key,evidence='user_selection')
        other=live.add_identity('E1',Identity(234,'Previous player','player',0,uid=456),0)
        tracker=live.self_tracker.export()
        entities=dict(live.actor_names)
        live.reset_measurement()
        replacement=live.entities.observe('E1',234,1)
        self.assertEqual(replacement.key,other.key)
        self.assertEqual(live.actor_names,entities)
        self.assertEqual(live.self_tracker.export(),tracker)
        self.assertEqual(live.self_key,self_ref.key)

    def test_reset_keeps_packet_personal_and_teammate_observations(self):
        live=LiveReplay();identity=parse_jp_player(fixture())
        prefix=identity.uid.to_bytes(8,'little')[:6]+bytes(58)
        bodies=[wire(20590,prefix),wire(20054,fixture()),wire(20054,fixture(actor=234,uid=0x8877665544330000)),damage()]
        seq=1
        for n,body in enumerate(bodies):live.feed(segment(body,n,seq));seq+=len(body)
        before=live.self_tracker.export();self_key=live.self_key;names=dict(live.actor_names)
        live.apply_command({'id':1,'type':'reset'})
        self.assertEqual(live.self_tracker.export(),before)
        self.assertEqual(live.actor_names,names);self.assertEqual(live.self_key,self_key)
        self.assertEqual(live.snapshot().view_summary()['damage'],0)
        self.assertEqual(live.encounters.history[-1].state,'ended')
        live.feed(segment(damage(amount=7),10,seq))
        self.assertEqual(live.snapshot().view_summary()['damage'],7)
        self.assertEqual(live.snapshot().summary()['damage'],107)
        self.assertFalse(live.finished)


class OwnershipTests(unittest.TestCase):
    def plan(self,created=100,rows=True):
        return {'processes':[{'pid':77,'created_filetime':created}], 'candidate_flows':
            [{'local_address':'192.0.2.2','local_port':2345,'remote_address':'192.0.2.1','remote_port':1234}] if rows else []}

    def test_discovery_after_first_packet_retains_complete_prelude(self):
        buf=OwnershipBuffer();one=segment(b'prefix',1);two=segment(b'appearance',2,seq=7)
        buf.push(one,{},0.32);buf.approve(self.plan(),0.5);buf.push(two,{},0.6)
        self.assertFalse(list(buf.drain(1.4)))
        self.assertEqual([s.payload for s,m in buf.drain(2)],[b'prefix',b'appearance'])

    def test_unknown_process_change_and_syn_reuse_do_not_export(self):
        buf=OwnershipBuffer();buf.push(segment(b'unowned'),{},0)
        self.assertFalse(list(buf.drain(2)))
        buf.approve(self.plan(),3);buf.push(segment(b'old'),{},3)
        buf.approve(self.plan(created=200),3.5);self.assertFalse(list(buf.drain(5)))
        buf.approve(self.plan(created=200),6);buf.push(segment(b'',syn=True,ack=False),{},6.1)
        buf.push(segment(b'reused',seq=2),{},6.2)
        self.assertFalse(list(buf.drain(8)))

    def test_bytes_and_zero_payload_entry_limits(self):
        buf=OwnershipBuffer(max_bytes=1024,max_packets=2)
        for n in range(20):buf.push(segment(timestamp_ns=n),{},0)
        self.assertEqual(len(buf.pending),2);self.assertLessEqual(buf.bytes,1024);self.assertEqual(buf.lost,18)

    def test_ending_approved_tuple_drains_without_a_restart(self):
        buf=OwnershipBuffer();buf.approve(self.plan(),0);buf.push(segment(b'last game data'),{},0.1)
        buf.approve(self.plan(rows=False),0.5)
        self.assertEqual([s.payload for s,m in buf.drain(1.5)],[b'last game data'])

    def test_same_nic_socket_churn_opens_native_capture_once(self):
        from unittest.mock import patch
        from mabi_dps.live import capture_worker
        one={'family':4,'local_address':'192.0.2.2','local_port':2345,
             'remote_address':'192.0.2.1','remote_port':1234,'pid':77}
        plans=[dict(self.plan(),candidate_flows=[one],identity_verified=True,capabilities={'administrator_token':True}),
               dict(self.plan(),candidate_flows=[one,dict(one,local_port=2346)],identity_verified=True,capabilities={'administrator_token':True})]
        class Pipe:
            messages=[]
            def send_bytes(self,data):self.messages.append(json.loads(data))
            def poll(self):return sum(x['type']=='attachment' for x in self.messages)>=2
            def recv_bytes(self,*args):return b'{"type":"stop"}'
        captures=[]
        class Capture:
            def __init__(self,rows,**kwargs):captures.append(self);self.closed=False
            def loss(self):return 0
            def poll(self):return None
            def close(self):self.closed=True
        clock=[0]
        def now():clock[0]+=0.1;return clock[0]
        with patch('mabi_dps.windows.discover',side_effect=plans), \
             patch('mabi_dps.native.NativeCapture',Capture), \
             patch('mabi_dps.native.adapter_identities',return_value={'192.0.2.2':{(bytes([1])*16,bytes([2])*6)}}), \
             patch('mabi_dps.live.time.monotonic',side_effect=now),patch('mabi_dps.live.time.sleep'):
            capture_worker(Pipe(),30)
        self.assertEqual(len(captures),1);self.assertTrue(captures[0].closed)


class PrivateIntegrationTests(unittest.TestCase):
    @unittest.skipUnless((ROOT/'prototypes/static-analysis/jp-identity-research-long-five-self-joins.jsonl').exists(),'private identity fixture')
    def test_five_actual_self_bindings_without_seed(self):
        live=LiveReplay(decompress=brotli_decoder())
        for row in read_capture(ROOT/'prototypes/static-analysis/jp-identity-research-long-five-self-joins.jsonl'):
            live.feed(row)
        bindings=[e['new'] for e in live.self_tracker.events if e.get('new') is not None]
        self.assertEqual(len(bindings),5);self.assertEqual(live.identity_records['personal_received'],5)
        self.assertIsNotNone(live.self_key);self.assertIsNone(live.self_tracker.operator_key)

    @unittest.skipUnless((ROOT/'prototypes/static-analysis/score2/personal-research-hp-mine.json').exists(),'private HP oracle')
    def test_recorded_hp_positive_controls_and_numeric_conservation(self):
        fixture=ROOT/'prototypes/static-analysis/jp-identity-research-long-focused-tcp.jsonl'
        if not fixture.exists():self.skipTest('private battle fixture')
        live=LiveReplay(decompress=brotli_decoder())
        for row in read_capture(fixture):live.feed(row)
        live.finish()
        oracle=json.loads((ROOT/'prototypes/static-analysis/score2/personal-research-hp-mine.json').read_text(encoding='utf-8'))
        residuals=[x for x in oracle['hp_intervals'] if not x['hits'] and len(x['attacks'])==1]
        expected={}
        for x in residuals:
            key=x['attacks'][0]['actor'],x['actor']
            count,total=expected.get(key,(0,0));expected[key]=count+1,total+x['delta']
        self.assertIn((34,114719),expected.values())
        for (actor,target),(count,total) in expected.items():
            # The control segment is monotonic and complete. Other targets have
            # healing/reset boundaries that the online reducer deliberately drops.
            if (count,total)!=(34,114719):continue
            hits=[h for h in live.estimated_hits if h.actor==actor and h.target==target]
            self.assertEqual((len(hits),sum(h.amount for h in hits)),(count,total))
        self.assertEqual(live.snapshot().summary()['hits'],65)
        self.assertEqual(live.snapshot().summary()['damage'],912305)


if __name__=='__main__':unittest.main()
