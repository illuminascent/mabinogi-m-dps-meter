//! Plain Hits display is independent of the authoritative damage ledger.
use mabi_meter_core::{capture_file::Input,protocol,replay::Replay,identity::EntityKey,transport::{Flow,Segment}};
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

fn hits(r:&Replay,actor:u32)->u64{r.meter.snapshot().rows.iter().find(|row|row.key.actor==actor).unwrap().display_hits}
#[test]fn one_inferred_attack_counts_without_inventing_explicit_hits_or_crit(){
 let mut x=Run::new();x.hp(100,0);x.hp(90,1);x.attack(101,1);x.finish();assert_eq!(hits(&x.r,101),1);let s=x.r.meter.snapshot();assert_eq!(s.display_hits,1);assert_eq!(s.summary.hits,0);assert_eq!(s.rows[0].explicit_hits,0);assert_eq!(s.rows[0].critical_rate,0.0);assert_eq!(s.summary.damage,10);
}
#[test]fn three_notifications_can_share_one_damage_allocation_and_count_three(){
 let mut x=Run::new();x.hp(100,0);x.hp(90,1);for _ in 0..3{x.attack(101,1);}x.finish();assert_eq!(x.r.meter.session.estimated_total.allocations,1);assert_eq!(hits(&x.r,101),3);assert_eq!(x.r.meter.session.estimated_total.damage,10);
}
#[test]fn explicit_plus_inferred_overlap_counts_one_hit(){
 let mut x=Run::new();x.hp(100,0);x.hp(90,1);x.raw(101,4,1);x.attack(101,1);x.finish();assert_eq!(hits(&x.r,101),1);assert_eq!(x.r.meter.session.total.hits,1);assert_eq!(x.r.meter.session.total.damage,4);assert_eq!(x.r.meter.session.estimated_total.damage,6);
}
#[test]fn partial_coverage_does_not_hide_later_implicit_hits(){
 let mut x=Run::new();x.raw(101,4,0);x.hp(100,1_000_000_000);x.hp(90,1_000_000_001);x.attack(101,1_000_000_001);x.finish();assert_eq!(hits(&x.r,101),2);assert_eq!(x.r.meter.session.total.damage,4);assert_eq!(x.r.meter.session.estimated_total.damage,10);
}
#[test]fn missing_current_notification_uses_one_hit_per_hp_obligation(){
 let mut x=Run::new();x.hp(100,0);x.hp(90,1);x.attack(101,1);x.r.tick(500_000_000).unwrap();x.hp(80,1_000_000_000);x.hp(70,2_000_000_000);x.finish();assert_eq!(hits(&x.r,101),3);assert_eq!(x.r.meter.session.estimated_total.damage,30);
}
#[test]fn mixed_sources_preserve_damage_and_count_their_own_observations(){
 let mut x=Run::new();x.hp(100,0);x.hp(80,1);x.raw(101,4,1);x.attack(101,1);for _ in 0..3{x.attack(202,1);}x.finish();assert_eq!(hits(&x.r,101),1);assert_eq!(hits(&x.r,202),3);assert_eq!(x.r.meter.snapshot().summary.damage,20);
}
#[test]fn target_split_and_archive_views_keep_their_counts(){
 use mabi_meter_core::meter::{Action,Command};
 let mut x=Run::new();x.hp(100,0);x.hp(90,1);x.attack(101,1);x.r.commit_pending().unwrap();assert_eq!(hits(&x.r,101),1);
 let m=&mut x.r.meter;let ack=m.command(Command{id:1,observed_revision:m.revision,encounter:m.current,action:Action::Target(Some(key(999)))});assert!(ack.applied);assert_eq!(ack.snapshot.display_hits,1);
 let m=&mut x.r.meter;assert!(m.command(Command{id:2,observed_revision:m.revision,encounter:m.current,action:Action::Reset}).applied);assert_eq!(m.snapshot().display_hits,0);
 let m=&mut x.r.meter;let ack=m.command(Command{id:3,observed_revision:m.revision,encounter:m.current,action:Action::Encounter(1)});assert!(ack.applied);assert_eq!(ack.snapshot.display_hits,1);
}
#[test]fn pending_late_join_carries_count_metadata_without_republishing_damage(){
 use mabi_meter_core::{meter::{Meter,Hit},protocol::Damage};let mut m=Meter::default();m.automatic_encounters=true;let me=m.registry.observe(0,1,101).unwrap();let peer=m.registry.observe(0,1,202).unwrap();let target=m.registry.observe(0,1,999).unwrap();m.registry.assign(Some(me)).unwrap();
 let hit=|source,timestamp_ns,amount|Hit{timestamp_ns,source,target,damage:Damage{actor:source.actor,target:999,amount,flags:[false;5],raw_flags:[0;7]}};
 m.add_estimate(hit(peer,1,10)).unwrap();let tag=m.last_hit_assignment(true).unwrap();m.set_display_hits(peer,target,&tag,3);assert_eq!(m.snapshot().display_hits,0);assert_eq!(tag.encounter(),None);
 m.add(hit(me,2,5)).unwrap();assert_eq!(tag.encounter(),Some(m.current));assert_eq!(m.snapshot().display_hits,4);assert_eq!(m.session.estimated_total.damage,10);assert_eq!(m.snapshot().summary.damage,15);
}
#[test]fn repeated_credit_commit_does_not_increment_hit_count(){
 use mabi_meter_core::{meter::{Meter,Hit,CreditCommit},protocol::Damage};let mut m=Meter::default();let source=m.registry.observe(0,1,101).unwrap();let target=m.registry.observe(0,1,999).unwrap();let h=Hit{timestamp_ns:1,source,target,damage:Damage{actor:101,target:999,amount:10,flags:[false;5],raw_flags:[0;7]}};
 assert_eq!(m.commit_credit(1,h.clone(),true,None).unwrap(),CreditCommit::Committed);let tag=m.last_hit_assignment(true).unwrap();m.set_display_hits(source,target,&tag,3);assert_eq!(m.commit_credit(1,h,true,None).unwrap(),CreditCommit::AlreadyCommitted);assert_eq!(m.snapshot().display_hits,3);assert_eq!(m.snapshot().summary.damage,10);
}
