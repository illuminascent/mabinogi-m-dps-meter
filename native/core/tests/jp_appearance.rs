use mabi_meter_core::{capture_file::unhex,identity::score2_layout::parse_jp_observed};

#[test] fn synthetic_jp_optional_branches_and_observed_gate_rejections() {
    let rows:serde_json::Value=serde_json::from_str(include_str!("fixtures/jp-appearance-synthetic.json")).unwrap();
    for row in rows["records"].as_array().unwrap() {
        let data=unhex(row["body_hex"].as_str().unwrap()).unwrap();let a=parse_jp_observed(&data).unwrap();
        assert_eq!(a.display_name,"JP\u{30fb}Dummy");assert_eq!(a.actor,123);assert!(a.fully_consumed);
        for end in 0..data.len() { assert!(parse_jp_observed(&data[..end]).is_err()); }
        let mut tail=data.clone();tail.push(1);assert!(parse_jp_observed(&tail).is_err());
        for mutation in row["gate_mutations"].as_array().unwrap() {
            let offset=mutation["offset"].as_u64().unwrap() as usize;let mut bad=data.clone();
            if let Some(xor)=mutation["xor_byte"].as_u64() {bad[offset]^=xor as u8;}
            else {let bytes=unhex(mutation["replace_hex"].as_str().unwrap()).unwrap();bad[offset..offset+bytes.len()].copy_from_slice(&bytes);}
            assert!(parse_jp_observed(&bad).is_err());
        }
        for server in [1u32,2,3,128] {
            let mut supported=data.clone();supported[a.uid_offset+8..a.uid_offset+12].copy_from_slice(&server.to_le_bytes());
            assert!(parse_jp_observed(&supported).is_ok());
        }
    }
}

#[test] fn jp_wire_appearance_can_reconcile_both_orders_without_claiming_personal_wire_binding() {
    use mabi_meter_core::{capture_file::Input,identity::{Character,tracking::{Evidence,EvidenceOrigin}},protocol,replay::Replay,transport::{Flow,Segment}};
    let rows:serde_json::Value=serde_json::from_str(include_str!("fixtures/jp-appearance-synthetic.json")).unwrap();
    let data=unhex(rows["records"][0]["body_hex"].as_str().unwrap()).unwrap();
    let identity=Character::score2_personal_prefix(&0x1122334455667788u64.to_le_bytes()[..6]).unwrap();
    for personal_first in [false,true] {
        let mut replay=Replay::new(true);let ev=Evidence {frame:1,timestamp_ns:1,origin:EvidenceOrigin::Semantic};
        if personal_first {replay.meter.registry.personal_evidence(identity.clone(),ev).unwrap();}
        replay.feed(Input::Segment(Segment {seq:1,payload:protocol::encode_frame(20054,&data),timestamp_ns:2,
            flow:Flow {src:"192.0.2.1".into(),sport:1234,dst:"192.0.2.2".into(),dport:2345},
            syn:false,fin:false,rst:false,ack:true})).unwrap();
        assert_eq!(replay.diagnostics.0["structured_player_appearance"],1);
        if !personal_first {assert_eq!(replay.meter.registry.self_key,None);replay.meter.registry.personal_evidence(identity.clone(),ev).unwrap();}
        assert_eq!(replay.meter.registry.self_key.unwrap().actor,123);assert!(replay.meter.snapshot().self_binding.primary_locked);
        assert_eq!(replay.meter.snapshot().summary.damage,0);
    }
}

#[test] #[ignore="requires private saved JP appearance corpus"]
fn structured_jp_corpus_exact_identity_full_consumption_and_truncation() {
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prototypes/static-analysis");
    let rows:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("jp-appearance-native-fixture.json")).unwrap()).unwrap();
    assert_eq!(rows.as_array().unwrap().len(),62);let mut prefixes=0;
    for row in rows.as_array().unwrap() {
        let data=unhex(row["payload_hex"].as_str().unwrap()).unwrap();let known=&row["known"];
        let a=parse_jp_observed(&data).unwrap();assert!(a.fully_consumed);assert_eq!(a.consumed,data.len());
        assert_eq!(a.actor as u64,known["actor"].as_u64().unwrap());
        assert_eq!(a.identity.uid,known["uid"].as_u64());assert_eq!(a.display_name,known["name"].as_str().unwrap());
        assert_eq!(a.uid_offset as u64,known["source_offset"].as_u64().unwrap());
        for end in 0..data.len() {assert!(parse_jp_observed(&data[..end]).is_err());prefixes+=1;}
        let mut tail=data.clone();tail.push(0);assert!(parse_jp_observed(&tail).is_err());
        for (offset,replacement) in [(0,vec![0;8]),(a.uid_offset,vec![0;8]),
            (a.uid_offset+8,129u32.to_le_bytes().to_vec()),(a.uid_offset+12,4097u32.to_le_bytes().to_vec()),
            (423,f32::NAN.to_le_bytes().to_vec()),(512,4097u32.to_le_bytes().to_vec())] {
            let mut bad=data.clone();bad[offset..offset+replacement.len()].copy_from_slice(&replacement);
            assert!(parse_jp_observed(&bad).is_err());
        }
    }
    assert_eq!(prefixes,212598);
}
