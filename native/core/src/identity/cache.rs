//! Versioned metadata/receipt storage. Loading never creates a runtime binding.
use super::{Registry,tracking::{Evidence,EvidenceOrigin}};
use crate::Result;
use serde_json::{json,Value};
use std::{collections::BTreeMap,fs::{self,OpenOptions},io::{Read,Write},path::Path};

const FORMAT:&str="mabi-dps-rust-identity-cache-v1";
const MAX_BYTES:usize=16*1024*1024;
#[derive(Debug,Default)]
pub struct Restore {
    pub metadata_records:usize,pub operator_receipt:Option<Value>,
    pub historical_personal:Option<Value>,
}
pub fn export(registry:&Registry)->Value {
    let mut names:BTreeMap<u64,(String,bool)>=registry.profile_names.iter()
        .map(|(uid,m)| (*uid,(m.name.clone(),m.conflicting))).collect();
    for m in registry.records.values() {
        if let (Some(uid),Some(name))=(m.uid,m.name.as_ref()) {
            let entry=names.entry(uid).or_insert((name.clone(),m.conflicting));
            entry.1 |= m.conflicting || &entry.0!=name;
        }
    }
    let personal=registry.tracking.personal.as_ref().map(|c| json!({"uid":c.uid.map(|uid|uid.to_string()),
        "code_server":c.code_server.map(|(code,server)|json!({"code":code,"server":server})),
        "role":"historical metadata; fresh personal evidence required"}));
    let binding=registry.self_binding();
    let operator=if binding.evidence==super::tracking::SelfEvidence::OperatorAssigned {
        binding.key.map(|key| json!({"process":key.process,"binding":key.binding,"actor":key.actor,
            "generation":key.generation,"command":binding.source.and_then(|s|match s.origin {
                EvidenceOrigin::Operator { command }=>Some(command),_=>None }),
            "role":"operator receipt; not a restorable current binding"}))
    } else { None };
    json!({"format":FORMAT,"protocol_profile":crate::protocol::PROFILE,
        "names":names.into_iter().map(|(uid,(name,conflicting))|json!({"uid":uid.to_string(),"name":name,
            "conflicting":conflicting})).collect::<Vec<_>>(),
        "historical_personal":personal,"operator_receipt":operator})
}
pub fn restore(registry:&mut Registry,value:&Value)->Result<Restore> {
    if value["format"].as_str()!=Some(FORMAT) || !matches!(value["protocol_profile"].as_str(),Some(profile) if profile==crate::protocol::PROFILE || profile=="mmh-jp.4-observed-v6" || profile=="mmh-jp.4-observed-v5") {
        return Err("unsupported identity cache format/profile".into());
    }
    let rows=value["names"].as_array().filter(|r|r.len()<=32768).ok_or("invalid identity cache names")?;
    let mut checked=BTreeMap::new();
    for row in rows {
        let uid=row["uid"].as_str().filter(|s|s.len()<=20).ok_or("invalid cache UID")?
            .parse::<u64>().map_err(|_|"invalid cache UID")?;
        if uid==0 { return Err("zero cache UID".into()); }
        let name=row["name"].as_str().ok_or("invalid cache name")?;Registry::check_metadata_name(name)?;
        let conflicting=row["conflicting"].as_bool().ok_or("invalid cache name status")?;
        if checked.insert(uid,(name.to_owned(),conflicting)).is_some() { return Err("duplicate cache UID".into()); }
    }
    if registry.profile_names.len()+checked.keys().filter(|uid|!registry.profile_names.contains_key(uid)).count()>32768 {
        return Err("identity cache directory capacity reached".into());
    }
    let mut loaded=0;
    for (uid,(name,conflicting)) in checked {
        if registry.profile_names.get(&uid).is_some_and(|m|!m.cached) { continue; }
        registry.name_metadata(uid,name,Evidence { frame:0,timestamp_ns:0,origin:EvidenceOrigin::CachedMetadata },true)?;
        registry.profile_names.get_mut(&uid).unwrap().conflicting |= conflicting;
        if conflicting { for m in registry.records.values_mut().filter(|m|m.uid==Some(uid) && m.cached) { m.conflicting=true; } }
        loaded+=1;
    }
    // Receipts/previous personal records are returned as historical diagnostics
    // only. No call to personal(), appearance(), assign() or observe() occurs.
    Ok(Restore { metadata_records:loaded,operator_receipt:value.get("operator_receipt").filter(|v|v.is_object()).cloned(),
        historical_personal:value.get("historical_personal").filter(|v|v.is_object()).cloned() })
}
pub fn load(registry:&mut Registry,path:&Path)->Result<Restore> {
    let mut f=fs::File::open(path).map_err(|e|e.to_string())?;let mut bytes=Vec::new();
    (&mut f).take((MAX_BYTES+1) as u64).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
    if bytes.len()>MAX_BYTES { return Err("identity cache byte limit".into()); }
    let value:Value=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;restore(registry,&value)
}
pub fn save(registry:&Registry,path:&Path)->Result<()> {
    let value=export(registry);
    if value["names"].as_array().is_some_and(|rows|rows.len()>32768) { return Err("identity cache metadata capacity".into()); }
    let bytes=serde_json::to_vec(&value).map_err(|e|e.to_string())?;
    if bytes.len()>MAX_BYTES { return Err("identity cache byte limit".into()); }
    let parent=path.parent().ok_or("identity cache path needs parent")?;
    fs::create_dir_all(parent).map_err(|e|e.to_string())?;
    let nonce=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e|e.to_string())?.as_nanos();
    let temporary=parent.join(format!(".mabi-identity-{}-{nonce}.tmp",std::process::id()));
    let result=(|| {
        let mut f=OpenOptions::new().create_new(true).write(true).open(&temporary).map_err(|e|e.to_string())?;
        f.write_all(&bytes).map_err(|e|e.to_string())?;f.sync_all().map_err(|e|e.to_string())?;drop(f);
        replace(&temporary,path)
    })();
    if result.is_err() { let _=fs::remove_file(&temporary); }result
}
#[cfg(not(windows))]
fn replace(from:&Path,to:&Path)->Result<()> { fs::rename(from,to).map_err(|e|e.to_string()) }
#[cfg(windows)]
fn replace(from:&Path,to:&Path)->Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name="kernel32")]
    extern "system" { fn MoveFileExW(from:*const u16,to:*const u16,flags:u32)->i32; }
    let a:Vec<u16>=from.as_os_str().encode_wide().chain(Some(0)).collect();
    let b:Vec<u16>=to.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileExW(a.as_ptr(),b.as_ptr(),1|8) }==0 { Err(std::io::Error::last_os_error().to_string()) } else { Ok(()) }
}
