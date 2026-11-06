//! Local identity reconciliation, independent of regional wire layouts.
//!
//! Only adapters with verified identity semantics may feed this module. Observed
//! UID-shaped fields in the legacy JP name scanner are not such an adapter.
use super::{Character, EntityKey};
use crate::Result;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceOrigin { Semantic, Packet { wire:i32 }, Operator { command:u64 }, CachedMetadata }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Evidence { pub frame: u64, pub timestamp_ns: u64, pub origin:EvidenceOrigin }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoteKind { SelfPlayerIdB, SelfMatchCode, SelfCode }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfEvidence { Unresolved, Candidate, Voted, PacketConfirmed, OperatorAssigned, Ambiguous }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelfBinding {
    pub key: Option<EntityKey>, pub evidence: SelfEvidence, pub primary_locked: bool, pub voter_locked: bool,
    pub candidates: Vec<EntityKey>, pub source: Option<Evidence>,
}
impl Default for SelfBinding {
    fn default() -> Self { Self { key: None, evidence: SelfEvidence::Unresolved,
        primary_locked: false, voter_locked:false, candidates: Vec::new(), source: None } }
}

#[derive(Clone, Debug)]
pub struct SeenAppearance { pub character: Character, pub source: Evidence, pub current: bool }

#[derive(Clone, Debug)]
pub struct Alias {
    pub old: EntityKey, pub new: EntityKey, pub character: Character,
    pub source: Evidence, pub active: bool,
}

