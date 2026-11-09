//! Adversarial end-to-end checks: semantic impact and boundary delivery, not
//! merely allocator accounting. These packets are synthetic, not accuracy truth.
use mabi_meter_core::{capture_file::Input,meter::{Action,Command},protocol,replay::Replay,session::Session,transport::{Flow,Segment}};

fn put64(p:&mut [u8],offset:usize,n:u64){p[offset..offset+8].copy_from_slice(&n.to_le_bytes());}
fn feed(r:&mut Replay,seq:&mut u32,wire:i32,body:Vec<u8>,time:u64){
    let payload=protocol::encode_frame(wire,&body);let len=payload.len() as u32;
    r.feed(Input::Segment(Segment{timestamp_ns:time,flow:Flow{src:"192.0.2.1".into(),dst:"192.0.2.2".into(),sport:1234,dport:2345},seq:*seq,payload,syn:false,fin:false,rst:false,ack:true})).unwrap();*seq+=len;
}
fn hp(r:&mut Replay,seq:&mut u32,n:u64,time:u64){let mut p=vec![0;21];put64(&mut p,0,999);p[8..12].copy_from_slice(&8u32.to_le_bytes());put64(&mut p,12,n);feed(r,seq,170205,p,time);}
fn attack(r:&mut Replay,seq:&mut u32,source:u64,flags:u8,time:u64){let mut p=vec![0;35];put64(&mut p,0,source);put64(&mut p,8,999);p[16..20].copy_from_slice(&7000001u32.to_le_bytes());p[24]=flags;feed(r,seq,20526,p,time);}
fn damage(r:&mut Replay,seq:&mut u32,source:u64,raw:u64,aux:u64,flags:u8,time:u64){let mut p=vec![0;53];put64(&mut p,0,source);put64(&mut p,8,999);put64(&mut p,16,raw);put64(&mut p,24,aux);p[32]=flags;feed(r,seq,21196,p,time);}
fn replay()->Replay{let mut r=Replay::new(true);r.live_scope=true;r}
fn other_flow(r:&mut Replay,seq:&mut u32,wire:i32,body:Vec<u8>,time:u64){
    let payload=protocol::encode_frame(wire,&body);let len=payload.len() as u32;
    r.feed(Input::Segment(Segment{timestamp_ns:time,flow:Flow{src:"192.0.2.1".into(),dst:"192.0.2.2".into(),sport:2234,dport:2345},seq:*seq,payload,syn:false,fin:false,rst:false,ack:true})).unwrap();*seq+=len;
}
fn other_hp(r:&mut Replay,seq:&mut u32,n:u64,time:u64){let mut p=vec![0;21];put64(&mut p,0,999);p[8..12].copy_from_slice(&8u32.to_le_bytes());put64(&mut p,12,n);other_flow(r,seq,170205,p,time);}
fn other_attack(r:&mut Replay,seq:&mut u32,time:u64){let mut p=vec![0;35];put64(&mut p,0,101);put64(&mut p,8,999);p[16..20].copy_from_slice(&7000001u32.to_le_bytes());other_flow(r,seq,20526,p,time);}
fn clock_candidate(r:&mut Replay,seq:&mut u32,counter:u32){let mut p=vec![0;24];put64(&mut p,0,101);p[8..12].copy_from_slice(&counter.to_le_bytes());feed(r,seq,170066,p,0);}

#[test]
fn auxiliary_field_changes_hp_reconciliation_but_preserves_raw_damage_evidence(){
    let mut r=replay();let mut seq=1;
    hp(&mut r,&mut seq,100,0);hp(&mut r,&mut seq,90,0);
    damage(&mut r,&mut seq,101,30,20,7,0);attack(&mut r,&mut seq,101,7,0);r.finish();
    assert_eq!((r.health.loss,r.health.explicit,r.health.allocated,r.health.unassigned),(10,10,0,0));
    assert_eq!(r.meter.session.total.damage,30);assert_eq!(r.meter.raw_hits[0].damage.amount,30);
    assert_eq!(r.meter.session.estimated_total.damage,0);
    assert!(!r.health.audit.back().unwrap().explicit_mismatch);
}

