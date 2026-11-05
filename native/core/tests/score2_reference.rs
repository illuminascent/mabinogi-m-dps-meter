use mabi_meter_core::{capture_file::unhex,identity::{Character,score2_layout::{parse_reference,walk_reference,self_vote_reference},
    tracking::{Tracking,Evidence,EvidenceOrigin,SelfEvidence,VoteKind},EntityKey}};

fn ev(frame:u64)->Evidence { Evidence { frame,timestamp_ns:frame,origin:EvidenceOrigin::Semantic } }
fn key(actor:u32)->EntityKey { EntityKey { process:1,binding:1,actor,generation:1 } }
fn fixtures()->Vec<(u8,usize,Vec<u8>)> {
    let data:serde_json::Value=serde_json::from_str(include_str!("fixtures/score2-appearance-synthetic.json")).unwrap();
    data["records"].as_array().unwrap().iter().map(|r| (r["variant"].as_u64().unwrap() as u8,
        r["uid_offset"].as_u64().unwrap() as usize,unhex(r["body_hex"].as_str().unwrap()).unwrap())).collect()
}
#[test] fn all_four_reference_layouts_and_truncation_boundaries() {
    let mut rejected=0;
    for (variant,offset,bytes) in fixtures() {
        let a=walk_reference(&bytes,variant).unwrap();
        assert_eq!(a.actor,123);assert_eq!(a.identity.uid,Some(0x1122334455667788));
        assert_eq!(a.identity.code_server,Some(([0x66,0x55,0x44,0x33],0x7788)));
        assert_eq!(a.display_name,"Research");assert_eq!(a.uid_offset,offset);assert!(a.fully_consumed);
        assert_eq!(parse_reference(&bytes).unwrap().variant,variant);
        for end in 0..bytes.len() { assert!(walk_reference(&bytes[..end],variant).is_err(),"variant {variant}, end {end}");rejected+=1; }
    }
    assert_eq!(rejected,7181);
}
#[test] fn malformed_identity_fields_and_partial_tail() {
    for (variant,offset,bytes) in fixtures() {
        for (relative,replacement) in [(0,0u64.to_le_bytes().to_vec()),(8,129u32.to_le_bytes().to_vec()),(12,4097u32.to_le_bytes().to_vec())] {
            let mut bad=bytes.clone();bad[offset+relative..offset+relative+replacement.len()].copy_from_slice(&replacement);
            assert!(walk_reference(&bad,variant).is_err());
        }
        let mut tail=bytes;tail.push(255);let a=walk_reference(&tail,variant).unwrap();
        assert!(!a.fully_consumed);assert_eq!(a.consumed,tail.len()-1);
    }
}
#[test] fn native_personal_code_is_not_display_name_or_zero_extended_uid() {
    let full=0x1122334455667788u64;
    let personal=Character::score2_personal_prefix(&full.to_le_bytes()[..6]).unwrap();
    assert_eq!(personal.uid,None);assert_eq!(personal.name_server,None);
    assert_eq!(personal.code_server,Some(([0x66,0x55,0x44,0x33],0x7788)));
    let appearance=Character::score2_appearance_uid(full).unwrap();
    let mut t=Tracking::default();t.personal(personal.clone(),ev(1)).unwrap();
    t.appearance(key(123),appearance,ev(2)).unwrap();assert_eq!(t.binding().key,Some(key(123)));
    assert_eq!(t.binding().evidence,SelfEvidence::PacketConfirmed);assert_eq!(t.personal.as_ref().unwrap().uid,Some(full));
    t.personal(personal,ev(3)).unwrap();assert_eq!(t.personal.as_ref().unwrap().uid,Some(full));
    assert!(Character::score2_personal_prefix(&[0;5]).is_err());
    // Native permits all server u16 values, including zero; do not invent<=64.
    assert_eq!(Character::score2_personal_prefix(&[0;6]).unwrap().code_server,Some(([0;4],0)));
}
#[test] fn code_conflict_never_falls_back_to_equal_human_names() {
    let a=Character { uid:None,code_server:Some(([1,2,3,4],7)),name_server:Some(("Same".into(),7)) };
    let b=Character { code_server:Some(([5,6,7,8],7)),..a.clone() };
    assert!(!a.matches(&b));
}
#[test] fn distinct_full_uids_do_not_join_through_equal_code() {
    let a=Character::score2_appearance_uid(0x1122334455667788).unwrap();
    let b=Character::score2_appearance_uid(0x9999334455667788).unwrap();
    assert_eq!(a.code_server,b.code_server);assert!(!a.matches(&b));
}
#[test] fn weak_bridge_cannot_merge_conflicting_known_uids() {
    let mut t=Tracking::default();
    let named=|uid| Character { uid,code_server:None,name_server:Some(("Same".into(),7)) };
    t.appearance(key(10),named(Some(1)),ev(1)).unwrap();t.retire(key(10),ev(2)).unwrap();
    t.appearance(key(20),named(None),ev(3)).unwrap();assert_eq!(t.group(key(10)),t.group(key(20)));
    t.retire(key(20),ev(4)).unwrap();t.appearance(key(30),named(Some(2)),ev(5)).unwrap();
    assert_ne!(t.group(key(10)),t.group(key(30)));assert_ne!(t.group(key(10)),t.group(key(20)));
}
#[test] fn nonconsecutive_votes_lock_but_primary_can_supersede() {
    let mut t=Tracking::default();
    for (frame,actor) in [(1,10),(2,20),(3,10),(4,20)] { t.vote(key(actor),VoteKind::SelfCode,ev(frame)).unwrap(); }
    assert_eq!(t.binding().key,Some(key(10)));assert!(t.binding().voter_locked);
    let c=Character::score2_appearance_uid(0x1122334455667788).unwrap();
    t.personal(Character::score2_personal_prefix(&0x1122334455667788u64.to_le_bytes()[..6]).unwrap(),ev(5)).unwrap();
    t.appearance(key(30),c,ev(6)).unwrap();assert_eq!(t.binding().key,Some(key(30)));assert!(t.binding().primary_locked);
    assert_eq!(self_vote_reference(&1u32.to_le_bytes()).unwrap(),1);
    assert!(self_vote_reference(&0u32.to_le_bytes()).is_err());assert!(self_vote_reference(&[1;3]).is_err());
}
#[test] fn cached_metadata_cannot_assert_self_or_restore_runtime_key() {
    let mut t=Tracking::default();let cached=Evidence { origin:EvidenceOrigin::CachedMetadata,..ev(1) };
    let c=Character::score2_appearance_uid(0x1122334455667788).unwrap();
    assert!(t.personal(c.clone(),cached).is_err());assert!(t.appearance(key(10),c,cached).is_err());
    assert!(t.vote(key(10),VoteKind::SelfCode,cached).is_err());assert!(t.assign(Some(key(10)),cached).is_err());
    assert_eq!(t.binding().key,None);
}
