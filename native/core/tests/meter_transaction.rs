use mabi_meter_core::{identity::EntityKey,meter::{Action,Aggregate,Command,CreditAdmission,CreditCommit,Event,Hit,Meter,MAX_CELLS},protocol::Damage};

fn key(actor:u32)->EntityKey {EntityKey{process:1,binding:1,actor,generation:1}}
fn hit(source:u32,target:u32,amount:u64)->Hit {
    Hit{timestamp_ns:17,source:key(source),target:key(target),damage:Damage{actor:source,target,amount,flags:[false;5],raw_flags:[0;7]}}
}

#[test]
fn retry_receipt_is_idempotent_across_kinds_and_manual_reset() {
    let mut m=Meter::default();
    assert_eq!(m.commit_credit(1,hit(1,90,7),false,Some(8)).unwrap(),CreditCommit::Committed);
    let revision=m.revision;
    assert_eq!(m.commit_credit(1,hit(1,90,7),false,Some(8)).unwrap(),CreditCommit::AlreadyCommitted);
    assert!(m.commit_credit(1,hit(1,90,8),false,Some(8)).is_err());
    assert_eq!(m.revision,revision);
    assert_eq!(m.session.total.damage,7);assert_eq!(m.raw_hits.len(),1);
    assert_eq!(m.skills.values().sum::<u128>(),7);
    assert!(m.command(Command{id:1,observed_revision:m.revision,encounter:m.current,action:Action::Reset}).applied);
    assert_eq!(m.commit_credit(1,hit(1,90,7),false,Some(8)).unwrap(),CreditCommit::AlreadyCommitted);
    m.commit_credit(2,hit(2,90,9),true,None).unwrap();
    assert!(m.commit_credit(1,hit(1,90,7),false,Some(8)).is_err());
    assert_eq!((m.session.total.damage,m.session.estimated_total.damage),(7,9));
    assert_eq!((m.snapshot().summary.damage,m.snapshot().summary.allocations),(9,1));
}

#[test]
fn rejected_amount_and_source_leave_receipt_and_meter_unchanged() {
    let mut m=Meter::default();m.session.total.damage=u128::MAX;
    assert!(m.commit_credit(10,hit(1,90,1),false,None).is_err());
    assert_eq!(m.revision,0);assert!(m.raw_hits.is_empty());assert!(m.session.sources.is_empty());
    m.session.total.damage=0;
    m.commit_credit(10,hit(1,90,1),false,None).unwrap();
    for actor in 2..=32768 {m.session.sources.insert(key(actor),Aggregate::default());}
    let revision=m.revision;
    assert!(m.credit_acceptance(key(32769),false).is_err());
    assert!(m.commit_credit(11,hit(32769,90,3),false,None).is_err());
    assert_eq!(m.revision,revision);assert_eq!(m.session.total.damage,1);assert_eq!(m.raw_hits.len(),1);
    // The same pending receipt can be retried once the admission prerequisite
    // changes; a failed attempt never reserves the token.
    m.session.sources.remove(&key(32768));
    m.commit_credit(11,hit(32769,90,3),false,None).unwrap();
    assert_eq!(m.session.total.damage,4);
}

#[test]
fn entire_registry_actor_universe_is_admissible() {
    let mut m=Meter::default();
    for actor in 1..=32768 {m.session.sources.insert(key(actor),Aggregate::default());}
    assert!(m.credit_acceptance(key(32768),false).is_ok());
    assert!(m.credit_acceptance(key(32768),true).is_ok());
    m.commit_credit(1,hit(32768,90,7),false,None).unwrap();
    assert_eq!(m.snapshot().rows[0].damage,7);
}

#[test]
fn over_universe_capacity_signal_is_distinct_from_immutable_amount_failure() {
    let mut m=Meter::default();
    // Real captured generation-zero identities may outlive Registry's metadata
    // slots. Their admission signal must be usable by the sole packet assigner.
    for actor in 1..=32768 {
        let mut source=key(actor);source.generation=0;
        m.session.sources.insert(source,Aggregate::default());
    }
    let mut overflow=key(32769);overflow.generation=0;
    assert_eq!(m.credit_admission(overflow,false),CreditAdmission::SourceCapacity);
    assert_eq!(m.credit_admission(overflow,true),CreditAdmission::Ready);
    assert_eq!(m.admitted_sources(false).count(),32768);
    let admitted=m.admitted_sources(false).next().unwrap();
    assert_eq!(m.credit_admission(admitted,false),CreditAdmission::Ready);
    let mut h=hit(admitted.actor,90,7);h.source=admitted;
    // Amount failure is not an actor-capacity failure and cannot authorize
    // retrying a different actor. It leaves the same receipt available.
    m.session.total.damage=u128::MAX;
    assert!(m.commit_credit(1,h.clone(),false,None).is_err());
    assert_eq!(m.credit_admission(admitted,false),CreditAdmission::Ready);
    assert!(m.raw_hits.is_empty());assert_eq!(m.revision,0);
    m.session.total.damage=0;m.commit_credit(1,h,false,None).unwrap();
    assert_eq!(m.session.total.damage,7);
    assert_eq!(m.session.sources[&admitted].damage,7);
}

#[test]
fn target_detail_pressure_preserves_all_target_actor_credit() {
    let mut m=Meter::default();
    for target in 1..=4097 {m.commit_credit(target as u64,hit(50000,target,1),false,Some(7)).unwrap();}
    let s=m.snapshot();assert!(s.partial);assert_eq!(s.target_count,4096);
    assert_eq!((s.summary.damage,s.rows[0].damage),(4097,4097));
    assert_eq!(s.rows[0].skills.iter().map(|s|s.explicit+s.estimated).sum::<u128>(),4097);
    assert_eq!(m.session.total.damage,4097);
}

