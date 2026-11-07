use mabi_meter_core::{hp_wire::{self,Error},capture_file::Input,protocol,replay::Replay,transport::{Flow,Segment}};

fn prefix(actor:u64,string:&[u8],dictionaries:[u32;3],mode:u8,hp:&[u8],iv:&[u8])->Vec<u8>{
    let mut p=actor.to_le_bytes().to_vec();p.extend([0;9]);p.extend((string.len() as u32).to_le_bytes());p.extend(string);
    p.extend([0;46+17*8+3*8+172+3*8+4*4]);p.push(231);p.extend([0;6*8+24]);
    for n in dictionaries{p.extend(n.to_le_bytes());p.resize(p.len()+n as usize*8,0);}
    p.extend([0;36]);p.extend((-3i32).to_le_bytes());p.extend([0;8]);p.extend(actor.to_le_bytes());p.push(mode);
    p.extend((hp.len() as u32).to_le_bytes());p.extend(hp);p.extend((iv.len() as u32).to_le_bytes());p.extend(iv);
    p.extend(0x7fc00001u32.to_le_bytes());p.push(219);p
}
fn update(actor:u64,hp:u64)->Vec<u8>{let mut p=actor.to_le_bytes().to_vec();p.extend(8u32.to_le_bytes());p.extend(hp.to_le_bytes());p.push(173);p}
fn segment(seq:&mut u32,wire:i32,p:&[u8],ns:u64)->Input{
    let payload=protocol::encode_frame(wire,p);let start=*seq;*seq+=payload.len() as u32;
    Input::Segment(Segment{flow:Flow{src:"server".into(),sport:16000,dst:"client".into(),dport:3},timestamp_ns:ns,seq:start,payload,syn:false,fin:false,rst:false,ack:true})
}
#[test]
fn structural_prefix_uses_declared_string_and_dictionaries_without_suffix_or_raw_value_gates(){
    let p=prefix(9,b"abcdef",[2,1,3],0,&111u64.to_le_bytes(),&[7;16]);
    let s=hp_wire::status(20054,&p).unwrap();
    assert_eq!((s.actor,s.offset,s.iv_offset,s.end),(9,626,651,672));
    assert_eq!((s.mode,s.parameter_marker,s.level,s.float_bits,s.following),(0,231,-3,0x7fc00001,219));
    assert_eq!(s.hp,111u64.to_le_bytes());assert_eq!(s.iv,[7;16]);
    let wrapped=[42u64.to_le_bytes().as_slice(),p.as_slice()].concat();
    assert_eq!(hp_wire::status(20517,&wrapped).unwrap().offset,634);
    for end in 0..p.len(){assert!(hp_wire::status(20054,&p[..end]).is_err(),"truncation {end}");}
    let mut negative=p.clone();negative[17..21].copy_from_slice(&(-1i32).to_le_bytes());
    assert_eq!(hp_wire::status(20054,&negative).err(),Some(Error::Bounds));
    let mut huge=p.clone();huge[17..21].copy_from_slice(&4097u32.to_le_bytes());
    assert_eq!(hp_wire::status(20054,&huge).err(),Some(Error::Limit));
    let mut mismatch=p.clone();mismatch[626]=10;
    assert_eq!(hp_wire::status(20054,&mismatch).err(),Some(Error::ActorMismatch));
}
#[test]
fn candidates_precede_opcode_entity_and_numeric_filters_and_decoys_do_not_become_hp(){
    let mut r=Replay::new(true);let mut seq=1;
    r.feed(segment(&mut seq,170318,&update(9,70),1)).unwrap(); // scalar-shaped string role
    r.feed(segment(&mut seq,170205,&update(u64::MAX,70),2)).unwrap();
    r.feed(segment(&mut seq,170205,&update(9,u64::MAX),3)).unwrap();
    let mut extended=update(9,70);extended.extend([1,2,3]);
    r.feed(segment(&mut seq,170206,&extended,4)).unwrap();
    assert_eq!(r.hp_evidence.counts["exact scalar candidate"],3);
    assert_eq!(r.hp_evidence.counts["extended scalar candidate"],1);
    let c=&r.hp_evidence.candidates;
    assert_eq!(c[0].outcome,"unconfirmed packet role");
    assert!(c[1].outcome.contains("numeric"));assert!(c[2].outcome.contains("OutOfRange"));
    assert!(c[3].outcome.contains("unsupported HP envelope"));
    assert_eq!(r.hp_evidence.observations.len(),1);assert_eq!(r.health.loss,0);assert_eq!(r.numeric.detected,0);
    assert!(r.meter.registry.records.is_empty());
}
#[test]
fn max_sentinel_preserves_finite_endpoint_damage_without_sentinel_arithmetic(){
    let mut r=Replay::new(true);let mut seq=1;
    for (i,hp) in [100,i64::MAX as u64,0,0].into_iter().enumerate(){r.feed(segment(&mut seq,170205,&update(9,hp),i as u64+1)).unwrap();}
    r.finish();assert_eq!(r.health.loss,100);assert_eq!(r.health.unassigned,100);assert_eq!(r.health.attribution.owed_amount(),0);
    assert_eq!(r.hp_evidence.comparisons["MAX sentinel"],1);
    assert_eq!(r.hp_evidence.comparisons["initial observation"],2);
    assert_eq!(r.hp_evidence.comparisons["unchanged"],1);
    assert!(r.hp_evidence.observations[2].previous.is_none());
}
#[test]
fn opaque_body_retains_audit_comparison_and_numeric_obligation(){
    let mut r=Replay::new(true);let mut seq=1;
    r.feed(segment(&mut seq,170205,&update(9,100),1)).unwrap();
    let mut opaque=protocol::encode_frame(21231,&[255;3]);opaque[8]=1;
    let start=seq;seq+=opaque.len() as u32;
    r.feed(Input::Segment(Segment{flow:Flow{src:"server".into(),sport:16000,dst:"client".into(),dport:3},timestamp_ns:2,seq:start,payload:opaque,syn:false,fin:false,rst:false,ack:true})).unwrap();
    r.feed(segment(&mut seq,170205,&update(9,50),3)).unwrap();r.finish();
    assert_eq!(r.hp_evidence.counts["opaque body"],1);
    let last=r.hp_evidence.observations.back().unwrap();assert_eq!(last.comparison,"gap comparison; canonical numeric decides credit");
    assert_eq!(last.previous.unwrap().1-last.hp,50);assert_eq!(r.health.loss,50);assert_eq!(r.health.unassigned,50);assert_eq!(r.health.attribution.owed_amount(),0);
}
#[test]
fn bounded_candidate_history_counts_evictions_without_losing_acknowledgment_totals(){
    let mut r=Replay::new(true);let mut seq=1;
    for i in 1..=8200{r.feed(segment(&mut seq,170318,&update(u64::MAX,70),i)).unwrap();}
    assert_eq!(r.hp_evidence.counts["exact scalar candidate"],8200);
    assert_eq!(r.hp_evidence.candidates.len(),8192);assert_eq!(r.hp_evidence.candidate_evictions,8);
    assert!(r.meter.registry.records.is_empty());
}

