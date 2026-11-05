//! Verified JP name adapters and a wire-independent reconciliation model.
use crate::{protocol::{u32le, u64le}, Result};
use std::{collections::BTreeMap, sync::OnceLock};
pub mod tracking;
pub mod score2_layout;
pub mod cache;
use tracking::{Evidence,EvidenceOrigin,SelfBinding,Tracking,VoteKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityKey {
    pub process: u64, pub binding: u64, pub actor: u32, pub generation: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Character {
    pub uid: Option<u64>,
    /// Score2 compares four identity-code bytes plus a u16 component.
    /// MMH calls that u16 the character index; its displayed server/realm is
    /// a separate appearance field. Neither is the human-readable player name.
    pub code_server: Option<([u8;4],u16)>,
    /// Additional verified exact-name identity, when a regional adapter supplies it.
    pub name_server: Option<(String, u16)>,
}
impl Character {
    pub fn score2_personal_prefix(p: &[u8]) -> Result<Self> {
        if p.len() < 6 { return Err("short personal identity prefix".into()); }
        Ok(Self { uid:None,code_server:Some((p[2..6].try_into().unwrap(),u16::from_le_bytes([p[0],p[1]]))),name_server:None })
    }
    pub fn score2_appearance_uid(uid: u64) -> Result<Self> {
        if uid == 0 { return Err("zero appearance UID".into()); }
        let b = uid.to_le_bytes();
        Ok(Self { uid:Some(uid),code_server:Some((b[2..6].try_into().unwrap(),u16::from_le_bytes([b[0],b[1]]))),name_server:None })
    }
    pub fn matches(&self, other: &Self) -> bool {
        match (self.uid, other.uid) {
            (Some(a), Some(b)) => a != 0 && a == b,
            _ if self.code_server.is_some() && other.code_server.is_some() => self.code_server == other.code_server,
            _ => matches!((&self.name_server, &other.name_server),
                (Some((a, sa)), Some((b, sb))) if !a.is_empty() && a == b && sa == sb),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Appearance {
    pub actor: u32, pub name: String, pub uid: Option<u64>, pub template: Option<String>,
    pub offset: usize, pub record_reference: Option<u64>,
}

/// JP170631 contains a bounded name/UID prefix. Its role and opaque tail are
/// unresolved; it never identifies which character is local and is not an anchor.
#[derive(Clone,Debug)]
pub struct NameRecord { pub uid:u64,pub leading_reference:u64,pub name:String,pub name_end:usize }
pub fn name_record(p:&[u8]) -> Option<NameRecord> {
    if !(30..=65536).contains(&p.len()) { return None; }
    let leading_reference=u64le(p,0);let uid=u64le(p,8);
    if uid==0 || leading_reference==0 || u64le(p,16)!=uid { return None; }
    if leading_reference.checked_mul(1<<20)? | (uid&65535) != uid { return None; }
    let n=u32le(p,24) as usize;
    if !(2..=64).contains(&n) || n%2!=0 || 28+n>p.len() { return None; }
    let utf16:Vec<u16>=p[28..28+n].chunks_exact(2).map(|s| u16::from_le_bytes([s[0],s[1]])).collect();
    let name=String::from_utf16(&utf16).ok()?;
    if name.trim().is_empty() || name.chars().any(char::is_control) { return None; }
    Some(NameRecord { uid,leading_reference,name,name_end:28+n })
}

#[derive(Clone,Debug)]
pub struct NameMetadata { pub name:String,pub conflicting:bool,pub cached:bool,pub source:Evidence }
pub fn monsters() -> &'static BTreeMap<String, String> {
    static TABLE: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut tables: BTreeMap<String, serde_json::Value> = serde_json::from_str(include_str!(
            "../../../assets/data/combat-names.json")).expect("embedded names");
        serde_json::from_value(tables.remove("Monsters").expect("Monsters table")).expect("monster strings")
    })
}
pub fn player(p: &[u8]) -> Option<Appearance> {
    if p.len() < 40 || u32le(p, 0) == 0 { return None; }
    let mut candidates = BTreeMap::new();
    for n in 8..p.len()-31 {
        let (uid, kind, len) = (u64le(p, n), u32le(p, n+8), u32le(p, n+12) as usize);
        if uid == 0 || !(1..=128).contains(&kind) || !(2..=64).contains(&len) || len%2 != 0
            || n+28+len > p.len() { continue; }
        if !(1_500_000_000_000..=2_200_000_000_000).contains(&u64le(p, n+16+len)) { continue; }
        let utf16: Vec<u16> = p[n+16..n+16+len].chunks_exact(2)
            .map(|s| u16::from_le_bytes([s[0], s[1]])).collect();
        let Ok(name) = String::from_utf16(&utf16) else { continue; };
        if name.trim().is_empty() || name.chars().any(char::is_control) { continue; }
        candidates.insert(name.clone(), Appearance { actor: u32le(p, 0), name,
            uid: Some(uid), template: None, offset: n, record_reference: None });
    }
    (candidates.len() == 1).then(|| candidates.into_values().next().unwrap())
}
pub fn npc(p: &[u8]) -> Option<Appearance> {
    if p.len() < 28 || u32le(p, 8) == 0 { return None; }
    let mut candidates = BTreeMap::new();
    for n in (16..=p.len()-12).rev() {
        if p[n..n+4] != [3,0,0,0] || p[n+8..n+12] != [0,0,0,0] { continue; }
        let template = p[n+4..n+8].iter().map(|b| format!("{b:02X}")).collect::<String>();
        if let Some(name) = monsters().get(&template) {
            candidates.entry(template.clone()).or_insert_with(|| Appearance { actor: u32le(p, 8),
                name: name.clone(), uid: None, template: Some(template), offset: n,
                record_reference: Some(u64le(p, 0)) });
        }
    }
    (candidates.len() == 1).then(|| candidates.into_values().next().unwrap())
}
pub fn parse(wire: i32, p: &[u8]) -> Option<Appearance> {
    match wire { 20054 => player(p), 20517 => npc(p),20051=>spawn(p), _ => None }
}
pub fn personal_prefix(p:&[u8])->Option<Character> {
    if !(64..=crate::protocol::MAX_DECODED).contains(&p.len()) || u16::from_le_bytes([p[0],p[1]])>64 || u32le(p,2)==0 { return None; }
    Character::score2_personal_prefix(p).ok()
}
pub fn spawn(p:&[u8])->Option<Appearance> {
    if p.len()<110 || u32le(p,0)==0 || u64le(p,p.len()-110)!=u64::MAX { return None; }
    let n=p.len()-102;
    if u32le(p,n)!=3 || u32le(p,n+8)!=0 { return None; }
    let template=p[n+4..n+8].iter().map(|b|format!("{b:02X}")).collect::<String>();
    Some(Appearance { actor:u32le(p,0),name:monsters().get(&template)?.clone(),uid:None,
        template:Some(template),offset:n,record_reference:None })
}

#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub name: Option<String>, pub uid: Option<u64>, pub template: Option<String>,
    pub kind: Option<&'static str>,
    pub cached: bool, pub conflicting: bool,
}

#[derive(Default, Debug)]
pub struct Registry {
    pub revision: u64,
    pub records: BTreeMap<EntityKey, Metadata>,
    current: BTreeMap<(u64,u64,u32), EntityKey>,
    generations: BTreeMap<(u64,u64,u32),u32>,
    pub self_key: Option<EntityKey>,
    pub operator_assigned: bool,
    pub tracking: Tracking,
    pub profile_names:BTreeMap<u64,NameMetadata>,
}
impl Registry {
    pub fn observe(&mut self, process: u64, binding: u64, actor: u32) -> Result<EntityKey> {
        if actor == 0 { return Err("zero entity ID".into()); }
        if let Some(k) = self.current.get(&(process,binding,actor)) { return Ok(*k); }
        if self.records.len() >= 32768 { return Err("entity capacity reached".into()); }
        let generation = self.generations.get(&(process,binding,actor)).copied().unwrap_or(0).checked_add(1)
            .ok_or("entity generation overflow")?;
        let key = EntityKey { process, binding, actor, generation };
        self.records.insert(key, Metadata::default()); self.current.insert((process,binding,actor), key);
        self.generations.insert((process,binding,actor),generation);
        self.revision += 1; Ok(key)
    }
    pub fn is_current(&self, key: EntityKey) -> bool {
        self.current.get(&(key.process,key.binding,key.actor)) == Some(&key)
    }
    pub fn appearance(&mut self, process: u64, binding: u64, a: Appearance) -> Result<EntityKey> {
        let mut key = self.observe(process,binding,a.actor)?;
        let old = &self.records[&key];
        let kind = if a.template.is_some() { Some("npc") } else if a.uid.is_some() { Some("player") } else { None };
        if old.uid.zip(a.uid).is_some_and(|(x,y)| x != y)
            || old.template.as_ref().zip(a.template.as_ref()).is_some_and(|(x,y)| x != y)
            || old.kind.zip(kind).is_some_and(|(x,y)| x != y) {
            self.retire(key)?; key = self.observe(process,binding,a.actor)?;
        }
        let meta = self.records.get_mut(&key).unwrap();
        if meta.name.as_ref().is_some_and(|n| n != &a.name) { meta.conflicting = true; }
        if !meta.conflicting { meta.name = Some(a.name); }
        meta.uid = a.uid.or(meta.uid); meta.template = a.template.or(meta.template.take());
        meta.kind = kind.or(meta.kind);
        if let Some(profile)=meta.uid.and_then(|uid| self.profile_names.get(&uid)) {
            if !profile.cached && (profile.conflicting || meta.name.as_ref().is_some_and(|name| name!=&profile.name)) { meta.conflicting=true; }
        }
        meta.cached = false; self.revision += 1; Ok(key)
    }
    pub fn retire(&mut self, key: EntityKey) -> Result<()> {
        if self.is_current(key) { self.current.remove(&(key.process,key.binding,key.actor)); }
        self.tracking.retire(key,self.semantic_source())?;
        self.sync_self(); self.revision += 1; Ok(())
    }
    pub fn cache_metadata(&mut self) {
        for v in self.records.values_mut() { v.cached = true; } self.revision += 1;
    }
    pub fn clear_current(&mut self) {
        self.current.clear(); self.tracking.clear_bindings(self.semantic_source());
        self.sync_self(); self.cache_metadata();
    }
    pub fn clear_character(&mut self) {
        self.tracking.clear_character(self.semantic_source()); self.clear_current();
    }
    /// Typed semantic input only: JP UID/server adapter is still disabled.
    pub fn character(&mut self, identity: Character) -> Result<()> {
        self.personal_evidence(identity,self.semantic_source())
    }
    pub fn personal_evidence(&mut self, identity: Character, source: Evidence) -> Result<()> {
        self.tracking.personal(identity,source)?; self.sync_self(); self.revision += 1; Ok(())
    }
    pub fn wire_personal(&mut self,process:u64,binding:u64,identity:Character,source:Evidence)->Result<()> {
        self.tracking.wire_personal(process,binding,identity,source)?;self.sync_self();self.revision+=1;Ok(())
    }
    pub fn verified_appearance(&mut self, key: EntityKey, identity: Character) -> Result<()> {
        self.appearance_evidence(key,identity,self.semantic_source())
    }
    pub fn appearance_evidence(&mut self, key: EntityKey, identity: Character, source: Evidence) -> Result<()> {
        if !self.is_current(key) { return Err("appearance is not current".into()); }
        if self.records[&key].uid.zip(identity.uid).is_some_and(|(a,b)| a != b) {
            return Err("verified UID requires a replacement incarnation".into());
        }
        self.tracking.appearance(key,identity.clone(),source)?;
        let meta = self.records.get_mut(&key).unwrap();
        if let Some((name,_)) = identity.name_server { meta.name = Some(name); meta.conflicting = false; }
        meta.uid = identity.uid.or(meta.uid); meta.cached = false; meta.kind = Some("player");
        if let Some(profile)=meta.uid.and_then(|uid| self.profile_names.get(&uid)) {
            if !profile.cached && (profile.conflicting || meta.name.as_ref().is_some_and(|name| name!=&profile.name)) { meta.conflicting=true; }
            else if meta.name.is_none() && !profile.conflicting { meta.name=Some(profile.name.clone());meta.cached=profile.cached; }
        }
        self.sync_self(); self.revision += 1; Ok(())
    }
    pub fn appearance_named_evidence(&mut self,key:EntityKey,identity:Character,name:Option<String>,source:Evidence) -> Result<()> {
        if let Some(name)=&name { Self::check_metadata_name(name)?; }
        self.appearance_evidence(key,identity,source)?;
        if let Some(name)=name {
            let meta=self.records.get_mut(&key).unwrap();
            if !meta.cached && meta.name.as_ref().is_some_and(|old|old!=&name) { meta.conflicting=true; }
            if !meta.conflicting { meta.name=Some(name); }meta.cached=false;
        }
        Ok(())
    }
    pub fn wire_appearance_context(&mut self,key:EntityKey,identity:Character,source:Evidence,context:(u64,u64))->Result<()> {
        self.tracking.wire_appearance(key,identity,source,context)?;self.sync_self();self.revision+=1;Ok(())
    }
    fn check_metadata_name(name:&str) -> Result<()> {
        if name.trim().is_empty() || name.encode_utf16().count()>128 || name.chars().any(char::is_control) {
            Err("invalid player metadata name".into())
        } else { Ok(()) }
    }
    /// This enriches matching metadata only. No raw actor ID, self binding,
    /// voter, incarnation, encounter or damage state is changed.
    pub fn name_metadata(&mut self,uid:u64,name:String,source:Evidence,cached:bool) -> Result<()> {
        if uid==0 { return Err("zero name metadata UID".into()); }Self::check_metadata_name(&name)?;
        if !self.profile_names.contains_key(&uid) && self.profile_names.len()>=32768 {
            return Err("name metadata directory capacity reached".into());
        }
        let profile=self.profile_names.entry(uid).or_insert(NameMetadata { name:name.clone(),conflicting:false,cached,source });
        if profile.cached && !cached { profile.name=name.clone();profile.conflicting=false; }
        else { profile.conflicting |= profile.name!=name; }
        profile.source=source;profile.cached=cached;
        for meta in self.records.values_mut().filter(|m| m.uid==Some(uid)) {
            if cached && meta.name.is_some() && !meta.cached { continue; }
            if profile.conflicting || (!meta.cached && meta.name.as_ref().is_some_and(|old| old!=&name)) { meta.conflicting=true; }
            else { meta.name=Some(name.clone());meta.cached=cached; }
        }
        self.revision+=1;Ok(())
    }
    pub fn verified_self_vote(&mut self, key: EntityKey, kind: VoteKind, source: Evidence) -> Result<()> {
        if !self.is_current(key) { return Err("stale self vote entity".into()); }
        self.tracking.vote(key,kind,source)?; self.sync_self(); self.revision += 1; Ok(())
    }
    fn semantic_source(&self) -> Evidence { Evidence { frame:self.revision+1,timestamp_ns:0,origin:EvidenceOrigin::Semantic } }
    fn sync_self(&mut self) {
        let binding = self.tracking.binding(); self.self_key = binding.key;
        self.operator_assigned = binding.evidence == tracking::SelfEvidence::OperatorAssigned;
    }
    pub fn assign(&mut self, key: Option<EntityKey>) -> Result<()> {
        self.assign_command(key,0)
    }
    pub fn assign_command(&mut self, key: Option<EntityKey>, command: u64) -> Result<()> {
        if key.is_some_and(|k| !self.is_current(k)) { return Err("stale self key".into()); }
        self.tracking.assign(key,Evidence { frame:0,timestamp_ns:0,origin:EvidenceOrigin::Operator { command } })?;
        self.sync_self(); self.revision += 1; Ok(())
    }
    pub fn self_binding(&self) -> SelfBinding { self.tracking.binding() }
    pub fn group(&self, key: EntityKey) -> EntityKey { self.tracking.group(key) }
    pub fn current_group_member(&self, key: EntityKey) -> Option<EntityKey> {
        self.tracking.current_group_member(key).or_else(|| self.is_current(key).then_some(key))
    }
    pub fn label(&self, key: EntityKey) -> String {
        let name = self.records.get(&key).filter(|m| !m.conflicting).and_then(|m| m.name.as_ref());
        if let Some(n) = name { format!("{n} [{}]",key.actor) }
        else if self.self_key == Some(key) { format!("<ME> [{}]",key.actor) }
        else { format!("Name pending ({})",key.actor) }
    }
}