#[test]
fn skill_detail_pressure_conserves_unknown_actor_bucket() {
    let mut m=Meter::default();
    for skill in 0..MAX_CELLS as u32 {m.skills.insert((1,key(1),key(90),Some(skill),false),0);}
    m.commit_credit(1,hit(1,90,7),false,Some(u32::MAX)).unwrap();
    let s=m.snapshot();assert!(s.partial);assert!(m.skills.len()<=MAX_CELLS);
    assert_eq!(s.rows[0].skills.iter().map(|s|s.explicit+s.estimated).sum::<u128>(),7);
    assert_eq!(s.rows[0].skills.iter().find(|s|s.id.is_none()).unwrap().explicit,7);
    assert_eq!(m.raw_hits[0].damage.amount,7);
}

#[test]
fn delayed_pending_skills_remain_transferable_after_compaction_and_repeated_skill() {
    let mut m=Meter::default();m.automatic_encounters=true;
    let me=m.registry.observe(1,1,1).unwrap();m.registry.assign(Some(me)).unwrap();
    m.commit_credit(1,hit(2,90,7),false,Some(8)).unwrap();
    for skill in 0..(MAX_CELLS-1) as u32 {m.skills.insert((1,key(999),key(90),Some(skill),false),0);}
    // A new Unknown bucket triggers compaction while the first skill is pending.
    m.commit_credit(2,hit(2,90,1),false,None).unwrap();
    let compacted=m.last_hit_assignment(false).unwrap();
    m.refine_recorded_skill(&hit(2,90,1),&compacted,Some(10),false).unwrap();
    // Reusing the old precise skill must not create a misleading smaller
    // exact bucket that steals the older hit's pending transfer.
    m.commit_credit(3,hit(2,90,3),false,Some(8)).unwrap();
    m.commit_credit(4,hit(1,90,1),false,None).unwrap();
    let s=m.snapshot();assert_eq!(s.summary.damage,12);
    assert_eq!(s.rows.iter().flat_map(|r|&r.skills).map(|s|s.explicit+s.estimated).sum::<u128>(),12);
    assert_eq!(m.skills.iter().filter(|(k,_)|k.0==0).map(|(_,amount)|amount).sum::<u128>(),0);
}

#[test]
fn published_unknown_refines_before_and_after_join_without_recredit_or_receipt_change() {
    let mut m=Meter::default();m.automatic_encounters=true;
    let me=m.registry.observe(1,1,1).unwrap();m.registry.assign(Some(me)).unwrap();
    let first=hit(2,90,7);m.commit_credit(1,first.clone(),false,None).unwrap();
    let a=m.last_hit_assignment(false).unwrap();assert_eq!(m.session.total.damage,7);
    let second=hit(2,90,3);m.commit_credit(2,second.clone(),false,None).unwrap();
    let b=m.last_hit_assignment(false).unwrap();
    m.refine_recorded_skill(&first,&a,Some(u32::MAX),false).unwrap();
    let own=hit(1,90,1);m.commit_credit(3,own.clone(),false,None).unwrap();
    let own_assignment=m.last_hit_assignment(false).unwrap();
    assert_eq!(a.encounter(),Some(1));assert_eq!(b.encounter(),Some(1));
    m.refine_recorded_skill(&second,&b,Some(8),false).unwrap();
    m.refine_recorded_skill(&second,&b,Some(8),false).unwrap();
    m.refine_recorded_skill(&own,&own_assignment,Some(9),false).unwrap();
    assert_eq!(m.commit_credit(3,own,false,None).unwrap(),CreditCommit::AlreadyCommitted);
    assert_eq!((m.session.total.damage,m.snapshot().summary.damage),(11,11));
    assert_eq!(m.raw_hits.len(),3);assert_eq!(m.skills.values().sum::<u128>(),11);
    assert_eq!(m.skills.get(&(1,key(2),key(90),Some(u32::MAX),false)),Some(&7));
    assert_eq!(m.skills.get(&(1,key(2),key(90),Some(8),false)),Some(&3));
    assert!(m.skills.keys().all(|k|k.0==1));
}

#[test]
fn archive_pressure_and_ended_policy_never_reject_session_credit() {
    let mut m=Meter::default();
    for id in 1..=40 {
        m.commit_credit(id,hit(1,90,1),false,None).unwrap();
        let command=Command{id,observed_revision:m.revision,encounter:m.current,action:Action::Reset};
        assert!(m.reset_command_ready(&command));assert!(m.command(command).applied);
        assert!(m.encounters.len()<=32);
    }
    assert_eq!(m.session.total.damage,40);assert!(m.partial);
    m.event(Event::End{evidence:99}).unwrap();
    m.commit_credit(41,hit(1,90,7),false,None).unwrap();
    assert_eq!(m.session.total.damage,47);assert_eq!(m.snapshot().summary.damage,0);
}

#[test]
fn aggregate_pressure_reclaims_archive_before_current_actor_amounts() {
    let mut m=Meter::default();let mut receipt=0;
    for encounter in 1..=2 {
        for actor in 1..=32768 {receipt+=1;m.commit_credit(receipt,hit(actor,90000,1),false,None).unwrap();}
        assert_eq!(m.snapshot().summary.damage,32768);
        let command=Command{id:encounter,observed_revision:m.revision,encounter:m.current,action:Action::Reset};
        assert!(m.command(command).applied);
    }
    receipt+=1;m.commit_credit(receipt,hit(1,90000,3),false,None).unwrap();
    assert_eq!(m.session.total.damage,65539);assert_eq!(m.snapshot().summary.damage,3);
    assert_eq!(m.snapshot().rows[0].damage,3);assert!(!m.encounters.contains_key(&1));assert!(m.partial);
}
