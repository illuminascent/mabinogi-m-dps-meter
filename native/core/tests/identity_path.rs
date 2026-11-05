use mabi_meter_core::{identity::{Character,EntityKey,Registry,tracking::{Evidence,EvidenceOrigin,Limits,SelfEvidence,Tracking,VoteKind}},
    meter::{Action,Command,Hit,Meter},protocol::Damage};

fn key(actor: u32) -> EntityKey { EntityKey { process:1,binding:1,actor,generation:1 } }
fn evidence(frame: u64) -> Evidence { Evidence { frame,timestamp_ns:frame*1_000_000,origin:EvidenceOrigin::Semantic } }

#[test] fn partial_appearance_refresh_retains_stronger_verified_fields() {
    let mut t=Tracking::default();
    let full=Character::score2_appearance_uid(0x1122334455667788).unwrap();
    let partial=Character { uid:None,..full.clone() };
    t.personal(full.clone(),evidence(1)).unwrap();t.appearance(key(20),full.clone(),evidence(2)).unwrap();
    t.appearance(key(20),partial,evidence(3)).unwrap();
    assert_eq!(t.appearances[&key(20)].character.uid,full.uid);assert_eq!(t.binding().key,Some(key(20)));
    let different=Character { uid:None,code_server:Some(([9;4],1)),name_server:None };
    assert!(t.appearance(key(20),different,evidence(4)).is_err());assert_eq!(t.binding().key,Some(key(20)));
}

#[test] fn process_replacement_requires_fresh_personal_identity() {
    let mut m=Meter::default();let old=m.registry.observe(1,1,20).unwrap();
    m.registry.personal_evidence(uid(77),evidence(1)).unwrap();
    m.registry.appearance_evidence(old,uid(77),evidence(2)).unwrap();
    m.event(mabi_meter_core::meter::Event::ProcessReplacement { evidence:10 }).unwrap();
    assert!(m.registry.tracking.personal.is_none());assert_eq!(m.registry.self_key,None);
    let new=m.registry.observe(2,2,20).unwrap();m.registry.appearance_evidence(new,uid(77),evidence(11)).unwrap();
    assert_eq!(m.registry.self_key,None);
}
fn uid(value: u64) -> Character { Character { uid:Some(value),code_server:None,name_server:None } }
fn named(value: Option<u64>,name: &str,server: u16) -> Character {
    Character { uid:value,code_server:None,name_server:Some((name.into(),server)) }
}
fn hit(m: &mut Meter,source: EntityKey,target: EntityKey,amount: u64,timestamp_ns: u64) {
    m.add(Hit { timestamp_ns,source,target,damage:Damage { actor:source.actor,target:target.actor,
        amount,flags:[true,false,false,false,false],raw_flags:[1,0,0,0,0,0,0] } }).unwrap();
}

