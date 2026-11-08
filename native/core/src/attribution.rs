//! The single owner of detected damage obligations and acknowledged publication.
//! Empty eligible attack support is a counted attribution failure.
use crate::{combat::Stamp,health::Estimate,identity::{EntityKey,damage_side_from_kind,damage_eligible_sides},meter::Hit,protocol::Damage};
use std::collections::VecDeque;

pub const MAX_ACTORS:usize=512;
pub const MAX_RELATIONSHIPS:usize=1024;
pub const MAX_CREDITS:usize=1024;
pub const MAX_OWED:usize=1024;
pub const MAX_CREDIT_FRAMES:usize=16;

#[derive(Clone,Debug)]
pub enum CreditKind { Explicit{hit:Hit,at:Stamp}, Inferred(Estimate) }
#[derive(Clone,Debug)]
pub struct Credit { pub id:u64,pub prepared_skill:Option<Option<u32>>,pub detail_coarsened:bool,pub kind:CreditKind,pub method:&'static str,pub ready_after:u64 }
impl Credit {
    pub fn source(&self)->EntityKey{match &self.kind{CreditKind::Explicit{hit,..}=>hit.source,CreditKind::Inferred(e)=>e.source}}
    pub fn amount(&self)->u64{match &self.kind{CreditKind::Explicit{hit,..}=>hit.damage.amount,CreditKind::Inferred(e)=>e.amount}}
    fn set_amount(&mut self,n:u64){match &mut self.kind{CreditKind::Explicit{hit,..}=>hit.damage.amount=n,CreditKind::Inferred(e)=>e.amount=n}}
    pub fn at(&self)->Stamp{match &self.kind{CreditKind::Explicit{at,..}=>*at,CreditKind::Inferred(e)=>e.at}}
    pub fn target(&self)->EntityKey{match &self.kind{CreditKind::Explicit{hit,..}=>hit.target,CreditKind::Inferred(e)=>e.target}}
}
#[derive(Clone,Debug)]
pub struct Candidate {pub source:EntityKey,pub skill:Option<u32>,pub weight:f64,pub frames:Vec<u64>}
#[derive(Clone)]
struct Actor {source:EntityKey,at:Stamp,kind:u8}
struct Relationship {source:EntityKey,target:EntityKey,first:Stamp,at:Stamp,idle_policy:u64}
struct Owed {credit:Credit,amount:u128}
struct Row {credit:Credit,amount:u128}
#[derive(Clone,Copy)]
enum Failure{NoActive,SideConflict,DeliveryCapacity}
pub struct Attribution {
    pub active_idle_ns:u64,
    actors:VecDeque<Actor>,relationships:VecDeque<Relationship>,outbox:VecDeque<Row>,owed:VecDeque<Owed>,
    next_receipt:u64,leased:Option<Credit>,support_revision:u64,owed_revision:u64,
    pub detected_explicit:u128,pub detected_inferred:u128,pub settled_inferred:u128,pub published:u128,pub published_explicit:u128,pub published_inferred:u128,
    pub unattributed_explicit:u128,pub unattributed_inferred:u128,pub unattributed_outcomes:u64,pub explicit_side_conflicts:u64,
    pub no_eligible_active_unattributed:u128,pub side_conflict_unattributed:u128,pub delivery_capacity_unattributed:u128,
    pub coarsened_obligations:u64,pub coarsened_publications:u64,pub rejected_publications:u64,pub raw_detail_coarsenings:u64,
    pub clipped_explicit_overkill:u128,pub clipped_explicit_records:u64,pub overkill_clip_unavailable:u128,pub max_explicit_finalize_work:usize,
}
impl Default for Attribution{fn default()->Self{Self{active_idle_ns:crate::meter::DEFAULT_IDLE_NS,actors:VecDeque::new(),relationships:VecDeque::new(),outbox:VecDeque::new(),owed:VecDeque::new(),next_receipt:0,leased:None,support_revision:0,owed_revision:0,detected_explicit:0,detected_inferred:0,settled_inferred:0,published:0,published_explicit:0,published_inferred:0,unattributed_explicit:0,unattributed_inferred:0,unattributed_outcomes:0,explicit_side_conflicts:0,no_eligible_active_unattributed:0,side_conflict_unattributed:0,delivery_capacity_unattributed:0,coarsened_obligations:0,coarsened_publications:0,rejected_publications:0,raw_detail_coarsenings:0,clipped_explicit_overkill:0,clipped_explicit_records:0,overkill_clip_unavailable:0,max_explicit_finalize_work:0}}}
impl Attribution {
    pub fn note_actor(&mut self,source:EntityKey,at:Stamp){self.register_actor(source,at,None);}
    fn register_actor(&mut self,source:EntityKey,at:Stamp,role:Option<u8>){
        if source.actor==0{return;}
        let old=self.actors.iter().position(|a|a.source==source).map(|i|self.actors.remove(i).unwrap());
        let kind=role.or_else(||old.as_ref().map(|a|a.kind)).unwrap_or(0);
        let changed=old.as_ref().is_none_or(|a|a.kind!=kind);
        if self.actors.len()==MAX_ACTORS{self.actors.pop_front();}
        self.actors.push_back(Actor{source,at,kind});if changed{self.support_revision+=1;}
    }
    pub fn note_actor_kind(&mut self,source:EntityKey,kind:&str,at:Stamp){
        let role=if damage_side_from_kind(kind)==crate::identity::DamageSide::Player{2}else if kind.eq_ignore_ascii_case("npc") || kind.eq_ignore_ascii_case("enemy"){1}else{0};
        self.register_actor(source,at,Some(role));
    }
    pub fn note_relationship(&mut self,source:EntityKey,target:EntityKey,at:Stamp){
        let old=self.relationships.iter().position(|r|r.source==source && r.target==target).map(|i|self.relationships.remove(i).unwrap());
        let mut first=at;let mut latest=at;
        if let Some(old)=&old{if old.idle_policy==self.active_idle_ns{
            if at.timestamp_ns>=old.at.timestamp_ns && at.timestamp_ns-old.at.timestamp_ns<self.active_idle_ns{first=old.first;}
            else if at.timestamp_ns<old.at.timestamp_ns{
                latest=old.at;first=old.first;
                if at.timestamp_ns<old.first.timestamp_ns && old.first.timestamp_ns-at.timestamp_ns<self.active_idle_ns{first=at;}
            }
        }}
        if self.relationships.len()==MAX_RELATIONSHIPS{self.relationships.pop_front();}
        self.relationships.push_back(Relationship{source,target,first,at:latest,idle_policy:self.active_idle_ns});if old.is_none(){self.support_revision+=1;}
        self.register_actor(source,at,None);
    }
    fn raw_credit(source:EntityKey,target:EntityKey,raw:u64,flags:[u8;7],at:Stamp,method:&'static str)->Credit{
        let hit=Hit{timestamp_ns:at.timestamp_ns,source,target,damage:Damage{actor:source.actor,target:target.actor,amount:raw,flags:[flags[0]&1!=0,flags[1]&2!=0,flags[1]&4!=0,flags[3]&8!=0,flags[4]&16!=0],raw_flags:flags}};
        Credit{id:0,prepared_skill:None,detail_coarsened:false,kind:CreditKind::Explicit{hit,at},method,ready_after:0}
    }
    fn same_scope(source:EntityKey,target:EntityKey)->bool{source.process==target.process && source.binding==target.binding}
    pub fn eligibility(&self,source:EntityKey,target:EntityKey)->bool{
        let side=|key|self.actors.iter().find(|a|a.source==key).map_or(crate::identity::DamageSide::Unknown,|a|if a.kind==2{crate::identity::DamageSide::Player}else{crate::identity::DamageSide::Unknown});
        source.actor!=0 && damage_eligible_sides(source,target,side(source),side(target))
    }
    pub fn active_candidates(&self,target:EntityKey,at:Stamp)->Vec<Candidate>{
        let mut support:Vec<_>=self.relationships.iter().filter(|r|{
            let first=if r.idle_policy==self.active_idle_ns{r.first}else{r.at};
            self.active_idle_ns>0 && r.target==target && Self::same_scope(r.source,target) && first.timestamp_ns<=at.timestamp_ns && (at.timestamp_ns<=r.at.timestamp_ns || at.timestamp_ns-r.at.timestamp_ns<self.active_idle_ns) && self.eligibility(r.source,target)
        }).map(|r|{let proof=if r.idle_policy==self.active_idle_ns{r.first}else{r.at};Candidate{source:r.source,skill:None,weight:1.0,frames:vec![proof.frame]}}).collect();
        support.sort_by_key(|c|c.source);support.dedup_by_key(|c|c.source);support
    }
    fn record_unattributed(&mut self,explicit:bool,amount:u128,was_settled:bool,reason:Failure){
        if amount==0{return;}self.unattributed_outcomes+=1;match reason{Failure::NoActive=>self.no_eligible_active_unattributed+=amount,Failure::SideConflict=>self.side_conflict_unattributed+=amount,Failure::DeliveryCapacity=>self.delivery_capacity_unattributed+=amount};
        if explicit{self.unattributed_explicit+=amount;}else{self.unattributed_inferred+=amount;if was_settled{self.settled_inferred-=amount;}}
    }
    pub fn submit_explicit(&mut self,source:EntityKey,target:EntityKey,raw:u64,flags:[u8;7],at:Stamp){
        self.submit_explicit_held(source,target,raw,flags,at,0);
    }
    pub fn submit_explicit_held(&mut self,source:EntityKey,target:EntityKey,raw:u64,flags:[u8;7],at:Stamp,ready_after:u64){
        if source.actor==0{self.submit_unbound_explicit_held(target,raw,flags,at,ready_after);return;}
        self.detected_explicit+=raw as u128;
        if !self.eligibility(source,target){self.explicit_side_conflicts+=1;self.record_unattributed(true,raw as u128,false,Failure::SideConflict);return;}
        self.note_relationship(source,target,at);let mut credit=Self::raw_credit(source,target,raw,flags,at,"explicit source claim awaiting HP reconciliation");credit.ready_after=ready_after;self.enqueue(credit);
    }
    pub fn submit_unbound_explicit(&mut self,target:EntityKey,raw:u64,flags:[u8;7],at:Stamp){
        self.submit_unbound_explicit_held(target,raw,flags,at,0);
    }
    pub fn submit_unbound_explicit_held(&mut self,target:EntityKey,raw:u64,flags:[u8;7],at:Stamp,ready_after:u64){
        self.detected_explicit+=raw as u128;
        let allocations=self.fallback(target,at,raw,0,&[],"forced unbound raw source from active attackers",false);
        if allocations.is_empty(){self.record_unattributed(true,raw as u128,false,Failure::NoActive);}
        for e in allocations{let mut credit=Self::raw_credit(e.source,target,e.amount,flags,at,"forced unbound raw source from active attackers");credit.ready_after=ready_after;self.enqueue(credit);}
    }
    /// Finalize a still-unleased packet, never a committed receipt. A coarsened
    /// row has lost packet identity: keep its amount and count missed clipping.
    /// `credited` removes only proven terminal excess, retaining auxiliary raw.
    pub fn finalize_explicit(&mut self,target:EntityKey,at:Stamp,raw:u64,credited:u64)->bool{
        let matches=|c:&Credit|matches!(c.kind,CreditKind::Explicit{..}) && !c.detail_coarsened && c.target()==target && c.at()==at;
        let excess=raw.saturating_sub(credited);
        self.max_explicit_finalize_work=self.max_explicit_finalize_work.max(2*(self.outbox.len()+self.owed.len()));
        let leased=self.leased.as_ref().is_some_and(matches);
        let total=self.outbox.iter().filter(|r|matches(&r.credit)).map(|r|r.amount).sum::<u128>()+self.owed.iter().filter(|r|matches(&r.credit)).map(|r|r.amount).sum::<u128>();
        if leased || credited>raw || total!=raw as u128{self.overkill_clip_unavailable+=excess as u128;return false;}
        // Scaling a preselected source claim is clipping, not a new allocation.
        // Cumulative integer endpoints conserve even a split unbound claim.
        let mut before=0u128;
        for (credit,amount) in self.outbox.iter_mut().map(|r|(&mut r.credit,&mut r.amount)).chain(self.owed.iter_mut().map(|r|(&mut r.credit,&mut r.amount))){
            if !matches(credit){continue;}
            let after=before+*amount;let clipped=if raw==0{0}else{after*credited as u128/raw as u128-before*credited as u128/raw as u128};before=after;
            *amount=clipped;credit.set_amount(clipped as u64);credit.ready_after=0;
            if excess>0{credit.method="explicit source claim with proven terminal overkill clipped";}
        }
        if excess>0{self.detected_explicit-=excess as u128;self.clipped_explicit_overkill+=excess as u128;self.clipped_explicit_records+=1;}
        true
    }
    /// Measurement/EOF flush has already resolved all retained HP budgets.
    pub fn release_explicit_holds(&mut self){
        for c in self.outbox.iter_mut().map(|r|&mut r.credit).chain(self.owed.iter_mut().map(|r|&mut r.credit)){c.ready_after=0;}
    }
    /// The only allocator/fallback. Current typed packet support and recent
    /// target-specific attack evidence enter one frequency-neutral allocation.
    fn fallback(&self,target:EntityKey,at:Stamp,amount:u64,interval:u64,typed:&[Candidate],method:&'static str,restricted:bool)->Vec<Estimate>{
        let mut candidates:Vec<Candidate>=typed.iter().take(MAX_RELATIONSHIPS).filter(|c|self.eligibility(c.source,target)).map(|c|Candidate{source:c.source,skill:c.skill,weight:c.weight,frames:c.frames.iter().take(MAX_CREDIT_FRAMES).copied().collect()}).collect();
        let broad=candidates.is_empty() && !restricted;
        if candidates.is_empty() && !restricted{candidates=self.active_candidates(target,at);}
        candidates.sort_by_key(|c|(c.source,c.skill));
        // Combining duplicate notifications uses the larger weight, never count.
        candidates.dedup_by(|a,b|{if a.source==b.source && a.skill==b.skill{b.weight=b.weight.max(a.weight);true}else{false}});
        if candidates.is_empty(){return Vec::new();}
        let method=if broad{"forced recent target-specific active attacker inference"}else{method};
        let max=candidates.iter().map(|c|if c.weight.is_finite(){c.weight.max(0.0)}else{0.0}).fold(0.0,f64::max).max(f64::MIN_POSITIVE);
        let mut shares:Vec<_>=candidates.into_iter().map(|c|{let w=if c.weight.is_finite(){c.weight.max(0.0)}else{0.0};let q=((w/max*(1u32<<24) as f64).round() as u32).max(1);(c,q,0u64,0u128)}).collect();
        let total=shares.iter().map(|s|s.1 as u128).sum::<u128>();let mut used=0;
        for(_,q,n,r)in &mut shares{let p=amount as u128*(*q as u128);*n=(p/total) as u64;*r=p%total;used+=*n;}
        let mut order:Vec<_>=(0..shares.len()).collect();let count=shares.len();let rotate=interval as usize%count;
        order.sort_by(|a,b|shares[*b].3.cmp(&shares[*a].3).then(((a+count-rotate)%count).cmp(&((b+count-rotate)%count))));
        for i in order.into_iter().take((amount-used) as usize){shares[i].2+=1;}
        shares.into_iter().filter(|s|s.2>0).map(|(c,_,n,_)|Estimate{source:c.source,target,amount:n,timestamp_ns:at.timestamp_ns,skill:c.skill,interval,method,frames:c.frames.into_iter().take(MAX_CREDIT_FRAMES).collect(),at}).collect()
    }
    pub fn resolve_budget(&mut self,target:EntityKey,at:Stamp,interval:u64,amount:u64,typed:&[Candidate],method:&'static str)->Vec<Estimate>{
        self.detected_inferred+=amount as u128;
        let selected=self.fallback(target,at,amount,interval,typed,method,false);
        if selected.is_empty(){self.record_unattributed(false,amount as u128,false,if typed.iter().any(|c|c.source.actor!=0 && !self.eligibility(c.source,target)){Failure::SideConflict}else{Failure::NoActive});}
        let mut allocations=Vec::new();for e in selected{self.settled_inferred+=e.amount as u128;let old=self.unattributed_inferred;self.enqueue(Credit{id:0,prepared_skill:None,detail_coarsened:false,kind:CreditKind::Inferred(e.clone()),method,ready_after:0});if old==self.unattributed_inferred{allocations.push(e);}}
        allocations
    }
    fn carry(&mut self,mut o:Owed){
        self.owed_revision+=1;
        let explicit=matches!(o.credit.kind,CreditKind::Explicit{..});let source=o.credit.source();
        if let Some(old)=self.owed.iter_mut().find(|old|old.credit.source()==source && matches!(old.credit.kind,CreditKind::Explicit{..})==explicit){
            old.amount+=o.amount;old.credit.ready_after=0;old.credit.detail_coarsened=true;old.credit.method="known source delivery coarsened under pressure";
            if let CreditKind::Explicit{hit,..}=&mut old.credit.kind{hit.damage.flags=[false;5];hit.damage.raw_flags=[0;7];self.raw_detail_coarsenings+=1;}
            if let CreditKind::Inferred(e)=&mut old.credit.kind{e.skill=None;e.frames.clear();}
            self.coarsened_obligations+=1;return;
        }
        if self.owed.len()==MAX_OWED{self.coarsened_obligations+=1;self.record_unattributed(explicit,o.amount,!explicit,Failure::DeliveryCapacity);return;}
        o.credit.id=0;o.credit.prepared_skill=None;self.owed.push_back(o);
    }
    pub fn revalidate_registry(&mut self,registry:&crate::identity::Registry){
        for actor in &mut self.actors{
            actor.kind=if registry.damage_side(actor.source)==crate::identity::DamageSide::Player{2}
                else if registry.records.get(&actor.source).is_some_and(|m|m.kind==Some("npc")){1}else{0};
        }
        self.relationships.retain(|r|registry.damage_eligible(r.source,r.target));
    }
    pub fn refresh_support(&mut self){self.settle_owed();}
    fn settle_owed(&mut self){
        let count=self.owed.len();for _ in 0..count{
            if self.outbox.len()>=MAX_CREDITS{break;}
            let mut o=self.owed.pop_front().unwrap();let n=o.amount.min(u64::MAX as u128)as u64;o.amount-=n as u128;o.credit.set_amount(n);
            if self.eligibility(o.credit.source(),o.credit.target()){self.enqueue(o.credit.clone());}else{self.record_unattributed(matches!(o.credit.kind,CreditKind::Explicit{..}),n as u128,matches!(o.credit.kind,CreditKind::Inferred(_)),Failure::SideConflict);}
            if o.amount>0{self.owed.push_back(o);}
        }
    }
    fn enqueue(&mut self,mut credit:Credit){
        if !self.eligibility(credit.source(),credit.target()){let explicit=matches!(credit.kind,CreditKind::Explicit{..});self.record_unattributed(explicit,credit.amount()as u128,!explicit,Failure::SideConflict);return;}
        if self.outbox.len()<MAX_CREDITS{let amount=credit.amount() as u128;self.outbox.push_back(Row{credit,amount});return;}
        let source=credit.source();let explicit=matches!(credit.kind,CreditKind::Explicit{..});
        // Once fine publication storage is exhausted, actor/origin identity
        // takes priority over another full scan for target-detail agreement.
        let index=self.outbox.iter().position(|r|r.credit.source()==source && matches!(r.credit.kind,CreditKind::Explicit{..})==explicit);
        if let Some(i)=index{
            // Coarsening has lost packet clipping identity. Do not extend the
            // front hold on every new hit and starve delivery under pressure.
            self.outbox[i].amount+=credit.amount() as u128;self.outbox[i].credit.ready_after=0;self.outbox[i].credit.detail_coarsened=true;self.outbox[i].credit.method="forced publication pressure; timing target skill flags and hit count detail coarsened";
            if let CreditKind::Explicit{hit,..}=&mut self.outbox[i].credit.kind{hit.damage.flags=[false;5];hit.damage.raw_flags=[0;7];self.raw_detail_coarsenings+=1;}
            if let CreditKind::Inferred(e)=&mut self.outbox[i].credit.kind{e.skill=None;e.frames.clear();e.method="forced publication pressure; timing target skill detail coarsened";}
            self.coarsened_publications+=1;return;
        }
        credit.method="known source delivery pending under publication pressure";
        self.carry(Owed{amount:credit.amount()as u128,credit});self.coarsened_publications+=1;
    }
    /// Peeking leases an immutable amount/receipt, so an enqueue before ack
    /// cannot accidentally acknowledge newly merged damage.
    pub fn front_ready_credit(&mut self,available:u64)->Option<Credit>{
        if self.leased.is_some(){return self.front_credit();}
        if self.outbox.is_empty(){self.settle_owed();}
        if self.outbox.front().is_some_and(|r|r.credit.ready_after!=0 && r.credit.ready_after>=available){return None;}
        self.front_credit()
    }
    pub fn front_credit(&mut self)->Option<Credit>{
        if let Some(leased)=&self.leased{return Some(leased.clone());}
        if self.outbox.is_empty(){self.settle_owed();}
        let row=self.outbox.front()?;let amount=row.amount.min(u64::MAX as u128) as u64;
        self.next_receipt+=1;let mut credit=row.credit.clone();credit.id=self.next_receipt;credit.set_amount(amount);self.leased=Some(credit.clone());Some(credit)
    }
    pub fn prepare_skill(&mut self,id:u64,skill:Option<u32>)->bool{
        let Some(leased)=self.leased.as_mut()else{return false;};if leased.id!=id{return false;}
        if let Some(prepared)=leased.prepared_skill{return prepared==skill;}
        leased.prepared_skill=Some(skill);true
    }
    pub fn ack_credit(&mut self,id:u64)->bool{
        let Some(leased)=&self.leased else{return false;};if leased.id!=id{return false;}let n=leased.amount();
        let explicit=matches!(leased.kind,CreditKind::Explicit{..});
        let Some(row)=self.outbox.front_mut()else{return false;};row.amount-=n as u128;if row.amount==0{self.outbox.pop_front();}self.leased=None;self.published+=n as u128;if explicit{self.published_explicit+=n as u128;}else{self.published_inferred+=n as u128;}true
    }
    /// An atomic consumer rejection changes support through the same fallback.
    /// Empty eligible replacement support records unattributed credit. Arithmetic
    /// retry keeps its original immutable lease and does not call this method.
    pub fn reject_leased(&mut self,id:u64)->bool{self.reject_leased_for(id,Failure::SideConflict)}
    fn reject_leased_for(&mut self,id:u64,reason:Failure)->bool{
        let Some(original)=self.leased.as_ref().filter(|c|c.id==id).cloned()else{return false;};let n=original.amount();
        let Some(row)=self.outbox.front_mut()else{return false;};row.amount-=n as u128;if row.amount==0{self.outbox.pop_front();}self.leased=None;self.rejected_publications+=1;
        let explicit=matches!(original.kind,CreditKind::Explicit{..});self.record_unattributed(explicit,n as u128,!explicit,reason);true
    }
    pub fn retry_credit(&mut self,id:u64,accepted:&[Candidate])->bool{
        let Some(original)=self.leased.as_ref().filter(|c|c.id==id).cloned()else{return false;};let n=original.amount();let target=original.target();let at=original.at();
        let active=self.active_candidates(target,at);
        let support:Vec<_>=accepted.iter().filter(|c|self.eligibility(c.source,target) && (c.source==original.source() || active.iter().any(|a|a.source==c.source))).take(MAX_ACTORS).cloned().collect();
        let allocations=self.fallback(target,at,n,id,&support,"forced eligible active attacker after consumer rejection",true);if allocations.is_empty(){return self.reject_leased_for(id,Failure::DeliveryCapacity);}
        let raw=match &original.kind{CreditKind::Explicit{hit,..}=>Some(hit.damage.raw_flags),_=>None};
        let Some(row)=self.outbox.front_mut()else{return false;};row.amount-=n as u128;if row.amount==0{self.outbox.pop_front();}self.leased=None;self.rejected_publications+=1;
        for e in allocations{let mut credit=if let Some(flags)=raw{Self::raw_credit(e.source,target,e.amount,flags,at,"forced eligible active attacker after consumer rejection")}else{Credit{id:0,prepared_skill:None,detail_coarsened:false,kind:CreditKind::Inferred(e),method:"forced eligible active attacker after consumer rejection",ready_after:0}};credit.detail_coarsened=original.detail_coarsened;self.enqueue(credit);}true
    }
    pub fn owed_amount(&self)->u128{self.owed.iter().map(|o|o.amount).sum()}
    pub fn unattributed_amount(&self)->u128{self.unattributed_explicit+self.unattributed_inferred}
    pub fn queued_amount(&self)->u128{self.outbox.iter().map(|r|r.amount).sum()}
    pub fn queued_len(&self)->usize{self.outbox.len()}
    pub fn supported_sources(&self)->impl Iterator<Item=EntityKey>+'_ {self.actors.iter().map(|a|a.source)}
    pub fn audit(&self)->serde_json::Value{
        let queued_raw=self.outbox.iter().filter(|r|matches!(r.credit.kind,CreditKind::Explicit{..})).map(|r|r.amount).sum::<u128>();
        let owed_raw=self.owed.iter().filter(|o|matches!(o.credit.kind,CreditKind::Explicit{..})).map(|o|o.amount).sum::<u128>();let queued=self.queued_amount();let owed=self.owed_amount();
        let mut audit=serde_json::json!({"format":"active-attribution-v2","detected_explicit":self.detected_explicit.to_string(),"detected_inferred":self.detected_inferred.to_string(),"total_obligations":(self.detected_explicit+self.detected_inferred).to_string(),"published":self.published.to_string(),"queued":queued.to_string(),"owed":owed.to_string(),"invariant":self.detected_explicit+self.detected_inferred==self.published+queued+owed+self.unattributed_amount(),"settled_inferred":self.settled_inferred.to_string(),"queued_rows":self.outbox.len(),"owed_rows":self.owed.len(),"actors":self.actors.len(),"relationships":self.relationships.len(),"support_revision":self.support_revision,"coarsened_obligations":self.coarsened_obligations,"coarsened_publications":self.coarsened_publications,"raw_detail_coarsenings":self.raw_detail_coarsenings,"rejected_publications":self.rejected_publications});
        audit["active_idle_ns"]=serde_json::json!(self.active_idle_ns);audit["unattributed"]=serde_json::json!(self.unattributed_amount().to_string());audit["unattributed_explicit"]=serde_json::json!(self.unattributed_explicit.to_string());audit["unattributed_inferred"]=serde_json::json!(self.unattributed_inferred.to_string());audit["unattributed_outcomes"]=serde_json::json!(self.unattributed_outcomes);audit["explicit_side_conflicts"]=serde_json::json!(self.explicit_side_conflicts);
        audit["observed_raw_explicit"]=serde_json::json!((self.detected_explicit+self.clipped_explicit_overkill).to_string());
        audit["overkill_clipped_explicit"]=serde_json::json!(self.clipped_explicit_overkill.to_string());audit["overkill_clipped_records"]=serde_json::json!(self.clipped_explicit_records);audit["overkill_clip_unavailable"]=serde_json::json!(self.overkill_clip_unavailable.to_string());audit["max_explicit_finalize_work"]=serde_json::json!(self.max_explicit_finalize_work);
        audit["unattributed_reasons"]=serde_json::json!({"no_eligible_active_unattributed":self.no_eligible_active_unattributed.to_string(),"side_conflict_unattributed":self.side_conflict_unattributed.to_string(),"delivery_capacity_unattributed":self.delivery_capacity_unattributed.to_string(),"invariant":self.no_eligible_active_unattributed+self.side_conflict_unattributed+self.delivery_capacity_unattributed==self.unattributed_amount()});
        audit["by_origin"]=serde_json::json!({"explicit":{"published":self.published_explicit.to_string(),"queued":queued_raw.to_string(),"owed":owed_raw.to_string(),"unattributed":self.unattributed_explicit.to_string(),"invariant":self.detected_explicit==self.published_explicit+queued_raw+owed_raw+self.unattributed_explicit},"inferred":{"published":self.published_inferred.to_string(),"queued":(queued-queued_raw).to_string(),"owed":(owed-owed_raw).to_string(),"unattributed":self.unattributed_inferred.to_string(),"invariant":self.detected_inferred==self.published_inferred+(queued-queued_raw)+(owed-owed_raw)+self.unattributed_inferred}});
        audit["owed_preview"]=serde_json::json!(self.owed.iter().take(8).map(|o|serde_json::json!({"source_actor":o.credit.source().actor,"target_actor":o.credit.target().actor,"context":o.credit.at().context,"amount":o.amount.to_string(),"raw":matches!(o.credit.kind,CreditKind::Explicit{..}),"detail_coarsened":o.credit.detail_coarsened})).collect::<Vec<_>>());
        audit["roster_preview"]=serde_json::json!(self.actors.iter().take(8).map(|a|serde_json::json!({"actor":a.source.actor,"process":a.source.process,"binding":a.source.binding,"generation":a.source.generation,"context":a.at.context,"frame":a.at.frame,"kind":a.kind})).collect::<Vec<_>>());
        audit["relationship_preview"]=serde_json::json!(self.relationships.iter().take(8).map(|r|serde_json::json!({"source_actor":r.source.actor,"target_actor":r.target.actor,"process":r.source.process,"binding":r.source.binding,"context":r.at.context,"frame":r.at.frame,"first_frame":r.first.frame,"first_ns":r.first.timestamp_ns,"last_ns":r.at.timestamp_ns,"idle_policy":r.idle_policy})).collect::<Vec<_>>());audit
    }
    pub fn owned_bytes(&self)->usize{std::mem::size_of::<Self>()+self.actors.capacity()*std::mem::size_of::<Actor>()+self.relationships.capacity()*std::mem::size_of::<Relationship>()+self.outbox.capacity()*std::mem::size_of::<Row>()+self.owed.capacity()*std::mem::size_of::<Owed>()+self.outbox.iter().map(|r|match &r.credit.kind{CreditKind::Inferred(e)=>e.frames.capacity()*8,_=>0}).sum::<usize>()+self.owed.iter().map(|o|match &o.credit.kind{CreditKind::Inferred(e)=>e.frames.capacity()*8,_=>0}).sum::<usize>()}
    pub fn ceiling_bytes()->usize{std::mem::size_of::<Self>()+MAX_ACTORS*std::mem::size_of::<Actor>()+MAX_RELATIONSHIPS*std::mem::size_of::<Relationship>()+MAX_CREDITS*(std::mem::size_of::<Row>()+MAX_CREDIT_FRAMES*8)+MAX_OWED*(std::mem::size_of::<Owed>()+MAX_CREDIT_FRAMES*8)}
}
