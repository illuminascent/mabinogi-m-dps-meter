use mabi_meter_core::{health::Health,identity::EntityKey,replay::Replay,meter::{Meter,Hit,Action,Command},protocol::Damage};
use mabi_meter_core::combat::{Stamp,Attack};
fn key(actor:u32)->EntityKey {EntityKey {process:0,binding:1,actor,generation:1}}
fn at(frame:u64)->Stamp{Stamp{process:1,continuity:0,context:1,frame,order:frame,timestamp_ns:frame*1_000_000,tcp_byte:frame*60}}
fn attack(source:EntityKey,target:EntityKey,frame:u64)->Attack{Attack{source,target,key1:7000001,key2:0,meta:[0;8],at:at(frame)}}
fn health()->Health{let mut h=Health::default();h.self_sources(true,[key(99)].into());h}
#[test]
fn capacity_rejection_cannot_discard_already_committed_expired_allocations(){
    let mut h=health();
    for actor in 1..=mabi_meter_core::health::MAX_SLOTS as u32{h.update(key(actor),100,at(0)).unwrap();}
    h.update(key(2),90,at(1)).unwrap();h.attack(attack(key(1),key(2),2)).unwrap();
    assert!(h.update(key(mabi_meter_core::health::MAX_SLOTS as u32+1),100,at(300)).is_err());
    assert_eq!(h.allocated,0); // Rejection has no expiry side effects.
    let estimates=h.tick(300_000_000);
    assert_eq!(estimates.iter().map(|e|e.amount as u128).sum::<u128>(),h.allocated);
    assert_eq!(h.allocated,10);
    let mut meter=Meter::default();
    assert_eq!(meter.registry.observe(0,1,1).unwrap(),key(1));assert_eq!(meter.registry.observe(0,1,2).unwrap(),key(2));
    for e in estimates{meter.add_estimate(Hit{timestamp_ns:e.timestamp_ns,source:e.source,target:e.target,damage:Damage{actor:e.source.actor,target:e.target.actor,amount:e.amount,flags:[false;5],raw_flags:[0;7]}}).unwrap();}
    assert_eq!(meter.session.estimated_total.damage as u128,h.allocated);
}
#[test] fn accounting_keeps_explicit_residual_multihits_and_loss_before_heal() {
    let (source,target)=(key(1),key(2));let mut h=health();
    h.update(target,1000,at(0)).unwrap();h.update(target,900,at(1)).unwrap();h.attack(attack(source,target,2)).unwrap();
    assert!(h.update(target,800,at(3)).unwrap().is_empty());
    h.attack(attack(key(99),target,4)).unwrap();h.explicit(key(99),target,30,at(5)).unwrap();
    h.attack(attack(source,target,6)).unwrap();h.attack(attack(source,target,7)).unwrap();
    h.update(target,1000,at(8)).unwrap();let estimates=h.flush();assert_eq!(estimates.len(),2);assert_eq!(estimates[0].amount,100);assert_eq!(estimates[1].amount,70);
    assert_eq!((h.loss,h.explicit,h.allocated,h.unassigned),(200,30,170,0));
    h.update(target,1000,at(9)).unwrap();h.update(target,900,at(10)).unwrap();h.attack(attack(source,target,11)).unwrap();h.clear();
    assert_eq!(h.flush().iter().map(|e|e.amount).sum::<u64>(),100);
    assert_eq!(h.unassigned,0);assert_eq!(h.loss,h.explicit+h.allocated);
}
#[test] fn encounters_history_and_new_battle_navigation() {
    let mut m=Meter::default();m.automatic_encounters=true;let source=m.registry.observe(0,1,1).unwrap();let target=m.registry.observe(0,1,2).unwrap();m.registry.assign(Some(source)).unwrap();
    let hit=|t|Hit {timestamp_ns:t,source,target,damage:Damage {actor:1,target:2,amount:10,flags:[false;5],raw_flags:[0;7]}};
    m.add(hit(1)).unwrap();m.tick(16_000_000_001);
    m.add(hit(17_000_000_001)).unwrap();assert_eq!(m.current,2);
    assert!(m.command(Command {id:1,encounter:2,observed_revision:m.revision,action:Action::Encounter(1)}).applied);
    assert_eq!(m.snapshot().encounter,1);m.tick(33_000_000_001);m.add(hit(34_000_000_001)).unwrap();
    assert_eq!(m.snapshot().encounter,3);assert_eq!(m.snapshot().target,None);
}
#[test] fn reset_partitions_without_clearing_warmed_names_or_self() {
    let mut m=Meter::default();let me=m.registry.observe(0,1,1).unwrap();m.registry.assign(Some(me)).unwrap();
    let teammate=m.registry.appearance(0,1,mabi_meter_core::identity::Appearance {actor:3,name:"Teammate".into(),uid:Some(5678),template:None,offset:0,record_reference:None}).unwrap();
    let target=m.registry.observe(0,1,2).unwrap();m.add(Hit{timestamp_ns:1,source:me,target,damage:Damage{actor:1,target:2,amount:10,flags:[false;5],raw_flags:[0;7]}}).unwrap();
    let revision=m.registry.revision;let ack=m.command(Command{id:1,observed_revision:m.revision,encounter:m.current,action:Action::Reset});
    assert!(ack.applied);assert_eq!(ack.snapshot.summary.damage,0);assert_eq!(m.registry.revision,revision);
    assert_eq!(m.registry.self_key,Some(me));assert!(m.registry.is_current(teammate));assert!(m.registry.label(teammate).contains("Teammate"));
    assert_eq!(m.encounters[&1].views[&None].total.damage,10);assert_eq!(m.raw_hits.len(),1);
}
#[test] fn nonself_combat_outside_encounters_is_still_captured() {
    let mut m=Meter::default();m.automatic_encounters=true;let me=m.registry.observe(0,1,1).unwrap();m.registry.assign(Some(me)).unwrap();
    let source=m.registry.observe(0,1,3).unwrap();let target=m.registry.observe(0,1,2).unwrap();
    m.add(Hit{timestamp_ns:1,source,target,damage:Damage{actor:3,target:2,amount:123,flags:[false;5],raw_flags:[0;7]}}).unwrap();
    assert_eq!(m.raw_hits.len(),1);assert_eq!(m.session.total.damage,123);assert_eq!(m.snapshot().summary.damage,0);
}
#[test] fn split_keeps_charts_separate_even_at_the_same_capture_time() {
    let mut m=Meter::default();let source=m.registry.observe(0,1,1).unwrap();let target=m.registry.observe(0,1,2).unwrap();
    let hit=|timestamp_ns,amount|Hit{timestamp_ns,source,target,damage:Damage{actor:1,target:2,amount,flags:[false;5],raw_flags:[0;7]}};
    m.add(hit(0,100)).unwrap();m.add(hit(1_000_000_000,1000)).unwrap();
    m.command(Command{id:1,observed_revision:m.revision,encounter:m.current,action:Action::Reset});
    m.add(hit(1_000_000_000,7)).unwrap();m.add(hit(2_000_000_000,7)).unwrap();
    assert_eq!(m.snapshot().chart.last().unwrap().1,14.0);
    m.command(Command{id:2,observed_revision:m.revision,encounter:m.current,action:Action::Encounter(1)});
    assert_eq!(m.snapshot().chart.last().unwrap().1,1100.0);
}
#[test] fn search_finds_targets_beyond_the_first_catalog_page() {
    let mut m=Meter::default();let source=m.registry.observe(0,1,1).unwrap();
    for actor in 2..=302 {let target=m.registry.observe(0,1,actor).unwrap();m.add(Hit{timestamp_ns:actor as u64,source,target,damage:Damage{actor:1,target:actor,amount:4,flags:[false;5],raw_flags:[0;7]}}).unwrap();}
    assert_eq!(m.snapshot().targets.len(),256);
    let ack=m.command(Command{id:1,observed_revision:m.revision,encounter:m.current,action:Action::TargetSearch("302".into())});
    assert!(ack.applied);assert_eq!(ack.snapshot.targets.len(),1);assert_eq!(ack.snapshot.targets[0].0.actor,302);
}
#[test] fn hp_observations_never_cross_connection_contexts() {
    let mut h=health();let (source,target)=(key(1),key(2));
    h.update(target,100,at(0)).unwrap();h.update(target,90,at(1)).unwrap();let mut a=attack(source,target,2);a.at.context=2;h.attack(a).unwrap();
    assert!(h.update(target,80,at(3)).unwrap().is_empty());h.attack(attack(source,target,4)).unwrap();h.clear_context(2);
    assert_eq!(h.flush()[0].amount,10);
}
#[test] fn symmetric_sparse_prior_is_frequency_neutral_conserved_and_keeps_packet_skill(){
    let mut h=health();let t=key(10);h.update(t,100,at(0)).unwrap();
    let mut totals=[0,0,0];
    for round in 0..3{
        let f=round*5+1;h.update(t,100-(round+1)*4,at(f)).unwrap();
        for (i,a) in [1,1,2,3].into_iter().enumerate(){h.attack(attack(key(a),t,f+1+i as u64)).unwrap();}
        h.update(t,100-(round+1)*4,at(f+5)).unwrap();
    }
    let estimates=h.flush();assert_eq!(estimates.len(),9);for e in estimates{assert_eq!(e.skill,Some(7000001));totals[(e.source.actor-1) as usize]+=e.amount;}
    assert_eq!(totals,[4,4,4]);assert_eq!(h.unassigned,0);
}
#[test] fn message_count_and_self_selection_do_not_exclude_packet_supported_damage(){
    let t=key(10);let mut h=health();h.attack(attack(key(1),t,0)).unwrap();h.update(t,100,at(1)).unwrap();
    h.update(t,90,at(2)).unwrap();h.attack(attack(key(1),t,11)).unwrap();assert_eq!(h.flush()[0].amount,10);assert_eq!(h.unassigned,0);
    h.update(t,90,at(12)).unwrap();h.update(t,80,at(13)).unwrap();h.attack(attack(key(99),t,14)).unwrap();h.attack(attack(key(1),t,15)).unwrap();assert_eq!(h.flush().iter().map(|e|e.amount).sum::<u64>(),10);
    h.update(t,10,at(16)).unwrap();h.update(t,0,at(17)).unwrap();h.explicit(key(99),t,15,at(18)).unwrap();assert!(h.flush().is_empty());
    assert!(h.audit.back().unwrap().explicit_mismatch);assert_eq!(h.loss,h.explicit+h.allocated+h.unassigned);
}
#[test] fn allocations_do_not_create_hits_or_criticals(){
    let mut m=Meter::default();let source=m.registry.observe(0,1,1).unwrap();let target=m.registry.observe(0,1,2).unwrap();
    m.add_estimate(Hit{timestamp_ns:1,source,target,damage:Damage{actor:1,target:2,amount:7,flags:[false;5],raw_flags:[0;7]}}).unwrap();
    let s=m.snapshot();assert_eq!((s.summary.damage,s.summary.hits,s.summary.allocations),(7,0,1));assert_eq!(s.rows[0].explicit_hits,0);
}
#[test] fn delayed_explicit_refines_one_interval_without_extending_attack_activity(){
    let mut h=health();let t=key(10);h.update(t,100,at(0)).unwrap();h.update(t,90,at(1)).unwrap();
    h.attack(attack(key(1),t,2)).unwrap();h.attack(attack(key(2),t,3)).unwrap();
    h.update(t,80,at(4)).unwrap();h.attack(attack(key(3),t,5)).unwrap();
    h.explicit(key(1),t,4,at(40)).unwrap();
    // Retention does not extend the distinct capture-local association policy.
    h.attack(attack(key(4),t,150)).unwrap();
    let estimates=h.tick(300_000_000);assert_eq!(estimates.len(),2);
    assert_eq!((estimates[0].source,estimates[0].amount),(key(2),6));assert_eq!((estimates[1].source,estimates[1].amount),(key(3),10));
    assert_eq!((h.loss,h.explicit,h.allocated,h.unassigned),(20,4,16,0));assert!(h.tick(400_000_000).is_empty());
    assert_eq!(h.refinements.len(),1);assert_eq!(h.refinements[0].amount,4);
}
#[test] fn unpublished_sentinel_does_not_erase_supported_hp_or_invent_explicit_damage(){
    let mut h=health();let t=key(10);h.update(t,100,at(0)).unwrap();h.update(t,90,at(1)).unwrap();h.attack(attack(key(1),t,2)).unwrap();
    h.sentinel(t,at(3));assert_eq!(h.flush()[0].amount,10);assert_eq!(h.unassigned,0);assert_eq!(h.explicit,0);assert_eq!(h.outcomes["unpublished sentinel; actor inference uncertain"],1);
}
#[test] fn later_activity_cannot_retroactively_make_an_older_budget_warm(){
    for next_hp in [90,100]{
        let mut h=health();let t=key(10);h.update(t,100,at(0)).unwrap();h.update(t,90,at(1)).unwrap();
        h.update(t,next_hp,at(2)).unwrap();h.attack(attack(key(1),t,3)).unwrap();
        let inferred=h.flush();assert!(inferred.is_empty());assert_eq!(h.loss,10);assert_eq!(h.unassigned,10);
    }
}
#[test] #[ignore="private packet fixture"] fn real_five_self_joins_and_inferred_damage() {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-fixtures");
    let mut r=Replay::new(false);let mut changes=Vec::new();let mut last=None;
    mabi_meter_core::capture_file::read(&root.join("jp-identity-research-long-five-self-joins.jsonl"),None,|input|{
        r.feed(input)?;if r.meter.registry.self_key!=last {last=r.meter.registry.self_key;if let Some(key)=last {changes.push(key.actor);}}Ok(())
    }).unwrap();r.finish();
    assert_eq!(r.diagnostics.0.get("personal_identity"),Some(&5));
    assert!(r.meter.registry.self_key.is_some());
    assert_eq!(changes,vec![2539237,1995596,3432913,2069622,3440374]);
    let r=Replay::load(&root.join("jp-identity-research-long-focused-tcp.jsonl"),None,false).unwrap();
    assert_eq!(r.meter.raw_hits.len(),65);assert_eq!(r.meter.raw_hits.iter().map(|h|h.damage.amount as u128).sum::<u128>(),912305);
    let ally:Vec<_>=r.meter.inferred_hits.iter().filter(|h|h.source.actor==1995609 && h.target.actor==1997639).collect();
    assert_eq!(ally.len(),34);assert_eq!(ally.iter().map(|h|h.damage.amount as u128).sum::<u128>(),114719);
}