#[test] fn votes_require_two_distinct_frames() {
    let mut t = Tracking::default();
    t.vote(key(10),VoteKind::SelfCode,evidence(1)).unwrap();
    assert_eq!(t.binding().evidence,SelfEvidence::Candidate);assert_eq!(t.binding().key,None);
    t.vote(key(10),VoteKind::SelfCode,evidence(1)).unwrap();
    assert_eq!(t.binding().key,None);
    t.vote(key(10),VoteKind::SelfCode,evidence(2)).unwrap();
    assert_eq!(t.binding().key,Some(key(10)));assert_eq!(t.binding().evidence,SelfEvidence::Voted);
    assert!(!t.binding().primary_locked);
}
#[test] fn primary_replaces_weak_vote_and_locks_other_votes() {
    let mut t = Tracking::default();
    t.vote(key(10),VoteKind::SelfCode,evidence(1)).unwrap();
    t.vote(key(10),VoteKind::SelfMatchCode,evidence(2)).unwrap();
    t.personal(uid(77),evidence(3)).unwrap();t.appearance(key(20),uid(77),evidence(4)).unwrap();
    assert_eq!(t.binding().key,Some(key(20)));assert!(t.binding().primary_locked);
    t.vote(key(30),VoteKind::SelfPlayerIdB,evidence(5)).unwrap();
    t.vote(key(30),VoteKind::SelfPlayerIdB,evidence(6)).unwrap();
    assert_eq!(t.binding().key,Some(key(20)));assert_eq!(t.binding().evidence,SelfEvidence::PacketConfirmed);
}
#[test] fn first_two_observation_voter_locks_until_reset() {
    let mut t = Tracking::default();
    for (frame,actor) in [(1,10),(2,10),(3,20),(4,20),(5,20)] {
        t.vote(key(actor),VoteKind::SelfCode,evidence(frame)).unwrap();
    }
    assert_eq!(t.binding().key,Some(key(10)));assert_eq!(t.binding().evidence,SelfEvidence::Voted);
    assert!(t.binding().voter_locked);
}
#[test] fn operator_override_does_not_destroy_automatic_identity() {
    let mut t = Tracking::default();t.personal(uid(77),evidence(1)).unwrap();
    t.appearance(key(20),uid(77),evidence(2)).unwrap();t.assign(Some(key(10)),evidence(3)).unwrap();
    assert_eq!(t.binding().key,Some(key(10)));assert_eq!(t.binding().evidence,SelfEvidence::OperatorAssigned);
    t.assign(None,evidence(4)).unwrap();assert_eq!(t.binding().key,Some(key(20)));
    assert_eq!(t.binding().evidence,SelfEvidence::PacketConfirmed);
}
#[test] fn character_enrichment_keeps_known_uid_and_current_entity() {
    let mut t = Tracking::default();t.personal(named(Some(77),"Example",4),evidence(1)).unwrap();
    t.appearance(key(20),uid(77),evidence(2)).unwrap();
    t.personal(named(None,"Example",4),evidence(3)).unwrap();
    assert_eq!(t.personal.as_ref().unwrap().uid,Some(77));assert_eq!(t.binding().key,Some(key(20)));
}
#[test] fn same_name_different_present_uid_never_joins() {
    let mut t = Tracking::default();t.personal(named(Some(77),"Example",4),evidence(1)).unwrap();
    t.appearance(key(20),named(Some(88),"Example",4),evidence(2)).unwrap();assert_eq!(t.binding().key,None);
    t.appearance(key(30),named(None,"Example",4),evidence(3)).unwrap();assert_eq!(t.binding().key,Some(key(30)));
}
#[test] fn missing_server_and_empty_identity_are_rejected() {
    let mut t = Tracking::default();
    assert!(t.personal(Character { uid:None,code_server:None,name_server:None },evidence(1)).is_err());
    assert!(t.personal(named(None,"",0),evidence(1)).is_err());
    assert!(t.personal(uid(0),evidence(1)).is_err());
    assert_eq!(t.binding().key,None);
}
#[test] fn both_orders_need_no_zone_event() {
    for personal_first in [false,true] {
        let mut t = Tracking::default();
        if personal_first { t.personal(uid(77),evidence(1)).unwrap(); }
        t.appearance(key(20),uid(77),evidence(2)).unwrap();
        if !personal_first { t.personal(uid(77),evidence(3)).unwrap(); }
        assert_eq!(t.binding().key,Some(key(20)));assert!(t.binding().primary_locked);
    }
}
#[test] fn replacement_aliases_follow_verified_retirement() {
    let mut t = Tracking::default();t.personal(uid(77),evidence(1)).unwrap();
    t.appearance(key(20),uid(77),evidence(2)).unwrap();
    t.appearance(key(30),uid(77),evidence(3)).unwrap();
    assert_eq!(t.binding().key,None);assert_eq!(t.group(key(20)),key(20));assert_eq!(t.group(key(30)),key(30));
    t.retire(key(20),evidence(4)).unwrap();assert_eq!(t.binding().key,Some(key(30)));
    assert_eq!(t.group(key(30)),key(20));assert_eq!(t.aliases.len(),1);
}
#[test] fn ambiguity_reverses_presentation_alias_without_rewriting_hits() {
    let mut m = Meter::default();
    let old = m.registry.observe(1,1,20).unwrap();let target = m.registry.observe(1,1,999).unwrap();
    m.registry.appearance_evidence(old,named(Some(77),"Example",4),evidence(1)).unwrap();hit(&mut m,old,target,100,0);
    m.registry.retire(old).unwrap();let new = m.registry.observe(1,1,30).unwrap();
    m.registry.appearance_evidence(new,named(Some(77),"Example",4),evidence(10)).unwrap();hit(&mut m,new,target,200,1_000_000_000);
    let s = m.snapshot();assert_eq!(s.source_count,1);assert_eq!(s.rows[0].damage,300);
    assert_eq!(s.rows[0].key,old);assert_eq!(s.rows[0].assignment_key,Some(new));
    let third = m.registry.observe(1,1,40).unwrap();
    m.registry.appearance_evidence(third,named(Some(77),"Example",4),evidence(20)).unwrap();hit(&mut m,third,target,30,2_000_000_000);
    let s = m.snapshot();assert_eq!(s.source_count,3);assert_eq!(s.summary.damage,330);
    assert_eq!(s.rows.iter().map(|r| r.damage).sum::<u128>(),330);
    assert_eq!(m.raw_hits[0].source,old);assert_eq!(m.raw_hits[1].source,new);
    m.registry.retire(third).unwrap();let s = m.snapshot();assert_eq!(s.summary.damage,330);
    assert!(s.rows.iter().any(|r| r.key == old && r.damage == 300));
}
#[test] fn character_switch_clears_old_operator_and_confirmation() {
    let mut t = Tracking::default();t.personal(uid(77),evidence(1)).unwrap();
    t.appearance(key(20),uid(77),evidence(2)).unwrap();t.assign(Some(key(20)),evidence(3)).unwrap();
    t.personal(uid(88),evidence(4)).unwrap();assert_eq!(t.binding().key,None);
    t.appearance(key(30),uid(88),evidence(5)).unwrap();assert_eq!(t.binding().key,Some(key(30)));
    assert_ne!(t.group(key(30)),t.group(key(20)));
}
#[test] fn clear_bindings_drops_votes_and_marker_without_forgetting_character() {
    let mut t = Tracking::default();t.personal(uid(77),evidence(1)).unwrap();
    t.appearance(key(20),uid(77),evidence(2)).unwrap();t.assign(Some(key(20)),evidence(3)).unwrap();
    t.clear_bindings(evidence(4));assert_eq!(t.binding().key,None);assert_eq!(t.personal,Some(uid(77)));
    let new = EntityKey { binding:2,..key(30) };t.appearance(new,uid(77),evidence(5)).unwrap();
    assert_eq!(t.binding().key,Some(new));assert_eq!(t.binding().evidence,SelfEvidence::PacketConfirmed);
}
#[test] fn bounded_directory_voters_aliases_and_event_history() {
    let limits = Limits { appearances:1,aliases:0,voters:1,events:2 };
    let mut t = Tracking::new(limits);t.appearance(key(10),uid(77),evidence(1)).unwrap();
    assert!(t.appearance(key(20),uid(88),evidence(2)).is_err());
    t.vote(key(10),VoteKind::SelfCode,evidence(3)).unwrap();
    assert!(t.vote(key(20),VoteKind::SelfCode,evidence(4)).is_err());
    t.assign(Some(key(10)),evidence(5)).unwrap();t.assign(None,evidence(6)).unwrap();
    assert_eq!(t.events.len(),2);assert!(t.event_evictions>0);
}
#[test] fn acknowledged_marker_remains_visible_outside_top16() {
    let mut m = Meter::default();let target = m.registry.observe(1,1,999).unwrap();
    let me = m.registry.observe(1,1,1).unwrap();hit(&mut m,me,target,1,0);
    for actor in 2..=20 { let k = m.registry.observe(1,1,actor).unwrap();hit(&mut m,k,target,100,1); }
    let ack = m.command(Command { id:1,observed_revision:0,encounter:m.current,action:Action::AssignSelf(Some(me)) });
    assert!(ack.applied);assert!(!ack.snapshot.rows.iter().any(|r| r.key == me));
    assert_eq!(ack.snapshot.self_binding.key,Some(me));
    assert_eq!(ack.snapshot.self_binding.evidence,SelfEvidence::OperatorAssigned);
}
#[test] fn raw_name_scanner_does_not_enable_personal_identity() {
    let mut r = Registry::default();let k = r.observe(1,1,10).unwrap();
    r.appearance(1,1,mabi_meter_core::identity::Appearance { actor:10,name:"Example".into(),uid:Some(77),
        template:None,offset:8,record_reference:None }).unwrap();
    assert_eq!(r.self_key,None);assert!(r.tracking.appearances.is_empty());assert!(r.tracking.personal.is_none());
    assert!(r.label(k).starts_with("Example"));
}
