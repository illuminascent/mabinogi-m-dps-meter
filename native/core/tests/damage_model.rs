use mabi_meter_core::{combat::{Attack,Combat,DamageModel,Execution,Stamp,MODEL_ACTORS,MODEL_COMPONENTS},identity::EntityKey};
fn key(actor:u32)->EntityKey{EntityKey{process:1,binding:1,actor,generation:1}}
fn at(frame:u64)->Stamp{Stamp{process:1,continuity:0,context:1,frame,order:frame,timestamp_ns:frame*1_000_000,tcp_byte:frame*100}}
#[test]
fn shared_component_fit_transfers_actor_power_without_actor_ability_cells(){
    let mut model=DamageModel::default();
    // Fully crossed clean impacts: actor2 has twice the power; ability20 has
    // triple potency; its second raw stage has ten times component impact.
    for _ in 0..2500{for (source,power) in [(key(1),1),(key(2),2)]{for (skill,potency) in [(10,1),(20,3)]{for (stage,fraction) in [(0,1),(1,10)]{
        let mut flags=[0;7];flags[1]=stage;
        model.observe_clean(source,key(99),skill,7,flags,100*power*potency*fraction);
    }}}}
    let a=model.weight(key(1),key(99),10,7,[0;7]);let b=model.weight(key(2),key(99),10,7,[0;7]);
    assert!(a.1 && b.1);assert!((b.0/a.0-2.0).abs()<0.1,"actor ratio {:?} {:?}",a,b);
    let ability=model.weight(key(1),key(99),20,7,[0;7]);assert!((ability.0/a.0-3.0).abs()<0.15);
    let mut flags=[0;7];flags[1]=1;let component=model.weight(key(1),key(99),20,7,flags);
    assert!((component.0/ability.0-10.0).abs()<0.5);
    let summary=model.summary();assert_eq!(summary["actors"],2);assert_eq!(summary["abilities"],2);assert_eq!(summary["components"],4);
}
#[test]
fn full_raw_flags_and_secondary_key_keep_small_ticks_distinct(){
    let mut model=DamageModel::default();let small=[0,0x88,1,0,1,0,0];let large=[0,6,0,0,1,0,0];
    for _ in 0..4000{
        model.observe_clean(key(1),key(9),604957820,0,small,3169);
        model.observe_clean(key(1),key(9),604957820,0,large,64555);
        model.observe_clean(key(1),key(9),604957820,7,large,8000);
    }
    let a=model.weight(key(1),key(9),604957820,0,small).0;let b=model.weight(key(1),key(9),604957820,0,large).0;
    assert!(b/a>15.0);assert!(b/model.weight(key(1),key(9),604957820,7,large).0>6.0);
    assert_eq!(model.summary()["components"],3);
}
#[test]
fn target_nuisance_does_not_become_free_actor_ability_factor(){
    let mut model=DamageModel::default();
    for _ in 0..3000{for (source,power) in [(key(1),1),(key(2),3)]{for (target,mitigation) in [(key(10),1),(key(11),2)]{
        model.observe_clean(source,target,20,0,[0;7],100*power*mitigation);
    }}}
    let a=model.weight(key(1),key(10),20,0,[0;7]).0;let b=model.weight(key(1),key(11),20,0,[0;7]).0;
    assert!((b/a-2.0).abs()<0.1);assert!((model.weight(key(2),key(10),20,0,[0;7]).0/a-3.0).abs()<0.15);
}
#[test]
fn actor_prior_uses_common_power_units_without_notification_or_support_count_weight(){
    let mut model=DamageModel::default();assert_eq!(model.actor_prior(key(1)),model.actor_prior(key(2)));
    for _ in 0..3000{for (source,power) in [(key(1),1),(key(2),3)]{for skill in [10,20]{model.observe_clean(source,key(99),skill,0,[0;7],100*power);}}}
    let a=model.actor_prior(key(1));let b=model.actor_prior(key(2));assert!(a.1 && b.1);assert!((b.0/a.0-3.0).abs()<0.15);
    let before=model.summary();for _ in 0..63{let _=model.weight(key(3),key(99),999,0,[0;7]);}
    let _=model.weight(key(4),key(99),999,0,[0;7]);assert_eq!(model.actor_prior(key(3)),model.actor_prior(key(4)));assert_eq!(before,model.summary());
}
#[test]
fn predictions_are_read_only_sparse_and_never_whole_cast_normalized(){
    let mut model=DamageModel::default();assert_eq!(model.weight(key(1),key(2),10,0,[0;7]),(1.0,false));
    model.observe_clean(key(1),key(2),10,0,[0;7],100);model.observe_clean(key(1),key(2),10,0,[0;7],100);
    let before=model.summary();for _ in 0..1000{assert!((model.weight(key(1),key(2),10,0,[0;7]).0-100.0).abs()<1e-9);}
    assert_eq!(before,model.summary());assert!(!model.weight(key(3),key(2),10,0,[0;7]).1);
    model.observe_clean(key(3),key(2),10,0,[0;7],0);assert_eq!(model.anchors,2);assert_eq!(model.rejected_zero,1);
    model.observe_clean(key(3),key(2),0,0,[0;7],999999);assert_eq!(model.anchors,2);assert_eq!(model.rejected_unknown_ability,1);
    assert_eq!(model.summary()["abilities"],1);assert!(!model.weight(key(1),key(2),0,0,[0;7]).1);
}
#[test]
fn inference_dependency_histories_reclaim_pending_contexts_and_obey_their_own_caps(){
    let mut combat=Combat::default();for frame in 0..10000{
        combat.attack(Attack{source:key(1),target:key(2),key1:20,key2:0,meta:[0;8],at:at(frame)});
        combat.execution(Execution{source:key(1),skill:20,action:Some("action".into()),variant:None,at:at(frame),confirmed:false}).unwrap();
    }
    assert_eq!(combat.attacks.len(),mabi_meter_core::combat::ATTACK_HISTORY);assert_eq!(combat.executions.len(),mabi_meter_core::combat::EXECUTION_HISTORY);assert!(combat.assignment_owned_bytes()<1_000_000);
    let mut timeline=mabi_meter_core::combat::Timeline::default();for frame in 0..10000{let mut stamp=at(frame);stamp.context=frame;timeline.event(stamp);}
    assert_eq!(timeline.bounds.len(),mabi_meter_core::combat::CLOCK_HISTORY);assert!(timeline.assignment_owned_bytes()<1_500_000);
    assert!(timeline.assignment_owned_bytes()<=timeline.assignment_ceiling_bytes());assert!(combat.assignment_owned_bytes()<=combat.assignment_ceiling_bytes());
}
#[test]
fn clock_context_saturation_retains_unknown_sample_without_stopping_decode(){
    use mabi_meter_core::combat::{Timeline,MAX_CLOCK_CONTEXTS};
    let mut timeline=Timeline::default();let mut body=[0;24];body[8..12].copy_from_slice(&100u32.to_le_bytes());
    for context in 0..MAX_CLOCK_CONTEXTS as u64{let mut stamp=at(context);stamp.context=context;timeline.observe(170066,&body,stamp).unwrap();timeline.event(stamp);}
    let mut overflow=at(200);overflow.context=1000;timeline.observe(170066,&body,overflow).unwrap();timeline.event(overflow);
    assert_eq!(timeline.clock_capacity_rejections,1);assert!(timeline.samples.back().unwrap().ticks.is_none());assert!(timeline.bounds[&200].lower.is_none());
    timeline.clear_context(0);timeline.observe(170066,&body,overflow).unwrap();assert!(timeline.samples.back().unwrap().ticks.is_some());
    assert!(timeline.assignment_owned_bytes()<=timeline.assignment_ceiling_bytes());
}
#[test]
fn hard_state_capacity_rejects_new_factors_atomically_and_keeps_owned_bytes_fixed(){
    let mut model=DamageModel::default();let bytes=model.owned_bytes();
    for actor in 1..=MODEL_ACTORS as u32{model.observe_clean(key(actor),key(9999),10,0,[0;7],100);}
    let before=model.summary();model.observe_clean(key(MODEL_ACTORS as u32+1),key(9999),999,9,[9;7],100);
    assert_eq!(model.capacity_rejections,1);assert_eq!(model.anchors,MODEL_ACTORS as u64);
    assert_eq!(model.summary()["abilities"],before["abilities"]);assert_eq!(model.summary()["components"],before["components"]);assert_eq!(model.owned_bytes(),bytes);
    let mut components=DamageModel::default();
    for stage in 0..MODEL_COMPONENTS as u32{components.observe_clean(key(1),key(2),10,stage,[0;7],100);}
    components.observe_clean(key(1),key(2),10,MODEL_COMPONENTS as u32,[0;7],100);
    assert_eq!(components.capacity_rejections,1);assert_eq!(components.summary()["components"],MODEL_COMPONENTS);
    assert!(bytes<200_000);
}
#[test]
fn packet_linked_skill_has_no_eight_message_cutoff_and_keeps_ambiguous_actions(){
    let mut combat=Combat::default();let source=key(1);let target=key(2);
    let mut attack=Attack{source,target,key1:123,key2:0,meta:[0;8],at:at(100)};
    assert_eq!(combat.linked_skill(&attack).skill,Some(123));
    attack.key1=0;
    for (frame,skill) in [(10,123),(20,456)]{combat.execution(Execution{source,skill,action:None,variant:None,at:at(frame),confirmed:true}).unwrap();}
    assert_eq!(combat.linked_skill(&attack).skill,None);assert_eq!(combat.linked_skill(&attack).outcome,"ambiguous");
    combat.executions.pop_back();assert_eq!(combat.linked_skill(&attack).skill,Some(123));
    attack.at.continuity=1;assert_eq!(combat.linked_skill(&attack).skill,None);
}
