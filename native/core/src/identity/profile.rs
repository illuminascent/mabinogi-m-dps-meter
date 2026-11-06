//! Bounded JP20559 profile/chat envelope. The actor/UID/name prefix is followed
//! by three declared UTF-16 fields, fixed presentation fields, a length-delimited
//! JPEG thumbnail and a final u32. Chat text and opaque tokens are not retained.
use crate::protocol::{u32le,u64le};
#[derive(Clone,Debug)]
pub struct Profile{pub actor:u32,pub uid:u64,pub name:String}
fn string(p:&[u8],at:&mut usize,max:usize)->Option<String>{
    let n=u32le(p.get(*at..*at+4)?,0) as usize;
    if n>max || n%2!=0{return None;}*at+=4;
    let s=String::from_utf16(&p.get(*at..*at+n)?.chunks_exact(2).map(|s|u16::from_le_bytes([s[0],s[1]])).collect::<Vec<_>>()).ok()?;
    *at+=n;Some(s)
}
pub fn parse(p:&[u8])->Option<Profile>{
    if !(49..=270336).contains(&p.len()){return None;}
    let actor=u64le(p,0);let uid=u64le(p,8);
    if actor==0 || actor>u32::MAX as u64 || uid==0{return None;}
    let mut at=16;let name=string(p,&mut at,64)?;
    if name.trim().is_empty() || name.chars().any(char::is_control){return None;}
    string(p,&mut at,4096)?;string(p,&mut at,1024)?;
    let tail=p.get(at..at+13)?;
    if u32le(tail,0)!=0 || tail[8]!=0{return None;}
    let n=u32le(tail,9) as usize;at+=13;
    if !(4..=262144).contains(&n) || at+n+4!=p.len(){return None;}
    let jpeg=&p[at..at+n];if jpeg[..2]!=[0xff,0xd8] || jpeg[n-2..]!=[0xff,0xd9]{return None;}
    Some(Profile{actor:actor as u32,uid,name})
}
