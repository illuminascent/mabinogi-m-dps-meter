use mabi_meter_core::{identity::EntityKey, protected_hp::{spawn_seed,Decoder,DecodeError}};

fn synthetic_spawn(cipher:&str,first_iv_byte:u8)->Vec<u8>{
    let mut p=vec![0;1000];p[..8].copy_from_slice(&9u64.to_le_bytes());
    p[572..580].copy_from_slice(&9u64.to_le_bytes());p[580]=1;
    p[581..585].copy_from_slice(&16u32.to_le_bytes());
    p[585..601].copy_from_slice(&mabi_meter_core::capture_file::unhex(cipher).unwrap());
    p[601..605].copy_from_slice(&16u32.to_le_bytes());
    for i in 0..16{p[605+i]=first_iv_byte+i as u8;}p[625]=1;
    p[890..898].copy_from_slice(&u64::MAX.to_le_bytes());p[898..902].copy_from_slice(&3u32.to_le_bytes());p
}
fn synthetic_update(cipher:&str)->Vec<u8>{
    let mut p=9u64.to_le_bytes().to_vec();p.extend(16u32.to_le_bytes());
    p.extend(mabi_meter_core::capture_file::unhex(cipher).unwrap());p.push(0);p
}
fn attack()->Vec<u8>{
    let mut p=vec![0;35];p[..8].copy_from_slice(&7u64.to_le_bytes());
    p[8..16].copy_from_slice(&9u64.to_le_bytes());p
}
fn feed(r:&mut mabi_meter_core::replay::Replay,seq:&mut u32,ns:u64,wire:i32,p:&[u8]){
    use mabi_meter_core::{capture_file::Input,transport::{Flow,Segment},protocol};
    let payload=protocol::encode_frame(wire,p);let size=payload.len() as u32;
    r.feed(Input::Segment(Segment{timestamp_ns:ns,flow:Flow{src:"server".into(),sport:16000,dst:"client".into(),dport:1234},seq:*seq,payload,syn:false,fin:false,rst:false,ack:true})).unwrap();
    *seq+=size;
}

#[test]
fn replay_gap_preserves_iv_without_using_prior_hp_and_process_replacement_clears_it(){
    use mabi_meter_core::{capture_file::Input,replay::Replay};
    let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;
    let initial=synthetic_spawn("c71938e787b6c816204a2c56c78a251d",0);
    let zero=synthetic_update("478642e2bcbd73ebb6feafa2dd8b6a48");
    let full=synthetic_update("c71938e787b6c816204a2c56c78a251d");
    feed(&mut r,&mut seq,1,20051,&initial);
    feed(&mut r,&mut seq,2,170206,&zero);feed(&mut r,&mut seq,3,20526,&attack());
    r.feed(Input::Loss{code:"synthetic_loss".into(),reset:false}).unwrap();
    assert_eq!(r.protected_hp.key_count(),1);
    assert_eq!((r.health.loss,r.health.unassigned,r.health.allocated),(111,111,0));
    feed(&mut r,&mut seq,4,170207,&full); // Reacquire baseline using retained IV.
    feed(&mut r,&mut seq,5,170208,&zero);feed(&mut r,&mut seq,6,20526,&attack());
    r.tick(300_000_000).unwrap();
    assert_eq!((r.health.loss,r.health.unassigned,r.health.allocated),(222,111,111));
    assert_eq!(r.meter.session.estimated_total.damage,111);
    assert_eq!(r.protected_hp_samples.len(),3);
    r.process_replacement(99).unwrap();assert_eq!(r.protected_hp.key_count(),0);
    feed(&mut r,&mut seq,400_000_000,170209,&full);
    assert_eq!(r.diagnostics.0["protected_hp_missing_iv"],1);
    assert_eq!(r.protected_hp_samples.len(),3);
    assert_eq!(r.health.allocated,111);
}

#[test]
fn replay_changed_iv_and_corrupt_scalar_do_not_bridge_hp_intervals(){
    use mabi_meter_core::replay::Replay;
    let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;
    let initial=synthetic_spawn("c71938e787b6c816204a2c56c78a251d",0);
    feed(&mut r,&mut seq,1,20051,&initial);feed(&mut r,&mut seq,2,20051,&initial);
    let old_zero=synthetic_update("478642e2bcbd73ebb6feafa2dd8b6a48");
    feed(&mut r,&mut seq,3,170206,&old_zero);feed(&mut r,&mut seq,4,20526,&attack());
    let changed=synthetic_spawn("220f68c39be88115862c958caffe0ba6",16);
    feed(&mut r,&mut seq,5,20051,&changed);
    assert_eq!(r.protected_hp_seeds.iter().map(|(_,_,_,_,changed)|*changed).collect::<Vec<_>>(),vec![true,false,true]);
    assert_eq!((r.health.loss,r.health.unassigned,r.health.allocated),(111,111,0));
    let four_hundred=synthetic_update("17a59de74d8dc87c118489480568e584");
    feed(&mut r,&mut seq,6,170208,&four_hundred);feed(&mut r,&mut seq,7,20526,&attack());
    let corrupt=synthetic_update("ce26dc2e00f7d307005dbdb02888da2d");
    feed(&mut r,&mut seq,8,170209,&corrupt);
    assert_eq!((r.health.loss,r.health.unassigned,r.health.allocated),(211,211,0));
    assert_eq!(r.diagnostics.0["protected_hp_rejected"],1);
    let fresh_zero=synthetic_update("de94339b5f5a44f7c99bad5f957c9460");
    feed(&mut r,&mut seq,9,170210,&fresh_zero);feed(&mut r,&mut seq,10,20526,&attack());
    r.tick(300_000_000).unwrap();
    assert_eq!((r.health.loss,r.health.unassigned,r.health.allocated),(211,211,0));
    assert_eq!(r.meter.session.estimated_total.damage,0);
    assert_eq!(r.protected_hp.key_count(),1); // Corruption invalidates timing, not an otherwise known IV.
}

