//! Best-effort display counts attached after damage commit. No resolver inputs.
use crate::{combat::{Attack,Combat,Stamp},damage_wire::Survey,health::{Estimate,Health},
    identity::EntityKey,meter::{HitAssignment,Meter}};
use std::collections::{BTreeSet,VecDeque};

pub const MAX_SEEN:usize=1024;
pub const MAX_FALLBACK:usize=1024;
struct Seen{frame:u64,source:EntityKey,target:EntityKey,assignment:HitAssignment,counted:bool}
#[derive(Clone,Copy,PartialEq,Eq)]
struct Fallback{source:EntityKey,target:EntityKey,interval:u64,frame:u64}
pub struct HitCounter{seen:VecDeque<Seen>,fallback:VecDeque<Fallback>,pub evidence_evictions:u64,pub fallback_evictions:u64}
impl Default for HitCounter{fn default()->Self{Self{seen:VecDeque::with_capacity(MAX_SEEN),fallback:VecDeque::with_capacity(MAX_FALLBACK),evidence_evictions:0,fallback_evictions:0}}}
fn same_scope(a:Stamp,b:Stamp)->bool{a.process==b.process && a.context==b.context && a.continuity==b.continuity}
fn explicit_pair(a:&Attack,wire:&Survey)->bool{
    wire.observations().any(|o|o.at.frame.checked_add(1)==Some(a.at.frame) && o.at.order.checked_add(1)==Some(a.at.order)
        && same_scope(o.at,a.at) && o.fields.raw_amount!=u64::MAX
        && o.fields.source_prefix==a.source.actor as u64 && o.fields.target_prefix==a.target.actor as u64
        && o.fields.raw_flags==a.meta[..7])
}

