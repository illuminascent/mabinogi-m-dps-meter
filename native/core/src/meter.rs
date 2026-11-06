use crate::{identity::{EntityKey, Registry,tracking::SelfBinding}, protocol::Damage, Result};
use std::collections::{BTreeMap,BTreeSet,VecDeque};
use std::sync::{Arc,OnceLock,atomic::{AtomicU64,Ordering}};
mod live_tracks;

pub const MAX_CELLS: usize = 131072;
const MAX_RAW_HITS: usize = 65536;

#[derive(Clone, Debug)]
pub struct Hit { pub timestamp_ns: u64, pub source: EntityKey, pub target: EntityKey, pub damage: Damage }
/// Stable membership shared with delayed skill resolution, including after a
/// late join moves a retained hit into an encounter. Zero means session only.
#[derive(Clone,Debug)]
pub struct HitAssignment(Arc<HitMembership>);
#[derive(Debug)]
struct HitMembership { encounter:AtomicU64, skill:OnceLock<Option<u32>>, boundary:u64 }
impl HitAssignment {
    pub fn encounter(&self)->Option<u64> { match self.0.encounter.load(Ordering::Relaxed) {0=>None,id=>Some(id)} }
}
#[derive(Clone, Default, Debug)]
pub struct Aggregate { pub damage: u128, pub hits: u64,pub allocations:u64, pub flags: [u64;5], pub start: Option<u64>, pub end: Option<u64> }
impl Aggregate {
    fn merge(&mut self, other: &Self) {
        // Every term is a disjoint subset of the already checked view total.
        self.damage = self.damage.checked_add(other.damage).expect("view damage conservation");
        self.hits = self.hits.checked_add(other.hits).expect("view hit conservation");
        self.allocations=self.allocations.checked_add(other.allocations).expect("allocation conservation");
        for (a,b) in self.flags.iter_mut().zip(other.flags) { *a = a.checked_add(b).expect("view flag conservation"); }
        if let Some(t) = other.start { self.start = Some(self.start.map_or(t,|old| old.min(t))); }
        if let Some(t) = other.end { self.end = Some(self.end.map_or(t,|old| old.max(t))); }
    }
    fn add(&mut self, h: &Hit) -> Result<()> {
        self.damage = self.damage.checked_add(h.damage.amount as u128).ok_or("damage overflow")?;
        self.hits = self.hits.checked_add(1).ok_or("hit overflow")?;
        for (n,v) in self.flags.iter_mut().enumerate() {
            *v = v.checked_add(u64::from(h.damage.flags[n])).ok_or("flag overflow")?;
        }
        self.start = Some(self.start.map_or(h.timestamp_ns,|t| t.min(h.timestamp_ns)));
        self.end = Some(self.end.map_or(h.timestamp_ns,|t| t.max(h.timestamp_ns))); Ok(())
    }
    pub fn duration(&self) -> f64 { self.end.unwrap_or(0).saturating_sub(self.start.unwrap_or(0)) as f64 / 1e9 }
    pub fn dps(&self) -> Option<f64> { let d = self.duration(); (d > 0.0).then(|| self.damage as f64/d) }
}
#[derive(Default)]
pub struct View { pub total: Aggregate, pub sources: BTreeMap<EntityKey,Aggregate>,pub estimates:BTreeMap<EntityKey,Aggregate>,pub estimated_total:Aggregate }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum EncounterState { Idle, Active, Ended, PendingReset }
pub struct Encounter { pub id: u64, pub state: EncounterState, pub views: BTreeMap<Option<EntityKey>,View> }
impl Encounter { fn new(id: u64) -> Self { Self { id,state: EncounterState::Idle,views: BTreeMap::new() } } }

