use mabi_meter_core::{capture_file::Input,identity::{Appearance,EntityKey,ownership::Spawn,tracking::{Evidence,EvidenceOrigin}},meter::{Hit,Meter},protocol::{self,Damage},replay::Replay,transport::{Flow,Segment}};
fn key(actor:u32)->EntityKey{EntityKey{process:0,binding:1,actor,generation:1}}
fn player(r:&mut Replay,actor:u32){r.meter.registry.appearance(0,1,Appearance{actor,name:format!("player{actor}"),uid:Some(actor as u64+1000),template:None,offset:0,record_reference:None}).unwrap();}
fn put(p:&mut[u8],at:usize,n:u64){p[at..at+8].copy_from_slice(&n.to_le_bytes());}
fn feed(r:&mut Replay,seq:&mut u32,w:i32,p:&[u8],ns:u64){let payload=protocol::encode_frame(w,p);let n=payload.len()as u32;r.feed(Input::Segment(Segment{timestamp_ns:ns,flow:Flow{src:"server".into(),dst:"client".into(),sport:16000,dport:3},seq:*seq,payload,syn:false,fin:false,rst:false,ack:true})).unwrap();*seq+=n;}
fn hp(r:&mut Replay,s:&mut u32,target:u32,n:u64,ns:u64){let mut p=[0;21];put(&mut p,0,target as u64);p[8..12].copy_from_slice(&8u32.to_le_bytes());put(&mut p,12,n);feed(r,s,170205,&p,ns);}
fn attack(r:&mut Replay,s:&mut u32,source:u32,target:u32,ns:u64){let mut p=[0;35];put(&mut p,0,source as u64);put(&mut p,8,target as u64);p[16..20].copy_from_slice(&77u32.to_le_bytes());feed(r,s,20526,&p,ns);}
fn raw(r:&mut Replay,s:&mut u32,source:u32,target:u32,n:u64,ns:u64){let mut p=[0;53];put(&mut p,0,source as u64);put(&mut p,8,target as u64);put(&mut p,16,n);feed(r,s,21196,&p,ns);}
fn replay()->Replay{let mut r=Replay::new(true);r.live_scope=true;r}
#[test]fn every_meter_entry_rejects_confirmed_friendly_fire_atomically(){
 let mut r=replay();player(&mut r,1);player(&mut r,2);let mut m=r.meter;
 let h=Hit{timestamp_ns:0,source:key(1),target:key(2),damage:Damage{actor:1,target:2,amount:4,flags:[false;5],raw_flags:[0;7]}};
 assert!(m.add(h.clone()).is_err());assert!(m.add_estimate(h.clone()).is_err());assert!(m.commit_credit(1,h,false,None).is_err());assert_eq!(m.session.total.damage,0);assert_eq!(m.session.estimated_total.damage,0);assert!(m.skills.is_empty());
}
#[test]fn friendly_raw_and_attack_cannot_credit_reconcile_or_train(){
 let mut r=replay();let mut s=1;player(&mut r,1);player(&mut r,2);hp(&mut r,&mut s,2,100,0);hp(&mut r,&mut s,2,90,0);attack(&mut r,&mut s,1,2,0);raw(&mut r,&mut s,1,2,4,0);r.finish();
 assert_eq!(r.meter.session.total.damage,0);assert_eq!(r.meter.session.estimated_total.damage,0);assert_eq!(r.health.loss,10);assert_eq!(r.health.explicit,0);assert_eq!(r.health.unassigned,10);assert_eq!(r.combat.damage_model.anchors,0);assert_eq!(r.health.attribution.audit()["invariant"],true);
}
#[test]fn later_player_identity_invalidates_pending_typed_support(){
 let mut r=replay();let mut s=1;hp(&mut r,&mut s,2,100,0);hp(&mut r,&mut s,2,90,0);attack(&mut r,&mut s,1,2,0);player(&mut r,1);player(&mut r,2);r.finish();assert_eq!(r.meter.session.estimated_total.damage,0);assert_eq!(r.health.unassigned,10);
}
#[test]fn player_owned_pets_share_player_side(){
 let mut r=replay();player(&mut r,1);player(&mut r,2);let evidence=Evidence{frame:1,timestamp_ns:0,origin:EvidenceOrigin::Packet{wire:20051}};
 for(actor,owner)in [(3,1),(4,2)]{r.meter.registry.owned_spawn(0,1,Spawn{actor,owner,template:9,outer:None,kind:4},evidence).unwrap();}
 for source in 1..=4{for target in 1..=4{assert!(!r.meter.registry.damage_eligible(key(source),key(target)));}}
}
#[test]fn cold_rejection_uses_configured_idle_not_presence_or_matching_timer(){
 for(age,expected)in [(499_999_999,10),(500_000_000,0),(500_000_001,0)]{
  let mut r=replay();r.meter.idle_ns=500_000_000;let mut s=1;attack(&mut r,&mut s,1,99,0);hp(&mut r,&mut s,99,100,age);hp(&mut r,&mut s,99,90,age);r.finish();assert_eq!(r.meter.session.estimated_total.damage,expected,"age={age}");assert_eq!(r.health.unassigned,10-expected);
 }
}
#[test]fn appearances_alone_never_create_active_attackers(){let mut r=replay();let mut s=1;for actor in 1..=20{player(&mut r,actor);}hp(&mut r,&mut s,99,100,0);hp(&mut r,&mut s,99,90,0);r.finish();assert_eq!(r.numeric.detected,10);assert_eq!(r.health.unassigned,10);assert_eq!(r.meter.session.estimated_total.damage,0);}
#[test]fn generic_npc_type_does_not_invent_a_shared_faction(){let mut m=Meter::default();for actor in [1,2]{m.registry.appearance(0,1,Appearance{actor,name:format!("npc{actor}"),uid:None,template:Some(format!("{actor:08X}")),offset:0,record_reference:None}).unwrap();}assert!(m.registry.damage_eligible(key(1),key(2)));}
#[test]fn blocked_delivery_is_revalidated_when_player_identity_arrives(){
 let mut r=replay();let mut s=1;r.meter.session.total.damage=u128::MAX;raw(&mut r,&mut s,1,2,7,0);assert_eq!(r.health.attribution.queued_amount(),7);
 r.meter.session.total.damage=0;player(&mut r,1);player(&mut r,2);r.finish();assert_eq!(r.meter.session.total.damage,0);assert_eq!(r.health.attribution.queued_amount(),0);assert_eq!(r.health.attribution.unattributed_explicit,7);assert_eq!(r.health.attribution.audit()["invariant"],true);
}
#[test]fn registry_gate_survives_bounded_side_cache_eviction(){
 use mabi_meter_core::combat::Stamp;let mut r=replay();player(&mut r,1);player(&mut r,2);let at=Stamp{process:0,context:1,continuity:0,frame:1,order:1,timestamp_ns:0,tcp_byte:0};
 for actor in 3..=600{r.health.note_actor_kind(key(actor),"npc",at);}r.health.attribution.submit_explicit(key(1),key(2),7,[0;7],at);r.finish();assert_eq!(r.meter.session.total.damage,0);assert_eq!(r.health.attribution.unattributed_explicit,7);assert_eq!(r.health.attribution.audit()["invariant"],true);
}
#[test]fn unknown_affiliation_is_not_reported_as_confirmed_enemy(){let r=replay();assert_eq!(r.meter.registry.damage_side(key(1)),mabi_meter_core::identity::DamageSide::Unknown);}
#[test]fn activity_survives_transport_gap_until_idle_expires(){let mut r=replay();let mut s=1;attack(&mut r,&mut s,1,99,0);r.feed(Input::Loss{code:"transport_gap".into(),reset:false}).unwrap();hp(&mut r,&mut s,99,100,100_000_000);hp(&mut r,&mut s,99,90,110_000_000);r.finish();assert_eq!(r.meter.session.estimated_total.damage,10);}
#[test]fn changed_idle_setting_is_used_by_the_next_fallback(){let mut r=replay();let mut s=1;r.meter.idle_ns=1_000_000_000;attack(&mut r,&mut s,1,99,0);r.meter.idle_ns=500_000_000;hp(&mut r,&mut s,99,100,600_000_000);hp(&mut r,&mut s,99,90,600_000_000);r.finish();assert_eq!(r.health.attribution.active_idle_ns,500_000_000);assert_eq!(r.meter.session.estimated_total.damage,0);assert_eq!(r.health.unassigned,10);}
#[test]fn reset_drains_both_full_outbox_and_known_source_delivery_debt_before_partition(){
 use mabi_meter_core::{combat::Stamp,meter::{Action,Command},session::Session};let mut r=replay();let at=Stamp{process:0,context:1,continuity:0,frame:1,order:1,timestamp_ns:0,tcp_byte:0};
 for actor in 1..=2048{r.health.attribution.submit_explicit(key(actor),key(99999),1,[0;7],at);}assert_eq!(r.health.attribution.queued_amount(),1024);assert_eq!(r.health.attribution.owed_amount(),1024);
 let mut s=Session::new(r,None);let ack=s.command(Command{id:1,observed_revision:s.replay.meter.revision,encounter:1,action:Action::Reset});assert!(ack.applied,"{:?}",ack.reason);assert_eq!(s.replay.meter.encounters[&1].views[&None].total.damage,2048);assert_eq!(s.replay.meter.snapshot().summary.damage,0);assert_eq!(s.replay.health.attribution.owed_amount(),0);
}
fn owned(r:&mut Replay,actor:u32,owner:u32){r.meter.registry.owned_spawn(0,1,Spawn{actor,owner,template:9,outer:None,kind:4},Evidence{frame:1,timestamp_ns:0,origin:EvidenceOrigin::Packet{wire:20051}}).unwrap();}
#[test]fn owned_entity_with_npc_or_unknown_owner_is_not_proven_player_side(){
 for npc_owner in [false,true]{let mut r=replay();player(&mut r,2);if npc_owner{r.meter.registry.appearance(0,1,Appearance{actor:1,name:"NPC".into(),uid:None,template:Some("00000001".into()),offset:0,record_reference:None}).unwrap();}owned(&mut r,3,1);
 assert_eq!(r.meter.registry.damage_side(key(3)),mabi_meter_core::identity::DamageSide::Unknown);assert!(r.meter.registry.damage_eligible(key(3),key(2)));let mut s=1;raw(&mut r,&mut s,3,2,7,0);r.finish();assert_eq!(r.meter.session.total.damage,7);assert_eq!(r.health.attribution.explicit_side_conflicts,0);}
}
#[test]fn later_player_owner_proof_revalidates_queued_owned_entity_credit(){let mut r=replay();let mut s=1;player(&mut r,2);owned(&mut r,3,1);r.meter.session.total.damage=u128::MAX;raw(&mut r,&mut s,3,2,7,0);assert_eq!(r.health.attribution.queued_amount(),7);r.meter.session.total.damage=0;player(&mut r,1);assert!(!r.meter.registry.damage_eligible(key(3),key(2)));r.finish();assert_eq!(r.meter.session.total.damage,0);assert_eq!(r.health.attribution.unattributed_explicit,7);}
#[test]fn conflicting_owner_proof_retracts_player_side_from_the_bounded_cache(){let mut r=replay();let mut s=1;player(&mut r,1);player(&mut r,2);owned(&mut r,3,1);raw(&mut r,&mut s,3,2,4,0);assert_eq!(r.meter.session.total.damage,0);owned(&mut r,3,4);assert_eq!(r.meter.registry.damage_side(key(3)),mabi_meter_core::identity::DamageSide::Unknown);raw(&mut r,&mut s,3,2,7,0);r.finish();assert_eq!(r.meter.session.total.damage,7);assert_eq!(r.health.attribution.unattributed_explicit,4);}
#[test]fn owner_side_revocation_reaches_cached_activity_outside_the_roster_sample(){let mut r=replay();let mut s=1;player(&mut r,1);owned(&mut r,3,1);attack(&mut r,&mut s,3,2,0);for actor in 1000..1700{r.meter.registry.observe(0,1,actor).unwrap();}owned(&mut r,3,4);player(&mut r,2);hp(&mut r,&mut s,2,100,100_000_000);hp(&mut r,&mut s,2,90,110_000_000);r.finish();assert_eq!(r.meter.registry.damage_side(key(3)),mabi_meter_core::identity::DamageSide::Unknown);assert_eq!(r.meter.session.estimated_total.damage,10);assert_eq!(r.health.unassigned,0);}
