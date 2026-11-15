//! Exact, name-independent HP/hit/floater source binding before credit commit.
//! A friendly or zero-impact effect cannot become a damaging hit merely by
//! occurring near another target's HP loss. Original wire fields stay intact.
use crate::{combat::{Attack,Stamp},damage_wire::Fields,identity::{EntityKey,Registry},replay::{key_json,stamp_json}};
use serde_json::{json,Value};
use std::collections::VecDeque;
pub const MAX_CELLS:usize=128;
pub const MAX_AUDIT:usize=64;
const LOCAL_NS:u64=100_000_000;
const LOCAL_ORDERS:u64=8;
#[derive(Clone,Copy)]
struct Presentation{source:EntityKey,at:Stamp,flags:[u8;7],skill:u32}
struct Cell{target:EntityKey,at:Stamp,loss:u64,presentations:u32,floater_seen:bool,hit:Option<Presentation>}
#[derive(Clone,Copy)]
pub struct Binding{pub wire_actor:EntityKey,pub hit_actor:EntityKey,pub target:EntityKey,pub at:Stamp,pub hp:Stamp,pub presentation:Stamp,pub amount:u64,pub skill:u32,pub flags:[u8;7]}
pub struct HitSource{cells:VecDeque<Cell>,audit:VecDeque<Binding>,bound:u64,amount:u128,cell_evictions:u64,audit_evictions:u64}
impl Default for HitSource{fn default()->Self{Self{cells:VecDeque::with_capacity(MAX_CELLS),audit:VecDeque::with_capacity(MAX_AUDIT),bound:0,amount:0,cell_evictions:0,audit_evictions:0}}}
fn scope(a:Stamp,b:Stamp)->bool{a.process==b.process && a.context==b.context && a.continuity==b.continuity}
fn player(r:&Registry,k:EntityKey)->bool{r.is_current(k) && r.records.get(&k).is_some_and(|m|m.kind==Some("player") && !m.conflicting)}
impl HitSource{
    pub fn clear(&mut self){self.cells.clear();}
    pub fn clear_context(&mut self,context:u64){self.cells.retain(|c|c.at.context!=context);}
    pub fn clear_target(&mut self,target:EntityKey){self.cells.retain(|c|c.target!=target);}
    pub fn hp(&mut self,target:EntityKey,hp:u64,at:Stamp,previous:Option<(Stamp,u64)>){
        self.cells.retain(|c|c.target!=target || c.at.context!=at.context);
        let Some((before,old))=previous else{return;};
        if !scope(before,at) || old<=hp || target.generation==0{return;}
        if self.cells.len()==MAX_CELLS{self.cells.pop_front();self.cell_evictions+=1;}
        self.cells.push_back(Cell{target,at,loss:old-hp,presentations:0,floater_seen:false,hit:None});
    }
    pub fn presentation(&mut self,a:&Attack,body:&[u8]){
        let Some(c)=self.cells.iter_mut().find(|c|c.target==a.target && scope(c.at,a.at))else{return;};
        c.presentations=c.presentations.saturating_add(1);
        if c.presentations!=1{c.hit=None;return;}
        if body.len()!=35 || crate::protocol::u64le(body,0)!=a.source.actor as u64 || crate::protocol::u64le(body,8)!=a.target.actor as u64
            || c.at.order.checked_add(1)!=Some(a.at.order) || a.at.timestamp_ns<c.at.timestamp_ns{return;}
        c.hit=Some(Presentation{source:a.source,at:a.at,flags:a.meta[..7].try_into().unwrap(),skill:a.key1});
    }
    pub fn floater(&mut self,wire_actor:Option<EntityKey>,target:EntityKey,f:&Fields,at:Stamp,r:&Registry)->Option<Binding>{
        let c=self.cells.iter_mut().find(|c|c.target==target && scope(c.at,at))?;
        let seen=c.floater_seen;c.floater_seen=true;
        if seen || c.presentations!=1 || f.body_length!=53 || f.raw_amount==u64::MAX || f.auxiliary_24!=0 || f.raw_amount!=c.loss{return None;}
        let hit=c.hit?;let wire_actor=wire_actor?;
        if hit.source==wire_actor || hit.flags!=f.raw_flags || at.order<=hit.at.order || at.order-hit.at.order>LOCAL_ORDERS
            || at.timestamp_ns<c.at.timestamp_ns || at.timestamp_ns-c.at.timestamp_ns>LOCAL_NS
            || !player(r,wire_actor) || !player(r,hit.source) || !r.is_current(target)
            || !r.damage_eligible(wire_actor,target) || !r.damage_eligible(hit.source,target){return None;}
        let b=Binding{wire_actor,hit_actor:hit.source,target,at,hp:c.at,presentation:hit.at,amount:f.raw_amount,skill:hit.skill,flags:f.raw_flags};
        self.bound+=1;self.amount+=f.raw_amount as u128;
        if self.audit.len()==MAX_AUDIT{self.audit.pop_front();self.audit_evictions+=1;}
        self.audit.push_back(b);Some(b)
    }
    pub fn ceiling_bytes()->usize{std::mem::size_of::<Self>()+MAX_CELLS*std::mem::size_of::<Cell>()+MAX_AUDIT*std::mem::size_of::<Binding>()}
    pub fn owned_bytes(&self)->usize{std::mem::size_of::<Self>()+self.cells.capacity()*std::mem::size_of::<Cell>()+self.audit.capacity()*std::mem::size_of::<Binding>()}
    pub fn audit(&self)->Value{json!({"format":"exact-hit-source-v1","bound":self.bound,"amount":self.amount.to_string(),"cells":self.cells.len(),"cell_evictions":self.cell_evictions,"audit_evictions":self.audit_evictions,"limits":{"cells":MAX_CELLS,"audit":MAX_AUDIT},"ceiling_bytes":Self::ceiling_bytes(),"owned_bytes":self.owned_bytes(),"records":self.audit.iter().map(|b|json!({"wire_actor":key_json(b.wire_actor),"hit_actor":key_json(b.hit_actor),"target":key_json(b.target),"at":stamp_json(b.at),"hp":stamp_json(b.hp),"presentation":stamp_json(b.presentation),"amount":b.amount,"skill":b.skill,"flags":b.flags})).collect::<Vec<_>>()})}
}
