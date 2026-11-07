//! Bounded JP combat evidence. Catalogs supply labels only; capture time is never
//! replaced by an inferred server timestamp. All adapters below are inbound JP.
use crate::{identity::EntityKey, protocol::{u32le,u64le}, Result};
use std::collections::{BTreeMap,BTreeSet,VecDeque};

pub const EVENT_NS:u64=32_000_000;
pub const EVENT_FRAMES:u64=8;
pub const HISTORY:usize=8192;
pub const ATTACK_HISTORY:usize=1024;
pub const EXECUTION_HISTORY:usize=256;
pub const CLOCK_HISTORY:usize=1024;
pub const MAX_CLOCK_CONTEXTS:usize=128;
/// At most512 UTF16 units in the supported action grammar, each needing at
/// most three UTF8 bytes. This also bounds non-wire callers of execution().
pub const ACTION_BYTES:usize=1536;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Stamp {
    pub process:u64,pub continuity:u64,pub context:u64,pub frame:u64,pub order:u64,pub timestamp_ns:u64,pub tcp_byte:u64,
}
impl Stamp {
    pub fn near(self,other:Self)->bool {
        self.process==other.process && self.continuity==other.continuity && self.context==other.context && self.order.abs_diff(other.order)<=EVENT_FRAMES
            && self.timestamp_ns.abs_diff(other.timestamp_ns)<=EVENT_NS
    }
}
#[derive(Clone,Debug)]
pub struct Attack {
    pub source:EntityKey,pub target:EntityKey,pub key1:u32,pub key2:u32,pub meta:[u8;8],pub at:Stamp,
}
/// Learned relative impact, not a decoded character attack stat. The four
/// factors are shared across observations; there is no actor-by-ability cell.
/// Full raw flags preserve unnamed hit stages instead of pooling them by crit.
pub const MODEL_ACTORS:usize=512;
pub const MODEL_ABILITIES:usize=512;
pub const MODEL_COMPONENTS:usize=2048;
pub const MODEL_TARGETS:usize=512;
#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord)]
struct Component {skill:u32,key2:u32,flags:[u8;7]}
#[derive(Clone,Copy,Default)]
struct Factor {log:f64,count:u64}
/// Fixed-capacity sorted vectors: prediction is logarithmic in each cap;
/// admission can shift at most the corresponding cap. No disk or history scan.
pub struct DamageModel {
    reference:Option<f64>,actors:Vec<(EntityKey,Factor)>,abilities:Vec<(u32,Factor)>,
    components:Vec<(Component,Factor)>,targets:Vec<(EntityKey,Factor)>,
    pub anchors:u64,pub rejected_zero:u64,pub capacity_rejections:u64,
    pub rejected_unknown_ability:u64,pub squared_log_residual:f64,
    ability_log_sum:f64,component_log_sum:f64,
}
impl Default for DamageModel {
    fn default()->Self{Self{reference:None,actors:Vec::with_capacity(MODEL_ACTORS),abilities:Vec::with_capacity(MODEL_ABILITIES),components:Vec::with_capacity(MODEL_COMPONENTS),targets:Vec::with_capacity(MODEL_TARGETS),anchors:0,rejected_zero:0,capacity_rejections:0,rejected_unknown_ability:0,squared_log_residual:0.0,ability_log_sum:0.0,component_log_sum:0.0}}
}
impl DamageModel {
    fn factor<K:Ord>(table:&[(K,Factor)],key:&K)->Factor{table.binary_search_by(|(k,_)|k.cmp(key)).ok().map_or_else(Factor::default,|i|table[i].1)}
    fn admit<K:Ord>(table:&mut Vec<(K,Factor)>,key:K,limit:usize)->Option<usize>{
        match table.binary_search_by(|(k,_)|k.cmp(&key)){Ok(i)=>Some(i),Err(i) if table.len()<limit=>{table.insert(i,(key,Factor::default()));Some(i)},Err(_)=>None}
    }
    /// Relative actor-power prior, in common units for every candidate. It is
    /// independent of notification multiplicity, target or inferred totals.
    /// The unconstrained reference scale cancels when used for a shared budget.
    pub fn actor_prior(&self,source:EntityKey)->(f64,bool){let actor=Self::factor(&self.actors,&source);(actor.log.exp(),actor.count>=2)}
    /// Positive predictions for one observed component. Unknown factors use a
    /// symmetric zero-log prior, inheriting available shared ability evidence.
    /// `trained` requires repeated observations of both actor and component;
    /// it does not certify factor identifiability or numerical actor accuracy.
    pub fn weight(&self,source:EntityKey,target:EntityKey,skill:u32,key2:u32,flags:[u8;7])->(f64,bool){
        let Some(reference)=self.reference else{return (1.0,false);};
        let actor=Self::factor(&self.actors,&source);let mut ability=Self::factor(&self.abilities,&skill);
        let mut component=Self::factor(&self.components,&Component{skill,key2,flags});let target=Self::factor(&self.targets,&target);
        // Key zero is a mixture of unknown abilities, not an observed shared
        // ability. Use a broad, equal-template prior rather than learn a fake
        // universal zero-skill potency. It does not count notification repeats.
        if skill==0{ability.log=self.ability_log_sum/self.abilities.len().max(1) as f64;component.log=self.component_log_sum/self.components.len().max(1) as f64;}
        let log=reference+actor.log+ability.log+component.log+target.log;
        (log.clamp(0.0,(u64::MAX as f64).ln()).exp(),actor.count>=2 && component.count>=2)
    }
    /// Caller must supply a positive, independently supported numeric impact:
    /// scoped concordant explicit/HP reconciliation or isolated unambiguous HP
    /// loss. Such a scope is provisional model support, not verified actor-role
    /// ground truth; caller must retain its source/provenance uncertainty.
    /// Never call this for estimates, forced credit, clipped/gap/conditional HP,
    /// aggregate ambiguous budgets, owner-role disagreement or opaque amounts.
    /// Each amount is a component observation, never a whole-cast normalizer.
    pub fn observe_clean(&mut self,source:EntityKey,target:EntityKey,skill:u32,key2:u32,flags:[u8;7],amount:u64){
        if amount==0{self.rejected_zero=self.rejected_zero.saturating_add(1);return;}
        if skill==0{self.rejected_unknown_ability=self.rejected_unknown_ability.saturating_add(1);return;}
        let key=Component{skill,key2,flags};
        // Admission is atomic: no partial factors for an unadmitted anchor.
        if (self.actors.len()==MODEL_ACTORS && self.actors.binary_search_by_key(&source,|(k,_)|*k).is_err())
            || (self.abilities.len()==MODEL_ABILITIES && self.abilities.binary_search_by_key(&skill,|(k,_)|*k).is_err())
            || (self.components.len()==MODEL_COMPONENTS && self.components.binary_search_by_key(&key,|(k,_)|*k).is_err())
            || (self.targets.len()==MODEL_TARGETS && self.targets.binary_search_by_key(&target,|(k,_)|*k).is_err()){
            self.capacity_rejections=self.capacity_rejections.saturating_add(1);return;
        }
        let i=Self::admit(&mut self.actors,source,MODEL_ACTORS).unwrap();
        let j=Self::admit(&mut self.abilities,skill,MODEL_ABILITIES).unwrap();
        let k=Self::admit(&mut self.components,key,MODEL_COMPONENTS).unwrap();
        let l=Self::admit(&mut self.targets,target,MODEL_TARGETS).unwrap();
        let observed=(amount as f64).ln();let reference=*self.reference.get_or_insert(observed);
        let predicted=reference+self.actors[i].1.log+self.abilities[j].1.log+self.components[k].1.log+self.targets[l].1.log;
        let residual=observed-predicted;
        // Normalized log least squares, diminishing with component support.
        // Four is the number of active factors; 1/2 is the estimator step,
        // unrelated to a game mechanic or a temporal admission threshold.
        let step=residual/(8.0*(self.components[k].1.count.saturating_add(1) as f64).sqrt());
        let old_ability=self.abilities[j].1.log;let old_component=self.components[k].1.log;
        for factor in [&mut self.actors[i].1,&mut self.abilities[j].1,&mut self.components[k].1,&mut self.targets[l].1]{
            factor.log=(factor.log+step).clamp(-64.0,64.0);factor.count=factor.count.saturating_add(1);
        }
        self.ability_log_sum+=self.abilities[j].1.log-old_ability;self.component_log_sum+=self.components[k].1.log-old_component;
        self.anchors=self.anchors.saturating_add(1);self.squared_log_residual+=residual*residual;
    }
    pub fn owned_bytes(&self)->usize{std::mem::size_of::<Self>()+self.actors.capacity()*std::mem::size_of::<(EntityKey,Factor)>()+self.abilities.capacity()*std::mem::size_of::<(u32,Factor)>()+self.components.capacity()*std::mem::size_of::<(Component,Factor)>()+self.targets.capacity()*std::mem::size_of::<(EntityKey,Factor)>()}
    pub fn summary(&self)->serde_json::Value{serde_json::json!({"version":"shared-impact-v1","structure":"reference + actor power + shared ability + raw component + target nuisance in log space","reference_log":self.reference,"actors":self.actors.len(),"abilities":self.abilities.len(),"components":self.components.len(),"targets":self.targets.len(),"caps":{"actors":MODEL_ACTORS,"abilities":MODEL_ABILITIES,"components":MODEL_COMPONENTS,"targets":MODEL_TARGETS},"anchors":self.anchors,"zero_rejections":self.rejected_zero,"unknown_ability_rejections":self.rejected_unknown_ability,"capacity_rejections":self.capacity_rejections,"squared_log_residual":self.squared_log_residual,"owned_bytes":self.owned_bytes(),"limits":"relative online fit constrained by shared factors, not independently identified physical stats; no inferred training; component observations do not imply a complete cast"})}
    pub fn clear(&mut self){self.reference=None;self.ability_log_sum=0.0;self.component_log_sum=0.0;self.actors.clear();self.abilities.clear();self.components.clear();self.targets.clear();}
}
#[derive(Clone,Debug)]
pub struct SkillMatch {pub skill:Option<u32>,pub frames:Vec<u64>,pub outcome:&'static str}
#[derive(Clone,Debug)]
pub struct ClockSample {pub at:Stamp,pub wire:i32,pub raw:u64,pub ticks:Option<u64>,pub epoch:u64,pub interpretation:&'static str}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct ClockPoint {pub epoch:u64,pub ticks:u64,pub frame:u64}
#[derive(Clone,Debug)]
pub struct TimeBounds {pub at:Stamp,pub lower:Option<ClockPoint>,pub upper:Option<ClockPoint>}
#[derive(Default)]
struct Clock {last:Option<(u32,u64)>,epoch:u64,ticks:u64}
#[derive(Default)]
pub struct Timeline {clocks:BTreeMap<u64,Clock>,pub samples:VecDeque<ClockSample>,pub evictions:u64,pub clock_capacity_rejections:u64,
    pub bounds:BTreeMap<u64,TimeBounds>,pending:BTreeMap<u64,BTreeSet<u64>>,last:BTreeMap<u64,ClockPoint>}
impl Timeline {
    pub fn observe(&mut self,wire:i32,p:&[u8],at:Stamp)->Result<()> {
        let (raw,kind)=match wire {
            170066 if p.len()==24 => (u32le(p,8) as u64,"clock64 candidate"),
            171124 if p.len()>=37 && p.len()==37+8*(u32le(p,28) as usize) => (u32le(p,p.len()-4) as u64,"clock64 candidate"),
            170197 if p.len()>=34 && p.len()==34+u32le(p,21) as usize && p[8]!=0 => (u64le(p,9),"event Unix-ms candidate"),
            170069 if p.len()==36 => (u64le(p,8)>>22,"instance creation-ms candidate"),
            _=>return Ok(()),
        };
        if raw==0{return Ok(());}
        let mut ticks=None;let mut epoch=0;
        if kind=="clock64 candidate" {
            if !self.clocks.contains_key(&at.context) && self.clocks.len()>=MAX_CLOCK_CONTEXTS{
                // Unknown ordering is a bounded accuracy loss, not a reason to
                // stop all semantic decoding or pretend an epoch was observed.
                self.clock_capacity_rejections=self.clock_capacity_rejections.saturating_add(1);
                if self.samples.len()==CLOCK_HISTORY{self.samples.pop_front();self.evictions+=1;}
                self.samples.push_back(ClockSample{at,wire,raw,ticks:None,epoch:0,interpretation:"clock context capacity; ordering unknown"});return Ok(());
            }
            let c=self.clocks.entry(at.context).or_default();let r=raw as u32;
            if let Some((old,time))=c.last {
                let delta=r.wrapping_sub(old);
                // Backward/reset or implausible forward motion starts another
                // epoch. Cross-context epochs are deliberately incomparable.
                let allowance=at.timestamp_ns.saturating_sub(time)/15_625_000+128;
                if delta as u64>allowance {c.epoch+=1;c.ticks=r as u64;}else{c.ticks+=delta as u64;}
            }else{c.ticks=r as u64;}
            c.last=Some((r,at.timestamp_ns));ticks=Some(c.ticks);epoch=c.epoch;
            let point=ClockPoint{epoch,ticks:c.ticks,frame:at.frame};
            if let Some(pending)=self.pending.remove(&at.context){for frame in pending{if let Some(bound)=self.bounds.get_mut(&frame){
                if bound.lower.is_none_or(|lower|lower.epoch==epoch && lower.ticks<=point.ticks){bound.upper=Some(point);}
            }}}
            self.last.insert(at.context,point);
        }
        if self.samples.len()==CLOCK_HISTORY{self.samples.pop_front();self.evictions+=1;}
        self.samples.push_back(ClockSample{at,wire,raw,ticks,epoch,interpretation:kind});Ok(())
    }
    pub fn event(&mut self,at:Stamp){
        if self.bounds.len()==CLOCK_HISTORY{if let Some((frame,bound))=self.bounds.pop_first(){let mut empty=false;if let Some(p)=self.pending.get_mut(&bound.at.context){p.remove(&frame);empty=p.is_empty();}if empty{self.pending.remove(&bound.at.context);}self.evictions+=1;}}
        self.bounds.insert(at.frame,TimeBounds{at,lower:self.last.get(&at.context).copied(),upper:None});self.pending.entry(at.context).or_default().insert(at.frame);
    }
    /// Only complete brackets constrain causality. A nearby clock is a bound,
    /// never an exact timestamp for HP. Sparse/unclosed bounds remain unknown.
    pub fn compatible(&self,a:Stamp,b:Stamp)->bool{
        if a.process!=b.process || a.continuity!=b.continuity || a.context!=b.context{return false;}
        let (Some(a),Some(b))=(self.bounds.get(&a.frame),self.bounds.get(&b.frame))else{return true;};
        if let (Some(al),Some(bl))=(a.lower,b.lower){if al.epoch!=bl.epoch{return false;}}
        let (Some(al),Some(au),Some(bl),Some(bu))=(a.lower,a.upper,b.lower,b.upper)else{return true;};
        al.epoch==au.epoch && al.epoch==bl.epoch && bl.epoch==bu.epoch && al.ticks<=bu.ticks.saturating_add(1) && bl.ticks<=au.ticks.saturating_add(1)
    }
    /// Consecutive HP observations can be arbitrarily far apart in time. Only
    /// an established epoch boundary invalidates their shared clock context.
    pub fn same_epoch(&self,a:Stamp,b:Stamp)->bool{
        if a.process!=b.process || a.continuity!=b.continuity || a.context!=b.context{return false;}
        match (self.bounds.get(&a.frame).and_then(|b|b.lower),self.bounds.get(&b.frame).and_then(|b|b.lower)){
            (Some(a),Some(b))=>a.epoch==b.epoch,_=>true,
        }
    }
    /// Replay advances Stamp.continuity before clearing a transport context;
    /// stamps remain incomparable across that boundary even after clock reuse.
    pub fn clear_context(&mut self,context:u64){self.clocks.remove(&context);self.last.remove(&context);self.pending.remove(&context);}
    pub fn clear(&mut self){self.clocks.clear();self.last.clear();self.pending.clear();}
    /// Owned capacities plus deliberately pessimistic 128-byte tree-node
    /// overhead per entry (including separately allocated pending set entries).
    /// This is an inference dependency estimate, not process RSS.
    pub fn assignment_owned_bytes(&self)->usize{
        std::mem::size_of::<Self>()+self.samples.capacity()*std::mem::size_of::<ClockSample>()
            +self.clocks.len()*(std::mem::size_of::<(u64,Clock)>()+128)
            +self.bounds.len()*(std::mem::size_of::<(u64,TimeBounds)>()+256)
            +self.pending.iter().map(|(_,set)|std::mem::size_of::<(u64,BTreeSet<u64>)>()+128+set.len()*(std::mem::size_of::<u64>()+128)).sum::<usize>()
            +self.last.len()*(std::mem::size_of::<(u64,ClockPoint)>()+128)
    }
    pub fn assignment_ceiling_bytes(&self)->usize{
        std::mem::size_of::<Self>()+CLOCK_HISTORY*std::mem::size_of::<ClockSample>()
            +MAX_CLOCK_CONTEXTS*(std::mem::size_of::<(u64,Clock)>()+128)
            +CLOCK_HISTORY*(std::mem::size_of::<(u64,TimeBounds)>()+256)
            // At most one pending-set context per retained bound and at most
            // CLOCK_HISTORY members in all pending sets combined.
            +CLOCK_HISTORY*(std::mem::size_of::<(u64,BTreeSet<u64>)>()+128+std::mem::size_of::<u64>()+128)
            +MAX_CLOCK_CONTEXTS*(std::mem::size_of::<(u64,ClockPoint)>()+128)
    }
}

#[derive(Clone,Debug)]
pub struct Action {pub actor:u32,pub name:String,pub skill:u32,pub tail:[u8;29]}
fn actor(p:&[u8],n:usize)->Option<u32>{let a=u64le(p,n);(a>0 && a<=u32::MAX as u64).then_some(a as u32)}
/// JP170064: actor, byte-counted UTF16, exactly 29 variant bytes.
pub fn action(p:&[u8])->Option<Action>{
    if p.len()<43{return None;}let a=actor(p,0)?;let n=u32le(p,8) as usize;
    if n==0 || n>1024 || n%2!=0 || p.len()!=12+n+29{return None;}
    let name=String::from_utf16(&p[12..12+n].chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect::<Vec<_>>()).ok()?;
    if name.chars().any(char::is_control){return None;}
    Some(Action{actor:a,name,skill:u32le(p,p.len()-4),tail:p[12+n..].try_into().ok()?})
}
#[derive(Clone,Debug)]
pub struct Effect {pub affected:EntityKey,pub instance:u64,pub id:u32,pub time_bits:u32,pub stacks:u32,pub source:Option<EntityKey>,pub first:Stamp,pub last:Stamp}
#[derive(Clone,Debug)]
pub struct Execution {pub source:EntityKey,pub skill:u32,pub action:Option<String>,pub variant:Option<[u8;29]>,pub at:Stamp,pub confirmed:bool}
#[derive(Default)]
pub struct Combat {
    pub damage_model:DamageModel,
    pub attacks:VecDeque<Attack>,pub executions:VecDeque<Execution>,
    pub effects:BTreeMap<(EntityKey,u64),Effect>,
    pub names:BTreeMap<u32,BTreeSet<String>>,pub evictions:u64,pub effect_updates:u64,pub removals:u64,
    pub confirmed_executions:u64,pub ended_effects:VecDeque<(Effect,Stamp)>,pub ended_evictions:u64,
}
impl Combat {
    pub fn attack(&mut self,a:Attack){if self.attacks.len()==ATTACK_HISTORY{self.attacks.pop_front();self.evictions+=1;}self.attacks.push_back(a);}
    pub fn execution(&mut self,mut e:Execution)->Result<()> {
        if let Some(name)=&mut e.action{if name.len()>ACTION_BYTES{return Err("action name capacity reached".into());}if name.capacity()>ACTION_BYTES{*name=name.as_str().to_owned();}}
        if let Some(name)=&e.action {
            if !self.names.contains_key(&e.skill) && self.names.len()>=32768{return Err("skill name capacity reached".into());}
            let names=self.names.entry(e.skill).or_default();
            if !names.contains(name) && names.len()>=64{return Err("skill variant capacity reached".into());}names.insert(name.clone());
        }
        if self.executions.len()==EXECUTION_HISTORY{self.executions.pop_front();self.evictions+=1;}
        self.executions.push_back(e);Ok(())
    }
    /// The observed crosswalk requires the same actor, numeric key and exact
    /// action/clock/state order. An unpaired state remains metadata only.
    pub fn state(&mut self,source:EntityKey,skill:u32,at:Stamp,clock_frame:Option<u64>){
        if let Some(e)=self.executions.iter_mut().rev().find(|e|e.source==source && e.at.context==at.context && e.at.continuity==at.continuity && e.at.order+2==at.order && e.skill==skill && clock_frame==Some(e.at.order+1)) {if !e.confirmed{self.confirmed_executions+=1;}e.confirmed=true;}
    }
    pub fn effect(&mut self,e:Effect)->Result<()> {
        let key=(e.affected,e.instance);
        if let Some(old)=self.effects.get_mut(&key){
            if old.id!=e.id || old.source.zip(e.source).is_some_and(|(a,b)|a!=b){return Err("effect instance conflict".into());}
            old.source=old.source.or(e.source);old.last=e.last;old.time_bits=e.time_bits;old.stacks=e.stacks;self.effect_updates+=1;
        }else{
            if self.effects.len()>=32768{return Err("effect instance capacity reached".into());}self.effects.insert(key,e);
        }Ok(())
    }
    pub fn remove(&mut self,affected:EntityKey,instance:u64){if self.effects.remove(&(affected,instance)).is_some(){self.removals+=1;}}
    pub fn remove_at(&mut self,affected:EntityKey,instance:u64,at:Stamp){
        if let Some(effect)=self.effects.remove(&(affected,instance)){
            self.removals+=1;if self.ended_effects.len()==HISTORY{self.ended_effects.pop_front();self.ended_evictions+=1;}
            self.ended_effects.push_back((effect,at));
        }
    }
    pub fn skill(&self,source:EntityKey,target:EntityKey,at:Stamp)->SkillMatch {
        let attacks:Vec<_>=self.attacks.iter().rev().take_while(|a|at.timestamp_ns.saturating_sub(a.at.timestamp_ns)<=EVENT_NS)
            .filter(|a|a.source==source && a.target==target && at.near(a.at)).collect();
        let keys:BTreeSet<_>=attacks.iter().filter_map(|a|(a.key1!=0).then_some(a.key1)).collect();
        let frames=attacks.iter().map(|a|a.at.frame).collect();
        if keys.len()==1 && attacks.iter().all(|a|a.key1!=0){SkillMatch{skill:keys.first().copied(),frames,outcome:"direct attack key"}}
        else{
            let executions:Vec<_>=self.executions.iter().filter(|e|e.source==source && e.confirmed && at.near(e.at)).collect();
            let execution_keys:BTreeSet<_>=executions.iter().filter_map(|e|(e.skill!=0).then_some(e.skill)).collect();
            if keys.is_empty() && !attacks.is_empty() && execution_keys.len()==1{
                let mut frames:Vec<_>=frames;frames.extend(executions.iter().map(|e|e.at.frame));
                SkillMatch{skill:execution_keys.first().copied(),frames,outcome:"confirmed action/state crosswalk"}
            }else{SkillMatch{skill:None,frames,outcome:if keys.len()>1 || execution_keys.len()>1{"ambiguous"}else{"unknown"}}}
        }
    }
    /// Packet linkage is supplied by the causal assigner. Do not impose the old
    /// eight-message gate again after it has linked an actual presentation.
    pub fn linked_skill(&self,attack:&Attack)->SkillMatch{
        if attack.key1!=0{return SkillMatch{skill:Some(attack.key1),frames:vec![attack.at.frame],outcome:"causally linked attack key"};}
        let executions:Vec<_>=self.executions.iter().filter(|e|e.source==attack.source && e.confirmed && e.at.process==attack.at.process && e.at.context==attack.at.context && e.at.continuity==attack.at.continuity && e.at.order<=attack.at.order && attack.at.timestamp_ns.abs_diff(e.at.timestamp_ns)<=100_000_000).collect();
        let keys:BTreeSet<_>=executions.iter().filter_map(|e|(e.skill!=0).then_some(e.skill)).collect();
        let mut frames=vec![attack.at.frame];frames.extend(executions.iter().map(|e|e.at.frame));
        SkillMatch{skill:if keys.len()==1{keys.first().copied()}else{None},frames,outcome:if keys.len()==1{"linked confirmed action/state"}else if keys.is_empty(){"unknown"}else{"ambiguous"}}
    }
    /// Only the histories/model read by assignment are included. Effect and
    /// name catalogs are existing independent metadata, not matching storage.
    pub fn assignment_owned_bytes(&self)->usize{
        std::mem::size_of::<VecDeque<Attack>>()+self.attacks.capacity()*std::mem::size_of::<Attack>()
            +std::mem::size_of::<VecDeque<Execution>>()+self.executions.capacity()*std::mem::size_of::<Execution>()
            +self.executions.iter().filter_map(|e|e.action.as_ref()).map(String::capacity).sum::<usize>()
            +self.damage_model.owned_bytes()
    }
    pub fn assignment_ceiling_bytes(&self)->usize{
        std::mem::size_of::<VecDeque<Attack>>()+ATTACK_HISTORY*std::mem::size_of::<Attack>()
            +std::mem::size_of::<VecDeque<Execution>>()+EXECUTION_HISTORY*(std::mem::size_of::<Execution>()+ACTION_BYTES)
            +self.damage_model.owned_bytes()
    }
    pub fn clear_context(&mut self,context:u64){self.attacks.retain(|a|a.at.context!=context);self.executions.retain(|e|e.at.context!=context);self.effects.retain(|_,e|e.last.context!=context);}
    pub fn clear(&mut self){self.attacks.clear();self.executions.clear();self.effects.clear();self.damage_model.clear();}
}