#[test]
fn replay_malformed_update_closes_pending_loss_and_requires_a_new_baseline(){
    use mabi_meter_core::replay::Replay;
    let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;
    let seed=synthetic_spawn("c71938e787b6c816204a2c56c78a251d",0);
    let zero=synthetic_update("478642e2bcbd73ebb6feafa2dd8b6a48");
    feed(&mut r,&mut seq,1,20051,&seed);
    feed(&mut r,&mut seq,2,170206,&zero);feed(&mut r,&mut seq,3,20526,&attack());
    let mut malformed=zero.clone();malformed[28]=1;
    feed(&mut r,&mut seq,4,170207,&malformed);
    assert_eq!((r.health.loss,r.health.unassigned,r.health.allocated),(111,111,0));
    assert_eq!(r.protected_hp.key_count(),1);
    let full=synthetic_update("c71938e787b6c816204a2c56c78a251d");
    feed(&mut r,&mut seq,5,170208,&full);
    feed(&mut r,&mut seq,6,170209,&zero);feed(&mut r,&mut seq,7,20526,&attack());
    r.tick(300_000_000).unwrap();
    assert_eq!((r.health.loss,r.health.unassigned,r.health.allocated),(222,111,111));
    assert_eq!(r.meter.session.estimated_total.damage,111);
}

#[test]
fn replay_rejected_seed_discards_prior_iv_for_both_known_and_untrusted_actor_headers(){
    use mabi_meter_core::replay::Replay;
    for untrusted_actor in [false,true]{
        let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;
        let seed=synthetic_spawn("c71938e787b6c816204a2c56c78a251d",0);
        let zero=synthetic_update("478642e2bcbd73ebb6feafa2dd8b6a48");
        feed(&mut r,&mut seq,1,20051,&seed);
        feed(&mut r,&mut seq,2,170206,&zero);feed(&mut r,&mut seq,3,20526,&attack());
        let mut rejected=seed.clone();
        if untrusted_actor{rejected[572]=10;}else{rejected[585]^=1;}
        feed(&mut r,&mut seq,4,20051,&rejected);
        assert_eq!((r.health.loss,r.health.unassigned,r.health.allocated),(111,111,0));
        assert_eq!(r.protected_hp.key_count(),0);
        assert_eq!(r.diagnostics.0["protected_hp_seed_rejected"],1);
        let full=synthetic_update("c71938e787b6c816204a2c56c78a251d");
        feed(&mut r,&mut seq,5,170208,&full);
        assert_eq!(r.diagnostics.0["protected_hp_missing_iv"],1);
        feed(&mut r,&mut seq,6,20051,&seed);
        feed(&mut r,&mut seq,7,170209,&zero);feed(&mut r,&mut seq,8,20526,&attack());
        r.tick(300_000_000).unwrap();
        assert_eq!((r.health.loss,r.health.unassigned,r.health.allocated),(222,111,111));
        assert_eq!(r.meter.session.estimated_total.damage,111);
    }
}

#[test]
#[ignore = "private raid scalar fixture"]
fn private_raid_all_protected_hp_and_independent_explicit_deltas() {
    let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-fixtures/protected-hp-raid-20261106.json");
    let fixture:serde_json::Value=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let p=mabi_meter_core::capture_file::unhex(fixture["spawn"]["payload_hex"].as_str().unwrap()).unwrap();
    let seed=spawn_seed(20051,&p).unwrap().unwrap();
    assert_eq!(seed.initial_hp,54_343_336);
    assert_eq!(seed.initial_hp,mabi_meter_core::protocol::u64le(&p,67));
    assert_eq!(seed.iv_offset,605);
    let target=EntityKey{process:1,binding:1,actor:seed.actor,generation:1};
    let mut decoder=Decoder::default();assert!(decoder.register(17,target,&seed).unwrap());
    let mut previous=seed.initial_hp;let mut explicit_count=0;let mut explicit_sum=0;
    let rows=fixture["updates"].as_array().unwrap();assert_eq!(rows.len(),8162);
    let mut wires=std::collections::BTreeMap::new();
    for row in rows {
        let p=mabi_meter_core::capture_file::unhex(row["payload_hex"].as_str().unwrap()).unwrap();
        let wire=row["wire"].as_i64().unwrap() as i32;*wires.entry(wire).or_insert(0)+=1;
        let hp=decoder.decode(17,target,wire,&p).unwrap();
        assert_eq!(hp,row["expected_hp"].as_u64().unwrap(),"frame {}",row["frame"]);
        let drop=previous.checked_sub(hp).expect("observed boss HP is monotonic");
        assert_eq!(drop,row["expected_drop"].as_u64().unwrap());
        if let Some(amount)=row["adjacent_explicit_amount"].as_u64(){assert_eq!(drop,amount);explicit_count+=1;explicit_sum+=amount;}
        previous=hp;
    }
    assert_eq!(previous,0);assert_eq!(explicit_count,1126);assert_eq!(explicit_sum,12_528_889);
    assert_eq!(wires,[(170206,1682),(170207,1639),(170208,1636),(170209,1604),(170210,1601)].into());
    let p=mabi_meter_core::capture_file::unhex(rows[0]["payload_hex"].as_str().unwrap()).unwrap();
    assert_eq!(decoder.decode(18,target,170207,&p),Err(DecodeError::MissingIv));
    assert_eq!(decoder.decode(17,EntityKey{generation:2,..target},170207,&p),Err(DecodeError::MissingIv));
}
