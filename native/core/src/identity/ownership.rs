//! Owned kind-4 JP20051/20517 profile, validated against the four confirmed
//! research pets. Variable action, two appearance traits and effect arrays are
//! consumed from declared positions; no search for owner-looking integers.
use super::{EntityKey,Registry,tracking::Evidence};
use crate::{protocol::{u32le,u64le},Result};
use std::collections::BTreeMap;
#[derive(Clone,Debug)]
pub struct Spawn {pub actor:u32,pub owner:u32,pub template:u32,pub outer:Option<u64>,pub kind:u32}
fn bounded_string(p:&[u8],at:usize)->Option<usize>{
    let n=u32le(p.get(at..at+4)?,0) as usize;
    if n>1024 || n%2!=0{return None;}
    let s=p.get(at+4..at+4+n)?;
    String::from_utf16(&s.chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect::<Vec<_>>()).ok()?;
    Some(at+4+n)
}
pub fn parse(wire:i32,body:&[u8])->Option<Spawn>{
    let (p,outer)=match wire {20051=>(body,None),20517=>(body.get(8..)?,Some(u64le(body,0))),_=>return None};
    if !(1300..=65536).contains(&p.len()){return None;}
    let a=u64le(p,0);
    if a==0 || a>u32::MAX as u64 || u32le(p,8)!=0x52cb67e4 || u32le(p,12)!=1
        || u64le(p,572)!=a || u32le(p,580)!=2048{return None;}
    let mut at=bounded_string(p,622)?.checked_add(29+24)?;
    for count_offset in [18,22]{
        let count=u32le(p.get(at+count_offset..at+count_offset+4)?,0) as usize;
        if count>64{return None;}
        let name_at=at.checked_add(71+4*count)?;
        let end=bounded_string(p,name_at)?;
        let scale=f32::from_bits(u32le(p.get(name_at-8..name_at-4)?,0));
        if !scale.is_finite() || scale<=0.0 || scale>10.0{return None;}
        at=end.checked_add(20)?;
    }
    let count=u32le(p.get(at+266..at+270)?,0) as usize;
    if count>1024{return None;}
    at=at.checked_add(417+28*count)?;
    if at+110!=p.len(){return None;}
    let owner=u64le(p,at);let kind=u32le(p,at+8);
    if owner==0 || owner>u32::MAX as u64 || owner==a || kind!=4 || u32le(p,at+12)!=0 || u32le(p,at+16)!=0{return None;}
    Some(Spawn{actor:a as u32,owner:owner as u32,template:u32le(p,at+24),outer,kind})
}
#[derive(Clone,Debug)]
pub struct Ownership {pub pet:EntityKey,pub owner:EntityKey,pub template:u32,pub kind:u32,pub outer:Option<u64>,pub evidence:Evidence,pub conflicting:bool,pub conflict_evidence:Option<(EntityKey,Evidence)>}
#[derive(Default,Debug)]
pub struct Owners {pub links:BTreeMap<EntityKey,Ownership>,pub conflicts:u64}
impl Registry {
    pub fn owned_spawn(&mut self,process:u64,binding:u64,spawn:Spawn,evidence:Evidence)->Result<EntityKey>{
        if spawn.kind!=4 || spawn.actor==0 || spawn.owner==0 || spawn.actor==spawn.owner{return Err("invalid owned spawn".into());}
        let mut pet=self.observe(process,binding,spawn.actor)?;
        if self.owners.links.get(&pet).is_some_and(|old|old.template!=spawn.template){self.retire(pet)?;pet=self.observe(process,binding,spawn.actor)?;}
        // An owner can be provisional; appearance enrichment never changes this
        // reference. Reusing that raw ID later cannot steal the historical pet.
        let owner=self.observe(process,binding,spawn.owner)?;
        if !self.owners.links.contains_key(&pet) && self.owners.links.len()>=32768{return Err("pet ownership capacity reached".into());}
        let link=self.owners.links.entry(pet).or_insert(Ownership{pet,owner,template:spawn.template,kind:spawn.kind,outer:spawn.outer,evidence,conflicting:false,conflict_evidence:None});
        if link.owner!=owner{link.conflicting=true;link.conflict_evidence=Some((owner,evidence));self.owners.conflicts+=1;}
        let meta=self.records.get_mut(&pet).unwrap();meta.kind=Some("pet");meta.template=Some(format!("{:08X}",spawn.template));
        self.revision+=1;Ok(pet)
    }
    pub fn owner(&self,pet:EntityKey)->Option<EntityKey>{self.owners.links.get(&pet).filter(|l|!l.conflicting).map(|l|l.owner)}
    pub fn owner_group(&self,key:EntityKey)->EntityKey{self.group(self.owner(key).unwrap_or(key))}
    pub fn is_self_source(&self,key:EntityKey)->bool{self.self_key.is_some_and(|own|self.owner_group(key)==self.group(own))}
}
