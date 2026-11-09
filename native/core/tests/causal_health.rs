//! Packet-semantic and saturation regressions for bounded causal reconciliation.
use mabi_meter_core::{combat::{Attack,DamageModel,Stamp},health::{Health,MAX_FACTS,MAX_PENDING,MAX_SLOTS,MAX_TARGET_FACTS,WAIT_NS},identity::EntityKey};
fn key(actor:u32)->EntityKey{EntityKey{process:0,binding:1,actor,generation:1}}
fn at(order:u64,time:u64)->Stamp{Stamp{process:0,continuity:0,context:1,frame:order,order,timestamp_ns:time,tcp_byte:order*60}}
fn attack(actor:u32,target:u32,order:u64,time:u64,skill:u32,flag:u8)->Attack{let mut meta=[0;8];meta[0]=flag;Attack{source:key(actor),target:key(target),key1:skill,key2:0,meta,at:at(order,time)}}
#[test]
fn batched_hp_and_presentations_align_one_to_one_without_reuse(){
    let mut h=Health::default();h.update(key(99),100,at(0,0)).unwrap();h.update(key(99),90,at(1,0)).unwrap();h.update(key(99),70,at(2,0)).unwrap();h.attack(attack(1,99,3,0,10,0)).unwrap();h.attack(attack(2,99,4,0,20,0)).unwrap();
    let a=h.flush();assert_eq!(a.iter().map(|a|(a.source.actor,a.amount)).collect::<Vec<_>>(),vec![(1,10),(2,20)]);
    let frames:Vec<_>=a.iter().flat_map(|a|a.frames.iter().copied()).collect();assert_eq!(frames,vec![3,4]);assert_eq!(h.loss,h.explicit+h.allocated+h.unassigned);
}
#[test]
fn unknown_notification_frequency_does_not_manufacture_actor_power(){
    let mut h=Health::default();h.update(key(99),200,at(0,0)).unwrap();h.update(key(99),100,at(1,0)).unwrap();
    for order in 2..65{h.attack(attack(1,99,order,0,10,0)).unwrap();}h.attack(attack(2,99,65,0,20,0)).unwrap();let a=h.flush();assert_eq!(a.iter().map(|a|(a.source.actor,a.amount)).collect::<Vec<_>>(),vec![(1,50),(2,50)]);
}
#[test]
fn zero_impact_explicit_excludes_source_but_preserves_older_hp_loss(){
    let mut h=Health::default();h.update(key(99),100,at(0,0)).unwrap();h.update(key(99),90,at(1,0)).unwrap();h.attack(attack(1,99,2,0,10,7)).unwrap();h.update(key(99),90,at(3,0)).unwrap();let mut flags=[0;7];flags[0]=7;h.explicit_fields(key(1),key(99),30,Some(0),flags,at(4,0)).unwrap();h.attack(attack(1,99,5,0,10,7)).unwrap();let a=h.flush();assert!(a.is_empty());assert_eq!((h.loss,h.explicit,h.allocated,h.unassigned),(10,0,0,10));
}
#[test]
fn sparse_mode_uses_common_units_for_trained_and_untrained_actors(){
    let mut model=DamageModel::default();for _ in 0..10{model.observe_clean(key(1),key(99),10,0,[0;7],100);}
    let mut h=Health::default();h.update(key(99),200,at(0,0)).unwrap();h.update(key(99),100,at(1,0)).unwrap();for order in 2..12{h.attack(attack(1,99,order,0,10,0)).unwrap();h.attack(attack(2,99,order+10,0,10,0)).unwrap();}
    let a=h.flush_with_model(&mut model);assert_eq!(a.iter().map(|a|(a.source.actor,a.amount)).collect::<Vec<_>>(),vec![(1,50),(2,50)]);
}
#[test]
fn a_partial_explicit_claim_can_bridge_later_hp_with_unique_origin_component(){
    let mut h=Health::default();h.update(key(99),100,at(0,0)).unwrap();h.update(key(99),90,at(1,0)).unwrap();h.attack(attack(1,99,2,0,10,7)).unwrap();h.attack(attack(2,99,3,0,20,8)).unwrap();h.update(key(99),80,at(4,1_000_000)).unwrap();h.attack(attack(3,99,5,1_000_000,30,9)).unwrap();
    let mut flags=[0;7];flags[0]=7;h.explicit_fields(key(1),key(99),14,Some(4),flags,at(6,150_000_000)).unwrap();let a=h.flush();assert_eq!((h.loss,h.explicit,h.allocated,h.unassigned),(20,4,16,0));assert_eq!(a.iter().map(|a|(a.source.actor,a.amount)).collect::<Vec<_>>(),vec![(2,6),(3,10)]);assert_eq!(h.refinements.len(),1);
}
#[test]
fn availability_retention_does_not_expire_companions_mid_recovered_batch(){
    let mut h=Health::default();h.set_available(3_000_000_000);h.update(key(99),100,at(0,0)).unwrap();h.update(key(99),90,at(1,0)).unwrap();h.attack(attack(1,99,200,0,10,0)).unwrap();assert!(h.tick(3_100_000_000).is_empty());let a=h.tick(3_000_000_000+WAIT_NS+1);assert_eq!(a.len(),1);assert_eq!(a[0].amount,10);
}
#[test]
fn no_actor_support_reports_unattributed_without_invented_credit(){
    let mut h=Health::default();h.update(key(99),100,at(0,0)).unwrap();h.update(key(99),90,at(1,0)).unwrap();assert!(h.tick(WAIT_NS+1).is_empty());assert_eq!((h.loss,h.allocated,h.no_support_loss,h.capacity_loss),(10,0,10,0));assert_eq!(h.attribution.owed_amount(),0);assert_eq!(h.unassigned,10);
}
#[test]
fn conditional_either_endpoint_prevents_model_training_but_not_supported_credit(){
    for first in [true,false]{let mut h=Health::default();let mut model=DamageModel::default();h.update_provenanced(key(99),100,at(0,0),first).unwrap();h.update_provenanced(key(99),90,at(1,0),!first).unwrap();h.explicit_fields(key(1),key(99),10,Some(10),[0;7],at(2,0)).unwrap();h.attack(attack(1,99,3,0,10,0)).unwrap();h.flush_with_model(&mut model);assert_eq!(model.anchors,0);assert_eq!(h.explicit,10);}
}
#[test]
fn allocations_never_train_and_clean_structural_explicit_anchor_can_train(){
    let mut h=Health::default();let mut model=DamageModel::default();h.update(key(99),100,at(0,0)).unwrap();h.update(key(99),90,at(1,0)).unwrap();h.attack(attack(1,99,2,0,10,0)).unwrap();h.flush_with_model(&mut model);assert_eq!(model.anchors,0);
    h.update(key(99),100,at(3,0)).unwrap();h.update(key(99),90,at(4,0)).unwrap();h.explicit_fields(key(1),key(99),10,Some(10),[0;7],at(5,0)).unwrap();h.attack(attack(1,99,6,0,10,0)).unwrap();h.flush_with_model(&mut model);assert_eq!(model.anchors,1);
}
#[test]
fn maximum_numeric_budget_uses_bounded_integer_conservation(){
    let mut h=Health::default();let amount=i64::MAX as u64-1;h.update(key(99),amount,at(0,0)).unwrap();h.update(key(99),0,at(1,0)).unwrap();for actor in 1..=3{h.attack(attack(actor,99,actor as u64+1,0,actor,0)).unwrap();}let a=h.flush();assert_eq!(a.iter().map(|a|a.amount as u128).sum::<u128>(),amount as u128);assert!(a.iter().map(|a|a.amount).max().unwrap()-a.iter().map(|a|a.amount).min().unwrap()<=1);
}
#[test]
fn fact_capacity_is_reclaimed_when_pressure_rotates_through_many_targets(){
    let mut h=Health::default();let mut order=0;
    for target in 100..600{h.update(key(target),100,at(order,0)).unwrap();order+=1;for _ in 0..MAX_TARGET_FACTS{h.attack(attack(1,target,order,0,10,0)).unwrap();order+=1;}}
    let s=h.resource_summary();assert!(s["facts"].as_u64().unwrap()<=MAX_FACTS as u64);assert!(s["fact_capacity"].as_u64().unwrap()<=MAX_FACTS as u64*2);assert!(s["conservative_resident_bytes"].as_u64().unwrap()<8*1024*1024);assert!(h.fact_evictions>0);
    h.tick(WAIT_NS+1);let s=h.resource_summary();assert_eq!(s["facts"],0);assert_eq!(s["fact_capacity"],0);
}
#[test]
fn mass_expiry_and_boundary_outbox_deliver_each_allocation_once(){
    let mut h=Health::default();for i in 0..MAX_PENDING{let target=100+i as u32;h.update(key(target),100,at(i as u64*3,0)).unwrap();h.update(key(target),99,at(i as u64*3+1,0)).unwrap();h.attack(attack(1,target,i as u64*3+2,0,10,0)).unwrap();}let a=h.tick(WAIT_NS+1);assert_eq!(a.len(),MAX_PENDING);assert_eq!(h.allocated,MAX_PENDING as u128);assert!(h.flush().is_empty());assert!(h.drain_ready().is_empty());
    let s=h.resource_summary();assert!(s["peak_conservative_resident_bytes"].as_u64().unwrap()<8*1024*1024);assert!(s["pending_capacity"].as_u64().unwrap()==0);
}
#[test]
fn bounded_baseline_capacity_rejection_has_no_unpublished_commit_side_effect(){
    let mut h=Health::default();for actor in 1..=MAX_SLOTS as u32{h.update(key(actor),100,at(0,0)).unwrap();}h.update(key(2),90,at(1,0)).unwrap();h.attack(attack(1,2,2,0,10,0)).unwrap();assert!(h.update(key(MAX_SLOTS as u32+1),100,at(3,WAIT_NS+1)).is_err());assert_eq!(h.allocated,0);assert_eq!(h.tick(WAIT_NS+1).iter().map(|a|a.amount).sum::<u64>(),10);
}
