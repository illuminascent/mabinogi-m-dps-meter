//! Consecutive HP accounting. Allocations are amounts, never observed hits.
use crate::{combat::{Attack,Stamp,HISTORY},identity::EntityKey,protocol::u64le,Result};
use std::collections::{BTreeMap,BTreeSet,VecDeque};
/// Evidence delivery may lag. This escrow does not widen attack activity.
pub const WAIT_NS:u64=200_000_000;
pub fn parse(p:&[u8])->Option<(u32,u64)>{
    let envelope=crate::hp_wire::envelope(p)?;
    if envelope.blob.len()!=8{return None;}
    let actor=envelope.actor;let hp=u64le(envelope.blob,0);
    (actor>0 && actor<=u32::MAX as u64 && hp<=i64::MAX as u64).then_some((actor as u32,hp))
}
#[derive(Clone,Debug)]
pub struct Estimate {pub source:EntityKey,pub target:EntityKey,pub amount:u64,pub timestamp_ns:u64,pub skill:Option<u32>,pub interval:u64,pub method:&'static str,pub frames:Vec<u64>,pub at:Stamp}
#[derive(Clone,Debug)]
pub struct Accounting {
    pub id:u64,pub target:EntityKey,pub at:Stamp,pub loss:u64,pub explicit:u64,pub residual:u64,
    pub unassigned:u64,pub candidates:Vec<EntityKey>,pub allocations:Vec<Estimate>,
    pub evidence:Vec<u64>,pub reason:&'static str,pub explicit_mismatch:bool,pub baseline:Stamp,pub hp_before:u64,pub hp_after:u64,
}
#[derive(Clone,Debug)]
pub struct Refinement {pub interval:u64,pub source:EntityKey,pub amount:u64,pub at:Stamp,pub attack_frames:Vec<u64>}
struct Pending {id:u64,amount:u64,at:Stamp,baseline:Stamp,hp_before:u64,hp_after:u64,activity_end:Option<u64>,attacks:Vec<Attack>,explicit:Vec<(EntityKey,u64,Stamp)>,trusted:bool}
struct Slot {hp:u64,pending:VecDeque<Pending>,before:Vec<Attack>,explicit_before:Vec<(EntityKey,u64,Stamp)>,baseline:Stamp,rotation:usize}
#[derive(Default)]
pub struct Health {
    pub refinements:VecDeque<Refinement>,pub refinement_evictions:u64,
    slots:BTreeMap<(u64,EntityKey),Slot>,pub audit:VecDeque<Accounting>,pub outcomes:BTreeMap<&'static str,u64>,
    pub loss:u128,pub explicit:u128,pub allocated:u128,pub unassigned:u128,pub audit_evictions:u64,next_id:u64,
    own:BTreeSet<EntityKey>,self_known:bool,
    pending_count:usize,evidence_count:usize,audit_bytes:usize,
}
impl Health {
    pub(crate) fn has_baseline(&self,context:u64,target:EntityKey)->bool{self.slots.contains_key(&(context,target))}
    pub(crate) fn retire_target(&mut self,target:EntityKey){
        let contexts:Vec<_>=self.slots.keys().filter(|(_,key)|*key==target).map(|(c,_)|*c).collect();
        for context in contexts{self.clear_target(context,target);}
    }
    pub fn sentinel(&mut self,target:EntityKey,at:Stamp){
        if let Some(slot)=self.slots.get_mut(&(at.context,target)){for p in &mut slot.pending{if p.at.near(at){p.trusted=false;}}}
        self.outcome("sentinel or out-of-range amount");
    }
    pub fn constrain(&mut self,timeline:&crate::combat::Timeline){
        let mut rejected=0;
        for slot in self.slots.values_mut(){for p in &mut slot.pending{if !timeline.same_epoch(p.baseline,p.at){p.trusted=false;}p.attacks.retain(|a|{let keep=timeline.compatible(p.at,a.at);if !keep{rejected+=1;}keep});}}
        self.evidence_count-=rejected as usize;
        if rejected>0{*self.outcomes.entry("incompatible clock brackets").or_default()+=rejected;}
    }
    pub fn self_sources(&mut self,known:bool,own:BTreeSet<EntityKey>){self.self_known=known;self.own=own;}
    fn outcome(&mut self,reason:&'static str){*self.outcomes.entry(reason).or_default()+=1;}
    fn close(&mut self,target:EntityKey,p:Pending,rotation:&mut usize,forced:Option<&'static str>)->Vec<Estimate>{
        self.evidence_count-=p.attacks.len()+p.explicit.len();
        let sum:u128=p.explicit.iter().map(|(_,n,_)|*n as u128).sum();
        let mismatch=sum>p.amount as u128;
        let matched=if mismatch{0}else{sum as u64};let residual=p.amount-matched;
        let mut by_source:BTreeMap<EntityKey,Vec<&Attack>>=BTreeMap::new();
        for a in &p.attacks {by_source.entry(a.source).or_default().push(a);}
        let candidates:Vec<_>=by_source.iter().filter(|(key,attacks)| {
            let used=p.explicit.iter().filter(|(source,_,_)|source==*key).count();attacks.len()>used
        }).map(|(k,_)|*k).collect();
        let unresolved_self=candidates.iter().any(|k|self.own.contains(k));
        let reason=forced.unwrap_or(if mismatch{"explicit HP mismatch"}else if !p.trusted{"capture continuity uncertain"}
            else if residual==0{"explicit accounted"}else if candidates.is_empty(){"no immediate candidates"}
            else if candidates.len()==1{"single immediate source"}else if !self.self_known || unresolved_self{"self exclusion unresolved"}else{"equal share"});
        let mut allocations=Vec::new();
        if matches!(reason,"single immediate source"|"equal share") {
            let n=candidates.len();let share=residual/n as u64;let remainder=residual%n as u64;
            for (i,source) in candidates.iter().enumerate(){
                let amount=share+u64::from((i+n-(*rotation%n))%n<(remainder as usize));
                if amount==0{continue;}
                let attacks=&by_source[source];let keys:BTreeSet<_>=attacks.iter().map(|a|a.key1).collect();
                let skill=if n==1 && keys.len()==1{keys.first().copied().filter(|k|*k!=0)}else{None};
                allocations.push(Estimate{source:*source,target,amount,timestamp_ns:p.at.timestamp_ns,skill,interval:p.id,method:reason,frames:attacks.iter().map(|a|a.at.frame).collect(),at:p.at});
            }
            *rotation=(*rotation+remainder as usize)%n;
        }
        let allocated=allocations.iter().map(|a|a.amount).sum::<u64>();let unassigned=residual-allocated;
        self.loss+=p.amount as u128;self.explicit+=matched as u128;self.allocated+=allocated as u128;self.unassigned+=unassigned as u128;self.outcome(reason);
        let a=Accounting{id:p.id,target,at:p.at,loss:p.amount,explicit:matched,residual,unassigned,candidates,allocations:allocations.clone(),evidence:p.attacks.iter().map(|a|a.at.frame).chain(p.explicit.iter().map(|(_,_,at)|at.frame)).collect(),reason,explicit_mismatch:mismatch,baseline:p.baseline,hp_before:p.hp_before,hp_after:p.hp_after};
        let cost=Self::audit_cost(&a);
        while self.audit.len()>=HISTORY || self.audit_bytes+cost>16*1024*1024{let old=self.audit.pop_front().unwrap();self.audit_bytes-=Self::audit_cost(&old);self.audit_evictions+=1;}
        self.audit_bytes+=cost;self.audit.push_back(a);allocations
    }
    fn audit_cost(a:&Accounting)->usize{256+a.evidence.len()*8+a.candidates.len()*32+a.allocations.iter().map(|a|128+a.frames.len()*8).sum::<usize>()}
    pub fn update(&mut self,target:EntityKey,hp:u64,at:Stamp)->Result<Vec<Estimate>>{
        if hp==i64::MAX as u64{self.clear_target(at.context,target);self.outcome("MAX HP sentinel");return Ok(Vec::new());}
        let key=(at.context,target);
        // Reject capacity before expiring any interval. Otherwise Err would
        // discard estimates already committed by expire(), breaking totals.
        if !self.slots.contains_key(&key) && self.slots.len()>=4096{return Err("HP accounting capacity reached".into());}
        if self.slots.get(&key).is_some_and(|s|hp<s.hp && (self.pending_count>=8192 || s.pending.len()>=256)){
            return Err("HP interval escrow capacity reached".into());
        }
        let result=self.expire(at);
        let Some(mut slot)=self.slots.remove(&key) else{
            self.slots.insert(key,Slot{hp,pending:VecDeque::new(),before:Vec::new(),explicit_before:Vec::new(),baseline:at,rotation:0});self.outcome("initial baseline");return Ok(result);
        };
        // A later HP observation ends this decrement's attack sequence. Keep
        // its existing evidence in escrow for late explicit joins only.
        for p in &mut slot.pending{p.activity_end.get_or_insert(at.order);}
        if hp>slot.hp{self.outcome("HP rise");}
        if hp<slot.hp{
            self.next_id+=1;
            let old_count=slot.before.len()+slot.explicit_before.len();
            let attacks:Vec<_>=slot.before.drain(..).filter(|a|at.near(a.at) && a.at.order>slot.baseline.order).collect();
            let explicit:Vec<_>=slot.explicit_before.drain(..).filter(|(_,_,s)|at.near(*s) && s.order>slot.baseline.order).collect();
            self.evidence_count-=old_count-attacks.len()-explicit.len();
            slot.pending.push_back(Pending{id:self.next_id,amount:slot.hp-hp,at,baseline:slot.baseline,hp_before:slot.hp,hp_after:hp,activity_end:None,attacks,explicit,trusted:true});self.pending_count+=1;
        }
        self.evidence_count-=slot.before.len()+slot.explicit_before.len();
        slot.before.clear();slot.explicit_before.clear();slot.hp=hp;slot.baseline=at;self.slots.insert(key,slot);Ok(result)
    }
    pub fn attack(&mut self,a:Attack)->Result<()> {
        let Some(slot)=self.slots.get_mut(&(a.at.context,a.target)) else{self.outcome("attack without baseline");return Ok(());};
        if self.evidence_count>=32768{return Err("HP evidence capacity reached".into());}
        if let Some(p)=slot.pending.iter_mut().rev().find(|p|p.at.near(a.at) && p.activity_end.is_none_or(|end|a.at.order<end)){
            if p.attacks.len()>=256{return Err("HP candidate capacity reached".into());}p.attacks.push(a);self.evidence_count+=1;
        }else{
            let old_count=slot.before.len();
            slot.before.retain(|old|old.at.near(a.at));
            self.evidence_count-=old_count-slot.before.len();
            if slot.before.len()>=256{return Err("HP candidate capacity reached".into());}slot.before.push(a);self.evidence_count+=1;
        }Ok(())
    }
    pub fn explicit(&mut self,source:EntityKey,target:EntityKey,amount:u64,at:Stamp)->Result<()> {
        let Some(slot)=self.slots.get_mut(&(at.context,target))else{self.outcome("explicit without baseline");return Ok(());};
        if self.evidence_count>=32768{return Err("HP evidence capacity reached".into());}
        let matches:Vec<_>=slot.pending.iter().enumerate().filter(|(_,p)|
            p.at.process==at.process && p.at.continuity==at.continuity && p.at.timestamp_ns.abs_diff(at.timestamp_ns)<=WAIT_NS
            && p.attacks.iter().any(|a|a.source==source) && p.explicit.iter().filter(|(s,_,_)|*s==source).count()<p.attacks.iter().filter(|a|a.source==source).count()
        ).map(|(i,_)|i).collect();
        if matches.len()>1{
            for i in matches{slot.pending[i].trusted=false;}self.outcome("ambiguous explicit interval");return Ok(());
        }
        let index=matches.first().copied().or_else(||slot.pending.iter().rposition(|p|p.at.near(at) && p.activity_end.is_none_or(|end|at.order<end)));
        if let Some(p)=index.map(|i|&mut slot.pending[i]){
            if !p.at.near(at) && !matches.is_empty(){
                if self.refinements.len()==HISTORY{self.refinements.pop_front();self.refinement_evictions+=1;}
                self.refinements.push_back(Refinement{interval:p.id,source,amount,at,attack_frames:p.attacks.iter().filter(|a|a.source==source).map(|a|a.at.frame).collect()});
            }
            if p.explicit.len()>=256{return Err("HP explicit capacity reached".into());}p.explicit.push((source,amount,at));self.evidence_count+=1;
        }else{
            let old_count=slot.explicit_before.len();
            slot.explicit_before.retain(|(_,_,old)|at.near(*old));
            self.evidence_count-=old_count-slot.explicit_before.len();
            if slot.explicit_before.len()>=256{return Err("HP explicit capacity reached".into());}slot.explicit_before.push((source,amount,at));self.evidence_count+=1;
        }Ok(())
    }
    pub fn tick(&mut self,now:u64)->Vec<Estimate>{
        let contexts:BTreeMap<_,_>=self.slots.iter().filter_map(|((c,_),s)|s.pending.front().map(|p|(*c,p.at))).collect();
        let mut result=Vec::new();for (_,mut at) in contexts{at.timestamp_ns=now;result.extend(self.expire(at));}result
    }
    /// Waiting closes the evidence buffer; it never extends candidate activity.
    pub fn expire(&mut self,at:Stamp)->Vec<Estimate>{
        let keys:Vec<_>=self.slots.iter().filter(|((c,_),s)|*c==at.context && s.pending.front().is_some_and(|p|at.timestamp_ns.saturating_sub(p.at.timestamp_ns)>WAIT_NS)).map(|(k,_)|*k).collect();
        let mut result=Vec::new();for key in keys{let mut slot=self.slots.remove(&key).unwrap();while slot.pending.front().is_some_and(|p|at.timestamp_ns.saturating_sub(p.at.timestamp_ns)>WAIT_NS){let p=slot.pending.pop_front().unwrap();self.pending_count-=1;result.extend(self.close(key.1,p,&mut slot.rotation,None));}self.slots.insert(key,slot);}result
    }
    pub fn clear_context(&mut self,context:u64){
        let keys:Vec<_>=self.slots.keys().filter(|(c,_)|*c==context).copied().collect();
        for (_,target) in keys{self.clear_target(context,target);}
    }
    pub fn clear_target(&mut self,context:u64,target:EntityKey){
        if let Some(mut slot)=self.slots.remove(&(context,target)){
            self.evidence_count-=slot.before.len()+slot.explicit_before.len();
            for p in slot.pending{self.pending_count-=1;self.close(target,p,&mut slot.rotation,Some("continuity invalidated"));}
        }
    }
    pub fn clear(&mut self){let contexts:BTreeSet<_>=self.slots.keys().map(|(c,_)|*c).collect();for c in contexts{self.clear_context(c);}}
    pub fn flush(&mut self)->Vec<Estimate>{
        let slots=std::mem::take(&mut self.slots);let mut result=Vec::new();for ((_,target),mut slot) in slots{for p in slot.pending{result.extend(self.close(target,p,&mut slot.rotation,None));}}self.pending_count=0;self.evidence_count=0;result
    }
}