#[cfg(test)]mod tests{
    use super::*;
    use crate::{meter::{Hit,CreditCommit},protocol::Damage};
    fn key(actor:u32)->EntityKey{EntityKey{process:0,binding:1,actor,generation:1}}
    fn at(frame:u64)->Stamp{Stamp{process:0,context:1,continuity:0,frame,order:frame,timestamp_ns:frame,tcp_byte:0}}
    fn hit(amount:u64)->Hit{Hit{timestamp_ns:3,source:key(101),target:key(999),damage:Damage{actor:101,target:999,amount,flags:[false;5],raw_flags:[0;7]}}}
    fn meter()->Meter{let mut m=Meter::default();m.registry.observe(0,1,101).unwrap();m.registry.observe(0,1,999).unwrap();m}
    fn estimate(interval:u64)->Estimate{Estimate{source:key(101),target:key(999),amount:10,timestamp_ns:3,skill:None,interval,method:"test",frames:vec![],at:at(3)}}
    #[test]fn late_explicit_counterpart_replaces_count_without_changing_damage(){
        let mut health=Health::default();health.update(key(999),100,at(0)).unwrap();
        let attack=Attack{source:key(101),target:key(999),key1:77,key2:0,meta:[0;8],at:at(2)};
        health.attack(attack.clone()).unwrap();health.update(key(999),90,at(3)).unwrap();let e=health.flush().pop().unwrap();
        let mut combat=Combat::default();combat.attack(attack);let mut wire=Survey::default();let mut m=meter();let mut counter=HitCounter::default();
        assert_eq!(m.commit_credit(1,hit(10),true,None).unwrap(),CreditCommit::Committed);
        counter.inferred(&e,m.last_hit_assignment(true).unwrap(),&combat,&wire,&health,&mut m);assert_eq!(m.snapshot().display_hits,1);
        let mut raw=[0;53];raw[..8].copy_from_slice(&101u64.to_le_bytes());raw[8..16].copy_from_slice(&999u64.to_le_bytes());raw[16..24].copy_from_slice(&4u64.to_le_bytes());wire.observe(at(1),3,&raw,true);
        m.commit_credit(2,hit(4),false,None).unwrap();counter.explicit(key(101),key(999),at(1),&combat,&wire,&mut m);
        assert_eq!(m.snapshot().display_hits,1);assert_eq!(m.snapshot().summary.damage,14);assert_eq!(m.session.estimated_total.damage,10);assert_eq!(m.session.total.hits,1);
        counter.explicit(key(101),key(999),at(1),&combat,&wire,&mut m);assert_eq!(m.snapshot().display_hits,1);
    }
    #[test]fn split_delivery_of_one_fallback_budget_counts_once(){
        let (mut m,mut counter)=(meter(),HitCounter::default());let (combat,wire,health)=(Combat::default(),Survey::default(),Health::default());let e=estimate(1);
        for receipt in [1,2]{m.commit_credit(receipt,hit(10),true,None).unwrap();counter.inferred(&e,m.last_hit_assignment(true).unwrap(),&combat,&wire,&health,&mut m);}
        assert_eq!(m.snapshot().display_hits,1);assert_eq!(m.snapshot().summary.damage,20);
    }
    #[test]fn counter_pressure_stays_bounded_and_never_rejects_damage(){
        let (mut m,mut counter)=(meter(),HitCounter::default());let (combat,wire,health)=(Combat::default(),Survey::default(),Health::default());
        for receipt in 1..=(MAX_FALLBACK+8)as u64{let e=estimate(receipt);m.commit_credit(receipt,hit(10),true,None).unwrap();counter.inferred(&e,m.last_hit_assignment(true).unwrap(),&combat,&wire,&health,&mut m);}
        assert_eq!(counter.fallback.len(),MAX_FALLBACK);assert_eq!(counter.fallback_evictions,8);assert!(counter.owned_bytes()<=HitCounter::ceiling_bytes());assert_eq!(m.snapshot().summary.damage,((MAX_FALLBACK+8)*10)as u128);
    }
}
impl HitCounter{
    pub fn ceiling_bytes()->usize{std::mem::size_of::<Self>()+MAX_SEEN*(std::mem::size_of::<Seen>()+64)+MAX_FALLBACK*std::mem::size_of::<Fallback>()}
    pub fn owned_bytes(&self)->usize{std::mem::size_of::<Self>()+self.seen.capacity()*(std::mem::size_of::<Seen>()+64)+self.fallback.capacity()*std::mem::size_of::<Fallback>()}
    pub fn inferred(&mut self,e:&Estimate,assignment:HitAssignment,combat:&Combat,wire:&Survey,health:&Health,meter:&mut Meter){
        let budget=health.audit.iter().find(|a|a.id==e.interval && a.target==e.target && a.at==e.at);
        let frames:BTreeSet<u64>=budget.into_iter().flat_map(|a|a.allocations.iter().filter(|x|x.source==e.source && x.skill==e.skill))
            .flat_map(|a|a.frames.iter().copied()).collect();
        let mut count=0u64;let mut supported=false;
        for a in combat.attacks.iter().filter(|a|a.source==e.source && a.target==e.target && same_scope(a.at,e.at)
            && frames.contains(&a.at.frame) && budget.is_some_and(|b|b.evidence.contains(&a.at.frame))){
            supported=true;
            if self.seen.iter().any(|s|s.frame==a.at.frame && s.source==e.source && s.target==e.target){continue;}
            let counted=!explicit_pair(a,wire);count+=u64::from(counted);
            if self.seen.len()==MAX_SEEN{self.seen.pop_front();self.evidence_evictions+=1;}
            self.seen.push_back(Seen{frame:a.at.frame,source:e.source,target:e.target,assignment:assignment.clone(),counted});
        }
        // Reconciliation can split one actor into a presentation-backed skill
        // row and an explicit-only Unknown row. That residual is not a second
        // observed hit merely because it takes another delivery row.
        if !supported && budget.is_some_and(|b|wire.observations().any(|o|b.evidence.contains(&o.at.frame)
            && same_scope(o.at,e.at) && o.fields.raw_amount!=u64::MAX
            && o.fields.source_prefix==e.source.actor as u64 && o.fields.target_prefix==e.target.actor as u64)){
            supported=true;
        }
        if !supported{
            // One best-effort hit per distinct source/target HP obligation.
            // Chunked delivery/retries cannot turn the same budget into hits.
            let key=Fallback{source:e.source,target:e.target,interval:e.interval,frame:e.at.frame};
            if !self.fallback.contains(&key){
                if self.fallback.len()==MAX_FALLBACK{self.fallback.pop_front();self.fallback_evictions+=1;}
                self.fallback.push_back(key);count=1;
            }
        }
        meter.set_display_hits(e.source,e.target,&assignment,count);
    }
    pub fn explicit(&mut self,source:EntityKey,target:EntityKey,at:Stamp,combat:&Combat,wire:&Survey,meter:&mut Meter){
        // A late explicit counterpart replaces the estimated observation's
        // contribution, including after a late join moved its membership.
        let Some(a)=combat.attacks.iter().find(|a|a.source==source && a.target==target && at.frame.checked_add(1)==Some(a.at.frame)
            && same_scope(at,a.at) && explicit_pair(a,wire))else{return;};
        for seen in self.seen.iter_mut().filter(|s|s.frame==a.at.frame && s.source==source && s.target==target && s.counted){
            meter.set_display_hits(source,target,&seen.assignment,seen.assignment.display_hits().saturating_sub(1));seen.counted=false;
        }
    }
}
