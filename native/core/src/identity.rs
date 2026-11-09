//! Verified JP name adapters and a wire-independent reconciliation model.
use crate::{protocol::{u32le, u64le}, Result};
use std::{collections::{BTreeMap,BTreeSet,VecDeque}, sync::OnceLock};
pub mod tracking;
pub mod score2_layout;
pub mod cache;
pub mod ownership;
pub mod personal;
pub mod profile;
use tracking::{Evidence,EvidenceOrigin,SelfBinding,Tracking,VoteKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityKey {
    pub process: u64, pub binding: u64, pub actor: u32, pub generation: u32,
}

/// Generic NPC type is not a faction. Player characters and their pets share
/// one confirmed side; other affiliations remain unknown until proved.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum DamageSide { Unknown, Player }
pub fn damage_side_from_kind(kind:&str)->DamageSide {
    if kind.eq_ignore_ascii_case("player") {DamageSide::Player}else{DamageSide::Unknown}
}
pub fn damage_eligible_sides(source:EntityKey,target:EntityKey,source_side:DamageSide,target_side:DamageSide)->bool {
    source.actor!=0 && source!=target && !(source.process==target.process && source.binding==target.binding && source.actor==target.actor)
        && !(source_side==DamageSide::Player && target_side==DamageSide::Player)
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
    // JP20517 embeds the same kind-3 fixed spawn suffix after its outer
    // reference. Require the repeated combat actor in the observed common
    // header before accepting an uncatalogued template; the legacy scan below
    // remains label-gated for other, structurally unverified variants.
    let body=&p[8..];
    if body.len()>=700 && u64le(body,0)==u64le(body,572) && u32le(body,580)==2048 {
        if let Some(mut a)=spawn(body){a.record_reference=Some(u64le(p,0));return Some(a);}
    }
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
    Some(Appearance { actor:u32le(p,0),name:monsters().get(&template).cloned().unwrap_or_else(||format!("NPC {template}")),uid:None,
        template:Some(template),offset:n,record_reference:None })
}

#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub explicit_damage: bool,
    pub name: Option<String>, pub uid: Option<u64>, pub template: Option<String>,
    pub kind: Option<&'static str>,
    pub cached: bool, pub conflicting: bool,
}

