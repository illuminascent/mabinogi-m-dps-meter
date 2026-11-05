//! JP170205 health deltas are estimates, kept separate from explicit JP21196.
use crate::{identity::EntityKey, protocol::{u32le,u64le}};
use std::collections::BTreeMap;

pub fn parse(p: &[u8]) -> Option<(u32,u64)> {
    if p.len()!=21 || u32le(p,8)!=8 { return None; }
    let actor=u64le(p,0); let hp=u64le(p,12);
    (actor>0 && actor<=u32::MAX as u64 && hp<=i64::MAX as u64).then_some((actor as u32,hp))
}
#[derive(Clone,Debug)]
pub struct Estimate { pub source:EntityKey,pub target:EntityKey,pub amount:u64,pub timestamp_ns:u64 }
#[derive(Default)]
struct Slot { hp:u64,pending:Option<Pending> }
struct Pending { amount:u64,timestamp_ns:u64,explicit:bool,attacks:Vec<EntityKey> }
#[derive(Default)]
pub struct Health { slots:BTreeMap<(u64,EntityKey),Slot> }
impl Health {
    fn close(target:EntityKey,p:Pending)->Option<Estimate> {
        if p.amount>0 && !p.explicit && p.attacks.len()==1 {
            Some(Estimate { source:p.attacks[0],target,amount:p.amount,timestamp_ns:p.timestamp_ns })
        } else { None }
    }
    pub fn update(&mut self,target:EntityKey,hp:u64,timestamp_ns:u64)->Result<Option<Estimate>,String> {
        self.update_in(0,target,hp,timestamp_ns)
    }
    pub fn update_in(&mut self,context:u64,target:EntityKey,hp:u64,timestamp_ns:u64)->Result<Option<Estimate>,String> {
        let key=(context,target);
        if !self.slots.contains_key(&key) && self.slots.len()>=4096 { return Err("HP inference capacity reached".into()); }
        if let Some(slot)=self.slots.get_mut(&key) {
            if hp>slot.hp { slot.hp=hp;slot.pending=None;return Ok(None); }
            let result=slot.pending.take().and_then(|p| Self::close(target,p));
            slot.pending=(hp<slot.hp).then_some(Pending { amount:slot.hp.saturating_sub(hp),timestamp_ns,explicit:false,attacks:Vec::new() });
            slot.hp=hp; Ok(result)
        } else { self.slots.insert(key,Slot { hp,pending:None }); Ok(None) }
    }
    pub fn attack(&mut self,source:EntityKey,target:EntityKey) {
        self.attack_in(0,source,target)
    }
    pub fn attack_in(&mut self,context:u64,source:EntityKey,target:EntityKey) {
        if let Some(p)=self.slots.get_mut(&(context,target)).and_then(|s|s.pending.as_mut()) {
            if p.attacks.len()<2 { p.attacks.push(source); }
        }
    }
    pub fn explicit(&mut self,target:EntityKey) {
        self.explicit_in(0,target)
    }
    pub fn explicit_in(&mut self,context:u64,target:EntityKey) {if let Some(p)=self.slots.get_mut(&(context,target)).and_then(|s|s.pending.as_mut()){p.explicit=true;}}
    pub fn clear(&mut self) { self.slots.clear(); }
    pub fn clear_context(&mut self,context:u64){self.slots.retain(|(c,_),_|*c!=context);}
    pub fn flush(&mut self)->Vec<Estimate> {
        let result=self.slots.iter_mut().filter_map(|((_,k),s)|s.pending.take().and_then(|p|Self::close(*k,p))).collect();self.clear();result
    }
}
