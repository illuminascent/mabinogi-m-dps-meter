//! Explicit-source coverage must never gain a second HP-derived credit.
use mabi_meter_core::{capture_file::Input,protocol,replay::Replay,identity::{EntityKey,Appearance},transport::{Flow,Segment}};
fn key(actor:u32)->EntityKey{EntityKey{process:0,binding:1,actor,generation:1}}
struct Run{r:Replay,seq:u32}
impl Run{
    fn new()->Self{let mut r=Replay::new(true);r.live_scope=true;Self{r,seq:1}}
    fn feed(&mut self,wire:i32,p:&[u8],t:u64){let payload=protocol::encode_frame(wire,p);let n=payload.len()as u32;self.r.feed(Input::Segment(Segment{timestamp_ns:t,flow:Flow{src:"192.0.2.1".into(),dst:"192.0.2.2".into(),sport:16000,dport:1234},seq:self.seq,payload,syn:false,fin:false,rst:false,ack:true})).unwrap();self.seq+=n;}
    fn hp(&mut self,n:u64,t:u64){let mut p=[0;21];p[..8].copy_from_slice(&999u64.to_le_bytes());p[8..12].copy_from_slice(&8u32.to_le_bytes());p[12..20].copy_from_slice(&n.to_le_bytes());self.feed(170205,&p,t);}
    fn attack(&mut self,source:u64,t:u64){let mut p=[0;35];p[..8].copy_from_slice(&source.to_le_bytes());p[8..16].copy_from_slice(&999u64.to_le_bytes());p[16..20].copy_from_slice(&77u32.to_le_bytes());self.feed(20526,&p,t);}
    fn raw(&mut self,source:u64,n:u64,t:u64){let mut p=[0;53];p[..8].copy_from_slice(&source.to_le_bytes());p[8..16].copy_from_slice(&999u64.to_le_bytes());p[16..24].copy_from_slice(&n.to_le_bytes());self.feed(21196,&p,t);}
    fn finish(&mut self){self.r.finish();assert!(self.r.semantic_error.is_none());let a=self.r.health.attribution.audit();assert_eq!(a["invariant"],true);assert_eq!(a["by_origin"]["explicit"]["invariant"],true);assert_eq!(a["by_origin"]["inferred"]["invariant"],true);assert_eq!(self.r.numeric.detected,self.r.health.explicit+self.r.health.allocated+self.r.health.unassigned);}
}
#[test]fn partial_and_zero_explicit_leave_hp_residual_unattributed(){
    for amount in [0,4]{let mut x=Run::new();x.hp(100,0);x.hp(90,1);x.attack(101,1);x.raw(101,amount,1);x.finish();assert_eq!(x.r.meter.session.total.damage,amount as u128);assert_eq!(x.r.meter.session.estimated_total.damage,0);assert_eq!(x.r.health.unassigned,10-amount as u128);assert_eq!(x.r.meter.registry.label(key(101)),"<YOU>");assert_eq!(x.r.meter.registry.self_key,None);}
}
#[test]fn mixed_support_allocates_only_to_peer(){
    let mut x=Run::new();x.hp(100,0);x.hp(80,1);x.attack(101,1);x.attack(202,1);x.raw(101,4,1);x.finish();assert_eq!(x.r.meter.session.estimates.len(),1);assert_eq!(x.r.meter.session.estimates[&key(202)].damage,16);assert_eq!(x.r.health.unassigned,0);
}
#[test]fn explicit_coverage_applies_to_later_targets_and_survives_role_cache_pressure(){
    let mut x=Run::new();x.raw(101,4,0);for actor in 1000..1700{x.r.meter.registry.observe(0,1,actor).unwrap();}x.hp(100,1_000_000_000);x.attack(101,1_000_000_001);x.hp(90,1_000_000_002);x.finish();assert_eq!(x.r.meter.session.estimated_total.damage,0);assert_eq!(x.r.health.unassigned,10);
}
#[test]fn identified_main_character_is_excluded_before_first_explicit(){
    let mut x=Run::new();let me=x.r.meter.registry.observe(0,1,101).unwrap();x.r.meter.registry.assign(Some(me)).unwrap();x.hp(100,0);x.attack(101,1);x.hp(90,1);x.finish();assert_eq!(x.r.meter.session.estimated_total.damage,0);assert_eq!(x.r.health.unassigned,10);assert_eq!(x.r.meter.registry.label(me),"<YOU>");
}
#[test]fn names_hide_ids_and_pet_coverage_is_separate_from_owner(){
    let mut x=Run::new();x.r.meter.registry.appearance(0,1,Appearance{actor:101,name:"Alice".into(),uid:Some(123),template:None,offset:0,record_reference:None}).unwrap();x.raw(101,4,0);assert_eq!(x.r.meter.registry.label(key(101)),"Alice");let pending=x.r.meter.registry.observe(0,1,202).unwrap();assert_eq!(x.r.meter.registry.label(pending),"Name pending");
    x.r.meter.registry.owned_spawn(0,1,mabi_meter_core::identity::ownership::Spawn{actor:303,owner:101,template:9,outer:None,kind:4},mabi_meter_core::identity::tracking::Evidence{frame:1,timestamp_ns:0,origin:mabi_meter_core::identity::tracking::EvidenceOrigin::Semantic}).unwrap();x.raw(303,2,0);x.hp(100,1);x.attack(303,2);x.hp(90,2);x.finish();assert_eq!(x.r.meter.session.estimated_total.damage,0);assert_eq!(x.r.health.unassigned,10);
}
#[test]fn sentinel_does_not_establish_explicit_coverage(){
    let mut x=Run::new();x.raw(101,u64::MAX,0);x.hp(100,1);x.attack(101,2);x.hp(90,2);x.finish();assert_eq!(x.r.meter.session.estimated_total.damage,10);assert!(!x.r.meter.registry.inference_excluded(key(101)));
}
#[test]fn queued_inferred_credit_is_rejected_after_direct_evidence_arrives(){
    let mut x=Run::new();let source=x.r.meter.registry.observe(0,1,101).unwrap();let target=x.r.meter.registry.observe(0,1,999).unwrap();let at=mabi_meter_core::combat::Stamp{process:0,context:2,continuity:0,frame:1,order:1,timestamp_ns:1,tcp_byte:0};
    x.r.health.attribution.resolve_budget(target,at,1,10,&[mabi_meter_core::attribution::Candidate{source,skill:None,weight:1.0,frames:vec![1]}],"test");let leased=x.r.health.front_credit().unwrap();x.r.meter.registry.note_explicit_source(source);x.r.retry_pending_credits().unwrap();assert_eq!(x.r.meter.session.estimated_total.damage,0);assert_eq!(x.r.health.attribution.unattributed_inferred,10);assert_eq!(x.r.health.attribution.queued_amount(),0);assert!(leased.id>0);assert_eq!(x.r.health.attribution.audit()["invariant"],true);
}
