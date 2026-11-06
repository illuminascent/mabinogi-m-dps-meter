//! Supported JP20590 personal-pet collection. The surrounding snapshot has
//! variable opaque sections. Recognize a unique COMPLETE bounded collection,
//! including repeated persistent keys, every record and the selected-pet suffix.
//! Never scan for an actor-looking integer or use character-specific offsets,
//! pet names, templates or persistent keys. Unsupported profiles fail closed.
use super::{personal_prefix,Character};
use crate::protocol::{u32le,u64le};
use std::collections::BTreeSet;

const MAX_SNAPSHOT:usize=1024*1024;
#[derive(Clone,Debug)]
pub struct Pet {pub persistent:u64,pub template:u32}
#[derive(Clone,Debug)]
pub struct Personal {pub character:Character,pub actor:u32,pub pets:Vec<Pet>,pub offset:usize,pub end:usize}
fn time(n:u64)->bool{n==0 || (1_500_000_000_000..=2_200_000_000_000).contains(&n)}
fn record(p:&[u8],at:&mut usize)->Option<(u64,u32,u32)>{
    let head=p.get(*at..at.checked_add(44)?)?;
    let key=u64le(head,0);let owner=u64le(head,8);let template=u32le(head,24);
    if key<=u32::MAX as u64 || owner==0 || owner>u32::MAX as u64 || key!=u64le(head,16) || template==0
        || !time(u64le(head,28)) || u64le(head,28)==0 || !(1..=64).contains(&u32le(head,36)){return None;}
    let n=u32le(head,40) as usize;if n>64 || n%2!=0{return None;}
    let text=p.get(*at+44..*at+44+n)?;
    let name=String::from_utf16(&text.chunks_exact(2).map(|s|u16::from_le_bytes([s[0],s[1]])).collect::<Vec<_>>()).ok()?;
    if name.chars().any(char::is_control){return None;}
    *at+=44+n;let state=p.get(*at..at.checked_add(71)?)?;
    if !time(u64le(state,0)) || u32le(state,8)>10000 || !time(u64le(state,20)) || !time(u64le(state,28)) || state[36..]!=[0;35]{return None;}
    *at+=71;let count=u32le(p.get(*at..*at+4)?,0) as usize;
    if !(1..=64).contains(&count){return None;}*at+=4;let mut stats=BTreeSet::new();
    for _ in 0..count{
        let entry=p.get(*at..*at+8)?;let id=u32le(entry,0);let value=f32::from_bits(u32le(entry,4));
        if id>4096 || !stats.insert(id) || !value.is_finite() || !(0.0..=1e9).contains(&value){return None;}*at+=8;
    }
    let tail=p.get(*at..*at+48)?;
    if u32le(tail,0)!=3 || u32le(tail,4)>10000 || u64le(tail,8)!=0 || u32le(tail,20)!=5
        || tail[24]!=0 || u32le(tail,25)!=4 || tail[33..]!=[0;15]{return None;}
    *at+=48;Some((key,owner as u32,template))
}
fn collection(p:&[u8],offset:usize,count:usize)->Option<(usize,u32,Vec<Pet>)>{
    let mut at=offset+4;let mut owner=None;let mut keys=BTreeSet::new();let mut pets=Vec::new();
    for _ in 0..count{
        let (key,actor,template)=record(p,&mut at)?;
        if !keys.insert(key) || owner.is_some_and(|old|old!=actor){return None;}owner=Some(actor);
        pets.push(Pet{persistent:key,template});
    }
    let suffix=p.get(at..at+40)?;let selected=u64le(suffix,4);
    if u32le(suffix,0)!=0 || !keys.contains(&selected) || u64le(suffix,12)!=selected || u32le(suffix,20)!=1
        || !time(u64le(suffix,24)) || !time(u64le(suffix,32)){return None;}
    Some((at+40,owner?,pets))
}
pub fn parse(p:&[u8])->Option<Personal>{
    let character=personal_prefix(p)?;
    if p.len()>MAX_SNAPSHOT || p.len()<112{return None;}
    let mut found=None;
    for offset in 64..p.len()-47{
        let count=u32le(p,offset) as usize;
        // Cheap grammar prefilter only; the owner field is never the search key.
        if !(1..=64).contains(&count) || u64le(p,offset+4)<=u32::MAX as u64 || u64le(p,offset+4)!=u64le(p,offset+20){continue;}
        if let Some((end,actor,pets))=collection(p,offset,count){
            if found.is_some(){return None;}
            found=Some(Personal{character:character.clone(),actor,pets,offset,end});
        }
    }
    found
}
