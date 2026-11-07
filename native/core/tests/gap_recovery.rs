use mabi_meter_core::{capture_file::Input,protocol,replay::Replay,transport::{Flow,Segment,TcpStream},Diagnostics};
fn segment(seq:u32,payload:Vec<u8>,time:u64)->Segment{Segment{seq,payload,timestamp_ns:time,
    flow:Flow{src:"192.0.2.1".into(),dst:"192.0.2.2".into(),sport:16000,dport:1234},syn:false,fin:false,rst:false,ack:true}}
fn amount(n:u64)->Vec<u8>{let mut p=vec![0;53];p[..4].copy_from_slice(&1u32.to_le_bytes());p[8..12].copy_from_slice(&2u32.to_le_bytes());p[16..24].copy_from_slice(&n.to_le_bytes());protocol::encode_frame(21196,&p)}
fn anchored(n:u64)->Vec<u8>{let mut p=protocol::encode_frame(20132,&[]);p.extend(amount(n));p}

#[test] fn timeout_recovers_retained_suffix_with_original_time_and_byte_position(){
    let mut s=TcpStream::new(true);let mut d=Diagnostics::default();
    s.feed(&segment(1000,vec![],0),&mut d).unwrap();
    assert!(s.feed(&segment(1010,anchored(37),1),&mut d).unwrap().is_empty());
    let fs=s.feed(&segment(1100,vec![],3_000_000_002),&mut d).unwrap();
    assert_eq!(fs.len(),2);assert_eq!(fs[0].stream_byte,10);assert!(fs[0].gap_before);assert!(!fs[1].gap_before);
    assert!(fs.iter().all(|f|f.timestamp_ns==Some(1)));assert_eq!(protocol::damage(&fs[1].payload).unwrap().amount,37);
    assert_eq!(d.0["tcp_gap_timeout"],1);assert_eq!(s.buffered(),0);
}
#[test] fn multiple_holes_have_separate_waits_and_never_join_partial_frames(){
    let mut s=TcpStream::new(true);let mut d=Diagnostics::default();
    s.feed(&segment(1000,amount(99)[..15].to_vec(),0),&mut d).unwrap();
    s.feed(&segment(1050,anchored(7),1),&mut d).unwrap();
    s.feed(&segment(1200,anchored(11),2_000_000_000),&mut d).unwrap();
    let first=s.feed(&segment(1300,vec![],3_000_000_002),&mut d).unwrap();
    assert_eq!(first.len(),2);assert_eq!(first[0].stream_byte,50);assert_eq!(protocol::damage(&first[1].payload).unwrap().amount,7);
    let second=s.feed(&segment(1300,vec![],5_000_000_001),&mut d).unwrap();
    assert_eq!(second.len(),2);assert_eq!(second[0].stream_byte,200);assert!(second[0].gap_before);
    assert_eq!(second[1].timestamp_ns,Some(2_000_000_000));assert_eq!(protocol::damage(&second[1].payload).unwrap().amount,11);
    assert_eq!(d.0["tcp_gap_timeout"],2);
}
#[test] fn recovery_preserves_first_arrival_overlap_across_sequence_wrap(){
    let mut s=TcpStream::new(true);let mut d=Diagnostics::default();let base=u32::MAX-20;
    s.feed(&segment(base,vec![],0),&mut d).unwrap();let p=anchored(73);let seq=base.wrapping_add(40);
    s.feed(&segment(seq,p[..35].to_vec(),1),&mut d).unwrap();
    let mut overlapping=p[20..].to_vec();overlapping[..15].fill(255);
    s.feed(&segment(seq.wrapping_add(20),overlapping,2),&mut d).unwrap();
    let fs=s.feed(&segment(seq,vec![],3_000_000_003),&mut d).unwrap();
    assert_eq!(fs.len(),2);assert_eq!(fs[0].stream_byte,40);assert_eq!(protocol::damage(&fs[1].payload).unwrap().amount,73);
    assert_eq!(fs[1].timestamp_ns,Some(2));
}
#[test] fn recovery_requires_an_anchor_and_a_late_fill_preserves_observation_time(){
    let mut s=TcpStream::new(true);let mut d=Diagnostics::default();
    s.feed(&segment(1000,vec![],0),&mut d).unwrap();let mut p=amount(999);p.extend(anchored(13));
    s.feed(&segment(1010,p,1),&mut d).unwrap();
    let fs=s.feed(&segment(1100,vec![],3_000_000_002),&mut d).unwrap();
    assert_eq!(fs.len(),2);assert_eq!(protocol::damage(&fs[1].payload).unwrap().amount,13);
    let mut s=TcpStream::new(true);let p=amount(17);
    s.feed(&segment(1000,p[..10].to_vec(),0),&mut d).unwrap();
    s.feed(&segment(1030,p[30..].to_vec(),1),&mut d).unwrap();
    let fs=s.feed(&segment(1010,p[10..30].to_vec(),2),&mut d).unwrap();
    assert_eq!(fs[0].timestamp_ns,Some(2));assert!(!fs[0].gap_before);
}
#[test] fn recovered_hole_preserves_damage_without_claiming_exact_packet_matching(){
    let mut r=Replay::new(true);let mut hp=vec![0;21];hp[..8].copy_from_slice(&2u64.to_le_bytes());hp[8..12].copy_from_slice(&8u32.to_le_bytes());hp[12..20].copy_from_slice(&100u64.to_le_bytes());
    let mut first=protocol::encode_frame(170205,&hp);let mut a=vec![0;35];a[..4].copy_from_slice(&1u32.to_le_bytes());a[8..12].copy_from_slice(&2u32.to_le_bytes());first.extend(protocol::encode_frame(20526,&a));
    let n=first.len() as u32;r.feed(Input::Segment(segment(1000,first,0))).unwrap();
    hp[12..20].copy_from_slice(&90u64.to_le_bytes());let mut suffix=protocol::encode_frame(20132,&[]);suffix.extend(protocol::encode_frame(170205,&hp));
    r.feed(Input::Segment(segment(1010+n,suffix,0))).unwrap();r.feed(Input::Segment(segment(1010+n,vec![],3_000_000_001))).unwrap();r.finish();
    assert_eq!(r.meter.session.estimated_total.damage,10);assert_eq!(r.health.loss,10);assert_eq!(r.health.unassigned,0);assert!(r.diagnostics.partial());
    let frames:Vec<_>=r.frame_history.iter().collect();assert_ne!(frames[0].at.continuity,frames.last().unwrap().at.continuity);
    assert_eq!(frames.last().unwrap().timestamp_ns,0);
}