#[test]
fn a_zero_impact_explicit_component_does_not_steal_another_actors_loss(){
    let mut r=replay();let mut seq=1;
    hp(&mut r,&mut seq,100,0);hp(&mut r,&mut seq,90,0);
    damage(&mut r,&mut seq,101,30,30,7,0);attack(&mut r,&mut seq,101,7,0);
    attack(&mut r,&mut seq,202,9,0);r.finish();
    assert_eq!((r.health.loss,r.health.explicit,r.health.allocated,r.health.unassigned),(10,0,10,0));
    assert_eq!(r.meter.session.estimated_total.damage,10);
    assert!(r.meter.inferred_hits.iter().all(|h|h.source.actor==202));
}

#[test]
fn unrelated_packet_density_does_not_define_a_damage_boundary(){
    let mut r=replay();let mut seq=1;hp(&mut r,&mut seq,100,0);hp(&mut r,&mut seq,90,0);
    for _ in 0..120{feed(&mut r,&mut seq,20000,Vec::new(),0);}
    attack(&mut r,&mut seq,101,0,0);r.finish();
    assert_eq!(r.meter.session.estimated_total.damage,10);
    assert_eq!((r.health.loss,r.health.allocated,r.health.unassigned),(10,10,0));
}

#[test]
fn explicit_origin_and_presentation_source_are_separate_roles(){
    let mut r=replay();let mut seq=1;hp(&mut r,&mut seq,100,0);hp(&mut r,&mut seq,90,0);
    damage(&mut r,&mut seq,101,30,20,7,0);attack(&mut r,&mut seq,202,7,0);r.finish();
    assert_eq!((r.health.explicit,r.health.allocated,r.health.unassigned),(10,0,0));
    assert_eq!(r.meter.raw_hits[0].source.actor,101);assert!(r.meter.inferred_hits.is_empty());
}

#[test]
fn delayed_explicit_bridge_across_later_hp_reconciles_exactly_once(){
    let mut r=replay();let mut seq=1;hp(&mut r,&mut seq,100,0);hp(&mut r,&mut seq,90,0);
    attack(&mut r,&mut seq,101,7,0);hp(&mut r,&mut seq,80,1_000_000);
    attack(&mut r,&mut seq,202,9,1_000_000);damage(&mut r,&mut seq,101,30,20,7,50_000_000);r.finish();
    assert_eq!((r.health.loss,r.health.explicit,r.health.allocated,r.health.unassigned),(20,10,10,0));
    assert_eq!(r.meter.session.estimated_total.damage,10);
    assert!(r.meter.inferred_hits.iter().all(|h|h.source.actor==202));
    assert_eq!(r.meter.skills.values().sum::<u128>(),40);
}

#[test]
fn changing_self_marker_cannot_change_packet_assignment(){
    let mut results=Vec::new();
    for selected in [None,Some(101),Some(202)]{
        let mut r=replay();let mut seq=1;
        for actor in [101,202]{r.meter.registry.observe(0,r.meter.binding,actor).unwrap();}
        let key=selected.map(|actor|r.meter.registry.current_key(0,r.meter.binding,actor).unwrap());r.meter.registry.assign(key).unwrap();
        hp(&mut r,&mut seq,100,0);hp(&mut r,&mut seq,90,0);attack(&mut r,&mut seq,101,0,0);attack(&mut r,&mut seq,202,0,0);r.finish();
        let mut values:Vec<_>=r.meter.inferred_hits.iter().map(|h|(h.source.actor,h.damage.amount)).collect();values.sort();
        assert_eq!(values.iter().map(|(_,n)|*n).sum::<u64>(),10);results.push(values);
    }
    assert_eq!(results[0],results[1]);assert_eq!(results[0],results[2]);
}

