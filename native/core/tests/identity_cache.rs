use mabi_meter_core::{identity::{Registry,Character,cache,tracking::{Evidence,EvidenceOrigin,SelfEvidence}},
    meter::{Action,Command},replay,session::Session};
use std::path::PathBuf;

struct CacheFile(PathBuf);
impl CacheFile {
    fn new()->Self {
        let n=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        Self(std::env::temp_dir().join(format!("mabi-rust-identity-{}-{n}.json",std::process::id())))
    }
}
impl Drop for CacheFile { fn drop(&mut self) { let _=std::fs::remove_file(&self.0); } }
fn ev(frame:u64)->Evidence { Evidence { frame,timestamp_ns:frame,origin:EvidenceOrigin::Semantic } }
fn c()->Character { Character::score2_appearance_uid(0x1122334455667788).unwrap() }
fn prefix()->Character { Character::score2_personal_prefix(&0x1122334455667788u64.to_le_bytes()[..6]).unwrap() }

#[test] fn persisted_names_and_operator_receipt_never_recreate_runtime_self() {
    let file=CacheFile::new();let mut old=Registry::default();let key=old.observe(1,1,10).unwrap();
    old.personal_evidence(prefix(),ev(1)).unwrap();
    old.appearance_named_evidence(key,c(),Some("Example".into()),ev(2)).unwrap();old.assign_command(Some(key),7).unwrap();
    cache::save(&old,&file.0).unwrap();let mut fresh=Registry::default();let restored=cache::load(&mut fresh,&file.0).unwrap();
    assert_eq!(restored.metadata_records,1);assert!(restored.operator_receipt.is_some());assert!(restored.historical_personal.is_some());
    assert!(fresh.records.is_empty());assert_eq!(fresh.self_key,None);assert!(fresh.tracking.personal.is_none());
    let coincident=fresh.observe(1,1,10).unwrap();assert_eq!(coincident,key);
    assert_eq!(fresh.self_key,None);assert!(fresh.label(coincident).starts_with("Name pending"));
    fresh.appearance_evidence(coincident,c(),ev(10)).unwrap();assert!(fresh.label(coincident).starts_with("Example"));
    assert!(fresh.records[&coincident].cached);assert_eq!(fresh.self_key,None);
    fresh.personal_evidence(prefix(),ev(11)).unwrap();assert_eq!(fresh.self_key,Some(coincident));
    assert_eq!(fresh.self_binding().evidence,SelfEvidence::PacketConfirmed);
}
#[test] fn accepted_command_snapshot_row_and_atomic_receipt_agree() {
    let file=CacheFile::new();let mut session=Session::new(replay::demo().unwrap(),Some(file.0.clone()));
    let selected=session.replay.meter.snapshot().rows[0].key;
    let ack=session.command(Command { id:7,observed_revision:0,encounter:1,action:Action::AssignSelf(Some(selected)) });
    assert!(ack.applied);assert!(ack.persistence.unwrap().is_ok());assert_eq!(ack.snapshot.last_command,7);
    assert_eq!(ack.snapshot.self_binding.key,Some(selected));assert!(ack.snapshot.rows.iter().any(|r|r.is_self && r.label.starts_with("<ME>")));
    let value:serde_json::Value=serde_json::from_slice(&std::fs::read(&file.0).unwrap()).unwrap();
    assert_eq!(value["operator_receipt"]["actor"].as_u64(),Some(selected.actor as u64));
    assert_eq!(value["operator_receipt"]["command"].as_u64(),Some(7));
    let clear=session.command(Command { id:8,observed_revision:0,encounter:1,action:Action::AssignSelf(None) });
    assert!(clear.applied);assert!(clear.persistence.unwrap().is_ok());assert_eq!(clear.snapshot.self_binding.key,None);
    let value:serde_json::Value=serde_json::from_slice(&std::fs::read(&file.0).unwrap()).unwrap();
    assert!(value["operator_receipt"].is_null());
}
#[test] fn previous_profile_caches_restore_only_metadata_and_never_participant_handles(){
    let mut old=Registry::default();let key=old.observe(1,1,10).unwrap();
    old.personal_evidence(prefix(),ev(1)).unwrap();old.appearance_named_evidence(key,c(),Some("Example".into()),ev(2)).unwrap();old.assign(Some(key)).unwrap();
    for profile in ["mmh-jp.4-observed-v5","mmh-jp.4-observed-v6","mmh-jp.4-observed-v7"]{
        let mut value=cache::export(&old);value["protocol_profile"]=serde_json::json!(profile);let mut fresh=Registry::default();
        assert_eq!(cache::restore(&mut fresh,&value).unwrap().metadata_records,1);assert!(fresh.records.is_empty());assert_eq!(fresh.participant(key),None);assert_eq!(fresh.self_key,None);
    }
}
#[test] fn save_failure_is_reported_without_undoing_accepted_command() {
    let parent=CacheFile::new();std::fs::write(&parent.0,b"regular file, not a directory").unwrap();
    let mut session=Session::new(replay::demo().unwrap(),Some(parent.0.join("child.json")));
    let selected=session.replay.meter.snapshot().rows[0].key;
    let ack=session.command(Command { id:1,observed_revision:0,encounter:1,action:Action::AssignSelf(Some(selected)) });
    assert!(ack.applied);assert!(ack.reason.is_none());assert!(ack.persistence.unwrap().is_err());
    assert_eq!(ack.snapshot.self_binding.key,Some(selected));assert_eq!(session.replay.meter.registry.self_key,Some(selected));
}
#[test] fn invalid_cache_is_rejected_before_loading_any_record() {
    let mut value=cache::export(&Registry::default());
    value["names"]=serde_json::json!([{"uid":"1","name":"Valid","conflicting":false},
        {"uid":"2","name":"\n","conflicting":false}]);
    let mut r=Registry::default();assert!(cache::restore(&mut r,&value).is_err());assert!(r.profile_names.is_empty());
    value["names"]=serde_json::json!([]);value["protocol_profile"]=serde_json::json!("old-profile");
    assert!(cache::restore(&mut r,&value).is_err());
}
#[test] fn stale_cache_name_cannot_poison_fresh_packet_name() {
    let mut cache_value=cache::export(&Registry::default());
    cache_value["names"]=serde_json::json!([{"uid":c().uid.unwrap().to_string(),"name":"Old","conflicting":false}]);
    let mut r=Registry::default();let key=r.observe(1,1,10).unwrap();
    r.appearance_named_evidence(key,c(),Some("Fresh".into()),ev(1)).unwrap();
    cache::restore(&mut r,&cache_value).unwrap();assert!(r.label(key).starts_with("Fresh"));assert!(!r.records[&key].conflicting);
    r.appearance_named_evidence(key,c(),Some("Fresh".into()),ev(2)).unwrap();assert!(!r.records[&key].conflicting);
}
#[test] fn conflicting_cached_name_does_not_become_a_display_label() {
    let mut cache_value=cache::export(&Registry::default());
    cache_value["names"]=serde_json::json!([{"uid":c().uid.unwrap().to_string(),"name":"Uncertain","conflicting":true}]);
    let mut r=Registry::default();cache::restore(&mut r,&cache_value).unwrap();let key=r.observe(1,1,10).unwrap();
    r.appearance_evidence(key,c(),ev(1)).unwrap();assert!(r.label(key).starts_with("Name pending"));
}
#[test] fn fresh_metadata_replaces_old_cached_label_without_a_false_conflict() {
    let mut value=cache::export(&Registry::default());
    value["names"]=serde_json::json!([{"uid":c().uid.unwrap().to_string(),"name":"Old","conflicting":false}]);
    let mut r=Registry::default();cache::restore(&mut r,&value).unwrap();let key=r.observe(1,1,10).unwrap();
    r.appearance_evidence(key,c(),ev(1)).unwrap();assert!(r.records[&key].cached);
    r.name_metadata(c().uid.unwrap(),"Fresh".into(),ev(2),false).unwrap();
    assert!(r.label(key).starts_with("Fresh"));assert!(!r.records[&key].cached);assert!(!r.records[&key].conflicting);
}
#[test] fn rejected_stale_command_does_not_create_a_cache_receipt() {
    let file=CacheFile::new();let mut session=Session::new(replay::demo().unwrap(),Some(file.0.clone()));
    let ack=session.command(Command { id:1,observed_revision:0,encounter:999,action:Action::Reset });
    assert!(!ack.applied);assert!(ack.persistence.is_none());assert!(!file.0.exists());
}