#[derive(Clone,Debug)]
pub struct Row {
    pub key: EntityKey, pub label: String, pub damage: u128, pub hits: u64,
    pub dps: Option<f64>, pub share: f64, pub critical_rate: f64, pub named: bool, pub is_self: bool,
    pub assignment_key: Option<EntityKey>,
    pub estimated_damage:u128,pub explicit_hits:u64,
    pub pet_damage:u128,pub skills:Vec<SkillRow>,
}
#[derive(Clone,Debug)]
pub struct SkillRow {pub id:Option<u32>,pub label:String,pub explicit:u128,pub estimated:u128}
#[derive(Clone,Debug)]
pub struct Snapshot {
    pub revision: u64, pub identity_revision: u64, pub encounter: u64,
    pub target: Option<EntityKey>, pub summary: Aggregate, pub source_count: usize,
    pub rows: Vec<Row>, pub targets: Vec<(EntityKey,String)>, pub target_count: usize,
    pub last_command: u64, pub partial: bool,
    pub self_binding: SelfBinding,
    pub encounters:Vec<(u64,EncounterState)>,pub current_encounter:u64,pub estimated_damage:u128,
    pub chart:Vec<(f64,f64,f64)>,
}
#[derive(Clone,Debug)]
pub enum Action { Target(Option<EntityKey>), TargetSearch(String), Encounter(u64), IdleSeconds(u64), AssignSelf(Option<EntityKey>), Reset }
#[derive(Clone,Debug)]
pub struct Command { pub id: u64, pub observed_revision: u64, pub encounter: u64, pub action: Action }
#[derive(Clone,Debug)]
pub struct Ack { pub id: u64, pub applied: bool, pub reason: Option<String>, pub snapshot: Snapshot,
    pub persistence:Option<Result<()>> }

/// These events require independent evidence; scene wire messages never produce them.
pub enum Event {
    Rebroadcast, Gap, InternalTransition { evidence: u64 },
    NewRun { evidence: u64 }, SameMapRetry { evidence: u64 }, CharacterSwitch { evidence: u64 },
    ProcessReplacement { evidence: u64 }, End { evidence: u64 }, TrailingCutscene,
}

