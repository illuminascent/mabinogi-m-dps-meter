use mabi_meter_core::{identity::{self,Character,Registry,tracking::{Evidence,EvidenceOrigin}},
    transport::Framer,protocol::encode_frame,Diagnostics};
fn evidence(frame:u64)->Evidence { Evidence { frame,timestamp_ns:frame,origin:EvidenceOrigin::Packet { wire:170631 } } }
fn record(leading:u64,server:u16,name:&str)->Vec<u8> {
    let uid=(leading<<20)|server as u64;
    let utf16:Vec<u8>=name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut p=leading.to_le_bytes().to_vec();p.extend(uid.to_le_bytes());p.extend(uid.to_le_bytes());
    p.extend((utf16.len() as u32).to_le_bytes());p.extend(utf16);p.extend([0;8]);p
}
#[test] fn bounded_metadata_prefix_and_opaque_tail() {
    let mut p=record(1234,7,"テスト");let n=identity::name_record(&p).unwrap();
    assert_eq!(n.uid,(1234<<20)|7);assert_eq!(n.leading_reference,1234);assert_eq!(n.name,"テスト");
    p.truncate(n.name_end);assert!(identity::name_record(&p).is_some());
    p.pop();assert!(identity::name_record(&p).is_none());
    let mut bad=record(1234,7,"Test");bad[16]^=1;assert!(identity::name_record(&bad).is_none());
    let mut bad=record(1234,7,"Test");bad[0]^=1;assert!(identity::name_record(&bad).is_none());
    let mut bad=record(1234,7,"Test");bad[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(identity::name_record(&bad).is_none());
    assert!(identity::name_record(&record(1234,0,"\n")).is_none());
    let mut bad=record(1234,7,"Test");bad[28..30].copy_from_slice(&0xd800u16.to_le_bytes());
    assert!(identity::name_record(&bad).is_none());
}
#[test] fn metadata_then_appearance_or_appearance_then_metadata() {
    let packet=identity::name_record(&record(1234,7,"Example")).unwrap();
    for before in [false,true] {
        let mut r=Registry::default();let key=r.observe(1,1,999).unwrap();
        if before { r.name_metadata(packet.uid,packet.name.clone(),evidence(1),false).unwrap(); }
        r.appearance_evidence(key,Character::score2_appearance_uid(packet.uid).unwrap(),evidence(2)).unwrap();
        if !before { r.name_metadata(packet.uid,packet.name.clone(),evidence(3),false).unwrap(); }
        assert!(r.label(key).starts_with("Example"));assert_eq!(r.self_key,None);
    }
}
#[test] fn leading_reference_is_never_a_combat_join_or_self_evidence() {
    let n=identity::name_record(&record(1234,7,"Example")).unwrap();
    let mut r=Registry::default();let key=r.observe(1,1,1234).unwrap();
    r.name_metadata(n.uid,n.name,evidence(1),false).unwrap();
    assert!(r.label(key).starts_with("Name pending"));assert_eq!(r.self_key,None);
    assert!(r.tracking.personal.is_none());assert!(r.tracking.appearances.is_empty());
}
#[test] fn metadata_cannot_establish_tcp_alignment() {
    let bytes=encode_frame(170631,&record(1234,7,"Example"));
    let mut f=Framer::default();let mut d=Diagnostics::default();
    assert!(f.feed(&bytes,&mut d).unwrap().is_empty());assert!(!f.aligned);
}
#[test] fn conflicting_uid_names_stay_unresolved() {
    let n=identity::name_record(&record(1234,7,"Example")).unwrap();
    let mut r=Registry::default();let key=r.observe(1,1,999).unwrap();
    r.appearance_evidence(key,Character::score2_appearance_uid(n.uid).unwrap(),evidence(1)).unwrap();
    r.name_metadata(n.uid,n.name,evidence(2),false).unwrap();
    r.name_metadata(n.uid,"Conflicting".into(),evidence(3),false).unwrap();
    assert!(r.records[&key].conflicting);assert!(r.label(key).starts_with("Name pending"));
}
