//! Bounded JP combat evidence. Catalogs supply labels only; capture time is never
//! replaced by an inferred server timestamp. All adapters below are inbound JP.
use crate::{identity::EntityKey, protocol::{u32le,u64le}, Result};
use std::collections::{BTreeMap,BTreeSet,VecDeque};

pub const EVENT_NS:u64=32_000_000;
pub const EVENT_FRAMES:u64=8;
pub const HISTORY:usize=8192;
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
pub struct Timeline {clocks:BTreeMap<u64,Clock>,pub samples:VecDeque<ClockSample>,pub evictions:u64,
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
            if !self.clocks.contains_key(&at.context) && self.clocks.len()>=4096{return Err("timeline capacity reached".into());}
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
        if self.samples.len()==HISTORY{self.samples.pop_front();self.evictions+=1;}
        self.samples.push_back(ClockSample{at,wire,raw,ticks,epoch,interpretation:kind});Ok(())
    }
    pub fn event(&mut self,at:Stamp){
        if self.bounds.len()==HISTORY{if let Some((frame,bound))=self.bounds.pop_first(){if let Some(p)=self.pending.get_mut(&bound.at.context){p.remove(&frame);}self.evictions+=1;}}
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
    pub fn clear_context(&mut self,context:u64){if let Some(c)=self.clocks.get_mut(&context){c.last=None;c.epoch+=1;}self.last.remove(&context);self.pending.remove(&context);}
    pub fn clear(&mut self){self.clocks.clear();self.last.clear();self.pending.clear();}
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
    pub attacks:VecDeque<Attack>,pub executions:VecDeque<Execution>,
    pub effects:BTreeMap<(EntityKey,u64),Effect>,
    pub names:BTreeMap<u32,BTreeSet<String>>,pub evictions:u64,pub effect_updates:u64,pub removals:u64,
    pub confirmed_executions:u64,pub ended_effects:VecDeque<(Effect,Stamp)>,pub ended_evictions:u64,
}
impl Combat {
    pub fn attack(&mut self,a:Attack){if self.attacks.len()==HISTORY{self.attacks.pop_front();self.evictions+=1;}self.attacks.push_back(a);}
    pub fn execution(&mut self,e:Execution)->Result<()> {
        if let Some(name)=&e.action {
            if !self.names.contains_key(&e.skill) && self.names.len()>=32768{return Err("skill name capacity reached".into());}
            let names=self.names.entry(e.skill).or_default();
            if !names.contains(name) && names.len()>=64{return Err("skill variant capacity reached".into());}names.insert(name.clone());
        }
        if self.executions.len()==HISTORY{self.executions.pop_front();self.evictions+=1;}
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
    pub fn clear_context(&mut self,context:u64){self.attacks.retain(|a|a.at.context!=context);self.executions.retain(|e|e.at.context!=context);self.effects.retain(|_,e|e.last.context!=context);}
    pub fn clear(&mut self){self.attacks.clear();self.executions.clear();self.effects.clear();}
}
