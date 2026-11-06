use mabi_meter_core::{identity::EntityKey,meter::{Action,Command,Event,Hit,HitAssignment,Meter},protocol::Damage};

const SECOND:u64=1_000_000_000;
fn key(actor:u32)->EntityKey {EntityKey{process:1,binding:1,actor,generation:1}}
fn meter()->Meter {
    let mut m=Meter::default();m.automatic_encounters=true;
    let me=m.registry.observe(1,1,1).unwrap();m.registry.assign(Some(me)).unwrap();m
}
fn hit(source:u32,target:u32,t:u64,amount:u64)->Hit {
    Hit{timestamp_ns:t,source:key(source),target:key(target),damage:Damage{actor:source,target,amount,flags:[false;5],raw_flags:[0;7]}}
}
fn add(m:&mut Meter,h:Hit,inferred:bool,skill:Option<u32>)->HitAssignment {
    if inferred {m.add_estimate(h.clone()).unwrap();}else{m.add(h.clone()).unwrap();}
    let a=m.last_hit_assignment(inferred).unwrap();m.attribute_recorded_skill(&h,&a,skill,inferred).unwrap();a
}
fn split(m:&mut Meter) {
    assert!(m.command(Command{id:m.last_command+1,observed_revision:m.revision,encounter:m.current,action:Action::Reset}).applied);
}

#[test] fn late_join_follows_transitive_source_target_graph_and_conserves_all_views() {
    let mut m=meter();
    // Reverse-time traversal alone misses B -> Z: it precedes B -> Y,
    // which joins A -> Y, which finally connects to the joined target X.
    let bz=add(&mut m,hit(3,12,SECOND,30),false,Some(103));
    add(&mut m,hit(3,11,2*SECOND,40),true,Some(104));
    add(&mut m,hit(2,11,3*SECOND,20),false,Some(102));
    add(&mut m,hit(2,10,4*SECOND,10),false,Some(101));
    let unrelated=add(&mut m,hit(4,13,4*SECOND,900),false,None);
    assert_eq!(m.snapshot().summary.damage,0);
    add(&mut m,hit(1,10,5*SECOND,50),false,Some(105));
    let s=m.snapshot();assert_eq!((s.summary.damage,s.summary.hits,s.summary.allocations),(150,4,1));
    assert_eq!((s.summary.start,s.summary.end),(Some(SECOND),Some(5*SECOND)));
    assert_eq!(s.summary.duration(),4.0);assert_eq!(s.summary.dps(),Some(37.5));
    assert_eq!(s.estimated_damage,40);assert_eq!(s.target_count,3);
    assert_eq!(*s.chart.last().unwrap(),(4.0,37.5,30.0));assert_eq!(bz.encounter(),Some(1));assert_eq!(unrelated.encounter(),None);
    assert_eq!((m.session.total.damage,m.session.estimated_total.damage),(1010,40));
    assert_eq!(m.raw_hits.len(),5);assert_eq!(m.inferred_hits.len(),1);
    assert_eq!(m.skills.iter().filter(|(k,_)|k.0==1).map(|(_,a)|a).sum::<u128>(),150);
    assert_eq!(m.skills.iter().filter(|(k,_)|k.0==0).map(|(_,a)|a).sum::<u128>(),900);
    assert_eq!(s.rows.iter().flat_map(|r|&r.skills).map(|s|s.explicit+s.estimated).sum::<u128>(),150);
    assert!(m.command(Command{id:1,observed_revision:m.revision,encounter:1,action:Action::Target(Some(key(11)))}).applied);
    let target=m.snapshot();assert_eq!((target.summary.damage,target.estimated_damage),(60,40));assert_eq!(target.chart.last().unwrap().1,60.0);
    // Repeated edges cannot import the same history twice.
    add(&mut m,hit(1,10,6*SECOND,1),false,None);
    assert_eq!(m.encounters[&1].views[&None].total.damage,111);
}

#[test] fn continuous_graph_backtracks_beyond_one_idle_window_but_stale_segments_stay_out() {
    let mut m=meter();
    let stale=add(&mut m,hit(2,10,0,900),false,None);
    for t in [20,30,40,50] {add(&mut m,hit(2,10,t*SECOND,10),false,None);}
    add(&mut m,hit(1,10,51*SECOND,10),false,None);
    assert_eq!((m.snapshot().summary.start,m.snapshot().summary.damage),(Some(20*SECOND),50));assert_eq!(stale.encounter(),None);
    let mut m=meter();add(&mut m,hit(2,10,0,900),false,None);
    add(&mut m,hit(1,10,15*SECOND,1),false,None);assert_eq!(m.snapshot().summary.damage,1);
}

