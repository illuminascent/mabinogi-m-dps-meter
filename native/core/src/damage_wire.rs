//! Observed JP21196 fields before legacy semantic gates. Field meaning and
//! HP-effective arithmetic is separate from raw explicit meter amounts.
use crate::{combat::Stamp, protocol::u64le, Result};
use serde_json::{json, Value};
use std::collections::VecDeque;

pub const PREFIX_BYTES:usize=53;
pub const EVIDENCE_CAPACITY:usize=1024;
/// Existing adapter sanity bound; not a newly inferred field property.
pub const LEGACY_MAX_AMOUNT:u64=1_000_000_000_000;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Fields {
    pub source_prefix:u64,pub target_prefix:u64,pub raw_amount:u64,pub auxiliary_24:u64,
    pub raw_flags:[u8;7],pub raw_tail:[u8;14],pub body_length:usize,
}
impl Fields {
    pub fn parse(body:&[u8])->Result<Self>{
        if body.len()<PREFIX_BYTES{return Err("short damage payload".into());}
        Ok(Self{source_prefix:u64le(body,0),target_prefix:u64le(body,8),raw_amount:u64le(body,16),auxiliary_24:u64le(body,24),raw_flags:body[32..39].try_into().unwrap(),raw_tail:body[39..53].try_into().unwrap(),body_length:body.len()})
    }
    /// Every known-prefix byte is retained, including meter-ignored fields.
    /// Extended bodies remain in raw recording/frame evidence.
    pub fn prefix_bytes(self)->[u8;PREFIX_BYTES]{
        let mut body=[0;PREFIX_BYTES];for (i,v) in [self.source_prefix,self.target_prefix,self.raw_amount,self.auxiliary_24].into_iter().enumerate(){body[i*8..i*8+8].copy_from_slice(&v.to_le_bytes());}body[32..39].copy_from_slice(&self.raw_flags);body[39..].copy_from_slice(&self.raw_tail);body
    }
    /// Classification AFTER parsing; this does not gate raw observation.
    pub fn legacy_supported(self)->bool{let source=self.source_prefix as u32;let target=self.target_prefix as u32;source!=0 && target!=0 && source!=target && self.raw_amount<=LEGACY_MAX_AMOUNT}
    /// Scoped observed difference hypothesis. None never clamps/substitutes raw;
    /// Some(0) predicts zero but does not assert an unchanged HP was published.
    pub fn hp_impact_candidate(self)->Option<u64>{if self.body_length!=PREFIX_BYTES || self.raw_amount==u64::MAX{return None;}self.raw_amount.checked_sub(self.auxiliary_24)}
}
#[derive(Clone,Copy,Debug)]
pub struct Observation{pub at:Stamp,pub available_ns:u64,pub fields:Fields}
#[derive(Default)]
pub struct Counts{pub seen:u64,pub opaque:u64,pub short:u64,pub parsed:u64,pub outside_legacy_gate:u64,pub extended:u64,pub zero_aux:u64,pub partial_aux:u64,pub equal_aux:u64,pub exceeds_raw:u64,pub out_of_range:u64}
pub struct Survey{pub counts:Counts,recent:VecDeque<Observation>,pub evicted:u64}
impl Default for Survey{fn default()->Self{Self{counts:Counts::default(),recent:VecDeque::with_capacity(EVIDENCE_CAPACITY),evicted:0}}}
impl Survey{
    /// Extracted-batch acknowledgement precedes semantic handlers. Same path for
    /// every actor; no self, teammate, identity or roster admission criterion.
    pub fn observe(&mut self,at:Stamp,available_ns:u64,body:&[u8],decoded:bool){
        self.counts.seen+=1;if !decoded{self.counts.opaque+=1;return;}let Ok(fields)=Fields::parse(body)else{self.counts.short+=1;return;};self.counts.parsed+=1;
        if !fields.legacy_supported(){self.counts.outside_legacy_gate+=1;}if fields.body_length>PREFIX_BYTES{self.counts.extended+=1;}if fields.raw_amount>LEGACY_MAX_AMOUNT{self.counts.out_of_range+=1;}
        if fields.auxiliary_24==0{self.counts.zero_aux+=1;}else if fields.auxiliary_24<fields.raw_amount{self.counts.partial_aux+=1;}else if fields.auxiliary_24==fields.raw_amount{self.counts.equal_aux+=1;}else{self.counts.exceeds_raw+=1;}
        if self.recent.len()==EVIDENCE_CAPACITY{self.recent.pop_front();self.evicted+=1;}self.recent.push_back(Observation{at,available_ns,fields});
    }
    pub fn retained(&self)->usize{self.recent.len()}
    pub fn observations(&self)->impl Iterator<Item=&Observation>{self.recent.iter()}
    pub fn owned_bytes(&self)->usize{std::mem::size_of::<Self>()+self.recent.capacity()*std::mem::size_of::<Observation>()}
    pub fn summary(&self)->Value{
        let c=&self.counts;json!({"format":"jp21196-raw-fields-v1","scope":"known wire prefix before legacy gates; not all-wire completeness",
            "candidate":"checked raw@16 minus auxiliary@24; scoped HP reconciliation; original wire amount retained; explicit credit may remove proven terminal overkill; game mechanism unresolved",
            "seen":c.seen,"opaque":c.opaque,"short":c.short,"parsed":c.parsed,"outside_legacy_gate":c.outside_legacy_gate,"extended":c.extended,
            "zero_aux":c.zero_aux,"partial_aux":c.partial_aux,"equal_aux":c.equal_aux,"exceeds_raw":c.exceeds_raw,"out_of_range":c.out_of_range,
            "retained":self.recent.len(),"evicted":self.evicted,"record_capacity":EVIDENCE_CAPACITY,"owned_bytes":self.owned_bytes()})
    }
    pub fn audit(&self)->Value{let mut value=self.summary();value["records"]=json!(self.recent.iter().map(|o|{let f=o.fields;json!({"frame":o.at.frame,"context":o.at.context,"process":o.at.process,"continuity":o.at.continuity,"order":o.at.order,"capture_ns":o.at.timestamp_ns,"available_ns":o.available_ns,"framer_byte":o.at.tcp_byte,
                "source_prefix":f.source_prefix,"target_prefix":f.target_prefix,"raw_amount":f.raw_amount,"auxiliary_24":f.auxiliary_24,"raw_flags":f.raw_flags,"raw_tail":f.raw_tail,"body_length":f.body_length,"legacy_supported":f.legacy_supported(),"hp_impact_candidate":f.hp_impact_candidate()})}).collect::<Vec<_>>());value
    }
}
