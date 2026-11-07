//! One numeric-HP comparison authority, independent of labels, packet caches,
//! decryption-key capacity and presentation association. Diagnostic ledgers are
//! consumers of this decision; they cannot veto a detected obligation.
use crate::{combat::Stamp,identity::EntityKey};
use std::collections::VecDeque;
pub const MAX_BASELINES:usize=1024;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Disposition{Initial,Unchanged,Decrease,Rise,ParallelOverlap,Sentinel}
#[derive(Clone,Copy,Debug)]
pub struct Observation{pub previous:Option<(Stamp,u64)>,pub disposition:Disposition,pub uncertain:bool}
struct Baseline{target:EntityKey,wire:u64,hp:u64,at:Stamp}
#[derive(Default)]
pub struct Numeric {
    baselines:VecDeque<Baseline>,
    pub observations:u64,pub decreases:u64,pub detected:u128,pub sentinels:u64,
    pub parallel_overlap:u64,pub comparisons_reclaimed:u64,
}
impl Numeric{
    pub fn observe(&mut self,target:EntityKey,wire:u64,hp:u64,at:Stamp,local_prior:Option<(Stamp,u64)>)->Observation{
        self.observations+=1;
        if hp==i64::MAX as u64{self.sentinels+=1;return Observation{previous:None,disposition:Disposition::Sentinel,uncertain:true};}
        let index=self.baselines.iter().position(|b|b.target==target && b.wire==wire);
        let old=index.and_then(|i|self.baselines.remove(i));
        // An unbound incarnation may acquire identity without losing an already
        // observed amount. Wire identity stays the same; confidence does not.
        let old=old.or_else(||self.baselines.iter().position(|b|b.target.process==target.process && b.target.binding==target.binding && b.target.actor==target.actor && b.target.generation==0 && b.wire==wire).and_then(|i|self.baselines.remove(i)));
        let previous=old.as_ref().map(|b|(b.at,b.hp));
        if let Some(b)=&old{
            // Concurrent TCP publications do not establish a second independent
            // HP budget. Unique lower endpoints are admitted; stale equal/higher
            // snapshots overlap the already observed decline. A rise requires a
            // locally observed rise from an aligned previous value.
            let parallel=b.at.context!=at.context;
            let aligned_rise=local_prior.is_some_and(|(_,n)|n==b.hp && hp>n);
            if parallel && hp>=b.hp && !aligned_rise{
                self.parallel_overlap+=1;self.baselines.push_back(old.unwrap());
                return Observation{previous,disposition:Disposition::ParallelOverlap,uncertain:true};
            }
        }
        let disposition=match previous{Some((_,n))if hp<n=>{self.decreases+=1;self.detected+=(n-hp)as u128;Disposition::Decrease},Some((_,n))if hp>n=>Disposition::Rise,Some(_)=>Disposition::Unchanged,None=>Disposition::Initial};
        let uncertain=previous.is_some_and(|(old,_)|old.context!=at.context || old.continuity!=at.continuity) || target.generation==0;
        if self.baselines.len()==MAX_BASELINES{self.baselines.pop_front();self.comparisons_reclaimed+=1;}
        self.baselines.push_back(Baseline{target,wire,hp,at});Observation{previous,disposition,uncertain}
    }
    /// Only a proven object replacement invalidates an object's comparison.
    /// Gaps, opaque packets, memory pressure and meter partitions do not.
    pub fn retire(&mut self,target:EntityKey){self.baselines.retain(|b|b.target!=target);}
    pub fn clear_process(&mut self,process:u64){self.baselines.retain(|b|b.target.process!=process);}
    pub fn owned_bytes(&self)->usize{std::mem::size_of::<Self>()+self.baselines.capacity()*std::mem::size_of::<Baseline>()}
    pub fn ceiling_bytes()->usize{std::mem::size_of::<Self>()+MAX_BASELINES*std::mem::size_of::<Baseline>()}
    pub fn audit(&self)->serde_json::Value{serde_json::json!({"authority":"numeric HP observation before association/capacity/identity enrichment","observations":self.observations,"detected_decreases":self.decreases,"detected_hp":self.detected.to_string(),"sentinels":self.sentinels,"parallel_overlap":self.parallel_overlap,"baseline_reclamations":self.comparisons_reclaimed,"retained":self.baselines.len(),"capacity":MAX_BASELINES,"owned_bytes":self.owned_bytes(),"ceiling_bytes":Self::ceiling_bytes(),"reclamation":"bounds future comparisons; already detected obligations belong to attribution and are never removed"})}
}
