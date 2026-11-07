//! Bounded diagnostic history plus non-evicting acknowledgment counters. The
//! raw recording is the durable evidence; ring eviction is explicit in audits.
use crate::{combat::Stamp,identity::EntityKey,replay::{key_json,stamp_json},hp_wire};
use serde_json::{json,Value};
use std::collections::{BTreeMap,BTreeSet,VecDeque};
const HISTORY:usize=8192;
#[derive(Clone,Debug)]
pub struct Candidate {
    pub at:Stamp,pub wire:Option<i32>,pub shape:&'static str,pub prefix_u64:Option<u64>,
    pub length:usize,pub raw:Vec<u8>,pub outcome:String,pub status:Option<Value>,pub semantic_reached:bool,
}
#[derive(Clone,Debug)]
pub struct Observation {
    pub target:EntityKey,pub at:Stamp,pub hp:u64,pub role:&'static str,
    pub conditional_key:bool,pub previous:Option<(Stamp,u64)>,pub comparison:&'static str,
}
#[derive(Clone,Debug)]
pub struct Unbound {pub at:Stamp,pub actor:u64,pub hp:u64,pub reason:&'static str}
#[derive(Default)]
pub struct Ledger {
    pub candidates:VecDeque<Candidate>,pub observations:VecDeque<Observation>,
    pub unbound:VecDeque<Unbound>,pub unbound_count:u64,pub unbound_evictions:u64,
    pending_bindings:BTreeSet<(u64,u32)>,
    binding_overflow:BTreeSet<u64>,
    pub counts:BTreeMap<String,u64>,pub outcomes:BTreeMap<String,u64>,pub comparisons:BTreeMap<String,u64>,
    pub acknowledged_candidates:u64,pub candidate_evictions:u64,pub observation_evictions:u64,pub baseline_evictions:u64,
    last:BTreeMap<(u64,EntityKey),(Stamp,u64)>,
    broken:BTreeSet<(u64,EntityKey)>,
}
impl Ledger {
    pub fn acknowledge(&mut self,at:Stamp,wire:Option<i32>,p:&[u8],decoded:bool){
        let shape=hp_wire::shape(p,decoded);
        *self.counts.entry(shape.into()).or_default()+=1;
        let status=matches!(wire,Some(20051|20054|20517));
        let family=matches!(wire,Some(170205..=170210));
        if !(shape.contains("candidate") || !decoded || status || family){return;}
        self.acknowledged_candidates+=1;
        if self.candidates.len()==HISTORY{self.candidates.pop_front();self.candidate_evictions+=1;}
        let detail=if status && decoded{Some(match hp_wire::status(wire.unwrap(),p){
            Ok(s)=>json!({"actor_wire":s.actor,"offset":s.offset,"end":s.end,"mode_raw":s.mode,"parameter_marker_raw":s.parameter_marker,"level_raw":s.level,
                "hp_length":s.hp.len(),"hp_hex":crate::replay::hex(&s.hp[..s.hp.len().min(64)]),"iv_length":s.iv.len(),"iv_hex":crate::replay::hex(&s.iv[..s.iv.len().min(64)]),"iv_offset":s.iv_offset,"float_bits":s.float_bits,"following_raw":s.following}),
            Err(e)=>json!({"prefix_failure":format!("{e:?}")}),
        })}else{None};
        self.candidates.push_back(Candidate{at,wire,shape,prefix_u64:p.get(..8).map(|v|crate::protocol::u64le(v,0)),status:detail,semantic_reached:false,
            length:p.len(),raw:p.iter().take(128).copied().collect(),outcome:if !decoded{"opaque; numeric role unavailable"}
            else if status{"status pending"}else if family{"HP pending"}else{"unconfirmed packet role"}.into()});
    }
    pub fn reached(&mut self,at:Stamp){
        if let Ok(index)=self.candidates.binary_search_by_key(&at.frame,|c|c.at.frame){let c=&mut self.candidates[index];c.at=at;c.semantic_reached=true;}
    }
    pub fn outcome(&mut self,frame:u64,reason:&str){
        *self.outcomes.entry(reason.into()).or_default()+=1;
        if let Ok(index)=self.candidates.binary_search_by_key(&frame,|c|c.at.frame){self.candidates[index].outcome=reason.into();}
    }
    pub fn forget(&mut self,context:u64,target:EntityKey){self.last.remove(&(context,target));self.broken.remove(&(context,target));}
    pub fn break_target(&mut self,context:u64,target:EntityKey){if self.last.contains_key(&(context,target)){self.broken.insert((context,target));}}
    pub fn partition(&mut self){self.broken.extend(self.last.keys().copied());}
    pub fn clear(&mut self){self.last.clear();self.broken.clear();self.pending_bindings.clear();self.binding_overflow.clear();}
    pub fn retire_context(&mut self,context:u64){
        self.last.retain(|(c,_),_|*c!=context);self.broken.retain(|(c,_)|*c!=context);
        self.pending_bindings.retain(|(c,_)|*c!=context);self.binding_overflow.remove(&context);
    }
    pub fn retire_target(&mut self,target:EntityKey){self.last.retain(|(_,key),_|*key!=target);self.broken.retain(|(_,key)|*key!=target);}
    pub fn blocked(&self,context:u64,actor:u32)->bool{self.binding_overflow.contains(&context)||self.pending_bindings.contains(&(context,actor))}
    pub fn resolve(&mut self,context:u64,actor:u32){self.pending_bindings.remove(&(context,actor));}
    pub fn block(&mut self,context:u64,actor:u32){
        if !self.blocked(context,actor) && self.pending_bindings.len()>=4096{
            self.binding_overflow.insert(context);*self.outcomes.entry("unresolved binding capacity; context quarantined".into()).or_default()+=1;return;
        }
        if !self.binding_overflow.contains(&context){self.pending_bindings.insert((context,actor));}
    }
    pub fn unbound(&mut self,at:Stamp,actor:u64,hp:u64,reason:&'static str){
        self.unbound_count+=1;
        if self.unbound.len()==HISTORY{self.unbound.pop_front();self.unbound_evictions+=1;}
        self.unbound.push_back(Unbound{at,actor,hp,reason});
        self.outcome(at.frame,"numeric HP decoded; binding unresolved");
    }
    /// Numeric comparisons across a continuity break are audit-only. They are
    /// never fed back as damage; the causal matcher starts a fresh baseline.
    pub fn observe(&mut self,target:EntityKey,hp:u64,at:Stamp,role:&'static str,conditional_key:bool){
        let key=(at.context,target);
        let previous=self.last.remove(&key);
        let broken=self.broken.remove(&key);
        let comparison=if hp==i64::MAX as u64{"MAX sentinel"}
            else if let Some((old,n))=previous{
                if broken || old.continuity!=at.continuity{"gap comparison; audit only"}
                else if hp<n{"decrease"}else if hp>n{"rise"}else{"unchanged"}
            }else{"initial observation"};
        *self.comparisons.entry(comparison.into()).or_default()+=1;
        if hp!=i64::MAX as u64{
            if self.last.len()>=4096{
                if let Some(k)=self.last.iter().min_by_key(|(_, (at,_))|at.frame).map(|(k,_)|*k){self.last.remove(&k);self.broken.remove(&k);self.baseline_evictions+=1;}
            }
            self.last.insert(key,(at,hp));
        }
        if self.observations.len()==HISTORY{self.observations.pop_front();self.observation_evictions+=1;}
        self.observations.push_back(Observation{target,at,hp,role,conditional_key,previous,comparison});
    }
    pub fn audit(&self)->Value{
        json!({"body_classifications":self.counts,"outcomes":self.outcomes,"comparisons":self.comparisons,
            "unbound_count":self.unbound_count,"unbound_evictions":self.unbound_evictions,"pending_bindings":self.pending_bindings.len(),"binding_overflow_contexts":self.binding_overflow,
            "unbound":self.unbound.iter().map(|o|json!({"at":stamp_json(o.at),"actor_wire":o.actor,"hp":o.hp,"reason":o.reason})).collect::<Vec<_>>(),
            "acknowledged_candidates":self.acknowledged_candidates,"candidate_evictions":self.candidate_evictions,"observation_evictions":self.observation_evictions,"baseline_evictions":self.baseline_evictions,
            "storage":"bounded previews; complete raw bodies remain in source recording; evictions counted",
            "crypto":"CBC padding checks structure, not authenticity; retained keys after opaque bodies or loss are conditional",
            "candidates":self.candidates.iter().map(|c|json!({"at":stamp_json(c.at),"wire":c.wire,"shape":c.shape,"prefix_u64":c.prefix_u64,"length":c.length,"prefix_hex":crate::replay::hex(&c.raw),"preview_truncated":c.raw.len()<c.length,"status":c.status,"outcome":c.outcome,"semantic_reached":c.semantic_reached})).collect::<Vec<_>>(),
            "observations":self.observations.iter().map(|o|json!({"target":key_json(o.target),"at":stamp_json(o.at),"hp":o.hp,"role":o.role,"conditional_key":o.conditional_key,"comparison":o.comparison,
                "previous":o.previous.map(|(at,hp)|json!({"at":stamp_json(at),"hp":hp})),
                "signed_change":o.previous.filter(|_|o.hp!=i64::MAX as u64).map(|(_,hp)|(o.hp as i128-hp as i128).to_string())})).collect::<Vec<_>>()})
    }
}