#[derive(Clone, Debug)]
pub struct TrackingEvent {
    pub revision: u64, pub reason: &'static str, pub key: Option<EntityKey>, pub source: Option<Evidence>,
}
#[derive(Clone,Debug)]
pub struct CandidateLink {pub old:EntityKey,pub new:EntityKey,pub source:Evidence,pub reason:&'static str}

#[derive(Clone, Copy, Debug)]
pub struct Limits { pub appearances: usize, pub aliases: usize, pub voters: usize, pub events: usize }
impl Default for Limits {
    fn default() -> Self { Self { appearances: 32768, aliases: 32768, voters: 128, events: 4096 } }
}

#[derive(Clone, Debug)]
struct Vote { count: u8, last: Evidence, kinds: BTreeSet<u8> }

#[derive(Debug)]
pub struct Tracking {
    actor_order:BTreeMap<EntityKey,u64>,
    pub candidates:BTreeMap<(EntityKey,EntityKey),CandidateLink>,
    current_index:IdentityIndex,retired_index:IdentityIndex,processes:BTreeSet<u64>,
    pub revision: u64,
    pub personal: Option<Character>, pub personal_source: Option<Evidence>,
    pub appearances: BTreeMap<EntityKey, SeenAppearance>,
    pub aliases: BTreeMap<(EntityKey, EntityKey), Alias>,
    pub events: VecDeque<TrackingEvent>, pub event_evictions: u64,
    automatic: SelfBinding, operator: Option<(EntityKey, Evidence)>,operator_retired:bool,
    votes: BTreeMap<EntityKey, Vote>, vote_frames: BTreeMap<(u64,u64),u64>,
    groups: BTreeMap<EntityKey,EntityKey>, limits: Limits,
    wire_context: Option<(u64,u64)>,
    appearance_contexts:BTreeMap<EntityKey,(u64,u64)>,
    closed_contexts:BTreeMap<(u64,u64),Evidence>,
}
impl Default for Tracking { fn default() -> Self { Self::new(Limits::default()) } }
impl Tracking {
    pub fn new(limits: Limits) -> Self {
        Self { current_index:IdentityIndex::default(),retired_index:IdentityIndex::default(),processes:BTreeSet::new(),revision: 0,personal: None,personal_source: None,appearances: BTreeMap::new(),
            actor_order:BTreeMap::new(),candidates:BTreeMap::new(),aliases: BTreeMap::new(),events: VecDeque::new(),event_evictions: 0,
            closed_contexts:BTreeMap::new(),automatic: SelfBinding::default(),operator: None,operator_retired:false,votes: BTreeMap::new(),
            vote_frames: BTreeMap::new(),groups: BTreeMap::new(),limits,wire_context:None,appearance_contexts:BTreeMap::new() }
    }
    /// Anonymous actors have stable ordinal handles too. Names never allocate
    /// participant identity; a supported alias only changes derived grouping.
    pub fn observe_actor(&mut self,key:EntityKey)->Result<()> {
        if !self.actor_order.contains_key(&key){
            if self.actor_order.len()>=32768{return Err("participant directory capacity reached".into());}
            self.actor_order.insert(key,self.actor_order.len() as u64+1);
        }Ok(())
    }
    pub fn participant(&self,key:EntityKey)->Option<u64>{self.actor_order.get(&self.group(key)).copied()}
    pub fn binding_context(&self,key:EntityKey)->Option<(u64,u64)>{self.appearance_contexts.get(&key).copied()}
    pub fn personal_context(&self)->Option<(u64,u64)>{self.wire_context}
    pub fn closed_contexts(&self)->impl Iterator<Item=((u64,u64),Evidence)>+'_ {self.closed_contexts.iter().map(|(c,e)|(*c,*e))}
    pub fn context_closed(&self,context:(u64,u64))->bool{self.closed_contexts.contains_key(&context)}
    pub fn transport_open(&mut self,context:(u64,u64)){self.closed_contexts.remove(&context);}
    /// Both TCP directions closed, or a reset. Retain actors; closure alone
    /// never identifies a replacement or changes the personal character.
    pub fn transport_closed(&mut self,context:(u64,u64),source:Evidence)->Result<()> {
        if !self.closed_contexts.contains_key(&context) && self.closed_contexts.len()>=4096{return Err("closed identity context capacity reached".into());}
        if self.closed_contexts.get(&context).is_none_or(|old|source.timestamp_ns<old.timestamp_ns){
            self.closed_contexts.insert(context,source);self.event("transport_context_closed",None,Some(source));
        }Ok(())
    }
    /// Some instance entries omit personal state. An exact independently parsed
    /// full UID can continue self after the old socket has verifiably closed.
    /// Code/name matches, overlapping sockets and delayed old frames cannot.
    pub fn follow_closed_personal_context(&mut self,key:EntityKey,c:&Character,source:Evidence,context:(u64,u64))->Result<bool> {
        let Some(old)=self.wire_context.filter(|old|*old!=context && old.0==context.0) else{return Ok(false);};
        if !self.closed_contexts.get(&old).is_some_and(|closed|closed.timestamp_ns<=source.timestamp_ns)
            || self.closed_contexts.contains_key(&context)
            || !self.personal.as_ref().is_some_and(|p|p.uid.zip(c.uid).is_some_and(|(a,b)|a==b))
            || self.personal_source.is_some_and(|p|(p.timestamp_ns,p.frame)>(source.timestamp_ns,source.frame))
            || self.matching_current(c,key.process).iter().any(|other|*other!=key && self.binding_context(*other)==Some(context)) {return Ok(false);}
        self.wire_personal(context.0,context.1,c.clone(),source)?;
        self.event("self_context_handoff",Some(key),Some(source));Ok(true)
    }
    pub fn matching_current(&self,c:&Character,process:u64)->Vec<EntityKey>{
        self.current_index.matching(c,process).filter(|key|c.matches(&self.appearances[key].character)).collect()
    }
    pub fn stale_identity(&self,c:&Character,source:Evidence,context:(u64,u64))->bool{
        if self.context_closed(context){return true;}
        self.matching_current(c,context.0).iter().any(|key|{
            let a=&self.appearances[key];a.source.timestamp_ns>source.timestamp_ns && self.appearance_contexts.get(key).is_some_and(|old|*old!=context)
        })
    }
    /// A current personal snapshot establishes the active world context. A
    /// stable identity there can displace older-context bindings individually.
    /// Concurrent same-context identities remain ambiguous; newest is no vote.
    pub fn context_replacements(&self,key:EntityKey,c:&Character,context:(u64,u64))->Vec<EntityKey>{
        if self.wire_context!=Some(context){return Vec::new();}
        let matching:Vec<_>=self.matching_current(c,key.process).into_iter().filter(|old|*old!=key).collect();
        if matching.iter().any(|old|self.appearance_contexts.get(old)==Some(&context)){return Vec::new();}
        let uids:BTreeSet<_>=matching.iter().filter_map(|k|self.appearances[k].character.uid).collect();
        if uids.len()>1{return Vec::new();}
        matching.into_iter().filter(|old|self.appearance_contexts.get(old).is_some_and(|old|*old!=context)).collect()
    }
    pub fn candidate(&mut self,old:EntityKey,new:EntityKey,source:Evidence)->Result<()> {
        if old==new || old.process!=new.process{return Ok(());}
        let pair=(old,new);
        if !self.candidates.contains_key(&pair) && self.candidates.len()>=4096{return Err("candidate identity capacity reached".into());}
        self.candidates.insert(pair,CandidateLink{old,new,source,reason:"unresolved concurrent identity; no alias"});Ok(())
    }
    fn event(&mut self, reason: &'static str, key: Option<EntityKey>, source: Option<Evidence>) {
        self.revision += 1;
        if self.limits.events == 0 { self.event_evictions += 1; return; }
        if self.events.len() == self.limits.events { self.events.pop_front(); self.event_evictions += 1; }
        self.events.push_back(TrackingEvent { revision: self.revision,reason,key,source });
    }
    fn validate(c: &Character) -> Result<()> {
        if c.uid == Some(0) { return Err("zero persistent UID".into()); }
        if let Some((name,_)) = &c.name_server {
            if name.trim().is_empty() || name.encode_utf16().count() > 128 || name.chars().any(char::is_control) {
                return Err("invalid exact character name".into());
            }
        }
        if c.uid.is_none() && c.code_server.is_none() && c.name_server.is_none() { return Err("identity has no verified join fields".into()); }
        Ok(())
    }
    /// Personal data may arrive before or after any appearance. Equal UIDs can
    /// enrich missing names; absent fields never erase already verified fields.
    pub fn personal(&mut self, mut c: Character, source: Evidence) -> Result<()> {
        if source.origin==EvidenceOrigin::CachedMetadata { return Err("cached metadata cannot assert current personal identity".into()); }
        Self::validate(&c)?;
        if self.personal_source.is_some_and(|old| source.frame<old.frame) { return Err("stale personal identity evidence".into()); }
        let relation = self.personal.as_ref().map(|old| old.matches(&c));
        if relation == Some(true) {
            let old = self.personal.as_ref().unwrap();
            c.uid = c.uid.or(old.uid);
            c.code_server = c.code_server.or(old.code_server);
            c.name_server = c.name_server.or_else(|| old.name_server.clone());
        } else if relation == Some(false) {
            self.automatic = SelfBinding::default(); self.votes.clear(); self.vote_frames.clear();
            self.operator = None;
            self.event("personal_identity_changed_or_unlinked",None,Some(source));
        }
        self.personal = Some(c); self.personal_source = Some(source);
        self.event("personal_identity",None,Some(source)); self.reconcile(); Ok(())
    }
    /// The verified JP prefix belongs to this TCP connection's observation set.
    /// Older appearances in other scenes remain metadata, never competing self IDs.
    pub fn wire_personal(&mut self,process:u64,binding:u64,c:Character,source:Evidence)->Result<()> {
        if source.origin==EvidenceOrigin::CachedMetadata{return Err("cached metadata cannot assert current personal identity".into());}
        Self::validate(&c)?;
        if self.context_closed((process,binding)){return Ok(());}
        if self.personal_source.is_some_and(|old|(old.timestamp_ns,old.frame)>(source.timestamp_ns,source.frame)) { return Ok(()); }
        self.wire_context=Some((process,binding));
        self.personal(c,source)
    }
    pub fn appearance(&mut self, key: EntityKey, mut c: Character, source: Evidence) -> Result<()> {
        if source.origin==EvidenceOrigin::CachedMetadata { return Err("cached metadata cannot restore an entity binding".into()); }
        Self::validate(&c)?;
        self.observe_actor(key)?;
        if !self.appearances.contains_key(&key) && self.appearances.len() >= self.limits.appearances {
            return Err("verified appearance directory capacity reached".into());
        }
        if let Some(old) = self.appearances.get(&key) {
            if old.character.uid.zip(c.uid).is_some_and(|(a,b)| a != b) {
                return Err("persistent UID changed without a new entity incarnation".into());
            }
            if (source.timestamp_ns,source.frame)<(old.source.timestamp_ns,old.source.frame) { return Err("stale appearance evidence".into()); }
            if !old.character.matches(&c) { return Err("appearance identity changed without a new entity incarnation".into()); }
            c.uid = c.uid.or(old.character.uid);
            c.code_server = c.code_server.or(old.character.code_server);
            c.name_server = c.name_server.or_else(|| old.character.name_server.clone());
        }
        if let Some(old)=self.appearances.get(&key){
            if old.current{self.current_index.remove(key,&old.character);}else{self.retired_index.remove(key,&old.character);}
        }
        self.current_index.insert(key,&c);self.processes.insert(key.process);
        self.appearances.insert(key,SeenAppearance { character:c.clone(),source,current:true });
        self.event(match source.origin{EvidenceOrigin::Packet{wire:20590}=>"personal_owner_binding",EvidenceOrigin::Packet{wire:20559}=>"profile_actor_binding",_=>"verified_appearance"},Some(key),Some(source));
        self.refresh_aliases(&c,key.process)?; self.reconcile(); Ok(())
    }
    pub fn wire_appearance(&mut self,key:EntityKey,c:Character,source:Evidence,context:(u64,u64))->Result<()> {
        self.appearance_context(key,context)?;self.appearance(key,c,source)
    }
    pub fn appearance_context(&mut self,key:EntityKey,context:(u64,u64))->Result<()> {
        if !self.appearance_contexts.contains_key(&key) && self.appearance_contexts.len()>=self.limits.appearances{return Err("Appearance context capacity reached".into());}
        self.appearance_contexts.insert(key,context);Ok(())
    }
    /// A verified retirement is different from a capture gap. Gaps must not call this.
    pub fn retire(&mut self, key: EntityKey, source: Evidence) -> Result<()> {
        self.candidates.retain(|_,c|c.old!=key && c.new!=key);
        let changed=self.appearances.get(&key).map(|a|a.character.clone());
        if let Some(a) = self.appearances.get_mut(&key) {self.current_index.remove(key,&a.character);self.retired_index.insert(key,&a.character);a.current = false; }
        self.votes.remove(&key);
        if self.operator.is_some_and(|(k,_)| k == key) { self.operator_retired=true; }
        if self.automatic.key == Some(key) { self.automatic = SelfBinding::default(); }
        self.event("binding_retired",Some(key),Some(source)); if let Some(c)=changed{self.refresh_aliases(&c,key.process)?;} self.reconcile(); Ok(())
    }
    pub fn clear_bindings(&mut self, source: Evidence) {
        self.closed_contexts.clear();
        self.candidates.clear();
        for (key,a) in self.appearances.iter_mut().filter(|(_,a)|a.current) {self.current_index.remove(*key,&a.character);self.retired_index.insert(*key,&a.character);a.current = false; }
        self.automatic = SelfBinding::default(); self.operator = None;
        self.votes.clear(); self.vote_frames.clear();
        self.event("bindings_invalidated_character_retained",None,Some(source));
    }
    pub fn clear_character(&mut self, source: Evidence) {
        self.clear_bindings(source); self.personal = None; self.personal_source = None;
        self.event("character_cleared",None,Some(source));
    }
    /// The caller verifies that the key is a current observed incarnation.
    pub fn assign(&mut self, key: Option<EntityKey>, source: Evidence) -> Result<()> {
        if source.origin==EvidenceOrigin::CachedMetadata { return Err("cached operator receipt requires fresh binding evidence".into()); }
        self.operator = key.map(|key| (key,source));
        self.operator_retired=false;
        self.event(if key.is_some() { "operator_assigned" } else { "operator_cleared" },key,Some(source));
        Ok(())
    }
    pub fn binding(&self) -> SelfBinding {
        if let Some((key,source)) = self.operator {
            let key=if self.operator_retired{self.current_group_member(key)}else{Some(key)};
            SelfBinding { key,evidence:SelfEvidence::OperatorAssigned,primary_locked:self.automatic.primary_locked,
                voter_locked:self.automatic.voter_locked,
                candidates:Vec::new(),source:Some(source) }
        } else { self.automatic.clone() }
    }
    /// Only the three verified self-identifying semantic kinds can vote. Damage
    /// and repeated arbitrary actor IDs have no API here and never cast votes.
    pub fn vote(&mut self, key: EntityKey, kind: VoteKind, source: Evidence) -> Result<()> {
        if source.origin==EvidenceOrigin::CachedMetadata { return Err("cached metadata cannot cast a self vote".into()); }
        if self.automatic.voter_locked { self.event("weak_vote_ignored_map_lock",Some(key),Some(source)); return Ok(()); }
        if source.frame == 0 { return Err("self vote needs frame provenance".into()); }
        if self.vote_frames.get(&(key.process,key.binding)).is_some_and(|last| *last >= source.frame) {
            self.event("duplicate_or_stale_self_vote",Some(key),Some(source)); return Ok(());
        }
        if !self.votes.contains_key(&key) && self.votes.len() >= self.limits.voters {
            return Err("self candidate capacity reached".into());
        }
        self.vote_frames.insert((key.process,key.binding),source.frame);
        let v = self.votes.entry(key).or_insert(Vote { count:0,last:source,kinds:BTreeSet::new() });
        v.count = v.count.saturating_add(1).min(2); v.last = source;
        v.kinds.insert(match kind { VoteKind::SelfPlayerIdB=>0,VoteKind::SelfMatchCode=>1,VoteKind::SelfCode=>2 });
        self.event("verified_self_vote",Some(key),Some(source)); self.reconcile(); Ok(())
    }
    fn reconcile(&mut self) {
        let matches: Vec<_> = self.personal.as_ref().map_or_else(Vec::new,|c|self.processes.iter().flat_map(|process|self.current_index.matching(c,*process))
            .filter(|k|self.wire_context.is_none_or(|scope|scope==self.appearance_contexts.get(k).copied().unwrap_or((k.process,k.binding))) && c.matches(&self.appearances[k].character)).collect::<BTreeSet<_>>().into_iter().collect());
        if matches.len() == 1 {
            self.votes.clear(); self.vote_frames.clear();
            let key = matches[0];
            if let Some(personal) = self.personal.as_mut() {
                if personal.uid.is_none() {
                    if let Some(uid) = self.appearances[&key].character.uid {
                        personal.uid = Some(uid);
                        self.event("personal_uid_learned_from_appearance",Some(key),Some(self.appearances[&key].source));
                    }
                }
            }
            self.automatic = SelfBinding { key:Some(key),evidence:SelfEvidence::PacketConfirmed,
                primary_locked:true,voter_locked:true,candidates:matches,source:Some(self.appearances[&key].source) };
        } else if matches.len() > 1 {
            self.votes.clear(); self.vote_frames.clear();
            self.automatic = SelfBinding { key:None,evidence:SelfEvidence::Ambiguous,
                primary_locked:true,voter_locked:true,candidates:matches,source:self.personal_source };
        } else {
            let voted: Vec<_> = self.votes.iter().filter(|(_,v)| v.count >= 2).map(|(k,_)| *k).collect();
            let candidates: Vec<_> = self.votes.keys().copied().collect();
            let (key,evidence,source) = if voted.len() == 1 {
                (Some(voted[0]),SelfEvidence::Voted,Some(self.votes[&voted[0]].last))
            } else if voted.len() > 1 { (None,SelfEvidence::Ambiguous,None) }
            else if !candidates.is_empty() { (None,SelfEvidence::Candidate,None) }
            else { (None,SelfEvidence::Unresolved,None) };
            self.automatic = SelfBinding { key,evidence,primary_locked:false,voter_locked:key.is_some(),candidates,source };
        }
    }
    fn refresh_aliases(&mut self,changed:&Character,process:u64) -> Result<()> {
        // ID replacement requires matching persistent identity AND retirement.
        // Two simultaneous matching actors remain ambiguous; newest is no tie-break.
        let current_index = &self.current_index;
        let retired_index = &self.retired_index;
        let mut additions = Vec::new();
        let affected=current_index.neighbors(changed,process);
        for new in &affected {
            let a=&self.appearances[new];
            if current_index.matching(&a.character,new.process).take(2).count() != 1 { continue; }
            for old in retired_index.matching(&a.character,new.process) {
                let b = &self.appearances[&old];
                if b.source.frame <= a.source.frame {
                    let pair = (old,*new);
                    if !self.aliases.contains_key(&pair) {
                        if self.aliases.len()+additions.len() >= self.limits.aliases { return Err("identity alias capacity reached".into()); }
                        additions.push((pair,Alias { old,new:*new,
                            character:a.character.clone(),source:a.source,active:true }));
                    }
                }
            }
        }
        if self.aliases.len()+additions.len() > self.limits.aliases { return Err("identity alias capacity reached".into()); }
        self.aliases.extend(additions);
        for alias in self.aliases.values_mut() {
            alias.active = current_index.matching(&alias.character,alias.new.process).take(2).count() <= 1;
        }
        // A missing-UID/name fallback is not transitive proof that two DIFFERENT
        // known UIDs are the same character. Retract weak bridge edges if later
        // evidence exposes such a conflict, instead of relying on union order.
        let mut proposed=BTreeMap::new();
        for a in self.aliases.values().filter(|a| a.active) {
            let x=Self::root(&proposed,a.old);let y=Self::root(&proposed,a.new);
            Self::join(&mut proposed,&self.actor_order,x,y);
        }
        let mut uid_sets:BTreeMap<EntityKey,BTreeSet<u64>>=BTreeMap::new();
        let mut code_sets:BTreeMap<EntityKey,BTreeSet<([u8;4],u16)>>=BTreeMap::new();
        let members:BTreeSet<_>=self.aliases.values().flat_map(|a|[a.old,a.new]).collect();
        for key in &members {
            let a=&self.appearances[key];
            let root=Self::root(&proposed,*key);
            if let Some(uid)=a.character.uid { uid_sets.entry(root).or_default().insert(uid); }
            if let Some(code)=a.character.code_server { code_sets.entry(root).or_default().insert(code); }
        }
        for a in self.aliases.values_mut().filter(|a| a.active) {
            let old=&self.appearances[&a.old].character;let new=&self.appearances[&a.new].character;
            let root=Self::root(&proposed,a.old);
            let uid_conflict=uid_sets.get(&root).is_some_and(|values| values.len()>1);
            let code_conflict=code_sets.get(&root).is_some_and(|values| values.len()>1);
            let strong_uid=old.uid.zip(new.uid).is_some_and(|(x,y)| x==y);
            let strong_code=old.code_server.zip(new.code_server).is_some_and(|(x,y)| x==y);
            if (uid_conflict && !strong_uid) || (code_conflict && !strong_uid && !strong_code) { a.active=false; }
        }
        self.groups.clear();
        for alias in self.aliases.values().filter(|a| a.active) {
            let a = Self::root(&self.groups,alias.old); let b = Self::root(&self.groups,alias.new);
            Self::join(&mut self.groups,&self.actor_order,a,b);
        }
        let keys: Vec<_> = self.groups.keys().copied().collect();
        for key in keys { let root = Self::root(&self.groups,key); self.groups.insert(key,root); }
        Ok(())
    }
    fn join(groups:&mut BTreeMap<EntityKey,EntityKey>,order:&BTreeMap<EntityKey,u64>,a:EntityKey,b:EntityKey){
        if a==b{return;}let first=|k|(*order.get(&k).unwrap_or(&u64::MAX),k);
        if first(a)<first(b){groups.insert(b,a);}else{groups.insert(a,b);}
    }
    fn root(groups: &BTreeMap<EntityKey,EntityKey>, mut key: EntityKey) -> EntityKey {
        while let Some(parent) = groups.get(&key) { if *parent == key { break; } key = *parent; } key
    }
    pub fn group(&self, key: EntityKey) -> EntityKey { Self::root(&self.groups,key) }
    pub fn current_group_member(&self, key: EntityKey) -> Option<EntityKey> {
        let group = self.group(key);
        self.appearances.iter().find(|(k,a)| a.current && self.group(**k) == group).map(|(k,_)| *k)
    }
}

/// Each fallback index excludes records that carry a stronger comparable key.
/// Code/server and display-name/server are never interchangeable.
#[derive(Default,Debug)]
struct IdentityIndex {
    uids: BTreeMap<(u64,u64),BTreeSet<EntityKey>>,
    codes: BTreeMap<(u64,bool,[u8;4],u16),BTreeSet<EntityKey>>,
    names: BTreeMap<(u64,bool,bool,String,u16),BTreeSet<EntityKey>>,
}
impl IdentityIndex {
    fn insert(&mut self,key:EntityKey,c:&Character){
            if let Some(uid) = c.uid { self.uids.entry((key.process,uid)).or_default().insert(key); }
            if let Some((code,server)) = c.code_server {
                self.codes.entry((key.process,false,code,server)).or_default().insert(key);
                if c.uid.is_none() { self.codes.entry((key.process,true,code,server)).or_default().insert(key); }
            }
            if let Some((name,server)) = &c.name_server {
                for omit_uid in [false,true] { for omit_code in [false,true] {
                    if (!omit_uid || c.uid.is_none()) && (!omit_code || c.code_server.is_none()) {
                        self.names.entry((key.process,omit_uid,omit_code,name.clone(),*server)).or_default().insert(key);
                    }
                }}
            }
    }
    fn remove(&mut self,key:EntityKey,c:&Character){
        if let Some(uid)=c.uid{if let Some(set)=self.uids.get_mut(&(key.process,uid)){set.remove(&key);}}
        if let Some((code,server))=c.code_server{for omit in [false,true]{if let Some(set)=self.codes.get_mut(&(key.process,omit,code,server)){set.remove(&key);}}}
        if let Some((name,server))=&c.name_server{for omit_uid in [false,true]{for omit_code in [false,true]{if let Some(set)=self.names.get_mut(&(key.process,omit_uid,omit_code,name.clone(),*server)){set.remove(&key);}}}}
    }
    fn neighbors(&self,c:&Character,process:u64)->BTreeSet<EntityKey>{
        let uid=c.uid.and_then(|uid|self.uids.get(&(process,uid)));
        let code=c.code_server.and_then(|(code,server)|self.codes.get(&(process,false,code,server)));
        let name=c.name_server.as_ref().and_then(|(name,server)|self.names.get(&(process,false,false,name.clone(),*server)));
        uid.into_iter().flatten().chain(code.into_iter().flatten()).chain(name.into_iter().flatten()).copied().collect()
    }
    fn matching<'a>(&'a self, c: &Character, process: u64) -> impl Iterator<Item=EntityKey> + 'a {
        let uid = c.uid.and_then(|uid| self.uids.get(&(process,uid)));
        let code = c.code_server.and_then(|(code,server)| self.codes.get(&(process,c.uid.is_some(),code,server)));
        let names = c.name_server.as_ref().and_then(|(name,server)|
            self.names.get(&(process,c.uid.is_some(),c.code_server.is_some(),name.clone(),*server)));
        uid.into_iter().flatten().chain(code.into_iter().flatten()).chain(names.into_iter().flatten()).copied()
    }
}
