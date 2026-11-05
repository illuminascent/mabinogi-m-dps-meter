//! Versioned bounded binary IPC. No native object memory crosses the pipe.
use mabi_meter_core::{Result,transport::{Flow,Segment}};
use std::{net::{IpAddr,Ipv4Addr,Ipv6Addr},collections::{BTreeMap,VecDeque}};
pub const MAX:usize=256*1024;
pub fn envelope(sequence:u64,kind:u8,data:&[u8])->Result<Vec<u8>> {
    if data.len()+10>MAX{return Err("IPC message exceeds limit".into());}
    let mut out=vec![1,kind];out.extend_from_slice(&sequence.to_le_bytes());out.extend_from_slice(data);Ok(out)
}
pub fn header<'a>(bytes:&'a [u8],expected:&mut u64)->Result<(u8,&'a [u8])> {
    if !(10..=MAX).contains(&bytes.len())||bytes[0]!=1{return Err("Unsupported capture IPC version/length".into());}
    let seq=u64::from_le_bytes(bytes[2..10].try_into().unwrap());
    if seq!=*expected{return Err("Capture IPC sequence discontinuity".into());}*expected+=1;Ok((bytes[1],&bytes[10..]))
}
fn ip_bytes(ip:&str)->Result<(u8,[u8;16])>{let mut b=[0;16];match ip.parse::<IpAddr>().map_err(|_|"Invalid IPC IP")?{IpAddr::V4(ip)=>{b[..4].copy_from_slice(&ip.octets());Ok((4,b))},IpAddr::V6(ip)=>Ok((6,ip.octets()))}}
pub fn encode_segments(epoch:u64,records:&[Segment])->Result<Vec<u8>> {
    if records.len()>1024{return Err("IPC record count exceeds limit".into());}
    let mut b=epoch.to_le_bytes().to_vec();b.extend_from_slice(&(records.len() as u32).to_le_bytes());
    for s in records {
        let (family,src)=ip_bytes(&s.flow.src)?;let (other,dst)=ip_bytes(&s.flow.dst)?;if family!=other{return Err("Mixed IPC IP families".into());}
        b.extend_from_slice(&s.timestamp_ns.to_le_bytes());b.push(family);b.extend_from_slice(&src);b.extend_from_slice(&dst);
        b.extend_from_slice(&s.flow.sport.to_le_bytes());b.extend_from_slice(&s.flow.dport.to_le_bytes());b.extend_from_slice(&s.seq.to_le_bytes());
        b.push(u8::from(s.syn)|u8::from(s.fin)<<1|u8::from(s.rst)<<2|u8::from(s.ack)<<3);
        b.extend_from_slice(&(s.payload.len() as u32).to_le_bytes());b.extend_from_slice(&s.payload);
        if b.len()+10>MAX{return Err("IPC batch exceeds limit".into());}
    }Ok(b)
}
pub fn decode_segments(b:&[u8])->Result<(u64,Vec<Segment>)> {
    if b.len()<12{return Err("Short IPC batch".into());}let epoch=u64::from_le_bytes(b[..8].try_into().unwrap());let count=u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
    if count>1024{return Err("IPC record count exceeds limit".into());}let mut offset=12;let mut result=Vec::new();
    for _ in 0..count {
        if b.len()-offset<54{return Err("Short IPC segment".into());}let h=&b[offset..offset+54];let family=h[8];
        let ip=|p:&[u8]|->Result<String>{match family{4 if p[4..].iter().all(|v|*v==0)=>Ok(Ipv4Addr::from(<[u8;4]>::try_from(&p[..4]).unwrap()).to_string()),6=>Ok(Ipv6Addr::from(<[u8;16]>::try_from(p).unwrap()).to_string()),_=>Err("Invalid IPC address family".into())}};
        let sport=u16::from_le_bytes(h[41..43].try_into().unwrap());let dport=u16::from_le_bytes(h[43..45].try_into().unwrap());
        let n=u32::from_le_bytes(h[50..54].try_into().unwrap()) as usize;offset+=54;
        if n>9000 || n>b.len()-offset || sport==0 || dport==0 || h[49]&!15!=0{return Err("Invalid IPC segment fields".into());}
        result.push(Segment {timestamp_ns:u64::from_le_bytes(h[..8].try_into().unwrap()),flow:Flow {src:ip(&h[9..25])?,sport,dst:ip(&h[25..41])?,dport},seq:u32::from_le_bytes(h[45..49].try_into().unwrap()),
            payload:b[offset..offset+n].to_vec(),syn:h[49]&1!=0,fin:h[49]&2!=0,rst:h[49]&4!=0,ack:h[49]&8!=0});offset+=n;
    }if offset!=b.len(){return Err("Unexpected IPC batch tail".into());}Ok((epoch,result))
}
type Connection=[(String,u16);2];
pub struct OwnershipBuffer {
    pending:VecDeque<(u64,Connection,u64,Segment)>,bytes:usize,
    approved:BTreeMap<Connection,(u64,u64)>,generations:BTreeMap<Connection,u64>,syns:BTreeMap<Connection,(Flow,u32)>,
    pub lost:u64,pub unowned:u64,
}
impl Default for OwnershipBuffer {fn default()->Self{Self {pending:VecDeque::new(),bytes:0,approved:BTreeMap::new(),generations:BTreeMap::new(),syns:BTreeMap::new(),lost:0,unowned:0}}}
impl OwnershipBuffer {
    pub fn clear(&mut self){self.pending.clear();self.bytes=0;self.approved.clear();self.generations.clear();self.syns.clear();}
    pub fn approve(&mut self,flows:&[Flow],now:u64) {
        for f in flows {let key=f.connection();self.approved.insert(key.clone(),(self.generations.get(&key).copied().unwrap_or(0),now+2250));}
        self.approved.retain(|_,(_,expiry)|*expiry>=now);
    }
    pub fn push(&mut self,s:Segment,now:u64) {
        let key=s.flow.connection();
        if s.syn && !s.ack && self.syns.get(&key)!=Some(&(s.flow.clone(),s.seq)) {
            if !self.generations.contains_key(&key) && self.generations.len()>=65536 {self.lost+=1;return;}
            self.syns.insert(key.clone(),(s.flow.clone(),s.seq));*self.generations.entry(key.clone()).or_default()+=1;self.approved.remove(&key);
        }
        let cost=s.payload.len()+256;if cost>64*1024*1024{self.lost+=1;return;}
        while !self.pending.is_empty() && (self.pending.len()>=32768 || self.bytes+cost>64*1024*1024) {let old=self.pending.pop_front().unwrap();self.bytes-=old.3.payload.len()+256;self.lost+=1;}
        let generation=self.generations.get(&key).copied().unwrap_or(0);self.pending.push_back((now,key,generation,s));self.bytes+=cost;
    }
    pub fn drain(&mut self,now:u64,force:bool)->Vec<Segment> {
        let mut records=Vec::new();while self.pending.front().is_some_and(|p|force||now.saturating_sub(p.0)>=1250) {
            let (_,key,generation,s)=self.pending.pop_front().unwrap();self.bytes-=s.payload.len()+256;
            if self.approved.get(&key).is_some_and(|(g,expiry)|*g==generation && *expiry>=now){records.push(s);}else{self.unowned+=1;}
        }records
    }
}
#[cfg(test)] mod tests {
    use super::*;
    fn segment()->Segment{Segment{timestamp_ns:20,flow:Flow{src:"192.0.2.1".into(),sport:1234,dst:"192.0.2.2".into(),dport:5678},seq:1,payload:vec![1,2,3],syn:false,fin:false,rst:false,ack:true}}
    #[test] fn binary_roundtrip_and_reject_truncation_sequence_tail(){let s=segment();let b=encode_segments(42,&[s.clone()]).unwrap();let (epoch,decoded)=decode_segments(&b).unwrap();assert_eq!(epoch,42);assert_eq!(decoded[0].payload,s.payload);assert_eq!(decoded[0].flow,s.flow);for n in 0..b.len(){assert!(decode_segments(&b[..n]).is_err());}let mut tail=b;tail.push(0);assert!(decode_segments(&tail).is_err());let e=envelope(1,3,&[]).unwrap();let mut sequence=1;assert!(header(&e,&mut sequence).is_ok());assert!(header(&e,&mut sequence).is_err());}
    #[test] fn ownership_prelude_and_syn_reuse(){let mut b=OwnershipBuffer::default();let s=segment();b.push(s.clone(),0);b.approve(&[s.flow.clone()],500);assert_eq!(b.drain(1250,false).len(),1);let mut syn=s.clone();syn.syn=true;syn.ack=false;b.push(syn,1300);assert!(b.drain(2550,false).is_empty());b.push(s.clone(),2600);b.approve(&[s.flow.clone()],2700);assert_eq!(b.drain(3850,false).len(),1);}
}
