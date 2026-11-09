//! Bounded packet-first HP reconciliation; owner grouping is downstream.
use crate::{attribution::{Attribution,Candidate},combat::{Attack,Stamp,ClockPoint,DamageModel},identity::EntityKey,protocol::u64le,Result};
use std::collections::{BTreeMap,BTreeSet,VecDeque};
use serde_json::{json,Value};
pub const WAIT_NS:u64=250_000_000;
pub const LOCAL_NS:u64=100_000_000;
pub const MAX_SLOTS:usize=1024;
pub const MAX_PENDING:usize=1024;
pub const MAX_TARGET_PENDING:usize=64;
pub const MAX_FACTS:usize=4096;
pub const MAX_TARGET_FACTS:usize=128;
pub const MAX_AUDIT:usize=512;
pub const MAX_AUDIT_BYTES:usize=1024*1024;
pub const MAX_READY:usize=1024;
pub const MAX_READY_FRAME_CAPACITY:usize=MAX_FACTS*4;
pub const MAX_ORPHAN_CLAIMS:usize=256;
pub fn parse(p:&[u8])->Option<(u32,u64)>{let e=crate::hp_wire::envelope(p)?;if e.blob.len()!=8{return None;}let hp=u64le(e.blob,0);(e.actor>0 && e.actor<=u32::MAX as u64 && hp<=i64::MAX as u64).then_some((e.actor as u32,hp))}
#[derive(Clone,Debug)]
pub struct Estimate{pub source:EntityKey,pub target:EntityKey,pub amount:u64,pub timestamp_ns:u64,pub skill:Option<u32>,pub interval:u64,pub method:&'static str,pub frames:Vec<u64>,pub at:Stamp}
#[derive(Clone,Debug)]
pub struct Accounting{pub id:u64,pub target:EntityKey,pub at:Stamp,pub loss:u64,pub explicit:u64,pub raw_explicit:u128,pub residual:u64,pub unassigned:u64,pub candidates:Vec<EntityKey>,pub allocations:Vec<Estimate>,pub evidence:Vec<u64>,pub reason:&'static str,pub explicit_mismatch:bool,pub baseline:Stamp,pub hp_before:u64,pub hp_after:u64}
#[derive(Clone,Debug)]
pub struct Refinement{pub interval:u64,pub source:EntityKey,pub amount:u64,pub at:Stamp,pub attack_frames:Vec<u64>}
#[derive(Clone,Copy,Default)]
struct Bounds{lower:Option<ClockPoint>,upper:Option<ClockPoint>}
impl Bounds{fn compatible(self,b:Self)->bool{
    if let(Some(a),Some(b))=(self.lower,b.lower){if a.epoch!=b.epoch{return false;}}
    match(self.lower,self.upper,b.lower,b.upper){(Some(a),Some(au),Some(b),Some(bu))=>a.epoch==au.epoch && a.epoch==b.epoch && b.epoch==bu.epoch && a.ticks<=bu.ticks.saturating_add(1) && b.ticks<=au.ticks.saturating_add(1),_=>true}
}}
#[derive(Clone)]
enum Kind{Attack(Attack),Explicit{source:EntityKey,raw:u64,impact:Option<u64>,flags:[u8;7]}}
#[derive(Clone)]
struct Fact{inference_excluded:bool,id:u64,at:Stamp,available:u64,kind:Kind,bounds:Bounds}
struct OrphanClaim{target:EntityKey,fact:Fact}
struct Pending{id:u64,amount:u64,at:Stamp,baseline:Stamp,hp_before:u64,hp_after:u64,end:Option<Stamp>,available:u64,deadline:u64,bounds:Bounds,baseline_bounds:Bounds,clock_uncertain:bool,sentinel_uncertain:bool,conditional:bool}
struct Slot{hp:u64,baseline:Stamp,baseline_training:bool,pending:VecDeque<Pending>,facts:VecDeque<Fact>,truncated:bool}
#[derive(Default)]
struct Assignment{attacks:Vec<usize>,explicit:Vec<usize>,component_flags:BTreeSet<[u8;7]>,component_sources:BTreeSet<(EntityKey,[u8;7])>,impact_sum:u128,ambiguous:bool,batched:bool,unsupported:bool}
type SlotKey=(u64,EntityKey);
pub struct Health{
    pub attribution:Attribution,
    orphan_claims:VecDeque<OrphanClaim>,pub orphan_claim_evictions:u64,
    pub refinements:VecDeque<Refinement>,pub refinement_evictions:u64,
    slots:BTreeMap<SlotKey,Slot>,deadlines:BTreeSet<(u64,u64,EntityKey,u64)>,fact_order:BTreeSet<(u64,u64,EntityKey)>,
    pub audit:VecDeque<Accounting>,pub outcomes:BTreeMap<&'static str,u64>,
    pub loss:u128,pub explicit:u128,pub allocated:u128,pub unassigned:u128,pub audit_evictions:u64,
    pub retention_ns:u64,pub local_capture_ns:u64,
    pub no_support_loss:u128,pub capacity_loss:u128,pub boundary_loss:u128,pub unsupported_loss:u128,pub clock_loss:u128,pub fact_evictions:u64,pub late_explicit:u64,
    pub peak_facts:usize,pub peak_pending:usize,pub peak_slots:usize,pub max_resolver_comparisons:u64,
    pending_count:usize,fact_count:usize,audit_bytes:usize,next_id:u64,next_fact:u64,available:u64,
    fact_capacity:usize,pending_capacity:usize,ready_frame_capacity:usize,peak_owned_bytes:usize,
    decision_age_bins:[u64;64],decision_age_max:u64,decision_count:u64,
    ready:VecDeque<Estimate>,dirty:BTreeSet<SlotKey>,last_committed:BTreeMap<SlotKey,Stamp>,last_registry_revision:Option<u64>,
}
impl Default for Health{fn default()->Self{Self{attribution:Attribution::default(),orphan_claims:VecDeque::new(),orphan_claim_evictions:0,refinements:VecDeque::new(),refinement_evictions:0,slots:BTreeMap::new(),deadlines:BTreeSet::new(),fact_order:BTreeSet::new(),audit:VecDeque::with_capacity(MAX_AUDIT),outcomes:BTreeMap::new(),loss:0,explicit:0,allocated:0,unassigned:0,audit_evictions:0,retention_ns:WAIT_NS,local_capture_ns:LOCAL_NS,no_support_loss:0,capacity_loss:0,boundary_loss:0,unsupported_loss:0,clock_loss:0,fact_evictions:0,late_explicit:0,peak_facts:0,peak_pending:0,peak_slots:0,max_resolver_comparisons:0,pending_count:0,fact_count:0,audit_bytes:0,next_id:0,next_fact:0,available:0,fact_capacity:0,pending_capacity:0,ready_frame_capacity:0,peak_owned_bytes:0,decision_age_bins:[0;64],decision_age_max:0,decision_count:0,ready:VecDeque::new(),dirty:BTreeSet::new(),last_committed:BTreeMap::new(),last_registry_revision:None}}}
impl Health{
    fn owned_bytes(&self)->usize{
        std::mem::size_of::<Self>()+self.attribution.owned_bytes()+self.orphan_claims.capacity()*std::mem::size_of::<OrphanClaim>()+self.slots.len()*(std::mem::size_of::<Slot>()+256)+self.fact_capacity*std::mem::size_of::<Fact>()+self.pending_capacity*std::mem::size_of::<Pending>()+self.fact_order.len()*96+self.deadlines.len()*112+self.audit_bytes+self.audit.capacity()*std::mem::size_of::<Accounting>()+self.ready.capacity()*std::mem::size_of::<Estimate>()+self.ready_frame_capacity*8+self.refinements.capacity()*std::mem::size_of::<Refinement>()+self.refinements.len()*8+self.last_committed.len()*128+self.dirty.len()*96+self.outcomes.len()*128
    }
    fn peak(&mut self){self.peak_owned_bytes=self.peak_owned_bytes.max(self.owned_bytes());}
    pub fn set_available(&mut self,now:u64){self.available=self.available.max(now);}
    fn now(&mut self,at:Stamp)->u64{self.available=self.available.max(at.timestamp_ns);self.available}
    /// Legacy compatibility; production receives self/coverage policy from Registry.
    pub fn self_sources(&mut self,_known:bool,_own:BTreeSet<EntityKey>){}
    pub fn note_actor(&mut self,source:EntityKey,at:Stamp){self.attribution.note_actor(source,at);self.allocated=self.attribution.settled_inferred;self.unassigned=self.attribution.unattributed_inferred;}
    pub fn refresh_support(&mut self){self.attribution.refresh_support();self.allocated=self.attribution.settled_inferred;self.unassigned=self.attribution.unattributed_inferred;}
    pub fn front_credit(&mut self)->Option<crate::attribution::Credit>{let credit=self.attribution.front_ready_credit(self.available);self.allocated=self.attribution.settled_inferred;self.unassigned=self.attribution.unattributed_inferred;credit}
    pub fn note_actor_kind(&mut self,source:EntityKey,kind:&str,at:Stamp){self.attribution.note_actor_kind(source,kind,at);self.allocated=self.attribution.settled_inferred;self.unassigned=self.attribution.unattributed_inferred;}
    pub fn unbound_explicit(&mut self,target:EntityKey,raw:u64,flags:[u8;7],at:Stamp)->Result<()>{self.unbound_explicit_fields(target,raw,None,flags,at)}
    pub fn unbound_explicit_fields(&mut self,target:EntityKey,raw:u64,impact:Option<u64>,flags:[u8;7],at:Stamp)->Result<()>{
        let now=self.now(at);let deadline=if self.slots.contains_key(&(at.context,target)){now.saturating_add(self.local_capture_ns).saturating_add(self.retention_ns)}else{0};self.attribution.submit_unbound_explicit_held(target,raw,flags,at,deadline);
        let kind=Kind::Explicit{source:EntityKey{process:at.process,binding:0,actor:0,generation:0},raw,impact,flags};if self.slots.contains_key(&(at.context,target)){self.push_fact((at.context,target),at,kind);}else{self.retain_orphan(target,at,kind);}
        Ok(())
    }
    fn outcome(&mut self,reason:&'static str){*self.outcomes.entry(reason).or_default()+=1;}
    fn same(a:Stamp,b:Stamp)->bool{a.process==b.process && a.context==b.context && a.continuity==b.continuity}
    fn relevant(p:&Pending,f:&Fact,local:u64)->bool{Self::same(p.at,f.at) && p.at.timestamp_ns.abs_diff(f.at.timestamp_ns)<=local}
    pub fn revalidate_registry(&mut self,registry:&crate::identity::Registry){
        if self.last_registry_revision==Some(registry.revision){return;}self.last_registry_revision=Some(registry.revision);
        self.attribution.revalidate_registry(registry);let mut removed=0u64;
        for(key,s)in &mut self.slots{
            let old_capacity=s.facts.capacity();let mut ids=Vec::new();
            for f in &mut s.facts{f.inference_excluded=match &f.kind{Kind::Attack(a)=>registry.inference_excluded(a.source),Kind::Explicit{..}=>true};}
            s.facts.retain(|f|{let valid=match &f.kind{Kind::Attack(a)=>registry.damage_eligible(a.source,a.target),Kind::Explicit{source,..}=>source.actor==0 || registry.damage_eligible(*source,key.1)};if !valid{ids.push(f.id);}valid});
            if !ids.is_empty(){s.truncated=true;for p in &mut s.pending{p.conditional=true;}for id in ids{if self.fact_order.remove(&(id,key.0,key.1)){self.fact_count-=1;removed+=1;}}}
            if s.facts.capacity()>s.facts.len()*2{s.facts.shrink_to_fit();}self.fact_capacity=self.fact_capacity-old_capacity+s.facts.capacity();
        }
        self.orphan_claims.retain(|o|match &o.fact.kind{Kind::Explicit{source,..}=>source.actor==0 || registry.damage_eligible(*source,o.target),_=>true});
        if removed>0{*self.outcomes.entry("authoritative friendly-fire facts excluded before association and training").or_default()+=removed;}
    }
    pub fn constrain(&mut self,timeline:&crate::combat::Timeline){
        // Only dirty target-local state: unrelated frames do not sweep all slots.
        let dirty=std::mem::take(&mut self.dirty);
        for key in dirty{if let Some(s)=self.slots.get_mut(&key){
            for f in &mut s.facts{if let Some(b)=timeline.bounds.get(&f.at.frame){f.bounds=Bounds{lower:b.lower,upper:b.upper};}}
            for p in &mut s.pending{
                if let Some(b)=timeline.bounds.get(&p.at.frame){p.bounds=Bounds{lower:b.lower,upper:b.upper};}
                if let Some(b)=timeline.bounds.get(&p.baseline.frame){p.baseline_bounds=Bounds{lower:b.lower,upper:b.upper};}
                if let(Some(a),Some(b))=(p.bounds.lower,p.baseline_bounds.lower){if a.epoch!=b.epoch{p.clock_uncertain=true;}}
            }
        }}
    }
    pub fn clock_context(&mut self,context:u64){
        // Called only for an actual clock publication, not for unrelated traffic.
        for(_,c,t,_)in self.deadlines.iter().filter(|(_,c,_,_)|*c==context){self.dirty.insert((*c,*t));}
    }
    pub fn conditional_target(&mut self,context:u64,target:EntityKey){if let Some(s)=self.slots.get_mut(&(context,target)){s.baseline_training=false;for p in &mut s.pending{p.conditional=true;}}}
    pub fn sentinel(&mut self,target:EntityKey,at:Stamp){
        let local=self.local_capture_ns;
        if let Some(s)=self.slots.get_mut(&(at.context,target)){for p in &mut s.pending{if Self::same(p.at,at) && p.at.timestamp_ns.abs_diff(at.timestamp_ns)<=local{p.sentinel_uncertain=true;}}}
        self.outcome("unpublished sentinel; actor inference uncertain");
    }
    fn retain_orphan(&mut self,target:EntityKey,at:Stamp,kind:Kind){
        if !matches!(kind,Kind::Explicit{..}){return;}
        if self.orphan_claims.len()==MAX_ORPHAN_CLAIMS{self.orphan_claims.pop_front();self.orphan_claim_evictions+=1;}
        self.next_fact+=1;let available=self.now(at);self.orphan_claims.push_back(OrphanClaim{target,fact:Fact{inference_excluded:match &kind{Kind::Attack(a)=>!self.attribution.inference_eligibility(a.source,a.target),Kind::Explicit{..}=>true},id:self.next_fact,at,available,kind,bounds:Bounds::default()}});
    }
    fn take_orphans(&mut self,target:EntityKey,baseline:Stamp,at:Stamp)->Vec<Fact>{
        let mut result=Vec::new();let count=self.orphan_claims.len();for _ in 0..count{let o=self.orphan_claims.pop_front().unwrap();
            let after_baseline=if Self::same(o.fact.at,baseline){o.fact.at.order>baseline.order}else{o.fact.at.timestamp_ns>=baseline.timestamp_ns};
            let eligible=o.target==target && o.fact.at.process==at.process && after_baseline && o.fact.at.timestamp_ns<=at.timestamp_ns.saturating_add(self.local_capture_ns) && self.available.saturating_sub(o.fact.available)<=self.retention_ns;
            let forbidden=matches!(&o.fact.kind,Kind::Explicit{source,..}if source.actor!=0 && !self.attribution.eligibility(*source,target));if forbidden{self.outcome("forbidden orphan explicit excluded");}else if eligible{result.push(o.fact);}else{self.orphan_claims.push_back(o);}
        }result
    }
    fn drop_fact(&mut self,key:SlotKey,id:u64,capacity:bool){
        if let Some(s)=self.slots.get_mut(&key){if let Some(i)=s.facts.iter().position(|f|f.id==id){let old=s.facts.capacity();let f=s.facts.remove(i).unwrap();self.fact_order.remove(&(f.id,key.0,key.1));self.fact_count-=1;if s.facts.capacity()>s.facts.len()*2{s.facts.shrink_to_fit();}self.fact_capacity=self.fact_capacity-old+s.facts.capacity();if capacity{s.truncated=true;self.fact_evictions+=1;}self.retain_orphan(key.1,f.at,f.kind);}}
    }
    fn push_fact(&mut self,key:SlotKey,at:Stamp,kind:Kind){
        if !self.slots.contains_key(&key){return;}
        while self.slots[&key].facts.len()>=MAX_TARGET_FACTS{let id=self.slots[&key].facts.front().unwrap().id;self.drop_fact(key,id,true);self.outcome("per-target fact capacity");}
        while self.fact_count>=MAX_FACTS{let Some((id,c,t))=self.fact_order.first().copied()else{break;};self.drop_fact((c,t),id,true);self.outcome("global fact capacity");}
        self.next_fact+=1;let available=self.now(at);
        let s=self.slots.get_mut(&key).unwrap();let old=s.facts.capacity();if old==0{s.facts=VecDeque::with_capacity(1);}s.facts.push_back(Fact{inference_excluded:match &kind{Kind::Attack(a)=>!self.attribution.inference_eligibility(a.source,a.target),Kind::Explicit{..}=>true},id:self.next_fact,at,available,kind,bounds:Bounds::default()});self.fact_capacity=self.fact_capacity-old+s.facts.capacity();self.fact_order.insert((self.next_fact,key.0,key.1));self.fact_count+=1;self.peak_facts=self.peak_facts.max(self.fact_count);self.dirty.insert(key);self.peak();
    }
    pub fn attack(&mut self,a:Attack)->Result<()>{if !self.attribution.eligibility(a.source,a.target){self.outcome("forbidden same-side attack excluded");return Ok(());}self.attribution.note_relationship(a.source,a.target,a.at);self.allocated=self.attribution.settled_inferred;self.unassigned=self.attribution.unattributed_inferred;let key=(a.at.context,a.target);if !self.slots.contains_key(&key){self.outcome("attack without baseline; compact source support retained");return Ok(());}self.push_fact(key,a.at,Kind::Attack(a));Ok(())}
    pub fn explicit(&mut self,source:EntityKey,target:EntityKey,amount:u64,at:Stamp)->Result<()>{self.explicit_fields(source,target,amount,Some(amount),[0;7],at)}
    pub fn explicit_fields(&mut self,source:EntityKey,target:EntityKey,raw:u64,impact:Option<u64>,flags:[u8;7],at:Stamp)->Result<()>{
        let now=self.now(at);let deadline=if self.slots.contains_key(&(at.context,target)){now.saturating_add(self.local_capture_ns).saturating_add(self.retention_ns)}else{0};self.attribution.submit_explicit_held(source,target,raw,flags,at,deadline);self.allocated=self.attribution.settled_inferred;self.unassigned=self.attribution.unattributed_inferred;if !self.attribution.eligibility(source,target){self.outcome("forbidden explicit source; raw observation retained outside attribution");return Ok(());}
        let key=(at.context,target);
        if impact==Some(0){self.outcome("zero HP impact; no positive budget claim");}
        if !self.slots.contains_key(&key){self.retain_orphan(target,at,Kind::Explicit{source,raw,impact,flags});self.outcome("explicit without baseline; claim retained");return Ok(());}
        if self.last_committed.get(&key).is_some_and(|old|Self::same(*old,at) && at.timestamp_ns.saturating_sub(old.timestamp_ns)>self.retention_ns){self.late_explicit+=1;self.outcome("explicit beyond retired history");}
        if impact.is_none(){self.outcome("unsupported explicit impact");}
        self.push_fact(key,at,Kind::Explicit{source,raw,impact,flags});Ok(())
    }
    pub fn update(&mut self,target:EntityKey,hp:u64,at:Stamp)->Result<Vec<Estimate>>{self.update_provenanced(target,hp,at,true)}
    /// Canonical numeric ingress. Finite comparison obligations cannot fail
    /// because association/baseline storage is full; output stays owned here.
    pub fn accept_numeric(&mut self,target:EntityKey,hp:u64,at:Stamp,prior:Option<(Stamp,u64)>,training_eligible:bool){
        self.observe_with_comparison(target,hp,at,prior,training_eligible);
    }
    /// Numeric comparison survives packet-window retirement. The independent
    /// ledger supplies the same-object predecessor; training stays disabled.
    pub fn update_with_comparison(&mut self,target:EntityKey,hp:u64,at:Stamp,prior:Option<(Stamp,u64)>,training_eligible:bool)->Result<Vec<Estimate>>{
        Ok(self.observe_with_comparison(target,hp,at,prior,training_eligible))
    }
    fn observe_with_comparison(&mut self,target:EntityKey,hp:u64,at:Stamp,prior:Option<(Stamp,u64)>,training_eligible:bool)->Vec<Estimate>{
        let key=(at.context,target);
        if prior.is_none(){
            // Canonical Initial is authoritative too: an association cache is
            // never a substitute predecessor after numeric eviction or when
            // two unresolved wire targets share an internal placeholder.
            self.clear_target(at.context,target);self.now(at);
            if hp!=i64::MAX as u64 && self.slots.len()<MAX_SLOTS{
                self.slots.insert(key,Slot{hp,baseline:at,baseline_training:training_eligible,pending:VecDeque::new(),facts:VecDeque::new(),truncated:false});self.peak_slots=self.peak_slots.max(self.slots.len());self.peak();
            }
            self.outcome("canonical initial baseline; no predecessor arithmetic");return self.drain_ready();
        }
        if let Some((baseline,before))=prior{
            let missing=self.slots.get(&key).is_none_or(|s|s.baseline!=baseline || s.hp!=before) || !Self::same(baseline,at);
            if missing && hp<before && before<=i64::MAX as u64 && hp<=i64::MAX as u64{
                self.clear_target(at.context,target);
                if self.slots.len()<MAX_SLOTS{
                    self.slots.insert(key,Slot{hp:before,baseline,baseline_training:false,pending:VecDeque::new(),facts:VecDeque::new(),truncated:false});
                    // Preserve the true predecessor stamp across a packet gap;
                    // explicitly build its uncertain budget rather than claim
                    // the two publications have continuous transport order.
                    self.next_id+=1;let available=self.now(at);let deadline=available.saturating_add(self.retention_ns);
                    let p=Pending{id:self.next_id,amount:before-hp,at,baseline,hp_before:before,hp_after:hp,end:None,available,deadline,bounds:Bounds::default(),baseline_bounds:Bounds::default(),clock_uncertain:true,sentinel_uncertain:false,conditional:true};
                    while self.pending_count>=MAX_PENDING{let Some((_,c,t,id))=self.deadlines.first().copied()else{break;};self.resolve_one((c,t),id,Some("global interval capacity"),&mut DamageModel::default());}
                    let s=self.slots.get_mut(&key).unwrap();s.pending=VecDeque::with_capacity(1);s.pending.push_back(p);self.pending_capacity+=s.pending.capacity();s.hp=hp;s.baseline=at;s.baseline_training=training_eligible;
                    self.deadlines.insert((deadline,key.0,key.1,self.next_id));self.pending_count+=1;self.peak_pending=self.peak_pending.max(self.pending_count);self.peak_slots=self.peak_slots.max(self.slots.len());self.peak();return self.drain_ready();
                }
                self.next_id+=1;let amount=before-hp;let claims=self.take_orphans(target,baseline,at);
                let mut claim_sum=0u128;let mut raw_sum=0u128;let mut support=Vec::new();let mut evidence=Vec::new();
                for f in &claims{if let Kind::Explicit{source,raw,impact,..}=&f.kind{claim_sum+=impact.unwrap_or(*raw) as u128;raw_sum+=*raw as u128;evidence.push(f.at.frame);if source.actor!=0{support.push(Candidate{source:*source,skill:None,weight:1.0,frames:vec![f.at.frame]});}}}
                let matched=claim_sum.min(amount as u128) as u64;let residual=amount-matched;
                let failed_before=self.attribution.unattributed_inferred;let no_active_before=self.attribution.no_eligible_active_unattributed;let capacity_before=self.attribution.delivery_capacity_unattributed;let allocations=self.attribution.resolve_budget(target,at,self.next_id,residual,&support,"forced baseline capacity numeric comparison");let unassigned=(self.attribution.unattributed_inferred-failed_before)as u64;self.no_support_loss+=self.attribution.no_eligible_active_unattributed-no_active_before;self.capacity_loss+=self.attribution.delivery_capacity_unattributed-capacity_before;
                self.loss+=amount as u128;self.explicit+=matched as u128;self.allocated=self.attribution.settled_inferred;self.unassigned=self.attribution.unattributed_inferred;
                self.append_audit(Accounting{id:self.next_id,target,at,loss:amount,explicit:matched,raw_explicit:raw_sum,residual,unassigned,candidates:allocations.iter().map(|e|e.source).collect(),allocations:allocations.clone(),evidence,reason:"forced baseline capacity numeric comparison",explicit_mismatch:claim_sum>amount as u128,baseline,hp_before:before,hp_after:hp});
                for e in allocations{if self.ready.len()<MAX_READY{self.ready_frame_capacity+=e.frames.capacity();self.ready.push_back(e);}}
                self.outcome("baseline capacity; numeric obligation retained");return self.drain_ready();
            }
            if missing && before<=i64::MAX as u64 && hp<=i64::MAX as u64{
                // An authoritative comparison may also describe healing or an
                // equal value. Align before observing it so a stale cache can
                // never turn that observation into invented damage.
                self.clear_target(at.context,target);
                if self.slots.len()<MAX_SLOTS{
                    self.slots.insert(key,Slot{hp,baseline:at,baseline_training:false,pending:VecDeque::new(),facts:VecDeque::new(),truncated:false});self.peak_slots=self.peak_slots.max(self.slots.len());
                }else{self.outcome("baseline capacity; nondecreasing numeric observation retained upstream");}
                return self.drain_ready();
            }
        }
        if !self.slots.contains_key(&key) && self.slots.len()>=MAX_SLOTS{self.outcome("baseline capacity; initial numeric observation retained upstream");return self.drain_ready();}
        self.observe_numeric(target,hp,at,training_eligible)
    }
    pub fn update_provenanced(&mut self,target:EntityKey,hp:u64,at:Stamp,training_eligible:bool)->Result<Vec<Estimate>>{
        if hp!=i64::MAX as u64 && !self.slots.contains_key(&(at.context,target)) && self.slots.len()>=MAX_SLOTS{self.outcome("HP baseline slot capacity");return Err("HP accounting capacity reached".into());}
        Ok(self.observe_numeric(target,hp,at,training_eligible))
    }
    fn observe_numeric(&mut self,target:EntityKey,hp:u64,at:Stamp,training_eligible:bool)->Vec<Estimate>{
        self.now(at);let key=(at.context,target);
        if hp==i64::MAX as u64{self.clear_target(at.context,target);self.outcome("MAX HP sentinel");return self.drain_ready();}
        if !self.slots.contains_key(&key){
            if self.slots.len()>=MAX_SLOTS{self.outcome("HP baseline slot capacity");return self.drain_ready();}
            self.slots.insert(key,Slot{hp,baseline:at,baseline_training:training_eligible,pending:VecDeque::new(),facts:VecDeque::new(),truncated:false});self.peak_slots=self.peak_slots.max(self.slots.len());self.outcome("initial baseline");self.peak();return Vec::new();
        }
        if !Self::same(self.slots[&key].baseline,at){self.clear_target(at.context,target);let result=self.observe_numeric(target,hp,at,training_eligible);let mut ready=self.drain_ready();ready.extend(result);return ready;}
        if hp<self.slots[&key].hp{
            while self.slots[&key].pending.len()>=MAX_TARGET_PENDING{let id=self.slots[&key].pending.front().unwrap().id;self.resolve_one(key,id,Some("per-target interval capacity"),&mut DamageModel::default());}
            while self.pending_count>=MAX_PENDING{let Some((_,c,t,id))=self.deadlines.first().copied()else{break;};self.resolve_one((c,t),id,Some("global interval capacity"),&mut DamageModel::default());}
        }
        let s=self.slots.get_mut(&key).unwrap();let old_pending_capacity=s.pending.capacity();for p in &mut s.pending{p.end.get_or_insert(at);}
        if hp<s.hp{
            self.next_id+=1;let deadline=self.available.saturating_add(self.retention_ns);
            if s.pending.capacity()==0{s.pending=VecDeque::with_capacity(1);}
            s.pending.push_back(Pending{id:self.next_id,amount:s.hp-hp,at,baseline:s.baseline,hp_before:s.hp,hp_after:hp,end:None,available:self.available,deadline,bounds:Bounds::default(),baseline_bounds:Bounds::default(),clock_uncertain:false,sentinel_uncertain:false,conditional:!training_eligible || !s.baseline_training});self.deadlines.insert((deadline,key.0,key.1,self.next_id));self.pending_count+=1;self.peak_pending=self.peak_pending.max(self.pending_count);
        }else if hp>s.hp{self.outcome("HP rise");}
        let s=self.slots.get_mut(&key).unwrap();self.pending_capacity=self.pending_capacity-old_pending_capacity+s.pending.capacity();s.hp=hp;s.baseline=at;s.baseline_training=training_eligible;self.dirty.insert(key);self.peak();self.drain_ready()
    }
    /// Bounded target-local alignment. FIFO is an uncertain batch alternative,
    /// not an assertion that publication order equals physical impact order.
    fn associations(&mut self,s:&Slot)->Vec<Assignment>{
        let mut work:Vec<_>=(0..s.pending.len()).map(|_|Assignment::default()).collect();let local=self.local_capture_ns;let mut comparisons=0u64;
        // Compute whether a local HP interval already has publication support.
        // This O(P*F) pass prevents a nested scan per attack/budget combination.
        let occupied:Vec<_>=s.pending.iter().map(|p|s.facts.iter().any(|f|{comparisons+=1;f.at.order>p.at.order && p.end.is_some_and(|e|f.at.order<e.order) && match &f.kind{Kind::Attack(_)=>true,Kind::Explicit{impact:Some(n),..}=>*n>=p.amount,_=>false}})).collect();
        for(fi,f)in s.facts.iter().enumerate(){if !matches!(f.kind,Kind::Attack(_)){continue;}let mut default=None;let mut orphan=None;
            for(pi,p)in s.pending.iter().enumerate(){comparisons+=1;if !Self::relevant(p,f,local){continue;}
                if f.at.order>=p.at.order && p.end.is_none_or(|e|f.at.order<e.order){default=Some(pi);}
                if f.at.order<p.at.order && f.at.order>p.baseline.order{default.get_or_insert(pi);}
                if orphan.is_none() && p.at.order<=f.at.order && work[pi].attacks.is_empty() && !occupied[pi] && p.end.is_some_and(|e|e.order<=f.at.order && e.timestamp_ns.saturating_sub(p.at.timestamp_ns)<=local){orphan=Some(pi);}
            }
            let choice=orphan.filter(|i|default.is_some_and(|d|*i<d)).or(default);
            if let Some(pi)=choice{work[pi].attacks.push(fi);if let Kind::Attack(a)=&f.kind{let flags=a.meta[..7].try_into().unwrap();work[pi].component_flags.insert(flags);work[pi].component_sources.insert((a.source,flags));}if Some(pi)!=default{work[pi].batched=true;work[pi].ambiguous=true;}}
        }
        for(fi,f)in s.facts.iter().enumerate(){let Kind::Explicit{source,impact,flags,..}=&f.kind else{continue;};if *impact==Some(0){continue;}let mut default=None;let mut bridges=Vec::new();
            for(pi,p)in s.pending.iter().enumerate(){comparisons+=1;if !Self::relevant(p,f,self.retention_ns){continue;}
                if Self::relevant(p,f,local){if f.at.order>=p.at.order && p.end.is_none_or(|e|f.at.order<e.order){default=Some(pi);}if f.at.order<p.at.order && f.at.order>p.baseline.order{default.get_or_insert(pi);}}
                let remaining=(p.amount as u128).saturating_sub(work[pi].impact_sum);
                if impact.is_some_and(|n|n>0 && n as u128<=remaining) && (work[pi].component_sources.contains(&(*source,*flags)) || (impact.is_some_and(|n|n as u128==remaining) && work[pi].component_flags.contains(flags))){bridges.push(pi);}
            }
            let chosen=if bridges.len()==1{Some(bridges[0])}else{default};if bridges.len()>1{for i in &bridges{work[*i].ambiguous=true;}}
            if let Some(pi)=chosen{work[pi].explicit.push(fi);work[pi].impact_sum+=impact.unwrap_or(0) as u128;work[pi].unsupported|=impact.is_none();if bridges.len()==1 && Some(pi)!=default{work[pi].ambiguous=true;}}
        }
        // Charge the remaining bounded explicit/presentation matching and
        // distinct-template loops too. This is a conservative work count.
        comparisons+=3*(s.facts.len()*s.facts.len()) as u64;
        self.max_resolver_comparisons=self.max_resolver_comparisons.max(comparisons);work
    }
    fn resolve_one(&mut self,key:SlotKey,id:u64,forced:Option<&'static str>,model:&mut DamageModel){
        let Some(mut s)=self.slots.remove(&key)else{return;};let Some(pi)=s.pending.iter().position(|p|p.id==id)else{self.slots.insert(key,s);return;};let old_fact_capacity=s.facts.capacity();let old_pending_capacity=s.pending.capacity();
        let forbidden:Vec<_>=s.facts.iter().filter(|f|match &f.kind{Kind::Attack(a)=>!self.attribution.eligibility(a.source,key.1),Kind::Explicit{source,..}=>source.actor!=0 && !self.attribution.eligibility(*source,key.1)}).map(|f|f.id).collect();
        for f in &s.facts{if forbidden.contains(&f.id) && self.fact_order.remove(&(f.id,key.0,key.1)){self.fact_count-=1;}}
        if !forbidden.is_empty(){s.truncated=true;s.facts.retain(|f|!forbidden.contains(&f.id));}
        let mut work=self.associations(&s);let mut assignment=std::mem::take(&mut work[pi]);let mut p=s.pending.remove(pi).unwrap();self.deadlines.remove(&(p.deadline,key.0,key.1,p.id));self.pending_count-=1;
        let orphan_claims=self.take_orphans(key.1,p.baseline,p.at);
        if !orphan_claims.is_empty(){p.conditional=true;for f in orphan_claims{assignment.unsupported|=matches!(f.kind,Kind::Explicit{impact:None,..});assignment.explicit.push(s.facts.len());s.facts.push_back(f);}}
        let age=self.available.saturating_sub(p.available);let bin=63-age.max(1).leading_zeros() as usize;self.decision_age_bins[bin]+=1;self.decision_age_max=self.decision_age_max.max(age);self.decision_count+=1;
        if assignment.attacks.iter().chain(assignment.explicit.iter()).any(|i|!p.bounds.compatible(s.facts[*i].bounds)){p.clock_uncertain=true;}if p.clock_uncertain{self.outcome("candidate clock disagreement; publication credit retained");}
        let mut consumed=BTreeSet::new();let mut paired=BTreeSet::new();let mut ambiguous=assignment.ambiguous;let mut sum=0u128;let mut raw_sum=0u128;
        // A zero-impact record may identify its own presentation, but cannot
        // consume any positive HP budget. Match without an amount comparison.
        for zero in s.facts.iter().filter(|f|matches!(f.kind,Kind::Explicit{impact:Some(0),..})){
            let Kind::Explicit{source,flags,..}=&zero.kind else{unreachable!()};
            // A zero claim has no numeric bridge to an older positive budget.
            // Keep it in its own publication cell so a later unchanged HP/zero
            // presentation cannot cancel a previous genuine hit of this actor.
            let mut lower=0;let mut upper=u64::MAX;
            for boundary in s.pending.iter().flat_map(|q|[Some(q.baseline),Some(q.at),q.end]).flatten().chain(std::iter::once(s.baseline)){
                if boundary.order<=zero.at.order{lower=lower.max(boundary.order);}else{upper=upper.min(boundary.order);}
            }
            let matches:Vec<_>=assignment.attacks.iter().copied().filter(|j|!paired.contains(j) && s.facts[*j].at.order>=lower && s.facts[*j].at.order<upper && Self::same(zero.at,s.facts[*j].at) && zero.at.timestamp_ns.abs_diff(s.facts[*j].at.timestamp_ns)<=self.local_capture_ns && match &s.facts[*j].kind{Kind::Attack(a)=>a.source==*source && a.meta[..7]==flags[..],_=>false}).collect();
            if matches.len()==1{let j=matches[0];paired.insert(j);consumed.insert(s.facts[j].id);consumed.insert(zero.id);}
            else if matches.len()>1{ambiguous=true;}
        }
        for i in &assignment.explicit{let f=&s.facts[*i];consumed.insert(f.id);let Kind::Explicit{source,raw,impact,flags}=&f.kind else{continue;};raw_sum+=*raw as u128;sum+=impact.unwrap_or(*raw) as u128;
            let mut matching:Vec<_>=assignment.attacks.iter().copied().filter(|j|!paired.contains(j) && match &s.facts[*j].kind{Kind::Attack(a)=>a.meta[..7]==flags[..],_=>false}).collect();
            if matching.iter().any(|j|matches!(&s.facts[*j].kind,Kind::Attack(a) if a.source==*source)){matching.retain(|j|matches!(&s.facts[*j].kind,Kind::Attack(a) if a.source==*source));}
            let pair=if matching.len()==1{Some(matching[0])}else{if matching.len()>1{ambiguous=true;}matching.iter().copied().filter(|j|s.facts[*j].at.order>=f.at.order).min_by_key(|j|s.facts[*j].at.order).or_else(||matching.first().copied())};
            if let Some(j)=pair{paired.insert(j);consumed.insert(s.facts[j].id);if let Kind::Attack(a)=&s.facts[j].kind{if a.source!=*source{ambiguous=true;self.outcome("explicit origin differs from presentation source");}}
                if f.at.order>=p.end.map_or(u64::MAX,|e|e.order){if self.refinements.len()==MAX_AUDIT{self.refinements.pop_front();self.refinement_evictions+=1;}self.refinements.push_back(Refinement{interval:p.id,source:*source,amount:impact.unwrap_or(0),at:f.at,attack_frames:vec![s.facts[j].at.frame]});self.outcome("cross-HP explicit reconciliation");}
            }
        }
        let mismatch=sum>p.amount as u128;let matched=sum.min(p.amount as u128) as u64;let residual=p.amount-matched;let unresolved=assignment.unsupported;
        let mut attacks=Vec::new();for i in &assignment.attacks{let f=&s.facts[*i];consumed.insert(f.id);if !paired.contains(i) && !f.inference_excluded{if let Kind::Attack(a)=&f.kind{attacks.push(a);}}}
        // A terminal endpoint is a raw-impact ceiling only for a structurally
        // isolated, concordant explicit/presentation pair. Never clip on mere
        // proximity, amount equality, a stale baseline or a role disagreement.
        // Candidate clock brackets remain annotations; stream-local evidence
        // does not become invalid solely because partial clocks disagree.
        let terminal_clip=if p.hp_after==0 && !unresolved && !p.sentinel_uncertain && !p.conditional && !ambiguous && !s.truncated && !assignment.batched && assignment.explicit.len()==1 && assignment.attacks.len()==1{
            let f=&s.facts[assignment.explicit[0]];
            match (&f.kind,&s.facts[assignment.attacks[0]].kind){
                (Kind::Explicit{source,raw,impact:Some(n),flags},Kind::Attack(a)) if *source==a.source && a.meta[..7]==flags[..] && *n>p.amount && *n<=*raw=>Some((f.id,raw-(n-p.amount))),
                _=>None,
            }
        }else{None};
        let mut clipped=false;
        for f in &s.facts{if consumed.contains(&f.id){if let Kind::Explicit{raw,..}=&f.kind{
            let credited=terminal_clip.filter(|(id,_)|*id==f.id).map_or(*raw,|(_,amount)|amount);
            if self.attribution.finalize_explicit(key.1,f.at,*raw,credited) && credited<*raw{clipped=true;self.outcome("terminal explicit overkill clipped");}
        }}}
        // Structural witness first, arithmetic check second. Inferred amounts,
        // role disagreements and multiple components are not training truth.
        if !unresolved && !p.clock_uncertain && !p.sentinel_uncertain && !p.conditional && !mismatch && !ambiguous && !s.truncated && assignment.explicit.len()==1 && assignment.attacks.len()==1{
            if let Kind::Explicit{source,impact:Some(n),flags,..}=&s.facts[assignment.explicit[0]].kind{if let Kind::Attack(a)=&s.facts[assignment.attacks[0]].kind{if *n>0 && *n==p.amount && *source==a.source && a.meta[..7]==flags[..] && a.key1!=0{model.observe_clean(*source,key.1,a.key1,a.key2,*flags,*n);self.outcome("clean impact training anchor");}}}
        }
        let mut groups:BTreeMap<(EntityKey,Option<u32>),(f64,bool,Vec<u64>)>=BTreeMap::new();let mut actor_trained:BTreeMap<EntityKey,bool>=BTreeMap::new();let mut templates:BTreeMap<(EntityKey,u32,u32,[u8;7]),f64>=BTreeMap::new();
        for a in attacks{let flags:[u8;7]=a.meta[..7].try_into().unwrap();let(w,trained)=model.weight(a.source,key.1,a.key1,a.key2,flags);let w=w.max(f64::MIN_POSITIVE);let g=groups.entry((a.source,(a.key1!=0).then_some(a.key1))).or_insert((0.0,true,Vec::new()));g.0+=w;g.1&=trained;g.2.push(a.at.frame);*actor_trained.entry(a.source).or_insert(true)&=trained;templates.insert((a.source,a.key1,a.key2,flags),w);}
        // Without trained component magnitudes, a notification flood cannot
        // manufacture actor power. Retain every hit fact but cap that actor's
        // uncertain potential at the mean over distinct component templates.
        // Repeated packets remain evidence, but cannot inflate a sparse prior.
        let sparse_mode=actor_trained.values().any(|trained|!*trained);
        for((source,skill),(weight,_,_))in &mut groups{if sparse_mode{let count=templates.keys().filter(|(s,_,_,_)|s==source).count();let sum:f64=templates.iter().filter(|((s,k,_,_),_)|s==source && (if *k==0{None}else{Some(*k)})==*skill).map(|(_,w)|*w).sum();*weight=sum/count.max(1) as f64;}}
        // Paired peer presentations can still support a positive residual.
        // Direct explicit sources remain excluded from HP allocation.
        if groups.is_empty() && residual>0{
            for i in &assignment.attacks{if s.facts[*i].inference_excluded{continue;}if let Kind::Attack(a)=&s.facts[*i].kind{let(w,trained)=model.weight(a.source,key.1,a.key1,a.key2,a.meta[..7].try_into().unwrap());groups.entry((a.source,(a.key1!=0).then_some(a.key1))).or_insert((w,trained,vec![a.at.frame]));}}
        }
        let mut candidates:Vec<_>=groups.keys().map(|(s,_)|*s).filter(|s|s.actor!=0).collect::<BTreeSet<_>>().into_iter().collect();let trained=groups.values().all(|(_,t,_)|*t);
        let method=if assignment.unsupported{"forced uncertain raw overlap and HP residual"}else if clipped{"explicit accounted; terminal overkill clipped"}else if residual==0{"explicit accounted"}else if groups.is_empty(){forced.unwrap_or("no eligible active attacker at retention limit")}else if p.sentinel_uncertain{"forced unpublished-sentinel actor inference"}else if p.clock_uncertain{"forced clock-uncertain actor inference"}else if p.conditional{"forced conditional-key actor inference"}else if s.truncated{"forced inference with truncated evidence"}else if assignment.batched{"forced batch sequence inference"}else if ambiguous{"forced ambiguous packet association"}else if candidates.len()==1{"single packet-supported actor"}else if trained{"constrained actor/component model"}else{"forced sparse actor/component prior"};
        let support:Vec<_>=groups.into_iter().map(|((source,skill),(weight,_,frames))|Candidate{source,skill,weight,frames}).collect();
        let excluded_before=self.attribution.explicit_source_unattributed;let failed_before=self.attribution.unattributed_inferred;let no_active_before=self.attribution.no_eligible_active_unattributed;let capacity_before=self.attribution.delivery_capacity_unattributed;let allocations=self.attribution.resolve_budget(key.1,p.at,p.id,residual,&support,method);let unassigned=(self.attribution.unattributed_inferred-failed_before)as u64;self.no_support_loss+=self.attribution.no_eligible_active_unattributed-no_active_before;self.capacity_loss+=self.attribution.delivery_capacity_unattributed-capacity_before;
        candidates.extend(allocations.iter().map(|e|e.source));candidates.sort();candidates.dedup();
        let method=if unassigned>0 && allocations.is_empty(){if self.attribution.delivery_capacity_unattributed>capacity_before{"unattributed: bounded publication capacity"}else if self.attribution.explicit_source_unattributed>excluded_before{"unattributed: explicit source excluded from HP inference"}else{"unattributed: no eligible active attacker"}}else{allocations.first().map_or(method,|e|e.method)};
        let allocated=allocations.iter().map(|a|a.amount).sum::<u64>();
        // Empty eligible active support is a counted attribution failure.
        // Later roster presence cannot retroactively supply an attacker.
        self.loss+=p.amount as u128;self.explicit+=matched as u128;self.allocated=self.attribution.settled_inferred;self.unassigned=self.attribution.unattributed_inferred;self.outcome(method);
        let evidence=assignment.attacks.iter().chain(assignment.explicit.iter()).map(|i|s.facts[*i].at.frame).collect();let a=Accounting{id:p.id,target:key.1,at:p.at,loss:p.amount,explicit:matched,raw_explicit:raw_sum,residual,unassigned,candidates,allocations:allocations.clone(),evidence,reason:method,explicit_mismatch:mismatch,baseline:p.baseline,hp_before:p.hp_before,hp_after:p.hp_after};self.append_audit(a);
        for e in allocations{if self.ready.len()<MAX_READY && self.ready_frame_capacity+e.frames.capacity()<=MAX_READY_FRAME_CAPACITY{self.ready_frame_capacity+=e.frames.capacity();self.ready.push_back(e);}}
        let _=allocated;
        for f in &s.facts{if consumed.contains(&f.id){if self.fact_order.remove(&(f.id,key.0,key.1)){self.fact_count-=1;}}}s.facts.retain(|f|!consumed.contains(&f.id));if s.facts.capacity()>s.facts.len()*2{s.facts.shrink_to_fit();}if s.pending.capacity()>s.pending.len()*2{s.pending.shrink_to_fit();}
        self.fact_capacity=self.fact_capacity-old_fact_capacity+s.facts.capacity();self.pending_capacity=self.pending_capacity-old_pending_capacity+s.pending.capacity();if self.last_committed.len()>=MAX_SLOTS && !self.last_committed.contains_key(&key){self.last_committed.pop_first();}self.last_committed.insert(key,p.at);self.slots.insert(key,s);self.peak();
    }
    fn audit_cost(a:&Accounting)->usize{std::mem::size_of::<Accounting>()+a.evidence.capacity()*8+a.candidates.capacity()*std::mem::size_of::<EntityKey>()+a.allocations.capacity()*std::mem::size_of::<Estimate>()+a.allocations.iter().map(|e|e.frames.capacity()*8).sum::<usize>()}
    fn append_audit(&mut self,a:Accounting){let cost=Self::audit_cost(&a);while !self.audit.is_empty() && (self.audit.len()>=MAX_AUDIT || self.audit_bytes+cost>MAX_AUDIT_BYTES){let old=self.audit.pop_front().unwrap();self.audit_bytes-=Self::audit_cost(&old);self.audit_evictions+=1;}self.audit_bytes+=cost;self.audit.push_back(a);}
    pub fn drain_ready(&mut self)->Vec<Estimate>{self.ready_frame_capacity=0;self.ready.drain(..).collect()}
    pub fn expire_with_model(&mut self,at:Stamp,model:&mut DamageModel)->Vec<Estimate>{self.tick_with_model(self.available.max(at.timestamp_ns),model)}
    pub fn expire(&mut self,at:Stamp)->Vec<Estimate>{self.expire_with_model(at,&mut DamageModel::default())}
    pub fn tick_with_model(&mut self,now:u64,model:&mut DamageModel)->Vec<Estimate>{
        self.set_available(now);while let Some((deadline,c,t,id))=self.deadlines.first().copied(){if deadline>=self.available{break;}self.resolve_one((c,t),id,None,model);}
        // Insertion order also orders monotonic availability; no inactive-slot scan.
        while let Some((id,c,t))=self.fact_order.first().copied(){let key=(c,t);let expired=self.slots.get(&key).and_then(|s|s.facts.iter().find(|f|f.id==id).map(|f|(s,f))).is_none_or(|(s,f)|self.available.saturating_sub(f.available)>self.retention_ns && !s.pending.iter().any(|p|Self::relevant(p,f,self.local_capture_ns)));if !expired{break;}self.drop_fact(key,id,false);self.outcome("unclaimed packet fact age expiry");}self.drain_ready()
    }
    pub fn tick(&mut self,now:u64)->Vec<Estimate>{self.tick_with_model(now,&mut DamageModel::default())}
    pub fn clear_target_with_model(&mut self,context:u64,target:EntityKey,model:&mut DamageModel){let key=(context,target);while let Some(id)=self.slots.get(&key).and_then(|s|s.pending.front().map(|p|p.id)){self.resolve_one(key,id,Some("boundary retirement without actor support"),model);}if let Some(s)=self.slots.remove(&key){self.fact_capacity-=s.facts.capacity();self.pending_capacity-=s.pending.capacity();for f in s.facts{self.fact_order.remove(&(f.id,key.0,key.1));self.fact_count-=1;self.retain_orphan(target,f.at,f.kind);}}self.dirty.remove(&key);self.last_committed.remove(&key);}
    pub fn clear_target(&mut self,context:u64,target:EntityKey){self.clear_target_with_model(context,target,&mut DamageModel::default());}
    pub fn clear_context_with_model(&mut self,context:u64,model:&mut DamageModel){let keys:Vec<_>=self.slots.range((context,EntityKey{process:0,binding:0,actor:0,generation:0})..).take_while(|((c,_),_)|*c==context).map(|(k,_)|*k).collect();for(_,t)in keys{self.clear_target_with_model(context,t,model);}}
    pub fn clear_context(&mut self,context:u64){self.clear_context_with_model(context,&mut DamageModel::default());}
    pub fn clear_with_model(&mut self,model:&mut DamageModel){while let Some(key)=self.slots.keys().next().copied(){self.clear_target_with_model(key.0,key.1,model);}}
    pub fn clear(&mut self){self.clear_with_model(&mut DamageModel::default());}
    pub(crate) fn retire_target(&mut self,target:EntityKey){let keys:Vec<_>=self.slots.keys().filter(|(_,t)|*t==target).copied().collect();for(c,t)in keys{self.clear_target(c,t);}}
    pub fn flush_with_model(&mut self,model:&mut DamageModel)->Vec<Estimate>{self.clear_with_model(model);self.attribution.release_explicit_holds();self.drain_ready()}
    pub fn flush(&mut self)->Vec<Estimate>{self.flush_with_model(&mut DamageModel::default())}
    pub fn resource_summary(&self)->Value{
        // Includes conservative container node allowances; not an RSS claim.
        let resident=self.owned_bytes();let returned_output_ceiling=MAX_READY*std::mem::size_of::<Estimate>()+MAX_READY_FRAME_CAPACITY*8;
        let owned_ceiling=std::mem::size_of::<Self>()+Attribution::ceiling_bytes()+MAX_ORPHAN_CLAIMS*std::mem::size_of::<OrphanClaim>()+MAX_SLOTS*(std::mem::size_of::<Slot>()+256)+2*MAX_FACTS*std::mem::size_of::<Fact>()+2*MAX_PENDING*std::mem::size_of::<Pending>()+MAX_FACTS*96+MAX_PENDING*112+MAX_AUDIT_BYTES+MAX_AUDIT*std::mem::size_of::<Accounting>()+MAX_READY*std::mem::size_of::<Estimate>()+MAX_READY_FRAME_CAPACITY*8+MAX_AUDIT*(std::mem::size_of::<Refinement>()+8)+MAX_SLOTS*(128+96)+64*128;
        let mut summary=json!({"format":"bounded-causal-health-v1","retention_ns":self.retention_ns,"local_capture_ns":self.local_capture_ns,"slots":self.slots.len(),"pending":self.pending_count,"facts":self.fact_count,"fact_capacity":self.fact_capacity,"pending_capacity":self.pending_capacity,"ready":self.ready.len(),"audit_bytes":self.audit_bytes,"conservative_resident_bytes":resident,"peak_conservative_resident_bytes":self.peak_owned_bytes,"conservative_owned_ceiling_bytes":owned_ceiling,"scratch_ceiling_bytes":512*1024,"returned_output_ceiling_bytes":returned_output_ceiling,"peak_slots":self.peak_slots,"peak_pending":self.peak_pending,"peak_facts":self.peak_facts,"max_resolver_comparisons":self.max_resolver_comparisons,"work_counter":"conservative charged target-local relational work, includes 3F^2 allowance","fact_evictions":self.fact_evictions,"no_support_loss":self.no_support_loss.to_string(),"capacity_loss":self.capacity_loss.to_string(),"boundary_no_support_loss":self.boundary_loss.to_string(),"unsupported_impact_exclusion":self.unsupported_loss.to_string(),"clock_epoch_exclusion":self.clock_loss.to_string(),"late_explicit_diagnostic":self.late_explicit,"late_explicit_diagnostic_scope":"capture distance from last retired interval; not proof of physical linkage or a measured lost explicit claim","decision_delay":{"clock":"monotonic availability since first observed HP budget, including forced boundary retirements; not network latency","count":self.decision_count,"maximum_ns":self.decision_age_max,"log2_ns_histogram":self.decision_age_bins.to_vec()},"limits":{"slots":MAX_SLOTS,"pending":MAX_PENDING,"per_target_pending":MAX_TARGET_PENDING,"facts":MAX_FACTS,"per_target_facts":MAX_TARGET_FACTS,"audit":MAX_AUDIT,"audit_bytes":MAX_AUDIT_BYTES,"ready":MAX_READY},"clock":"availability retirement, capture locality; not server time"});
        summary["attribution"]=json!({"credit_outbox":self.attribution.queued_len(),"owed_pending":self.attribution.owed_amount().to_string(),"unattributed":self.attribution.unattributed_amount().to_string(),"unattributed_inferred":self.attribution.unattributed_inferred.to_string(),"queued_credit_amount":self.attribution.queued_amount().to_string(),"coarsened_obligations":self.attribution.coarsened_obligations,"coarsened_publications":self.attribution.coarsened_publications,"raw_detail_coarsenings":self.attribution.raw_detail_coarsenings,"orphan_claims":self.orphan_claims.len(),"orphan_claim_evictions":self.orphan_claim_evictions});summary
    }
}