#[test] fn cycles_same_timestamp_and_later_bridges_merge_once() {
    let mut m=meter();
    add(&mut m,hit(2,10,SECOND,2),false,None);add(&mut m,hit(2,11,SECOND,3),false,None);
    add(&mut m,hit(3,11,SECOND,4),false,None);add(&mut m,hit(3,10,SECOND,5),false,None);
    let separate=add(&mut m,hit(4,12,SECOND,6),false,None);
    add(&mut m,hit(1,10,2*SECOND,1),false,None);assert_eq!(m.snapshot().summary.damage,15);
    add(&mut m,hit(2,12,3*SECOND,7),false,None);
    assert_eq!(m.snapshot().summary.damage,28);assert_eq!(separate.encounter(),Some(1));
    assert_eq!(*m.snapshot().chart.last().unwrap(),(2.0,14.0,5.6));
}

#[test] fn deferred_skills_follow_migration_and_allocations_arriving_after_join_backdate() {
    let mut m=meter();let h=hit(2,10,SECOND,7);m.add(h.clone()).unwrap();let a=m.last_hit_assignment(false).unwrap();
    add(&mut m,hit(1,10,2*SECOND,11),false,None);
    m.attribute_recorded_skill(&h,&a,Some(777),false).unwrap();
    add(&mut m,hit(2,10,SECOND+1,5),true,Some(888));
    let s=m.snapshot();assert_eq!((s.summary.damage,s.summary.start),(23,Some(SECOND)));
    assert_eq!(m.skills.get(&(1,key(2),key(10),Some(777),false)),Some(&7));
    assert_eq!(m.skills.get(&(1,key(2),key(10),Some(888),true)),Some(&5));
    assert!(m.skills.keys().all(|k|k.0==1));assert_eq!(s.chart.last().unwrap().1,23.0);
}

#[test] fn split_is_a_sequence_boundary_even_with_identical_capture_times() {
    let mut m=meter();let old=add(&mut m,hit(2,10,SECOND,900),false,None);split(&mut m);
    let new=add(&mut m,hit(2,10,SECOND,7),false,None);add(&mut m,hit(1,10,SECOND,11),false,None);
    assert_eq!(m.current,2);assert_eq!(m.snapshot().summary.damage,18);
    assert_eq!(old.encounter(),None);assert_eq!(new.encounter(),Some(2));assert_eq!(m.registry.self_key,Some(key(1)));
}

#[test] fn automatic_new_encounter_keeps_archived_hits_and_charts_unchanged() {
    let mut m=meter();add(&mut m,hit(1,10,0,100),false,None);m.tick(15*SECOND);
    let pending=add(&mut m,hit(2,10,16*SECOND,7),false,None);add(&mut m,hit(1,10,17*SECOND,11),false,None);
    assert_eq!(m.current,2);assert_eq!((m.snapshot().summary.damage,m.snapshot().summary.start),(18,Some(16*SECOND)));
    assert_eq!(pending.encounter(),Some(2));assert_eq!(m.encounters[&1].views[&None].total.damage,100);
    m.command(Command{id:1,observed_revision:m.revision,encounter:2,action:Action::Encounter(1)});
    assert_eq!(*m.snapshot().chart.last().unwrap(),(0.0,0.0,20.0));
}

#[test] fn reused_actor_incarnations_and_verified_run_boundaries_do_not_connect() {
    let mut m=meter();add(&mut m,hit(2,10,SECOND,900),false,None);
    let mut newer=hit(1,10,2*SECOND,11);newer.target.generation+=1;m.add(newer).unwrap();
    assert_eq!(m.snapshot().summary.damage,11);
    let mut m=meter();let old=add(&mut m,hit(2,10,SECOND,900),false,None);
    m.event(Event::NewRun{evidence:7}).unwrap();
    let me=m.registry.observe(1,m.binding,1).unwrap();m.registry.assign(Some(me)).unwrap();
    // Even an old raw reference supplied by a caller cannot cross the boundary.
    let peer=add(&mut m,hit(2,10,2*SECOND,7),false,None);assert_eq!(m.snapshot().summary.damage,0);
    let mut h=hit(1,10,3*SECOND,11);h.source=me;m.add(h).unwrap();
    assert_eq!(m.snapshot().summary.damage,18);assert_eq!(old.encounter(),None);assert_eq!(peer.encounter(),Some(2));
}

#[test] fn incoming_self_damage_can_join_a_graph_without_self_outgoing_damage() {
    let mut m=meter();add(&mut m,hit(10,2,SECOND,7),false,None);add(&mut m,hit(10,1,2*SECOND,11),false,None);
    assert_eq!((m.snapshot().summary.damage,m.snapshot().summary.start),(18,Some(SECOND)));
}

#[test] fn bounded_history_reports_partial_and_backtracks_only_to_retained_damage() {
    let mut m=meter();for t in 0..65_537 {m.add(hit(2,10,t,1)).unwrap();}
    m.add(hit(1,10,65_537,1)).unwrap();let s=m.snapshot();
    assert!(s.partial);assert_eq!((s.summary.damage,s.summary.start),(65_536,Some(2)));
    assert_eq!(m.session.total.damage,65_538);assert_eq!(m.raw_hits.len(),65_536);assert_eq!(m.evicted_hits,2);
}