#[derive(Default, Debug)]
pub struct Registry {
    carried:BTreeSet<EntityKey>,
    room_carried:BTreeSet<EntityKey>,
    pub personal_bindings:VecDeque<PersonalProof>,pub personal_binding_evictions:u64,
    pub owners:ownership::Owners,
    pub timing:BTreeMap<EntityKey,IdentityTiming>,
    uid_members:BTreeMap<u64,BTreeSet<EntityKey>>,
    pub revision: u64,
    pub records: BTreeMap<EntityKey, Metadata>,
    current: BTreeMap<(u64,u64,u32), EntityKey>,
    generations: BTreeMap<(u64,u64,u32),u32>,
    pub self_key: Option<EntityKey>,
    pub operator_assigned: bool,
    pub tracking: Tracking,
    pub profile_names:BTreeMap<u64,NameMetadata>,
}
#[derive(Clone,Debug,Default)]
pub struct IdentityTiming {pub first:Option<crate::combat::Stamp>,pub last:Option<crate::combat::Stamp>,pub received:Option<Evidence>,pub resolved:Option<Evidence>}
#[derive(Clone,Debug)]
pub struct PersonalProof{pub key:EntityKey,pub evidence:Evidence,pub offset:usize,pub end:usize,pub pets:Vec<personal::Pet>}
impl Registry {
    pub fn damage_side(&self,key:EntityKey)->DamageSide {
        let player=|key|self.records.get(&key).is_some_and(|m|m.kind.is_some_and(|k|damage_side_from_kind(k)==DamageSide::Player))
            || self.personal_bindings.iter().any(|p|p.key==key);
        if player(key) || self.owner(key).is_some_and(player) {DamageSide::Player}else{DamageSide::Unknown}
    }
    pub fn damage_eligible(&self,source:EntityKey,target:EntityKey)->bool {
        damage_eligible_sides(source,target,self.damage_side(source),self.damage_side(target))
    }
    pub fn inference_excluded(&self,key:EntityKey)->bool {
        self.self_key==Some(key) || self.records.get(&key).is_some_and(|m|m.explicit_damage)
    }
    pub fn note_explicit_source(&mut self,key:EntityKey){
        if let Some(m)=self.records.get_mut(&key){if !m.explicit_damage{m.explicit_damage=true;self.revision+=1;}}
    }
    pub fn note_observed(&mut self,key:EntityKey,at:crate::combat::Stamp){
        let t=self.timing.entry(key).or_default();t.first.get_or_insert(at);
        if t.last.is_none_or(|old|at.timestamp_ns>=old.timestamp_ns){t.last=Some(at);}
        if self.carried.remove(&key){self.revision+=1;}
        self.room_carried.remove(&key);
    }
    pub fn observe_at(&mut self,process:u64,binding:u64,actor:u32,at:crate::combat::Stamp)->Result<EntityKey>{
        let key=self.observe(process,binding,actor)?;self.note_observed(key,at);Ok(key)
    }
    pub fn observe(&mut self, process: u64, binding: u64, actor: u32) -> Result<EntityKey> {
        if actor == 0 { return Err("zero entity ID".into()); }
        if let Some(k) = self.current.get(&(process,binding,actor)) { return Ok(*k); }
        if self.records.len() >= 32768 { return Err("entity capacity reached".into()); }
        let generation = self.generations.get(&(process,binding,actor)).copied().unwrap_or(0).checked_add(1)
            .ok_or("entity generation overflow")?;
        let key = EntityKey { process, binding, actor, generation };
        self.tracking.observe_actor(key)?;
        self.records.insert(key, Metadata::default()); self.current.insert((process,binding,actor), key);
        self.generations.insert((process,binding,actor),generation);
        self.revision += 1; Ok(key)
    }
    pub fn is_current(&self, key: EntityKey) -> bool {
        self.current.get(&(key.process,key.binding,key.actor)) == Some(&key)
    }
    pub fn current_key(&self,process:u64,binding:u64,actor:u32)->Option<EntityKey>{self.current.get(&(process,binding,actor)).copied()}
    /// Apply before metadata can retire/recreate the raw actor. A late recovered
    /// identity may describe an older incarnation even when its UID differs.
    pub fn stale_actor_identity(&self,process:u64,binding:u64,actor:u32,source:Evidence)->bool{
        self.current_key(process,binding,actor).is_some_and(|key|{
            self.tracking.appearances.get(&key).is_some_and(|a|(a.source.timestamp_ns,a.source.frame)>(source.timestamp_ns,source.frame))
                || self.owners.links.get(&key).is_some_and(|a|(a.evidence.timestamp_ns,a.evidence.frame)>(source.timestamp_ns,source.frame))
        })
    }
    pub fn appearance(&mut self, process: u64, binding: u64, a: Appearance) -> Result<EntityKey> {
        let mut key = self.observe(process,binding,a.actor)?;
        let old = &self.records[&key];
        let kind = if a.template.is_some() { Some("npc") } else if a.uid.is_some() { Some("player") } else { None };
        if old.uid.zip(a.uid).is_some_and(|(x,y)| x != y)
            || a.uid.is_some_and(|uid|self.tracking.appearances.get(&key).is_some_and(|prior|Character::score2_appearance_uid(uid).is_ok_and(|c|!prior.character.matches(&c))))
            || old.template.as_ref().zip(a.template.as_ref()).is_some_and(|(x,y)| x != y)
            || old.kind.zip(kind).is_some_and(|(x,y)| x != y) {
            self.retire(key)?; key = self.observe(process,binding,a.actor)?;
        }
        let meta = self.records.get_mut(&key).unwrap();
        if meta.name.as_ref().is_some_and(|n| n != &a.name) { meta.conflicting = true; }
        if !meta.conflicting { meta.name = Some(a.name); }
        meta.uid = a.uid.or(meta.uid); meta.template = a.template.or(meta.template.take());
        if let Some(uid)=meta.uid {self.uid_members.entry(uid).or_default().insert(key);}
        meta.kind = kind.or(meta.kind);
        if let Some(profile)=meta.uid.and_then(|uid| self.profile_names.get(&uid)) {
            if !profile.cached && (profile.conflicting || meta.name.as_ref().is_some_and(|name| name!=&profile.name)) { meta.conflicting=true; }
        }
        meta.cached = false; self.revision += 1; Ok(key)
    }
    pub fn retire(&mut self, key: EntityKey) -> Result<()> {
        self.retire_evidenced(key,self.semantic_source())
    }
    fn retire_evidenced(&mut self,key:EntityKey,source:Evidence)->Result<()>{
        self.carried.remove(&key);
        self.room_carried.remove(&key);
        if self.is_current(key) { self.current.remove(&(key.process,key.binding,key.actor)); }
        self.tracking.retire(key,source)?;
        self.sync_self(); self.revision += 1; Ok(())
    }
    pub fn cache_metadata(&mut self) {
        self.carried.extend(self.current.values().copied());
        for v in self.records.values_mut() { v.cached = true; } self.revision += 1;
    }
    pub fn carry_room(&mut self){
        self.room_carried=self.current.values().copied().collect();self.cache_metadata();
    }
    pub fn clear_current(&mut self) {
        self.carried.clear();
        self.room_carried.clear();
        self.current.clear(); self.tracking.clear_bindings(self.semantic_source());
        self.sync_self(); self.cache_metadata();
    }
    pub fn clear_character(&mut self) {
        self.tracking.clear_character(self.semantic_source()); self.clear_current();
    }
    /// Typed semantic input; wire adapters use provenance-bearing methods.
    pub fn character(&mut self, identity: Character) -> Result<()> {
        self.personal_evidence(identity,self.semantic_source())
    }
    pub fn personal_evidence(&mut self, identity: Character, source: Evidence) -> Result<()> {
        self.tracking.personal(identity,source)?; self.sync_self(); self.revision += 1; Ok(())
    }
    pub fn wire_personal(&mut self,process:u64,binding:u64,identity:Character,source:Evidence)->Result<()> {
        let previous=self.tracking.personal_context();
        self.tracking.wire_personal(process,binding,identity,source)?;
        if previous!=self.tracking.personal_context() && self.tracking.personal_context()==Some((process,binding)){
            self.reconcile_wire_context((process,binding),None)?;
        }
        self.sync_self();self.revision+=1;Ok(())
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
        self.carried.remove(&key);self.room_carried.remove(&key);
        let meta = self.records.get_mut(&key).unwrap();
        if let Some((name,_)) = identity.name_server { meta.name = Some(name); meta.conflicting = false; }
        meta.uid = identity.uid.or(meta.uid); meta.cached = false; meta.kind = Some("player");
        if let Some(uid)=meta.uid {self.uid_members.entry(uid).or_default().insert(key);}
        let timing=self.timing.entry(key).or_default();timing.received.get_or_insert(source);timing.resolved.get_or_insert(source);
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
    pub fn wire_named_appearance(&mut self,key:EntityKey,identity:Character,name:Option<String>,source:Evidence,context:(u64,u64))->Result<()> {
        let handoff=self.tracking.follow_closed_personal_context(key,&identity,source,context)?;
        // A fresh full identity for the actor independently identified by the
        // personal owner record can resolve its UID, including a conflicting
        // old UID with the same six-byte prefix. Never extend the prefix to UID.
        if identity.uid.is_some() && self.personal_bindings.back().is_some_and(|p|p.key==key && p.evidence.timestamp_ns<=source.timestamp_ns)
            && self.tracking.binding_context(key)==Some(context)
            && self.tracking.personal.as_ref().is_some_and(|p|p.code_server.is_some() && p.code_server==identity.code_server){
            self.tracking.personal(identity.clone(),source)?;
        }
        let mut replaced=self.tracking.context_replacements(key,&identity,context);
        // A verified room boundary also supports scoped replacement within one
        // socket. If the old actor is observed in the new room it is no longer
        // merely carried, and a concurrent claim must remain ambiguous.
        if replaced.is_empty(){
            let matching:Vec<_>=self.tracking.matching_current(&identity,key.process).into_iter().filter(|old|*old!=key).collect();
            let uids:BTreeSet<_>=matching.iter().filter_map(|old|self.tracking.appearances[old].character.uid).collect();
            if !matching.is_empty() && uids.len()<=1 && matching.iter().all(|old|self.room_carried.contains(old)){replaced=matching;}
        }
        let mut inherited=replaced.iter().find_map(|old|self.records.get(old).filter(|m|identity.uid.is_some() && m.uid==identity.uid && !m.conflicting).and_then(|m|m.name.clone()));
        if inherited.is_none(){
            let names:BTreeSet<_>=identity.uid.and_then(|uid|self.uid_members.get(&uid)).into_iter().flatten()
                .filter_map(|old|self.records.get(old).filter(|m|!m.conflicting).and_then(|m|m.name.clone())).collect();
            if names.len()==1{inherited=names.into_iter().next();}
        }
        for old in replaced{
            self.retire_evidenced(old,source)?;
        }
        self.tracking.appearance_context(key,context)?;
        self.appearance_named_evidence(key,identity.clone(),name.or(inherited),source)?;
        let duplicates:Vec<_>=self.tracking.matching_current(&identity,key.process).into_iter().filter(|old|*old!=key).collect();
        for old in duplicates{self.tracking.candidate(old,key,source)?;}
        if handoff{
            // Reconcile fresh peer appearances that arrived before self proved
            // this context. A second pass is scoped to existing verified records;
            // it cannot manufacture presence or resolve same-context duplicates.
            self.reconcile_wire_context(context,Some(key))?;
        }
        Ok(())
    }
    fn reconcile_wire_context(&mut self,context:(u64,u64),exclude:Option<EntityKey>)->Result<()>{
        let earlier:Vec<_>=self.tracking.appearances.iter().filter(|(key,a)|Some(**key)!=exclude && a.current
            && self.tracking.binding_context(**key)==Some(context)).map(|(k,a)|(*k,a.character.clone(),a.source)).collect();
        for (key,c,evidence) in earlier{self.wire_named_appearance(key,c,None,evidence,context)?;}Ok(())
    }
    pub fn personal_actor(&mut self,process:u64,binding:u64,context:(u64,u64),p:personal::Personal,source:Evidence,at:crate::combat::Stamp)->Result<Option<EntityKey>>{
        if self.tracking.context_closed(context){return Ok(None);}
        if self.stale_actor_identity(process,binding,p.actor,source){return Ok(None);}
        if self.tracking.personal_source.is_some_and(|old|(old.timestamp_ns,old.frame)>(source.timestamp_ns,source.frame)){return Ok(None);}
        // The owner record supplies a local actor and six-byte character code,
        // not a full UID for a new incarnation. Use a UID only if a fresh,
        // independently verified identity already binds this exact actor here.
        let mut character=p.character.clone();
        if let Some(current)=self.current_key(process,binding,p.actor){
            if !self.carried.contains(&current) && self.tracking.binding_context(current)==Some(context){
                if let Some(a)=self.tracking.appearances.get(&current).filter(|a|a.character.uid.is_some() && a.character.code_server==character.code_server
                    && self.tracking.personal_source.is_none_or(|old|a.source.timestamp_ns>=old.timestamp_ns)){
                    character=a.character.clone();
                }
            }
        }
        self.wire_personal(process,context.1,character.clone(),source)?;
        let mut key=self.observe(process,binding,p.actor)?;
        if self.tracking.appearances.get(&key).is_some_and(|a|!a.character.matches(&character))
            || self.records[&key].uid.is_some_and(|uid|Character::score2_appearance_uid(uid).is_ok_and(|old|!old.matches(&character))){
            self.retire(key)?;key=self.observe(process,binding,p.actor)?;
        }
        // The validated owner field directly identifies the current local actor.
        // It can replace the prior local binding even when the socket/world
        // context integer did not change. This is stronger than two concurrent
        // appearance claims, which intentionally remain unresolved.
        if let Some(old)=self.personal_bindings.back().map(|p|p.key).filter(|old|*old!=key && self.is_current(*old)){
            self.retire_evidenced(old,source)?;
        }
        self.wire_named_appearance(key,character,None,source,context)?;self.note_observed(key,at);
        if self.personal_bindings.len()==128{self.personal_bindings.pop_front();self.personal_binding_evictions+=1;}
        self.personal_bindings.push_back(PersonalProof{key,evidence:source,offset:p.offset,end:p.end,pets:p.pets});
        Ok(Some(key))
    }
    pub fn participant(&self,key:EntityKey)->Option<u64>{self.tracking.participant(key)}
    pub fn binding_state(&self,key:EntityKey)->&'static str{
        if !self.is_current(key){"retired"}else if self.carried.contains(&key){"carried"}else if self.tracking.appearances.contains_key(&key){"verified"}else{"observed"}
    }
    pub fn remembered_players(&self)->usize{
        self.current.values().filter(|k|self.records.get(k).is_some_and(|m|m.kind==Some("player")))
            .filter_map(|k|self.participant(*k)).collect::<BTreeSet<_>>().len()
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
        for key in self.uid_members.get(&uid).into_iter().flatten() {
            let meta=self.records.get_mut(key).unwrap();
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
        let name = self.records.get(&key).filter(|m| !m.conflicting).and_then(|m| m.name.as_ref())
            .or_else(||if self.records.get(&key).is_some_and(|m|m.conflicting){None}else{self.records.get(&self.group(key)).filter(|m|!m.conflicting).and_then(|m|m.name.as_ref())});
        if let Some(n) = name { n.clone() }
        else if self.self_key == Some(key) || (self.self_key.is_none() && self.records.get(&key).is_some_and(|m|m.explicit_damage) && self.owner(key).is_none()) { "<YOU>".into() }
        else { "Name pending".into() }
    }
}
