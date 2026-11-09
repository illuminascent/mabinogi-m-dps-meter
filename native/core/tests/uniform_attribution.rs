use mabi_meter_core::{attribution::{Attribution,Candidate,CreditKind,MAX_CREDITS,MAX_OWED},combat::{Attack,Stamp},health::Health,identity::EntityKey};
fn key(actor:u32)->EntityKey{EntityKey{process:1,binding:1,actor,generation:1}}
fn at(frame:u64)->Stamp{Stamp{process:1,context:1,continuity:1,frame,order:frame,timestamp_ns:frame*1_000_000,tcp_byte:frame}}
fn attack(source:u32,target:u32,frame:u64)->Attack{Attack{source:key(source),target:key(target),key1:700,key2:0,meta:[0;8],at:at(frame)}}
fn consume(a:&mut Attribution)->(u128,u128){let(mut raw,mut inferred)=(0,0);let mut previous=0;while let Some(c)=a.front_credit(){assert!(c.id>previous);previous=c.id;match &c.kind{CreditKind::Explicit{hit,..}=>raw+=hit.damage.amount as u128,CreditKind::Inferred(e)=>inferred+=e.amount as u128};assert!(a.ack_credit(c.id));}assert_eq!(a.detected_explicit+a.detected_inferred,a.published+a.queued_amount()+a.owed_amount()+a.unattributed_amount());(raw,inferred)}

