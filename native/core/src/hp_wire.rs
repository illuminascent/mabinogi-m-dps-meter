//! Structural HP evidence. Recognition does not assert an opcode's meaning,
//! actor width, numeric range, or availability of a decryption key.
use crate::protocol::{u32le,u64le};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Error { Bounds, Limit, ActorMismatch, Unsupported }

pub struct Envelope<'a> { pub actor:u64,pub blob:&'a [u8],pub trailer:u8 }
pub fn envelope(p:&[u8])->Option<Envelope<'_>> {
    let n=u32le(p.get(8..12)?,0) as usize;
    if n==0 || n>i32::MAX as usize || n.checked_add(13)?!=p.len(){return None;}
    Some(Envelope{actor:u64le(p,0),blob:&p[12..12+n],trailer:p[12+n]})
}

pub struct Status<'a> {
    pub actor:u64,pub mode:u8,pub hp:&'a [u8],pub iv:&'a [u8],
    pub offset:usize,pub iv_offset:usize,pub end:usize,
    pub parameter_marker:u8,pub level:i32,pub float_bits:u32,pub following:u8,
}
struct Cursor<'a>{p:&'a [u8],at:usize}
impl<'a> Cursor<'a>{
    fn take(&mut self,n:usize)->Result<&'a [u8],Error>{
        let end=self.at.checked_add(n).ok_or(Error::Bounds)?;
        let p=self.p.get(self.at..end).ok_or(Error::Bounds)?;self.at=end;Ok(p)
    }
    fn skip(&mut self,n:usize)->Result<(),Error>{self.take(n).map(|_|())}
    fn byte(&mut self)->Result<u8,Error>{Ok(self.take(1)?[0])}
    fn word(&mut self)->Result<u32,Error>{Ok(u32le(self.take(4)?,0))}
    fn long(&mut self)->Result<u64,Error>{Ok(u64le(self.take(8)?,0))}
    fn count(&mut self,max:usize)->Result<usize,Error>{
        let n=self.word()? as i32;if n<0{return Err(Error::Bounds);}
        if n as usize>max{return Err(Error::Limit);}Ok(n as usize)
    }
    fn blob(&mut self)->Result<&'a [u8],Error>{let n=self.count(65536)?;self.take(n)}
}
/// The observed JP common prefix; unrelated appearance/action suffixes are not
/// prerequisites. Counts are operational bounds, not claims about game limits.
pub fn status(wire:i32,p:&[u8])->Result<Status<'_>,Error>{
    let base=match wire{20051|20054=>0,20517=>8,_=>return Err(Error::Unsupported)};
    let mut c=Cursor{p,at:base};
    let outer=c.long()?;c.skip(9)?;
    let n=c.count(4096)?;c.skip(n)?;c.skip(46)?;
    c.skip(17*8+3*8+172+3*8+4*4)?;
    let parameter_marker=c.byte()?;c.skip(6*8+24)?;
    for _ in 0..3{let count=c.count(4096)?;c.skip(count*8)?;}
    c.skip(36)?;let level=c.word()? as i32;c.skip(8)?;
    let offset=c.at;let actor=c.long()?;
    if actor!=outer{return Err(Error::ActorMismatch);}
    let mode=c.byte()?;let hp=c.blob()?;
    let n=c.count(65536)?;let iv_offset=c.at;let iv=c.take(n)?;
    let float_bits=c.word()?;let following=c.byte()?;
    Ok(Status{actor,mode,hp,iv,offset,iv_offset,end:c.at,parameter_marker,level,float_bits,following})
}

/// Exactly one classification for every delivered body, before semantic gates.
/// Descriptors are deliberately broad: strings and non-HP packets also match.
pub fn shape(p:&[u8],decoded:bool)->&'static str{
    if !decoded{return "opaque body";}
    if p.len()<12{return "short body";}
    let n=u32le(p,8) as i32;
    if n<0 || (n as usize)>p.len()-12{return "no bounded blob";}
    if n==0{return "zero blob";}
    let tail=p.len()-12-n as usize;
    let scalar=n==8 || n%16==0;
    match (tail<=1,scalar){
        (true,true)=>"exact scalar candidate",(true,false)=>"exact other candidate",
        (false,true)=>"extended scalar candidate",(false,false)=>"extended other candidate",
    }
}