#[test]
fn decoder_error_is_sticky_while_recorded_input_can_continue_without_reusing_ack_ids(){
    let mut session=mabi_meter_core::session::Session::new(Replay::new(true),None);let mut seq=1;
    session.feed_recorded(segment(&mut seq,170205,&update(9,100),5));
    session.feed_recorded(segment(&mut seq,170205,&update(9,50),4)); // Invalid capture order.
    assert!(session.decoding_error.is_some());
    let frames=session.replay.frames;
    for ns in 6..10{session.feed_recorded(segment(&mut seq,170205,&update(9,0),ns));session.tick_recorded(ns);}
    assert_eq!(session.replay.frames,frames);assert_eq!(session.replay.diagnostics.0["semantic_decoder_paused"],1);
    assert!(session.replay.feed(segment(&mut seq,170205,&update(9,0),20)).is_err());
    assert_eq!(session.replay.frames,frames);
}

#[test]
fn unresolved_identity_overflow_is_quarantined_and_retired_without_fatal_decoder_error(){
    let mut ledger=mabi_meter_core::hp_evidence::Ledger::default();
    for actor in 1..=4096{ledger.block(2,actor);}
    ledger.block(4,5000);assert!(ledger.blocked(4,9999));
    assert_eq!(ledger.outcomes["unresolved binding capacity; context quarantined"],1);
    ledger.retire_context(2);ledger.retire_context(4);
    assert!(!ledger.blocked(4,9999));ledger.block(6,5000);assert!(ledger.blocked(6,5000));
}