pub struct Meter {
    pub skill_names:BTreeMap<u32,BTreeSet<String>>,
    pub skills:BTreeMap<(u64,EntityKey,EntityKey,Option<u32>,bool),u128>,
    pub skill_matches:VecDeque<(crate::combat::Stamp,EntityKey,EntityKey,crate::combat::SkillMatch)>,pub skill_evictions:u64,
    pub registry: Registry,
    pub session:View,
    pub encounters: BTreeMap<u64,Encounter>, pub current: u64, pub binding: u64,
    pub revision: u64, pub selected: Option<EntityKey>, pub partial: bool,
    pub last_command: u64, last_seen_command: u64,
    transitions: BTreeSet<u64>, pub raw_hits: VecDeque<Hit>, pub evicted_hits: u64,
    cells: usize,
    pub viewing:Option<u64>,pub inferred_hits:VecDeque<Hit>,pub idle_ns:u64,pub last_self_activity:Option<u64>,pub automatic_encounters:bool,
    raw_encounters:VecDeque<HitAssignment>,inferred_encounters:VecDeque<HitAssignment>,target_query:String,
    combat_boundary:u64,pending_nodes:BTreeMap<EntityKey,u64>,latest_combat_ns:u64,backtrack_floor_ns:u64,
}
impl Default for Meter {
    fn default() -> Self {
        Self { skill_names:BTreeMap::new(),skills:BTreeMap::new(),skill_matches:VecDeque::new(),skill_evictions:0,registry: Registry::default(),session:View::default(),encounters: [(1,Encounter::new(1))].into(),current: 1,
            binding: 1,revision: 0,selected: None,partial: false,last_command: 0,last_seen_command: 0,
            transitions: BTreeSet::new(),raw_hits: VecDeque::new(),evicted_hits: 0,cells: 0,
            viewing:None,inferred_hits:VecDeque::new(),idle_ns:15_000_000_000,last_self_activity:None,automatic_encounters:false,
            raw_encounters:VecDeque::new(),inferred_encounters:VecDeque::new(),target_query:String::new(),
            combat_boundary:0,pending_nodes:BTreeMap::new(),latest_combat_ns:0,backtrack_floor_ns:0 }
    }
}
impl Meter {
    pub fn last_hit_encounter(&self,inferred:bool)->Option<u64>{
        self.last_hit_assignment(inferred).and_then(|a|a.encounter())
    }
    pub fn last_hit_assignment(&self,inferred:bool)->Option<HitAssignment>{
        (if inferred{&self.inferred_encounters}else{&self.raw_encounters}).back().cloned()
    }
    pub fn attribute_recorded_skill(&mut self,h:&Hit,assignment:&HitAssignment,skill:Option<u32>,inferred:bool)->Result<()> {
        if assignment.0.skill.get().is_some(){return Err("hit skill already attributed".into());}
        self.attribute_skill(h,assignment.encounter().unwrap_or(0),skill,inferred)?;
        assignment.0.skill.set(skill).expect("single core writer");Ok(())
    }
    pub fn attribute_skill(&mut self,h:&Hit,encounter:u64,skill:Option<u32>,inferred:bool)->Result<()> {
        let key=(encounter,h.source,h.target,skill,inferred);
        if !self.skills.contains_key(&key) && self.skills.len()>=MAX_CELLS{return Err("skill aggregate capacity reached".into());}
        let a=self.skills.entry(key).or_default();*a=a.checked_add(h.damage.amount as u128).ok_or("skill damage overflow")?;self.revision+=1;Ok(())
    }
    pub fn skill_evidence(&mut self,at:crate::combat::Stamp,source:EntityKey,target:EntityKey,matched:crate::combat::SkillMatch){
        if self.skill_matches.len()==8192{self.skill_matches.pop_front();self.skill_evictions+=1;}
        self.skill_matches.push_back((at,source,target,matched));
    }
    pub fn add(&mut self, h: Hit) -> Result<()> {
        self.add_kind(h,false)
    }
    pub fn add_estimate(&mut self,h:Hit)->Result<()> { self.add_kind(h,true) }
    fn add_kind(&mut self,h:Hit,inferred:bool)->Result<()> {
        self.latest_combat_ns=self.latest_combat_ns.max(h.timestamp_ns);
        // Capture totals/history accept every observed source. Encounter policy
        // only controls the selected measurement, never packet eligibility.
        let (total,sources)=if inferred{(&mut self.session.estimated_total,&mut self.session.estimates)}else{(&mut self.session.total,&mut self.session.sources)};
        if !sources.contains_key(&h.source) && sources.len()>=16384{return Err("Session source capacity reached".into());}
        let mut next_total=total.clone();next_total.add(&h)?;let mut next_source=sources.get(&h.source).cloned().unwrap_or_default();next_source.add(&h)?;
        if inferred {next_total.hits-=1;next_source.hits-=1;next_total.allocations+=1;next_source.allocations+=1;}
        *total=next_total;sources.insert(h.source,next_source);
        let history=if inferred { &mut self.inferred_hits } else { &mut self.raw_hits };
        let tags=if inferred{&mut self.inferred_encounters}else{&mut self.raw_encounters};
        let assignment=HitAssignment(Arc::new(HitMembership{encounter:AtomicU64::new(0),skill:OnceLock::new(),boundary:self.combat_boundary}));
        let evicted=if history.len() == MAX_RAW_HITS { history.pop_front();self.evicted_hits += 1;tags.pop_front() }else{None};
        history.push_back(h.clone());tags.push_back(assignment.clone());self.revision+=1;
        if evicted.is_some_and(|a|a.encounter().is_none() && a.0.boundary==self.combat_boundary){
            self.partial=true;
            if self.evicted_hits % MAX_RAW_HITS as u64==0 {self.rebuild_pending_nodes();}
        }
        if self.automatic_encounters {
            self.tick(h.timestamp_ns);
            let own=self.registry.self_key;
            let involved=self.registry.is_self_source(h.source) || own==Some(h.target);
            if matches!(self.encounters[&self.current].state,EncounterState::Ended|EncounterState::Idle|EncounterState::PendingReset) {
                if own.is_some() && !involved {
                    for node in [h.source,h.target] {self.pending_nodes.entry(node).and_modify(|t|*t=(*t).max(h.timestamp_ns)).or_insert(h.timestamp_ns);}
                    if self.pending_nodes.len()>MAX_RAW_HITS*4 {self.rebuild_pending_nodes();}
                    return Ok(());
                }
                if self.encounters[&self.current].state==EncounterState::Ended {
                    if let Some(v)=self.encounters[&self.current].views.get(&None) {
                        self.backtrack_floor_ns=self.backtrack_floor_ns.max(v.total.end.unwrap_or(0).max(v.estimated_total.end.unwrap_or(0)));
                    }
                    self.encounters.get_mut(&self.current).unwrap().state=EncounterState::PendingReset;
                }
                self.viewing=None;self.selected=None;self.target_query.clear();
            }
            if involved || own.is_none() { self.last_self_activity=Some(self.last_self_activity.map_or(h.timestamp_ns,|old|old.max(h.timestamp_ns))); }
            // HP allocations can be delivered after self joins. Their capture
            // timestamps may legitimately precede the current view's start.
            if inferred && h.timestamp_ns<self.backtrack_floor_ns {return Ok(());}
        }
        if self.encounters[&self.current].state == EncounterState::PendingReset {
            if self.encounters.len() >= 32 { return Err("encounter archive capacity reached; export required".into()); }
            self.current += 1; self.encounters.insert(self.current,Encounter::new(self.current)); self.selected = None;self.viewing=None;self.target_query.clear();
        }
        if self.encounters[&self.current].state == EncounterState::Ended {
            return Err("damage after end needs a verified new-run decision".into());
        }
        self.add_to_encounter(&h,inferred)?;
        assignment.0.encounter.store(self.current,Ordering::Relaxed);
        if self.automatic_encounters && [h.source,h.target].iter().any(|node|self.pending_nodes.get(node).is_some_and(|t|t.abs_diff(h.timestamp_ns)<self.idle_ns)) {
            self.consolidate(&h)?;
        }
        Ok(())
    }
    fn add_to_encounter(&mut self,h:&Hit,inferred:bool)->Result<()> {
        let encounter = self.encounters.get_mut(&self.current).unwrap();
        let needed = [None,Some(h.target)].into_iter().filter(|k|
            !encounter.views.get(k).is_some_and(|v| if inferred {v.estimates.contains_key(&h.source)} else {v.sources.contains_key(&h.source)})).count();
        if self.cells+needed > MAX_CELLS { return Err("aggregate cell capacity reached".into()); }
        let targets = encounter.views.len().saturating_sub(1);
        if !encounter.views.contains_key(&Some(h.target)) && targets >= 4096 { return Err("target capacity reached".into()); }
        if !encounter.views.get(&None).is_some_and(|v| v.sources.contains_key(&h.source))
            && encounter.views.get(&None).map_or(0,|v| v.sources.len()) >= 16384 { return Err("source capacity reached".into()); }
        // Work on copies before committing, so numeric overflow cannot partially update totals.
        let mut updates = Vec::new();
        for k in [None,Some(h.target)] {
            let v = encounter.views.get(&k);
            let mut total = v.map_or_else(Aggregate::default,|v| if inferred {v.estimated_total.clone()} else {v.total.clone()}); total.add(&h)?;
            let mut source = v.and_then(|v| if inferred {v.estimates.get(&h.source)} else {v.sources.get(&h.source)}).cloned().unwrap_or_default(); source.add(&h)?;
            if inferred{total.hits-=1;source.hits-=1;total.allocations+=1;source.allocations+=1;}
            updates.push((k,total,source));
        }
        for (k,total,source) in updates {
            let view = encounter.views.entry(k).or_default();
            if inferred { view.estimated_total=total;view.estimates.insert(h.source,source); }
            else { view.total=total;view.sources.insert(h.source,source); }
        }
        self.cells += needed; encounter.state = EncounterState::Active;
        Ok(())
    }
    fn rebuild_pending_nodes(&mut self) {
        self.pending_nodes.clear();
        for (h,a) in self.raw_hits.iter().zip(&self.raw_encounters).chain(self.inferred_hits.iter().zip(&self.inferred_encounters)) {
            if a.encounter().is_none() && a.0.boundary==self.combat_boundary && h.timestamp_ns>=self.backtrack_floor_ns {
                for node in [h.source,h.target]{self.pending_nodes.entry(node).and_modify(|t|*t=(*t).max(h.timestamp_ns)).or_insert(h.timestamp_ns);}
            }
        }
    }
    fn consolidate(&mut self,joining:&Hit)->Result<()> {
        let hits=self.raw_hits.iter().zip(&self.raw_encounters).map(|(h,a)|(h,a,false))
            .chain(self.inferred_hits.iter().zip(&self.inferred_encounters).map(|(h,a)|(h,a,true)))
            .filter(|(h,a,_)|a.encounter().is_none() && a.0.boundary==self.combat_boundary && h.timestamp_ns>=self.backtrack_floor_ns).collect();
        for (h,a,inferred) in live_tracks::connected(hits,joining,self.idle_ns) {
            self.add_to_encounter(&h,inferred)?;
            if let Some(skill)=a.0.skill.get() {
                self.attribute_skill(&h,self.current,*skill,inferred)?;
                let key=(0,h.source,h.target,*skill,inferred);
                let amount=self.skills.get_mut(&key).expect("pending skill conservation");
                *amount=amount.checked_sub(h.damage.amount as u128).expect("pending skill conservation");
                if *amount==0{self.skills.remove(&key);}
            }
            a.0.encounter.store(self.current,Ordering::Relaxed);
        }
        self.rebuild_pending_nodes();Ok(())
    }
    fn partition_live_tracks(&mut self) {
        self.combat_boundary+=1;self.pending_nodes.clear();self.backtrack_floor_ns=self.latest_combat_ns;
    }
    pub fn activity(&mut self,source:EntityKey,target:EntityKey,timestamp_ns:u64) {
        if self.encounters[&self.current].state==EncounterState::Active && (self.registry.is_self_source(source)||self.registry.self_key==Some(target)) {
            self.last_self_activity=Some(timestamp_ns);
        }
    }
    pub fn tick(&mut self,now:u64) {
        if self.automatic_encounters && self.last_self_activity.is_some_and(|t|now.saturating_sub(t)>=self.idle_ns)
            && self.encounters[&self.current].state==EncounterState::Active {
            self.encounters.get_mut(&self.current).unwrap().state=EncounterState::Ended;self.revision+=1;
        }
    }
    pub fn event(&mut self, event: Event) -> Result<()> {
        let character_switch = matches!(&event,Event::CharacterSwitch { .. } | Event::ProcessReplacement { .. });
        let (evidence,reset,bindings,end) = match event {
            Event::Rebroadcast => { self.registry.cache_metadata(); return Ok(()); }
            Event::Gap => { self.partial = true; self.registry.cache_metadata(); return Ok(()); }
            Event::TrailingCutscene => return Ok(()),
            Event::InternalTransition { evidence } => (evidence,false,true,false),
            Event::NewRun { evidence } | Event::SameMapRetry { evidence } |
            Event::CharacterSwitch { evidence } | Event::ProcessReplacement { evidence } => (evidence,true,true,false),
            Event::End { evidence } => (evidence,false,false,true),
        };
        // Coalesce only the same evidence token, never unrelated timestamps.
        if self.transitions.contains(&evidence) { return Ok(()); }
        if self.transitions.len() >= 4096 { return Err("transition evidence capacity reached".into()); }
        self.transitions.insert(evidence);
        if reset || end || bindings { self.partition_live_tracks(); }
        if bindings { self.binding += 1; self.registry.clear_current(); }
        if character_switch { self.registry.clear_character(); }
        if reset { self.encounters.get_mut(&self.current).unwrap().state = EncounterState::PendingReset; }
        if end && self.encounters[&self.current].state != EncounterState::PendingReset {
            self.encounters.get_mut(&self.current).unwrap().state = EncounterState::Ended;
        }
        self.revision += 1; Ok(())
    }
    pub fn snapshot(&self) -> Snapshot {
        let encounter = &self.encounters[&self.viewing.unwrap_or(self.current)]; let view = encounter.views.get(&self.selected);
        let mut summary = view.map_or_else(Aggregate::default,|v| v.total.clone());
        let estimated_damage=view.map_or(0,|v|v.estimated_total.damage);
        if let Some(v)=view {summary.merge(&v.estimated_total);}let duration = summary.duration();
        let mut grouped: BTreeMap<EntityKey,Aggregate> = BTreeMap::new();
        let mut estimated:BTreeMap<EntityKey,Aggregate>=BTreeMap::new();
        for (key,a) in view.into_iter().flat_map(|v| v.estimates.iter()) {estimated.entry(self.registry.owner_group(*key)).or_default().merge(a);}
        for (key,a) in view.into_iter().flat_map(|v| v.sources.iter().chain(v.estimates.iter())) {
            grouped.entry(self.registry.owner_group(*key)).or_default().merge(a);
        }
        let mut ranked: Vec<_> = grouped.iter().collect();
        ranked.sort_by(|(ka,a),(kb,b)| b.damage.cmp(&a.damage).then_with(|| ka.cmp(kb)));
        let count = ranked.len();
        let skill_groups=self.skill_groups(encounter.id);
        let rows = ranked.into_iter().take(16).map(|(k,a)| {
            let assignment_key = self.registry.current_group_member(*k);
            let label_key = assignment_key.unwrap_or(*k);
            Row { key: *k,label: self.registry.label(label_key),assignment_key,
            pet_damage:view.into_iter().flat_map(|v|v.sources.iter().chain(v.estimates.iter())).filter(|(source,_)|self.registry.owner(**source).is_some() && self.registry.owner_group(**source)==*k).map(|(_,a)|a.damage).sum(),
            skills:skill_groups.get(k).cloned().unwrap_or_default(),
            estimated_damage:estimated.get(k).map_or(0,|a|a.damage),explicit_hits:a.hits-estimated.get(k).map_or(0,|a|a.hits),
            damage: a.damage,hits: a.hits,dps: (duration > 0.0).then(|| a.damage as f64/duration),
            share: if summary.damage == 0 { 0.0 } else { a.damage as f64/summary.damage as f64 },
            critical_rate: a.flags[0] as f64/(a.hits-estimated.get(k).map_or(0,|a|a.hits)).max(1) as f64,
            named: self.registry.records.get(&label_key).is_some_and(|m| m.name.is_some() && !m.conflicting),
            is_self: self.registry.self_key.is_some_and(|self_key| self.registry.group(self_key) == *k) }
        }).collect();
        let target_count = encounter.views.keys().filter(|k| k.is_some()).count();
        let targets = encounter.views.keys().filter_map(|k| *k).map(|k|(k,self.registry.label(k)))
            .filter(|(_,name)|name.to_lowercase().contains(&self.target_query)).take(256).collect();
        let chart=self.chart(&summary);
        Snapshot { revision: self.revision,identity_revision: self.registry.revision,encounter: encounter.id,
            target: self.selected,summary,source_count: count,rows,targets,target_count,
            last_command: self.last_command,partial: self.partial,self_binding:self.registry.self_binding(),
            current_encounter:self.current,encounters:self.encounters.iter().map(|(id,e)|(*id,e.state)).collect(),estimated_damage,
            chart }
    }
    fn skill_groups(&self,encounter:u64)->BTreeMap<EntityKey,Vec<SkillRow>>{
        let mut groups:BTreeMap<EntityKey,BTreeMap<Option<u32>,(u128,u128)>>=BTreeMap::new();
        for ((e,source,target,skill,inferred),amount) in &self.skills {
            if *e==encounter && self.selected.is_none_or(|t|t==*target) {
                let row=groups.entry(self.registry.owner_group(*source)).or_default().entry(*skill).or_default();
                if *inferred{row.1+=amount;}else{row.0+=amount;}
            }
        }
        groups.into_iter().map(|(owner,rows)|(owner,rows.into_iter().map(|(id,(explicit,estimated))|SkillRow{id,label:id.map_or_else(||"Unknown skill".into(),|id|self.skill_names.get(&id).filter(|names|names.len()==1).and_then(|names|names.first()).cloned().unwrap_or_else(||format!("Skill {id}"))),explicit,estimated}).collect())).collect()
    }
    fn chart(&self,summary:&Aggregate)->Vec<(f64,f64,f64)> {
        let (Some(start),Some(end))=(summary.start,summary.end) else {return Vec::new();};
        let selected=self.viewing.unwrap_or(self.current);
        let mut hits:Vec<_>=self.raw_hits.iter().zip(&self.raw_encounters).chain(self.inferred_hits.iter().zip(&self.inferred_encounters))
            .filter(|(h,tag)|tag.encounter()==Some(selected) && h.timestamp_ns>=start && h.timestamp_ns<=end && self.selected.is_none_or(|k|k==h.target)).map(|(h,_)|h).collect();
        hits.sort_by_key(|h|h.timestamp_ns);
        let retained:u128=hits.iter().map(|h|h.damage.amount as u128).sum();
        let mut cumulative=summary.damage.saturating_sub(retained);let mut rolling=0u128;let mut first=0;
        let stride=(hits.len()/599).max(1);let mut points=Vec::new();
        for (i,h) in hits.iter().enumerate() {
            cumulative+=h.damage.amount as u128;rolling+=h.damage.amount as u128;
            while first<i && h.timestamp_ns.saturating_sub(hits[first].timestamp_ns)>5_000_000_000 {rolling-=hits[first].damage.amount as u128;first+=1;}
            if i%stride==0 || i+1==hits.len() {let t=h.timestamp_ns.saturating_sub(start) as f64/1e9;points.push((t,if t>0.0 {cumulative as f64/t}else{0.0},rolling as f64/5.0));}
        }
        points
    }
    pub fn command(&mut self, c: Command) -> Ack {
        let result = if c.id <= self.last_seen_command { Err("command sequence is not increasing".into()) }
        else {
            self.last_seen_command = c.id;
            if c.encounter != self.viewing.unwrap_or(self.current) { Err("stale encounter".into()) }
            else if c.observed_revision > self.revision { Err("future revision".into()) }
            else { match c.action {
                Action::AssignSelf(k) => self.registry.assign_command(k,c.id),
                Action::Target(k) => {
                    if k.is_some() && !self.encounters[&self.viewing.unwrap_or(self.current)].views.contains_key(&k) {
                        Err("unknown target incarnation".into())
                    } else { self.selected = k; Ok(()) }
                }
                Action::Encounter(id)=>{
                    if !self.encounters.contains_key(&id) {Err("unknown encounter".into())}
                    else {self.viewing=Some(id);self.selected=None;self.target_query.clear();Ok(())}
                }
                Action::TargetSearch(query)=>{
                    if query.len()>1024{Err("Target search exceeds limit".into())}else{self.target_query=query.to_lowercase();Ok(())}
                }
                Action::IdleSeconds(seconds)=>{
                    if !(1..=300).contains(&seconds){Err("Idle timeout must be 1..300 seconds".into())}
                    else{self.idle_ns=seconds*1_000_000_000;Ok(())}
                }
                Action::Reset => {
                    if self.encounters.len()>=32 {Err("encounter archive capacity reached; export required".into())}
                    else {
                        self.partition_live_tracks();
                        self.encounters.get_mut(&self.current).unwrap().state=EncounterState::Ended;
                        self.current+=1;self.encounters.insert(self.current,Encounter::new(self.current));
                        self.viewing=None;self.selected=None;self.target_query.clear();self.last_self_activity=None;
                        Ok(())
                    }
                }
            }}
        };
        if result.is_ok() { self.last_command = c.id; self.revision += 1; }
        Ack { id: c.id,applied: result.is_ok(),reason: result.err(),snapshot: self.snapshot(),persistence:None }
    }
}