#[test]
fn reset_commits_old_pending_credit_to_old_encounter_before_partition(){
    let mut r=replay();let mut seq=1;hp(&mut r,&mut seq,100,0);hp(&mut r,&mut seq,90,0);attack(&mut r,&mut seq,101,0,0);
    let mut s=Session::new(r,None);let revision=s.replay.meter.revision;
    let ack=s.command(Command{id:1,encounter:1,observed_revision:revision,action:Action::Reset});assert!(ack.applied,"{:?}",ack.reason);
    assert_eq!(ack.snapshot.summary.damage,0);assert_eq!(ack.snapshot.encounter,2);
    assert_eq!(s.replay.meter.encounters[&1].views[&None].estimated_total.damage,10);
    hp(&mut s.replay,&mut seq,100,1_000_000);hp(&mut s.replay,&mut seq,95,1_000_000);attack(&mut s.replay,&mut seq,202,0,1_000_000);s.replay.finish();
    assert_eq!(s.replay.meter.encounters[&2].views[&None].estimated_total.damage,5);
    assert_eq!(s.replay.meter.session.estimated_total.damage,15);
    assert_eq!(s.replay.meter.skills.values().sum::<u128>(),15);
}

#[test]
fn rejected_reset_has_no_commit_or_partition_side_effect(){
    let mut r=replay();let mut seq=1;hp(&mut r,&mut seq,100,0);hp(&mut r,&mut seq,90,0);attack(&mut r,&mut seq,101,0,0);
    let mut s=Session::new(r,None);
    let ack=s.command(Command{id:1,encounter:1,observed_revision:u64::MAX,action:Action::Reset});assert!(!ack.applied);
    assert_eq!(s.replay.health.allocated,0);assert_eq!(s.replay.meter.current,1);
    s.replay.finish();assert_eq!(s.replay.meter.snapshot().summary.damage,10);
}

#[test]
fn gap_preserves_finite_endpoint_damage_with_uncertain_association(){
    let mut r=replay();let mut seq=1;hp(&mut r,&mut seq,100,0);hp(&mut r,&mut seq,90,0);attack(&mut r,&mut seq,101,0,0);
    r.feed(Input::Loss{code:"adversarial_gap".into(),reset:false}).unwrap();
    assert_eq!(r.meter.session.estimated_total.damage,10);
    hp(&mut r,&mut seq,50,1_000_000);hp(&mut r,&mut seq,40,1_000_000);attack(&mut r,&mut seq,101,0,1_000_000);r.finish();
    assert_eq!(r.health.loss,60);assert_eq!(r.meter.session.estimated_total.damage,60);
}

#[test]
fn max_sentinel_breaks_arithmetic_instead_of_manufacturing_damage(){
    let mut r=replay();let mut seq=1;hp(&mut r,&mut seq,100,0);hp(&mut r,&mut seq,i64::MAX as u64,0);hp(&mut r,&mut seq,90,0);hp(&mut r,&mut seq,80,0);attack(&mut r,&mut seq,101,0,0);r.finish();
    assert_eq!(r.health.loss,20);assert_eq!(r.meter.session.estimated_total.damage,20);
}

#[test]
fn accepted_raw_numeric_with_unsupported_impact_cannot_duplicate_the_hp_budget(){
    for extension in [false,true]{
        let mut r=replay();let mut seq=1;hp(&mut r,&mut seq,100,0);hp(&mut r,&mut seq,90,0);
        let mut body=vec![0;53];put64(&mut body,0,101);put64(&mut body,8,999);put64(&mut body,16,20);put64(&mut body,24,if extension{10}else{30});body[32]=7;
        if extension{body.extend([0,0]);}
        feed(&mut r,&mut seq,21196,body,0);attack(&mut r,&mut seq,101,7,0);r.finish();
        assert_eq!(r.meter.session.total.damage,20);
        assert_eq!((r.health.loss,r.health.explicit,r.health.allocated,r.health.unassigned),(10,10,0,0));
        assert_eq!(r.meter.session.estimated_total.damage,0);
    }
}

#[test]
fn unpublished_damage_sentinel_does_not_erase_observed_hp_loss_and_actor_support(){
    let mut r=replay();let mut seq=1;hp(&mut r,&mut seq,100,0);hp(&mut r,&mut seq,90,0);
    damage(&mut r,&mut seq,101,u64::MAX,0,7,0);attack(&mut r,&mut seq,101,7,0);r.finish();
    assert_eq!(r.meter.session.total.damage,0);
    assert_eq!((r.health.loss,r.health.explicit,r.health.allocated,r.health.unassigned),(10,0,10,0));
    assert_eq!(r.meter.session.estimated_total.damage,10);
}

