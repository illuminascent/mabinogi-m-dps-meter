use mabi_meter_core::{health::Health,identity::EntityKey,replay::Replay,meter::{Meter,Hit,Action,Command},protocol::Damage};
fn key(actor:u32)->EntityKey {EntityKey {process:0,binding:1,actor,generation:1}}
#[test] fn inference_suppresses_explicit_ambiguity_heals_and_loss() {
    let (source,target)=(key(1),key(2));let mut h=Health::default();
    h.update(target,1000,0).unwrap();h.update(target,900,1).unwrap();h.attack(source,target);
    assert_eq!(h.update(target,800,2).unwrap().unwrap().amount,100);
    h.attack(source,target);h.explicit(target);assert!(h.update(target,700,3).unwrap().is_none());
    h.attack(source,target);h.attack(source,target);assert!(h.update(target,600,4).unwrap().is_none());
    h.attack(source,target);assert!(h.update(target,1000,5).unwrap().is_none());
    h.update(target,900,6).unwrap();h.attack(source,target);h.clear();assert!(h.flush().is_empty());
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
    let mut h=Health::default();let (source,target)=(key(1),key(2));
    h.update_in(1,target,100,0).unwrap();h.update_in(1,target,90,1).unwrap();h.attack_in(2,source,target);
    assert!(h.update_in(1,target,80,2).unwrap().is_none());h.attack_in(1,source,target);h.clear_context(2);
    assert_eq!(h.flush()[0].amount,10);
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