#[test]
fn partial_explicit_and_zero_claim_cannot_erase_remaining_hp(){
    for (raw,impact,expected_raw,expected_inferred) in [(4,Some(4),4,6),(4,Some(0),4,10),(4,None,4,6)]{
        let mut h=Health::default();h.update(key(2),100,at(1)).unwrap();h.update(key(2),90,at(2)).unwrap();h.explicit_fields(key(1),key(2),raw,impact,[0;7],at(3)).unwrap();h.attack(attack(1,2,4)).unwrap();h.flush();
        assert_eq!(consume(&mut h.attribution),(expected_raw,0));assert_eq!(h.unassigned,expected_inferred);assert_eq!(h.attribution.owed_amount(),0);
    }
}
#[test]
fn cold_attack_history_is_rejected_across_every_clear_boundary(){
 for boundary in 0..5{let mut h=Health::default();h.attack(attack(1,2,1)).unwrap();h.tick(10_000_000_000);h.update(key(2),100,at(20_001)).unwrap();h.update(key(2),90,at(20_002)).unwrap();match boundary{0=>h.clear(),1=>h.clear_target(1,key(2)),2=>h.clear_context(1),3=>{h.flush();},_=>{h.tick(21_000_000_000);}}assert_eq!(consume(&mut h.attribution),(0,0));assert_eq!(h.unassigned,10);assert_eq!(h.attribution.owed_amount(),0);}
}
#[test]
fn actorless_damage_is_counted_unattributed_and_future_presence_cannot_settle_it(){
 let mut h=Health::default();h.update(key(2),100,at(1)).unwrap();h.update(key(2),90,at(2)).unwrap();h.flush();h.clear();assert_eq!(h.attribution.owed_amount(),0);assert_eq!(h.unassigned,10);h.note_actor(key(1),at(500));assert_eq!(consume(&mut h.attribution),(0,0));assert_eq!(h.attribution.unattributed_inferred,10);
}
#[test]
fn uniform_cause_by_boundary_matrix(){
    for source in [1,99]{for cause in 0..5{for boundary in 0..4{
        let mut h=Health::default();h.self_sources(true,[key(99)].into());h.note_actor(key(source),at(0));h.update(key(2),100,at(1)).unwrap();h.update(key(2),90,at(2)).unwrap();
        match cause{0=>{},1=>h.attack(attack(source,2,3)).unwrap(),2=>h.explicit_fields(key(source),key(2),4,Some(4),[0;7],at(3)).unwrap(),3=>{h.attack(attack(source,2,3)).unwrap();h.explicit_fields(key(source),key(2),0,Some(0),[0;7],at(4)).unwrap();},_=>h.explicit_fields(key(source),key(2),4,None,[0;7],at(3)).unwrap()}
        match boundary{0=>{h.flush();},1=>h.clear_target(1,key(2)),2=>h.clear_context(1),_=>{h.tick(1_000_000_000);}}
        let(raw,inferred)=consume(&mut h.attribution);assert_eq!(raw+inferred,if cause==1{10}else if cause==2 || cause==4{4}else{0},"source={source},cause={cause},boundary={boundary}");assert_eq!(h.unassigned,if cause==1{0}else if cause==2 || cause==4{6}else{10});
    }}}
}
#[test]
fn ready_pressure_retains_all_amount_and_lease_ack_cannot_swallow_new_merge(){
    let mut a=Attribution::default();let support=[Candidate{source:key(1),skill:None,weight:1.0,frames:vec![]}];
    for i in 0..MAX_CREDITS{a.resolve_budget(key(2),at(i as u64),i as u64,1,&support,"test");}
    let leased=a.front_credit().unwrap();assert_eq!(leased.amount(),1);
    for i in MAX_CREDITS..MAX_CREDITS*3{a.resolve_budget(key(2),at(i as u64),i as u64,1,&support,"test");}
    assert_eq!(a.front_credit().unwrap().amount(),1);assert!(!a.ack_credit(leased.id+1));assert!(a.ack_credit(leased.id));
    assert_eq!(consume(&mut a),(0,(MAX_CREDITS*3-1) as u128));assert_eq!(a.settled_inferred,(MAX_CREDITS*3) as u128);assert!(a.coarsened_publications>0);
}
#[test]
fn missing_raw_source_is_not_replaced_by_later_roster_and_known_raw_pressure_preserves_origin(){
 let mut a=Attribution::default();let flags=[1,6,0,8,16,0,0];a.submit_unbound_explicit(key(2),7,flags,at(1));assert_eq!(a.owed_amount(),0);assert_eq!(a.unattributed_explicit,7);a.note_actor_kind(key(1),"Player",at(3));assert!(a.front_credit().is_none());for i in 0..MAX_CREDITS*2{a.submit_explicit(key(1),key(2),1,flags,at(i as u64));}assert_eq!(consume(&mut a),((MAX_CREDITS*2)as u128,0));assert!(a.raw_detail_coarsenings>0);assert_eq!(a.settled_inferred,0);
}
#[test]
fn missing_active_support_never_grows_owed_or_reassigns_to_future_roster(){
 let mut a=Attribution::default();for i in 0..MAX_OWED+1{let mut stamp=at(i as u64);stamp.context=100+i as u64;a.resolve_budget(key(2+i as u32),stamp,i as u64,1,&[],"test");}assert_eq!(a.owed_amount(),0);assert_eq!(a.unattributed_inferred,(MAX_OWED+1)as u128);a.note_actor_kind(key(10_000),"Player",at(50_000));assert_eq!(consume(&mut a),(0,0));
}
#[test]
fn numeric_comparison_after_clear_preserves_amount_and_true_old_stamp(){
 let mut h=Health::default();h.update(key(2),100,at(1)).unwrap();h.clear();let mut next=at(20);next.continuity=2;h.update_with_comparison(key(2),90,next,Some((at(1),100)),false).unwrap();let mut companion=attack(1,2,21);companion.at.continuity=2;h.attack(companion).unwrap();h.flush();assert_eq!(consume(&mut h.attribution),(0,10));assert_eq!(h.audit.back().unwrap().baseline,at(1));
}
#[test]
fn appearances_and_known_roles_do_not_supply_attack_activity(){
 let mut a=Attribution::default();a.note_actor_kind(key(3),"NPC",at(1));a.note_actor_kind(key(1),"Player",at(1));a.note_actor_kind(key(9),"NPC",at(1));a.resolve_budget(key(9),at(2),1,10,&[],"test");assert_eq!(consume(&mut a),(0,0));assert_eq!(a.unattributed_inferred,10);
}
#[test]
fn self_damage_claim_is_a_counted_conflict_and_never_reduces_hp_budget(){
 let mut h=Health::default();h.update(key(1),100,at(1)).unwrap();h.update(key(1),90,at(2)).unwrap();h.explicit_fields(key(1),key(1),4,Some(4),[0;7],at(3)).unwrap();h.flush();assert_eq!(consume(&mut h.attribution),(0,0));assert_eq!(h.explicit,0);assert_eq!(h.unassigned,10);assert_eq!(h.attribution.unattributed_explicit,4);assert_eq!(h.attribution.explicit_side_conflicts,1);
}
#[test]
fn orphan_explicit_claim_uses_the_same_overlap_policy_before_baseline_or_at_capacity(){
    for saturated in [false,true]{let mut h=Health::default();
        if saturated{for actor in 10..10+mabi_meter_core::health::MAX_SLOTS as u32{h.update(key(actor),100,at(0)).unwrap();}}
        h.explicit_fields(key(1),key(2),4,Some(4),[0;7],at(2)).unwrap();h.accept_numeric(key(2),90,at(3),Some((at(1),100)),false);h.flush();assert_eq!(consume(&mut h.attribution),(4,0));assert_eq!((h.loss,h.explicit,h.allocated,h.unassigned),(10,4,0,6));
    }
}
#[test]
fn unmatched_raw_claim_survives_clear_before_later_numeric_comparison(){
    let mut h=Health::default();h.update(key(2),100,at(1)).unwrap();h.explicit_fields(key(1),key(2),4,Some(4),[0;7],at(2)).unwrap();h.clear();h.accept_numeric(key(2),90,at(4),Some((at(1),100)),false);h.flush();assert_eq!(consume(&mut h.attribution),(4,0));assert_eq!(h.unassigned,6);
}
#[test]
fn anonymous_raw_claim_and_hp_residual_keep_distinct_unattributed_origins(){
 let mut h=Health::default();h.update(key(2),100,at(1)).unwrap();h.update(key(2),90,at(2)).unwrap();h.unbound_explicit_fields(key(2),4,Some(4),[0;7],at(3)).unwrap();h.flush();h.note_actor_kind(key(1),"Player",at(5));assert_eq!(consume(&mut h.attribution),(0,0));assert_eq!(h.attribution.unattributed_explicit,4);assert_eq!(h.attribution.unattributed_inferred,6);assert_eq!(h.attribution.audit()["invariant"],true);
}
#[test]
fn complete_leased_credit_is_immutable_during_explicit_pressure_compaction(){
    let mut a=Attribution::default();for i in 0..MAX_CREDITS{a.submit_explicit(key(1),key(2),1,[1,0,0,0,0,0,0],at(i as u64));}let leased=a.front_credit().unwrap();a.submit_explicit(key(1),key(2),1,[0;7],at(2_000));let again=a.front_credit().unwrap();assert_eq!(leased.id,again.id);match (&leased.kind,&again.kind){(CreditKind::Explicit{hit:a,..},CreditKind::Explicit{hit:b,..})=>assert_eq!(a.damage,b.damage),_=>unreachable!()};assert!(a.ack_credit(leased.id));assert_eq!(consume(&mut a),((MAX_CREDITS) as u128,0));
}
#[test]
fn active_relationship_support_requires_entity_scope(){
 let mut a=Attribution::default();let mut foreign=key(1);foreign.binding=2;a.note_relationship(foreign,key(2),at(1));a.resolve_budget(key(2),at(2),1,10,&[],"test");assert_eq!(consume(&mut a),(0,0));assert_eq!(a.unattributed_inferred,10);a.note_relationship(key(4),key(2),at(3));a.resolve_budget(key(2),at(4),2,10,&[],"test");assert_eq!(consume(&mut a),(0,10));assert_eq!(a.unattributed_inferred,10);
}
#[test]
fn warm_attack_fallback_survives_transport_context_or_continuity_change(){
 for continuity in [false,true]{let mut a=Attribution::default();let mut old=at(1);if continuity{old.continuity=100;}else{old.context=100;}a.note_relationship(key(1),key(2),old);a.resolve_budget(key(2),at(2),1,10,&[],"test");assert_eq!(consume(&mut a),(0,10));assert_eq!(a.unattributed_inferred,0);}
}
#[test]
fn known_npc_bystanders_are_not_attackers_until_an_actual_targeted_attack(){
 for victim_kind in ["Player","Pet"]{let mut a=Attribution::default();a.note_actor_kind(key(1),victim_kind,at(1));a.note_actor_kind(key(3),"NPC",at(1));a.resolve_budget(key(1),at(2),1,10,&[],"test");assert_eq!(consume(&mut a),(0,0));a.note_relationship(key(3),key(1),at(3));a.resolve_budget(key(1),at(4),2,10,&[],"test");assert_eq!(consume(&mut a),(0,10));assert_eq!(a.unattributed_inferred,10);}
}
#[test]
fn broad_support_excludes_the_same_victim_wire_actor_in_an_older_generation(){
    let mut a=Attribution::default();let mut old=key(2);old.generation=0;a.note_actor_kind(old,"Player",at(1));a.resolve_budget(key(2),at(2),1,10,&[],"test");assert!(a.front_credit().is_none());assert_eq!(a.unattributed_inferred,10);
}
#[test]
fn prepared_skill_is_frozen_on_the_owned_receipt_until_acknowledgement(){
    let mut a=Attribution::default();a.submit_explicit(key(1),key(2),4,[0;7],at(1));let c=a.front_credit().unwrap();assert_eq!(c.prepared_skill,None);assert!(a.prepare_skill(c.id,None));assert_eq!(a.front_credit().unwrap().prepared_skill,Some(None));assert!(!a.prepare_skill(c.id,Some(77)));assert_eq!(a.front_credit().unwrap().prepared_skill,Some(None));assert!(a.ack_credit(c.id));
    a.submit_explicit(key(1),key(2),4,[0;7],at(2));let next=a.front_credit().unwrap();assert!(next.id>c.id);assert!(a.prepare_skill(next.id,Some(77)));assert_eq!(a.front_credit().unwrap().prepared_skill,Some(Some(77)));assert!(a.ack_credit(next.id));
}
#[test]
fn rejected_leased_raw_payload_keeps_its_original_flags_after_underlying_compaction(){
    let mut a=Attribution::default();let original=[1,0,0,0,0,0,0];for i in 0..MAX_CREDITS{a.submit_explicit(key(1),key(2),1,original,at(i as u64));}let leased=a.front_credit().unwrap();a.note_relationship(key(3),key(2),at(0));a.submit_explicit(key(1),key(2),1,[0;7],at(2_000));
    assert!(!a.front_credit().unwrap().detail_coarsened);assert!(a.retry_credit(leased.id,&[Candidate{source:key(3),skill:None,weight:1.0,frames:vec![]}]));
    let mut rerouted=false;let mut amount=0u128;while let Some(c)=a.front_credit(){amount+=c.amount() as u128;if c.source()==key(3){match &c.kind{CreditKind::Explicit{hit,..}=>assert_eq!(hit.damage.raw_flags,original),_=>panic!("raw retry changed origin")};assert!(!c.detail_coarsened);rerouted=true;}assert!(a.ack_credit(c.id));}
    assert!(rerouted);assert_eq!(amount,(MAX_CREDITS+1) as u128);assert_eq!(a.audit()["invariant"],true);
}
#[test]
fn delivery_pressure_retains_known_sources_and_reports_identity_capacity_failure(){
 let mut a=Attribution::default();for i in 0..MAX_CREDITS+MAX_OWED+1{a.submit_explicit(key(10+i as u32),key(900_000),1,[0;7],at(i as u64));}assert_eq!(a.queued_len(),MAX_CREDITS);assert_eq!(a.owed_amount(),MAX_OWED as u128);assert_eq!(a.unattributed_explicit,1);assert_eq!(consume(&mut a),((MAX_CREDITS+MAX_OWED)as u128,0));assert_eq!(a.unattributed_explicit,1);
}
#[test]
fn canonical_initial_never_uses_a_cached_numeric_predecessor(){
    for victim in [key(2),EntityKey{actor:0,..key(2)}]{let mut h=Health::default();h.note_actor(key(1),at(0));h.accept_numeric(victim,100,at(1),None,true);h.accept_numeric(victim,90,at(2),None,true);h.flush();assert_eq!(h.loss,0);assert_eq!(consume(&mut h.attribution),(0,0));}
}
#[test]
fn canonical_initial_retires_previously_detected_pending_amount_before_realigning(){
    let mut h=Health::default();h.attack(attack(1,2,0)).unwrap();h.accept_numeric(key(2),100,at(1),None,true);h.accept_numeric(key(2),90,at(2),Some((at(1),100)),true);h.accept_numeric(key(2),80,at(3),None,true);h.flush();assert_eq!(h.loss,10);assert_eq!(consume(&mut h.attribution),(0,10));
}
#[test]
fn friendly_player_pet_claims_never_train_or_reduce_independent_enemy_hp_credit(){
    let mut h=Health::default();h.note_actor_kind(key(1),"Player",at(0));h.note_actor_kind(key(2),"Player",at(0));h.note_actor_kind(key(3),"NPC",at(0));h.attack(attack(3,2,0)).unwrap();h.update(key(2),100,at(1)).unwrap();h.update(key(2),90,at(2)).unwrap();h.explicit_fields(key(1),key(2),4,Some(4),[0;7],at(3)).unwrap();h.attack(attack(1,2,4)).unwrap();let mut model=mabi_meter_core::combat::DamageModel::default();h.flush_with_model(&mut model);assert_eq!(consume(&mut h.attribution),(0,10));assert_eq!(h.explicit,0);assert_eq!(h.attribution.unattributed_explicit,4);assert_eq!(model.anchors,0);assert_eq!(h.attribution.explicit_side_conflicts,1);
}
#[test]
fn npc_kind_is_uncertain_and_does_not_by_itself_prove_a_shared_friendly_side(){
    let mut a=Attribution::default();a.note_actor_kind(key(1),"NPC",at(1));a.note_actor_kind(key(2),"NPC",at(1));assert!(a.eligibility(key(1),key(2)));a.note_relationship(key(1),key(2),at(1));a.resolve_budget(key(2),at(2),1,10,&[],"test");assert_eq!(consume(&mut a),(0,10));
}
#[test]
fn warm_activity_uses_the_configured_meter_idle_limit_and_never_future_attacks(){
    let mut a=Attribution::default();a.active_idle_ns=2_000_000_000;a.note_relationship(key(1),key(2),at(1));a.resolve_budget(key(2),at(2000),1,10,&[],"test");a.resolve_budget(key(2),at(2001),2,10,&[],"test");assert_eq!(consume(&mut a),(0,10));assert_eq!(a.unattributed_inferred,10);
    let mut future=Attribution::default();future.note_relationship(key(1),key(2),at(2));future.resolve_budget(key(2),at(1),1,10,&[],"test");assert_eq!(consume(&mut future),(0,0));assert_eq!(future.unattributed_inferred,10);
}
#[test]
fn metadata_refresh_never_refreshes_attack_activity(){
    let mut a=Attribution::default();a.active_idle_ns=1_000_000_000;a.note_relationship(key(1),key(2),at(1));a.note_actor_kind(key(1),"Player",at(2000));a.note_actor_kind(key(2),"NPC",at(2000));a.resolve_budget(key(2),at(2001),1,10,&[],"test");assert_eq!(consume(&mut a),(0,0));assert_eq!(a.unattributed_inferred,10);
}
#[test]
fn typed_friendly_candidates_and_missing_active_capacity_replacements_are_rejected(){
    let mut a=Attribution::default();a.note_actor_kind(key(1),"Player",at(0));a.note_actor_kind(key(2),"Player",at(0));a.resolve_budget(key(2),at(1),1,10,&[Candidate{source:key(1),skill:None,weight:1.0,frames:vec![]}],"test");assert_eq!(consume(&mut a),(0,0));assert_eq!(a.unattributed_inferred,10);
    a.submit_explicit(key(3),key(2),7,[0;7],at(2));let c=a.front_credit().unwrap();a.note_actor(key(4),at(2));assert!(a.retry_credit(c.id,&[Candidate{source:key(4),skill:None,weight:1.0,frames:vec![]}]));assert_eq!(a.unattributed_explicit,7);assert!(a.front_credit().is_none());
}
#[test]
fn authoritative_late_friendly_proof_prunes_facts_orphans_and_activity_before_training(){
    let mut h=Health::default();h.update(key(2),100,at(1)).unwrap();h.update(key(2),90,at(2)).unwrap();h.attack(attack(1,2,3)).unwrap();h.explicit_fields(key(1),key(2),10,Some(10),[0;7],at(4)).unwrap();let mut registry=mabi_meter_core::identity::Registry::default();let source=registry.observe(1,1,1).unwrap();let target=registry.observe(1,1,2).unwrap();registry.records.get_mut(&source).unwrap().kind=Some("player");registry.records.get_mut(&target).unwrap().kind=Some("player");for i in 100..700{h.note_actor(key(i),at(5));}h.revalidate_registry(&registry);assert!(h.attribution.active_candidates(key(2),at(5)).is_empty());let mut model=mabi_meter_core::combat::DamageModel::default();h.flush_with_model(&mut model);assert_eq!(model.anchors,0);assert_eq!(h.explicit,0);assert_eq!(h.unassigned,10);let c=h.front_credit().unwrap();assert!(h.attribution.reject_leased(c.id));assert_eq!(consume(&mut h.attribution),(0,0));assert_eq!(h.attribution.audit()["invariant"],true);
}
#[test]
fn later_same_pair_attack_preserves_the_prior_continuous_warm_span(){
    let mut h=Health::default();h.attack(attack(1,2,0)).unwrap();h.update(key(2),100,at(100)).unwrap();h.update(key(2),90,at(110)).unwrap();h.attack(attack(1,2,300)).unwrap();h.tick(400_000_000);let c=h.front_credit().unwrap();assert_eq!(c.source(),key(1));assert_eq!(c.amount(),10);match &c.kind{CreditKind::Inferred(e)=>assert_eq!(e.frames,vec![0]),_=>panic!("expected HP credit")};assert!(h.attribution.ack_credit(c.id));assert_eq!(h.unassigned,0);
}
#[test]
fn shorter_idle_policy_does_not_reinterpret_longer_merged_attack_gaps(){
    let mut a=Attribution::default();a.active_idle_ns=10_000_000_000;a.note_relationship(key(1),key(2),at(0));a.note_relationship(key(1),key(2),at(5000));a.active_idle_ns=1_000_000_000;a.resolve_budget(key(2),at(3000),1,10,&[],"test");assert_eq!(consume(&mut a),(0,0));assert_eq!(a.unattributed_inferred,10);a.resolve_budget(key(2),at(5500),2,10,&[],"test");assert_eq!(consume(&mut a),(0,10));
}