#[test]
fn sparse_notification_counts_do_not_substitute_for_damage_potency(){
    let mut r=replay();let mut seq=1;hp(&mut r,&mut seq,2000,0);hp(&mut r,&mut seq,1000,0);
    for _ in 0..63{attack(&mut r,&mut seq,101,0,0);}
    attack(&mut r,&mut seq,202,0,0);r.finish();
    let mut amounts:Vec<_>=r.meter.inferred_hits.iter().map(|h|(h.source.actor,h.damage.amount)).collect();amounts.sort();
    assert_eq!(amounts,vec![(101,500),(202,500)]);
    assert!(r.health.audit.back().unwrap().reason.contains("sparse") || r.health.audit.back().unwrap().reason.contains("forced"));
}

#[test]
fn mixed_model_coverage_does_not_change_aggregation_units_between_actors(){
    let mut r=replay();let mut seq=1;
    let actor=r.meter.registry.observe(0,r.meter.binding,101).unwrap();let target=r.meter.registry.observe(0,r.meter.binding,999).unwrap();
    for _ in 0..2{r.combat.damage_model.observe_clean(actor,target,7000001,0,[0;7],100);}
    hp(&mut r,&mut seq,2000,0);hp(&mut r,&mut seq,1000,0);
    for source in [101,202]{for _ in 0..10{attack(&mut r,&mut seq,source,0,0);}}
    r.finish();
    let mut amounts:Vec<_>=r.meter.inferred_hits.iter().map(|h|(h.source.actor,h.damage.amount)).collect();amounts.sort();
    assert_eq!(amounts,vec![(101,500),(202,500)]);
    assert_eq!(r.combat.damage_model.anchors,2,"inferred credit cannot become a model training target");
}

#[test]
fn parallel_publication_chains_do_not_add_the_same_target_decline_twice(){
    let mut r=replay();let(mut first,mut other)=(1,1);
    hp(&mut r,&mut first,100,0);hp(&mut r,&mut first,90,0);attack(&mut r,&mut first,101,0,0);
    other_hp(&mut r,&mut other,100,0);other_hp(&mut r,&mut other,90,0);other_attack(&mut r,&mut other,0);r.finish();
    assert_eq!(r.health.loss,10);assert_eq!(r.meter.session.estimated_total.damage,10);
}

#[test]
fn publication_chain_handoff_retains_unique_lower_endpoint(){
    let mut r=replay();let(mut first,mut other)=(1,1);
    hp(&mut r,&mut first,100,0);hp(&mut r,&mut first,90,0);attack(&mut r,&mut first,101,0,0);
    other_hp(&mut r,&mut other,50,300_000_000);
    other_hp(&mut r,&mut other,40,300_000_000);other_attack(&mut r,&mut other,300_000_000);r.finish();
    assert_eq!(r.health.loss,60,"unique lower endpoint remains damage with uncertain timing");
    assert_eq!(r.meter.session.estimated_total.damage,60);
}

#[test]
fn target_churn_reclaims_empty_packet_and_budget_buffer_capacities(){
    use mabi_meter_core::{combat::{Attack,Stamp},health::Health,identity::EntityKey};
    let mut h=Health::default();let source=EntityKey{process:0,binding:1,actor:101,generation:1};
    for i in 0..512u64{
        let target=EntityKey{actor:10_000+i as u32,..source};let now=i*300_000_001;
        let at=Stamp{process:0,continuity:0,context:1,frame:i*130+1,order:i*130+1,timestamp_ns:now,tcp_byte:i*8192};
        h.update(target,1000,at).unwrap();h.update(target,999,Stamp{frame:at.frame+1,order:at.order+1,..at}).unwrap();
        for j in 0..128{h.attack(Attack{source,target,key1:7000001,key2:0,meta:[0;8],at:Stamp{frame:at.frame+2+j,order:at.order+2+j,..at}}).unwrap();}
        let output=h.tick(now+300_000_001);assert_eq!(output.iter().map(|e|e.amount).sum::<u64>(),1);
        let resources=h.resource_summary();
        assert_eq!(resources["fact_capacity"].as_u64(),Some(0),"empty target facts must not retain a hidden historical allocation");
        assert_eq!(resources["pending_capacity"].as_u64(),Some(0),"empty pending buffers must be reclaimed");
        let covered=resources["peak_conservative_resident_bytes"].as_u64().unwrap()+resources["scratch_ceiling_bytes"].as_u64().unwrap()+resources["returned_output_ceiling_bytes"].as_u64().unwrap();
        assert!(covered<10*1024*1024,"Health including scratch/output exceeded10MiB during churn: {covered}");
    }
    assert_eq!(h.loss,512);assert_eq!(h.allocated,512);
}

