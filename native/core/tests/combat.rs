use mabi_meter_core::{combat::{self,Stamp,Timeline,Combat,Attack,Execution,Effect},identity::{EntityKey,Registry,ownership::{self,Spawn},tracking::{Evidence,EvidenceOrigin}},meter::{Meter,Hit,Action,Command},protocol::{self,Damage},replay::Replay,capture_file::Input,transport::{Flow,Segment}};
fn key(actor:u32)->EntityKey{EntityKey{process:1,binding:1,actor,generation:1}}
fn at(frame:u64)->Stamp{Stamp{process:1,continuity:0,context:1,frame,order:frame,timestamp_ns:frame*1_000_000,tcp_byte:frame*60}}
fn ev()->Evidence{Evidence{frame:1,timestamp_ns:1,origin:EvidenceOrigin::Packet{wire:20051}}}
fn put32(p:&mut [u8],at:usize,n:u32){p[at..at+4].copy_from_slice(&n.to_le_bytes());}
fn put64(p:&mut [u8],at:usize,n:u64){p[at..at+8].copy_from_slice(&n.to_le_bytes());}
fn pet()->Vec<u8>{
    let mut p=vec![0;1410];put64(&mut p,0,12);put32(&mut p,8,0x52cb67e4);put32(&mut p,12,1);put64(&mut p,572,12);put32(&mut p,580,2048);
    put32(&mut p,622,2);p[626..628].copy_from_slice(&[65,0]);let start=628+29+24;
    for pos in [start,start+95]{put32(&mut p,pos+63,1f32.to_bits());}
    let tail=start+190+417;p.truncate(tail+110);put64(&mut p,tail,10);put32(&mut p,tail+8,4);put32(&mut p,tail+24,0xf0000001);p
}
#[test] fn pet_profile_consumes_exact_body_and_rejects_truncation_counts_and_forged_tail(){
    let p=pet();let s=ownership::parse(20051,&p).unwrap();assert_eq!((s.actor,s.owner,s.template),(12,10,0xf0000001));
    let mut nested=55u64.to_le_bytes().to_vec();nested.extend(&p);assert_eq!(ownership::parse(20517,&nested).unwrap().outer,Some(55));
    for n in 0..p.len(){assert!(ownership::parse(20051,&p[..n]).is_none());}
    let mut bad=p.clone();put32(&mut bad,681+18,u32::MAX);assert!(ownership::parse(20051,&bad).is_none());
    let mut bad=p.clone();bad.push(0);assert!(ownership::parse(20051,&bad).is_none());
    let mut bad=p;put64(&mut bad,572,13);assert!(ownership::parse(20051,&bad).is_none());
}
#[test] fn pet_before_owner_split_conflict_reuse_and_owner_conservation(){
    let mut m=Meter::default();let spawn=Spawn{actor:12,owner:10,template:99,outer:None,kind:4};
    let pet=m.registry.owned_spawn(1,1,spawn.clone(),ev()).unwrap();let owner=m.registry.observe(1,1,10).unwrap();
    m.registry.appearance(1,1,mabi_meter_core::identity::Appearance{actor:10,name:"Owner".into(),uid:Some(123),template:None,offset:0,record_reference:None}).unwrap();
    m.registry.assign(Some(owner)).unwrap();let target=m.registry.observe(1,1,20).unwrap();
    for (source,amount) in [(pet,7),(owner,11)]{let h=Hit{timestamp_ns:1,source,target,damage:Damage{actor:source.actor,target:20,amount,flags:[false;5],raw_flags:[0;7]}};m.add(h.clone()).unwrap();m.attribute_skill(&h,1,Some(9999999),false).unwrap();}
    let s=m.snapshot();assert_eq!(s.rows.len(),1);assert_eq!((s.rows[0].damage,s.rows[0].pet_damage),(18,7));assert!(s.rows[0].is_self);assert_eq!(s.rows[0].skills[0].explicit,18);assert_eq!(m.raw_hits[0].source,pet);
    m.command(Command{id:1,observed_revision:m.revision,encounter:1,action:Action::Reset});assert_eq!(m.registry.owner(pet),Some(owner));
    let mut conflicting=spawn.clone();conflicting.owner=11;m.registry.owned_spawn(1,1,conflicting,ev()).unwrap();assert_eq!(m.registry.owner(pet),None);
    m.registry.retire(pet).unwrap();let newer=m.registry.owned_spawn(1,1,spawn,ev()).unwrap();assert_ne!(newer,pet);assert_eq!(m.registry.owner(newer),Some(owner));assert_eq!(m.registry.owner(pet),None);
    let mut fresh=Registry::default();let cache=mabi_meter_core::identity::cache::export(&m.registry);mabi_meter_core::identity::cache::restore(&mut fresh,&cache).unwrap();assert!(fresh.owners.links.is_empty());assert!(fresh.records.is_empty());
}
#[test] fn clocks_unwrap_reset_separate_contexts_and_bound_without_fabricating_hp_time(){
    let mut t=Timeline::default();let mut p=vec![0;24];put64(&mut p,0,1);put32(&mut p,8,u32::MAX-15);t.observe(170066,&p,at(0)).unwrap();t.event(at(1));
    put32(&mut p,8,16);let mut next=at(2);next.timestamp_ns=500_000_000;t.observe(170066,&p,next).unwrap();
    assert_eq!(t.samples[1].ticks,Some(u32::MAX as u64+17));assert_eq!(t.samples[1].epoch,0);assert!(t.bounds[&1].upper.is_some());
    t.event(next);assert!(t.same_epoch(at(1),next));
    put32(&mut p,8,1);t.observe(170066,&p,at(3)).unwrap();assert_eq!(t.samples.back().unwrap().epoch,1);
    let mut other=at(4);other.context=2;t.observe(170066,&p,other).unwrap();assert_eq!(t.samples.back().unwrap().epoch,0);assert!(!t.compatible(at(1),other));
    t.clear_context(1);put32(&mut p,8,50);t.observe(170066,&p,at(5)).unwrap();assert_eq!(t.samples.back().unwrap().ticks,Some(50));
}
#[test] fn action_crosswalk_unknown_keys_and_effect_snapshots_are_not_damage(){
    let mut c=Combat::default();let source=key(1);let target=key(2);let at=at(1);
    c.execution(Execution{source,skill:987654321,action:Some("JP_Internal_Name".into()),variant:Some([0;29]),at,confirmed:false}).unwrap();
    c.state(source,987654321,Stamp{frame:3,order:3,..at},Some(2));assert!(c.executions[0].confirmed);
    c.attack(Attack{source,target,key1:0,key2:123,meta:[7;8],at:Stamp{frame:4,order:4,..at}});
    assert_eq!(c.skill(source,target,Stamp{frame:5,order:5,..at}).skill,Some(987654321));
    let e=Effect{affected:target,instance:7,id:99999999,time_bits:60f32.to_bits(),stacks:1,source:Some(source),first:at,last:at};
    c.effect(e.clone()).unwrap();c.effect(e.clone()).unwrap();assert_eq!(c.effects.len(),1);assert_eq!(c.effect_updates,1);
    let mut sibling=e.clone();sibling.instance=8;c.effect(sibling).unwrap();c.remove(target,7);assert_eq!(c.effects.len(),1);assert_eq!(c.effects[&(target,8)].source,Some(source));
    let mut conflict=e;conflict.instance=8;conflict.id=1;assert!(c.effect(conflict).is_err());
}
fn feed(r:&mut Replay,seq:&mut u32,wire:i32,p:Vec<u8>,time:u64){let payload=protocol::encode_frame(wire,&p);let n=payload.len() as u32;r.feed(Input::Segment(Segment{timestamp_ns:time,flow:Flow{src:"192.0.2.1".into(),dst:"192.0.2.2".into(),sport:1234,dport:2345},seq:*seq,payload,syn:false,fin:false,rst:false,ack:true})).unwrap();*seq+=n;}
#[test] fn hp_amount_uses_confirmed_action_when_direct_attack_skill_is_zero(){
    let mut r=Replay::new(true);let mut seq=1;let mut hp=vec![0;21];put64(&mut hp,0,2);put32(&mut hp,8,8);put64(&mut hp,12,100);feed(&mut r,&mut seq,170205,hp.clone(),0);
    let mut action=vec![0;43];put64(&mut action,0,1);put32(&mut action,8,2);action[12]=65;put32(&mut action,39,99999999);feed(&mut r,&mut seq,170064,action,0);
    let mut clock=vec![0;24];put64(&mut clock,0,1);put32(&mut clock,8,100);feed(&mut r,&mut seq,170066,clock,0);
    let mut state=vec![0;20];put64(&mut state,0,1);put32(&mut state,12,99999999);feed(&mut r,&mut seq,171117,state,0);
    put64(&mut hp,12,90);feed(&mut r,&mut seq,170205,hp,0);
    let mut a=vec![0;35];put32(&mut a,0,1);put32(&mut a,8,2);feed(&mut r,&mut seq,20526,a,0);r.finish();
    assert_eq!(r.meter.session.estimated_total.damage,10);assert_eq!(r.meter.skills.keys().next().unwrap().3,Some(99999999));assert_eq!(r.health.audit.back().unwrap().allocations[0].skill,None); // Packet key is unknown; the meter records the linked confirmed action.
}
#[test] fn replay_resolves_late_attack_skill_and_preserves_explicit_amounts(){
    let mut r=Replay::new(true);let mut seq=1;let mut p=vec![0;53];put32(&mut p,0,1);put32(&mut p,8,2);put64(&mut p,16,123);feed(&mut r,&mut seq,21196,p,0);
    let mut a=vec![0;35];put32(&mut a,0,1);put32(&mut a,8,2);put32(&mut a,16,0xf0000001);feed(&mut r,&mut seq,20526,a,1);r.finish();
    assert_eq!(r.meter.session.total.damage,123);assert_eq!(r.meter.skills.values().sum::<u128>(),123);assert_eq!(r.meter.skills.keys().next().unwrap().3,Some(0xf0000001));assert_eq!(r.meter.raw_hits[0].damage.amount,123);
}
#[test] fn live_replay_late_join_routes_both_finalized_and_deferred_skills(){
    for join_time in [10_000_000,100_000_000] {
        let mut r=Replay::new(true);r.live_scope=true;r.meter.automatic_encounters=true;
        let me=r.meter.registry.observe(0,1,1).unwrap();r.meter.registry.assign(Some(me)).unwrap();let mut seq=1;
        let mut p=vec![0;53];put32(&mut p,0,3);put32(&mut p,8,2);put64(&mut p,16,123);feed(&mut r,&mut seq,21196,p.clone(),0);
        let mut a=vec![0;35];put32(&mut a,0,3);put32(&mut a,8,2);put32(&mut a,16,0xf0000001);feed(&mut r,&mut seq,20526,a,1);
        assert_eq!(r.meter.snapshot().summary.damage,0);
        put32(&mut p,0,1);put64(&mut p,16,7);feed(&mut r,&mut seq,21196,p,join_time);r.finish();
        let s=r.meter.snapshot();assert_eq!((s.summary.damage,s.summary.start),(130,Some(0)));
        assert_eq!(r.meter.skills.values().sum::<u128>(),130);assert!(r.meter.skills.keys().all(|k|k.0==1));
        assert_eq!(s.rows.iter().flat_map(|r|&r.skills).map(|s|s.explicit+s.estimated).sum::<u128>(),130);
    }
}
#[test] fn late_join_audit_uses_earliest_estimate_and_latest_explicit_time(){
    let mut r=Replay::new(true);r.meter.automatic_encounters=true;
    let me=r.meter.registry.observe(1,1,1).unwrap();r.meter.registry.assign(Some(me)).unwrap();
    let h=|source,t,amount|Hit{timestamp_ns:t,source:key(source),target:key(10),damage:Damage{actor:source,target:10,amount,flags:[false;5],raw_flags:[0;7]}};
    r.meter.add_estimate(h(2,1,40)).unwrap();r.meter.add(h(1,2,10)).unwrap();
    let audit=r.audit();let view=audit["encounters"][0]["views"].as_array().unwrap().iter().find(|v|v["target"].is_null()).unwrap();
    assert_eq!(view["start_ns"],1);assert_eq!(view["end_ns"],2);assert_eq!(r.meter.snapshot().summary.damage,50);
}
#[test] fn unknown_npc_template_is_accepted_only_by_fixed_suffix_profile(){
    let mut body=vec![0;700];put64(&mut body,0,12);put64(&mut body,572,12);put32(&mut body,580,2048);
    put64(&mut body,590,u64::MAX);put32(&mut body,598,3);put32(&mut body,602,0xf0000001);
    let mut p=55u64.to_le_bytes().to_vec();p.extend(body);
    let parsed=mabi_meter_core::identity::npc(&p).unwrap();assert_eq!(parsed.actor,12);assert_eq!(parsed.template.as_deref(),Some("010000F0"));
    put64(&mut p,8+572,13);assert!(mabi_meter_core::identity::npc(&p).is_none());
}
#[test] fn reverse_direction_decode_loss_does_not_clear_inbound_hp_or_actions(){
    let mut r=Replay::new(true);let mut seq=1;
    for hp in [100,90]{let mut p=vec![0;21];put64(&mut p,0,2);put32(&mut p,8,8);put64(&mut p,12,hp);feed(&mut r,&mut seq,170205,p,0);}
    let mut a=vec![0;35];put32(&mut a,0,1);put32(&mut a,8,2);put32(&mut a,16,123);feed(&mut r,&mut seq,20526,a,0);
    r.feed(Input::Segment(Segment{timestamp_ns:0,flow:Flow{src:"192.0.2.2".into(),dst:"192.0.2.1".into(),sport:2345,dport:1234},seq:1,payload:vec![255;40],syn:false,fin:false,rst:false,ack:true})).unwrap();
    r.finish();assert_eq!(r.meter.session.estimated_total.damage,10);assert_eq!(r.combat.attacks.len(),1);
}
#[test] fn socket_refresh_preserves_incarnations_and_finite_endpoint_obligation(){
    let mut r=Replay::new(true);let mut seq=1;let mut hp=vec![0;21];put64(&mut hp,0,2);put32(&mut hp,8,8);put64(&mut hp,12,100);feed(&mut r,&mut seq,170205,hp.clone(),0);
    let before=r.meter.registry.records.keys().next().copied().unwrap();
    r.feed(Input::Segment(Segment{timestamp_ns:1,flow:Flow{src:"192.0.2.1".into(),dst:"192.0.2.2".into(),sport:1234,dport:2345},seq,payload:Vec::new(),syn:false,fin:true,rst:false,ack:true})).unwrap();seq+=1;
    put64(&mut hp,12,50);feed(&mut r,&mut seq,170205,hp,2);let mut a=vec![0;35];put32(&mut a,0,1);put32(&mut a,8,2);feed(&mut r,&mut seq,20526,a,2);r.finish();
    assert!(r.meter.registry.is_current(before));assert_eq!(r.health.outcomes["canonical initial baseline; no predecessor arithmetic"],1);assert_eq!(r.meter.session.estimated_total.damage,50);
}
#[test] fn ambiguous_skills_conserve_amount_and_do_not_merge_equal_labels(){
    let mut c=Combat::default();let (source,target)=(key(1),key(2));
    for (f,skill) in [(1,0xf0000001),(2,0xf0000002)]{c.attack(Attack{source,target,key1:skill,key2:0,meta:[0;8],at:at(f)});}
    let matched=c.skill(source,target,at(3));assert_eq!(matched.skill,None);assert_eq!(matched.outcome,"ambiguous");
    let mut m=Meter::default();let h=Hit{timestamp_ns:3,source,target,damage:Damage{actor:1,target:2,amount:123,flags:[false;5],raw_flags:[0;7]}};m.attribute_skill(&h,1,matched.skill,false).unwrap();
    assert_eq!(m.skills.values().sum::<u128>(),123);assert_eq!(m.skills.keys().next().unwrap().3,None);
}
#[test] #[ignore="private four-person research adapters"] fn research_pet_action_effect_and_timing_profiles(){
    let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-fixtures/combat-research-20261106.json");
    let rows:Vec<serde_json::Value>=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();let mut pets=Vec::new();let mut actions=0;let mut t=Timeline::default();let mut c=Combat::default();let mut clocks=std::collections::BTreeMap::new();
    for row in &rows{
        let hex=row["payload_hex"].as_str().unwrap();let p:Vec<u8>=(0..hex.len()).step_by(2).map(|i|u8::from_str_radix(&hex[i..i+2],16).unwrap()).collect();let w=row["wire"].as_i64().unwrap() as i32;
        if matches!(w,20051|20517){let s=ownership::parse(w,&p).unwrap();pets.push((s.actor,s.owner));for n in 0..p.len(){assert!(ownership::parse(w,&p[..n]).is_none());}}
        let at=Stamp{process:1,continuity:0,context:1,frame:row["frame"].as_u64().unwrap(),order:row["frame"].as_u64().unwrap(),timestamp_ns:row["timestamp_ns"].as_u64().unwrap(),tcp_byte:0};
        if w==170064{let a=combat::action(&p).unwrap();assert_ne!(a.skill,0);actions+=1;c.execution(Execution{source:key(a.actor),skill:a.skill,action:Some(a.name),variant:Some(a.tail),at,confirmed:false}).unwrap();}
        if w==170066{clocks.insert(protocol::u32le(&p,0),at.order);}
        if w==171117{let actor=protocol::u32le(&p,0);c.state(key(actor),protocol::u32le(&p,12),at,clocks.get(&actor).copied());}
        t.observe(w,&p,at).unwrap();
    }
    pets.sort();assert_eq!(pets,[(2941120,2941007),(2941739,2941301),(2942018,2941497),(2942169,2941536)]);assert_eq!(actions,353);assert_eq!(c.confirmed_executions,353);assert!(!t.samples.is_empty());
}