#[test]
fn later_same_actor_flags_do_not_turn_an_earlier_role_disagreement_into_training_truth(){
    // Mimics the historical easy-raid exceptions: an explicit source-role claim
    // disagrees with its own interval's presentation, then the same source/flags
    // appear after several later HP publications. That later match is a decoy.
    let mut r=replay();let mut seq=1;hp(&mut r,&mut seq,1000,0);hp(&mut r,&mut seq,900,0);
    attack(&mut r,&mut seq,202,7,0);damage(&mut r,&mut seq,101,100,0,7,0);
    hp(&mut r,&mut seq,800,0);attack(&mut r,&mut seq,202,9,0);
    hp(&mut r,&mut seq,700,0);attack(&mut r,&mut seq,101,7,0);r.finish();
    assert_eq!(r.combat.damage_model.anchors,0);
    assert_eq!((r.health.loss,r.health.explicit,r.health.allocated),(300,100,200));
    let mut inferred:Vec<_>=r.meter.inferred_hits.iter().map(|h|(h.source.actor,h.damage.amount)).collect();inferred.sort();
    assert_eq!(inferred,vec![(101,100),(202,100)]);
}

#[test]
fn a_candidate_clock_reset_does_not_invalidate_a_same_object_hp_decrease(){
    for reset_after_loss in [false,true]{
        let mut r=replay();let mut seq=1;clock_candidate(&mut r,&mut seq,100);hp(&mut r,&mut seq,100,0);
        if reset_after_loss{hp(&mut r,&mut seq,90,0);}
        clock_candidate(&mut r,&mut seq,50);
        if !reset_after_loss{hp(&mut r,&mut seq,90,0);}
        attack(&mut r,&mut seq,101,0,0);r.finish();
        assert_eq!((r.health.loss,r.health.allocated,r.health.unassigned),(10,10,0));
        assert_eq!(r.meter.session.estimated_total.damage,10);
        assert!(r.health.audit.back().unwrap().reason.contains("clock"),"candidate clock disagreement needs an uncertainty label");
    }
}

#[test]
fn clock_reset_keeps_explicit_reconciliation_but_excludes_the_training_anchor(){
    let mut r=replay();let mut seq=1;clock_candidate(&mut r,&mut seq,100);hp(&mut r,&mut seq,100,0);
    clock_candidate(&mut r,&mut seq,50);hp(&mut r,&mut seq,90,0);
    damage(&mut r,&mut seq,101,10,0,0,0);attack(&mut r,&mut seq,101,0,0);r.finish();
    assert_eq!((r.health.loss,r.health.explicit,r.health.allocated,r.health.unassigned),(10,10,0,0));
    assert_eq!(r.combat.damage_model.anchors,0);
}

#[test]
fn disjoint_candidate_clock_brackets_are_not_physical_event_exclusion_proof(){
    let mut r=replay();let mut seq=1;clock_candidate(&mut r,&mut seq,100);hp(&mut r,&mut seq,100,0);hp(&mut r,&mut seq,90,0);
    clock_candidate(&mut r,&mut seq,104);clock_candidate(&mut r,&mut seq,108);
    attack(&mut r,&mut seq,101,0,0);clock_candidate(&mut r,&mut seq,112);r.finish();
    assert_eq!((r.health.loss,r.health.allocated,r.health.unassigned),(10,10,0));
    assert!(r.health.audit.back().unwrap().reason.contains("clock"));
}
