use mabi_meter_core::{capture_file::{self,Input},identity::{Appearance,Character,Registry},
    meter::{Action,Command,EncounterState,Event,Hit,Meter},protocol::{self,Damage},replay::{self,Replay},
    transport::{Flow,Framer,Segment,TcpStream,distance},Diagnostics};
use std::io::Write;

fn payload(actor: u32,target: u32,amount: u64) -> Vec<u8> {
    let mut p = vec![0;53]; p[..4].copy_from_slice(&actor.to_le_bytes());
    p[8..12].copy_from_slice(&target.to_le_bytes()); p[16..24].copy_from_slice(&amount.to_le_bytes()); p
}
fn frame(amount: u64) -> Vec<u8> { protocol::encode_frame(21196,&payload(101,999,amount)) }
fn segment(seq: u32,payload: Vec<u8>,time: u64) -> Segment {
    Segment { seq,payload,timestamp_ns:time,flow:Flow { src:"192.0.2.1".into(),sport:1234,
        dst:"192.0.2.2".into(),dport:2345 },syn:false,fin:false,rst:false,ack:true }
}
fn appearance(actor: u32,uid: u64,name: &str) -> Appearance {
    Appearance { actor,uid:Some(uid),name:name.into(),template:None,offset:8,record_reference:None }
}
fn character(uid: Option<u64>,name: Option<(&str,u16)>) -> Character {
    Character { uid,code_server:None,name_server:name.map(|(n,s)| (n.into(),s)) }
}
fn add(m: &mut Meter,actor: u32,target: u32,amount: u64,time: u64) {
    let source = m.registry.observe(1,m.binding,actor).unwrap();
    let target = m.registry.observe(1,m.binding,target).unwrap();
    m.add(Hit { timestamp_ns:time,source,target,damage:Damage { actor,target:target.actor,amount,
        flags:[false;5],raw_flags:[0;7] } }).unwrap();
}
fn cmd(m: &mut Meter,id: u64,action: Action) -> mabi_meter_core::meter::Ack {
    m.command(Command { id,observed_revision:m.revision,encounter:m.current,action })
}

#[test] fn verified_map_and_low_ids() {
    assert_eq!(protocol::normalize(21196),21045); assert_eq!(protocol::normalize(20517),220517);
    let mut p = payload(101,999,1200); p[4..8].fill(255); p[32]=1;p[33]=6;p[35]=8;p[36]=16;
    let d = protocol::damage(&p).unwrap(); assert_eq!(d.actor,101);assert_eq!(d.flags,[true;5]);
    assert_eq!(d.raw_flags,[1,6,0,8,16,0,0]);
}
#[test] fn damage_bounds_and_zero() {
    assert_eq!(protocol::damage(&payload(1,2,0)).unwrap().amount,0);
    for (a,t,n) in [(0,2,1),(1,0,1),(1,1,1),(1,2,1_000_000_000_001)] {
        assert!(protocol::damage(&payload(a,t,n)).is_err());
    }
    assert!(protocol::damage(&[0;52]).is_err());
}
#[test] fn every_partial_boundary() {
    let f = frame(1200);
    for n in 0..f.len() { assert!(protocol::read_frame(&f[..n]).unwrap().is_none()); }
    assert_eq!(protocol::read_frame(&f).unwrap().unwrap().consumed,f.len());
}
#[test] fn invalid_header_and_control() {
    let mut f = frame(1); f[8]=2; assert!(protocol::read_frame(&f).is_err());
    let mut c = b"MMCG".to_vec();c.extend(3u32.to_le_bytes());c.extend(b"abc");
    let f = protocol::read_frame(&c).unwrap().unwrap();assert_eq!(f.wire,None);assert_eq!(f.payload,b"abc");
}
#[test] fn brotli_roundtrip_truncation_and_expansion() {
    let original = vec![b'x';100000]; let mut packed = Vec::new();
    { let mut w = brotli::CompressorWriter::new(&mut packed,4096,5,22);w.write_all(&original).unwrap(); }
    assert_eq!(protocol::decompress(&packed,100000).unwrap(),original);
    assert!(protocol::decompress(&packed,99999).is_err());
    assert!(protocol::decompress(&packed[..packed.len()/2],100000).is_err());
    assert!(protocol::decompress(&[255;32],100000).is_err());
}
#[test] fn damaged_compressed_frame_retains_wire() {
    let mut f = protocol::encode_frame(21196,&[255;32]);f[8]=1;
    let f = protocol::read_frame(&f).unwrap().unwrap();assert!(!f.decoded);assert_eq!(f.wire,Some(21196));
}
#[test] fn damage_header_cannot_align() {
    let mut f = Framer::default();let mut d = Diagnostics::default();
    assert!(f.feed(&frame(500),&mut d).unwrap().is_empty());
    let mut anchored = protocol::encode_frame(20132,&[]);anchored.extend(frame(500));
    assert_eq!(f.feed(&anchored,&mut d).unwrap().len(),2);
}
#[test] fn split_out_of_order_and_retransmission() {
    let f = frame(123);let mut s = TcpStream::new(true);let mut d = Diagnostics::default();
    assert!(s.feed(&segment(1000,f[..10].to_vec(),0),&mut d).unwrap().is_empty());
    assert!(s.feed(&segment(1030,f[30..].to_vec(),1),&mut d).unwrap().is_empty());
    assert_eq!(s.feed(&segment(1010,f[10..30].to_vec(),2),&mut d).unwrap().len(),1);
    assert!(s.feed(&segment(1000,f,3),&mut d).unwrap().is_empty());
    assert_eq!(d.0["retransmission"],1);
}
#[test] fn first_arrival_overlap_and_wraparound() {
    assert_eq!(distance(3,u32::MAX-1),5);
    let f = frame(123);let mut s = TcpStream::new(true);let mut d = Diagnostics::default();
    let start = u32::MAX-15;
    s.feed(&segment(start,f[..10].to_vec(),0),&mut d).unwrap();
    s.feed(&segment(start.wrapping_add(20),f[20..].to_vec(),1),&mut d).unwrap();
    let mut closing = f[10..].to_vec();closing[10..].fill(255);
    let result = s.feed(&segment(start.wrapping_add(10),closing,2),&mut d).unwrap();
    assert_eq!(protocol::damage(&result[0].payload).unwrap().amount,123);
}
#[test] fn syn_fin_and_gap() {
    let f = frame(1);let mut s = TcpStream::new(false);let mut d = Diagnostics::default();
    let mut first = segment(1000,f.clone(),0);first.syn=true;
    assert_eq!(s.feed(&first,&mut d).unwrap().len(),1);
    let mut fin = segment(1001+f.len() as u32+10,Vec::new(),1);fin.fin=true;
    s.feed(&fin,&mut d).unwrap();assert!(!s.closed);
    s.feed(&segment(1001+f.len() as u32,vec![0;10],2),&mut d).unwrap();assert!(s.closed);
    let mut s = TcpStream::new(true);s.feed(&segment(1000,f[..9].to_vec(),0),&mut d).unwrap();
    s.feed(&segment(1100,vec![1],1),&mut d).unwrap();
    s.feed(&segment(1200,frame(1),3_000_000_002),&mut d).unwrap();assert!(d.0.contains_key("tcp_gap_timeout"));
}
#[test] fn pending_pressure_is_visible() {
    let mut s = TcpStream::new(true);let mut d = Diagnostics::default();
    s.feed(&segment(1,vec![],0),&mut d).unwrap();
    for n in 0..257 { s.feed(&segment(100+n*2,vec![0],1),&mut d).unwrap(); }
    assert_eq!(d.0["tcp_pending_overflow"],1);assert_eq!(s.buffered(),0);
}
#[test] fn json_rejects_bad_types_and_unicode_hex_without_panicking() {
    let valid = br#"{"timestamp_ns":0,"src":"a","dst":"b","sport":1,"dport":2,"seq":0}"#;
    assert!(capture_file::json_record(valid).is_ok());
    let bad = br#"{"timestamp_ns":true,"src":"a","dst":"b","sport":1,"dport":2,"seq":0}"#;
    assert!(capture_file::json_record(bad).is_err());
    assert!(capture_file::unhex("ああ").is_err());assert!(capture_file::unhex("f").is_err());
    assert_eq!(capture_file::unhex("0aFF").unwrap(),[10,255]);
}
#[test] fn both_identity_arrival_orders() {
    for before in [false,true] {
        let mut r = Registry::default();let key = r.observe(1,1,42).unwrap();
        let c = character(Some(88),None);
        if before { r.character(c.clone()).unwrap(); }
        r.verified_appearance(key,c.clone()).unwrap();
        if !before { r.character(c).unwrap(); }
        assert_eq!(r.self_key,Some(key));assert!(!r.operator_assigned);
    }
}
#[test] fn uid_conflict_missing_server_and_ambiguity() {
    assert!(!character(Some(1),Some(("same",7))).matches(&character(Some(2),Some(("same",7)))));
    assert!(!character(None,None).matches(&character(None,Some(("same",0)))));
    assert!(character(None,Some(("same",7))).matches(&character(None,Some(("same",7)))));
    let mut r = Registry::default();let a = r.observe(1,1,10).unwrap();let b = r.observe(1,1,11).unwrap();
    let c = character(Some(88),None);r.character(c.clone()).unwrap();r.verified_appearance(a,c.clone()).unwrap();
    assert_eq!(r.self_key,Some(a));r.verified_appearance(b,c).unwrap();assert_eq!(r.self_key,None);
}
#[test] fn uid_replacement_keeps_historical_name_and_rejects_stale_marker() {
    let mut r = Registry::default();let old = r.appearance(1,1,appearance(42,100,"First")).unwrap();
    r.assign(Some(old)).unwrap();let new = r.appearance(1,1,appearance(42,200,"Second")).unwrap();
    assert_ne!(old,new);assert_eq!(r.self_key,None);assert!(r.assign(Some(old)).is_err());
    assert!(r.label(old).starts_with("First"));assert!(r.label(new).starts_with("Second"));
}
#[test] fn late_names_enrich_same_key() {
    let mut r = Registry::default();let old = r.observe(1,1,42).unwrap();r.assign(Some(old)).unwrap();
    assert!(r.label(old).starts_with("<ME>"));
    assert_eq!(r.appearance(1,1,appearance(42,100,"Example")).unwrap(),old);
    assert!(r.label(old).starts_with("Example"));
}
#[test] fn top16_uses_all_sources_and_target_denominator() {
    let mut m = Meter::default();for actor in 1..=20 { add(&mut m,actor,999,actor as u64,0); }
    add(&mut m,1,999,1000,10_000_000_000);add(&mut m,20,998,5000,20_000_000_000);
    let snapshot = m.snapshot();assert_eq!(snapshot.rows.len(),16);assert_eq!(snapshot.source_count,20);
    assert_eq!(snapshot.summary.damage,6210);assert_eq!(snapshot.rows[0].key.actor,20);
    let target = m.registry.observe(1,1,999).unwrap();assert!(cmd(&mut m,1,Action::Target(Some(target))).applied);
    let snapshot = m.snapshot();assert_eq!(snapshot.summary.damage,1210);assert_eq!(snapshot.rows[0].key.actor,1);
    assert_eq!(snapshot.rows[0].dps,Some(100.1));assert!(snapshot.rows.iter().map(|r| r.share).sum::<f64>() < 1.0);
}
#[test] fn marker_ack_survives_reorder_offscreen_and_target_change() {
    let mut m = Meter::default();add(&mut m,1,999,1,0);let selected = m.snapshot().rows[0].key;
    let observed = m.revision;
    for actor in 2..=20 { add(&mut m,actor,998,100,1_000_000_000); }
    assert!(!m.snapshot().rows.iter().any(|r| r.key == selected));
    let ack = m.command(Command { id:1,observed_revision:observed,encounter:m.current,action:Action::AssignSelf(Some(selected)) });
    assert!(ack.applied);assert_eq!(ack.snapshot.last_command,1);assert_eq!(m.registry.self_key,Some(selected));
    let target = m.registry.observe(1,1,999).unwrap();assert!(cmd(&mut m,2,Action::Target(Some(target))).applied);
    let s = m.snapshot();assert_eq!(s.rows[0].key,selected);assert!(s.rows[0].label.starts_with("<ME>"));
    assert!(!s.rows[0].named);assert!(cmd(&mut m,3,Action::AssignSelf(None)).applied);assert_eq!(m.registry.self_key,None);
}
#[test] fn stale_and_replayed_commands_are_acknowledged() {
    let mut m = Meter::default();add(&mut m,1,999,1,0);
    assert!(cmd(&mut m,1,Action::AssignSelf(None)).applied);
    assert!(!cmd(&mut m,1,Action::AssignSelf(None)).applied);
    let result = m.command(Command { id:2,observed_revision:m.revision+1,encounter:m.current,action:Action::Reset });
    assert!(!result.applied);assert_eq!(result.snapshot.last_command,1);
}
#[test] fn reset_coalescing_rebroadcast_gap_and_internal_transition() {
    let mut m = Meter::default();add(&mut m,1,999,100,0);let original = m.current;
    m.event(Event::Rebroadcast).unwrap();m.event(Event::Gap).unwrap();
    assert_eq!(m.snapshot().summary.damage,100);assert_eq!(m.current,original);assert!(m.partial);
    m.event(Event::InternalTransition { evidence:1 }).unwrap();assert_eq!(m.snapshot().summary.damage,100);
    m.event(Event::NewRun { evidence:2 }).unwrap();m.event(Event::NewRun { evidence:2 }).unwrap();
    assert_eq!(m.current,original);add(&mut m,1,999,7,1_000_000_000);
    assert_eq!(m.current,original+1);assert_eq!(m.snapshot().summary.damage,7);
    assert_eq!(m.encounters[&original].views[&None].total.damage,100);
}
#[test] fn duplicate_end_and_trailing_cutscene_do_not_reopen() {
    let mut m = Meter::default();add(&mut m,1,999,100,0);
    m.event(Event::End { evidence:1 }).unwrap();m.event(Event::End { evidence:1 }).unwrap();
    m.event(Event::TrailingCutscene).unwrap();assert_eq!(m.encounters[&m.current].state,EncounterState::Ended);
    assert_eq!(m.encounters.len(),1);m.event(Event::SameMapRetry { evidence:2 }).unwrap();
    add(&mut m,1,999,7,1);assert_eq!(m.encounters.len(),2);
}
#[test] fn totals_survive_raw_history_eviction() {
    let mut m = Meter::default();for n in 0..65540 { add(&mut m,1,999,100,n); }
    assert_eq!(m.raw_hits.len(),65536);assert_eq!(m.evicted_hits,4);assert_eq!(m.snapshot().summary.damage,6554000);
}
#[test] fn attack_and_scene_never_add_damage_or_reset() {
    let mut r = Replay::new(true);r.feed(Input::Segment(segment(1000,frame(500),0))).unwrap();
    let mut next = protocol::encode_frame(20526,&[0;35]);next.extend(protocol::encode_frame(20344,&[]));
    next.extend(protocol::encode_frame(20552,&[]));
    r.feed(Input::Segment(segment(1062,next,1))).unwrap();assert_eq!(r.meter.snapshot().summary.damage,500);
    assert_eq!(r.meter.current,1);assert_eq!(r.diagnostics.0["scene_marker"],2);
}
#[test] fn backwards_capture_time_is_rejected() {
    let mut r = Replay::new(true);r.feed(Input::Segment(segment(0,vec![],10))).unwrap();
    assert!(r.feed(Input::Segment(segment(0,vec![],9))).is_err());
}
fn ipv4_tcp(data: &[u8],vlan: bool) -> Vec<u8> {
    let mut packet = vec![0;12];
    packet.extend_from_slice(if vlan { &[0x81,0,0,1,8,0] } else { &[8,0] });
    let mut ip = vec![0;20];ip[0]=0x45;ip[2..4].copy_from_slice(&((40+data.len()) as u16).to_be_bytes());
    ip[8]=64;ip[9]=6;ip[12..16].copy_from_slice(&[192,0,2,1]);ip[16..20].copy_from_slice(&[192,0,2,2]);
    packet.extend(ip);let mut tcp = vec![0;20];tcp[..2].copy_from_slice(&1234u16.to_be_bytes());
    tcp[2..4].copy_from_slice(&2345u16.to_be_bytes());tcp[4..8].copy_from_slice(&1000u32.to_be_bytes());
    tcp[12]=0x50;tcp[13]=0x10;packet.extend(tcp);packet.extend_from_slice(data);packet
}
#[test] fn ethernet_vlan_and_ipv6() {
    for vlan in [false,true] {
        let packet = ipv4_tcp(&frame(12),vlan);
        let s = capture_file::tcp_packet(&packet,1,123).unwrap().unwrap();
        assert_eq!(s.flow.src,"192.0.2.1");assert_eq!(s.flow.dport,2345);assert_eq!(s.payload,frame(12));
    }
    let mut ip = vec![0;60];ip[0]=0x60;ip[4..6].copy_from_slice(&20u16.to_be_bytes());ip[6]=6;
    ip[23]=1;ip[39]=2;ip[40..42].copy_from_slice(&1234u16.to_be_bytes());
    ip[42..44].copy_from_slice(&2345u16.to_be_bytes());ip[52]=0x50;
    assert_eq!(capture_file::tcp_packet(&ip,229,0).unwrap().unwrap().flow.src,"::1");
    ip[6]=44;assert!(capture_file::tcp_packet(&ip,229,0).is_err());
}
fn wifi_packet(body: &[u8],ds: u16,qos: bool,ht: bool,aggregate: bool) -> Vec<u8> {
    let fc=0x0008|ds|if qos {0x0080} else {0}|if ht {0x8000} else {0};
    let mut p=vec![0;if ds==0x0300 {30} else {24}];
    p[..2].copy_from_slice(&fc.to_le_bytes());
    if qos {p.extend([if aggregate {0x80} else {0},0]);if ht {p.extend([0;4]);}}
    p.extend(body);p
}
fn wifi_snap(ethernet: &[u8]) -> Vec<u8> {
    let mut p=vec![0xaa,0xaa,3,0,0,0];p.extend_from_slice(&ethernet[12..]);p
}
#[test] fn packet_monitor_wifi_dispatch_preserves_tcp_and_damage() {
    let eth=ipv4_tcp(&frame(123),false);
    let expected=capture_file::tcp_packet(&eth,1,987).unwrap().unwrap();
    for ds in [0,0x0100,0x0200,0x0300] {for (qos,ht) in [(false,false),(false,true),(true,false),(true,true)] {
        for vlan in [false,true] {
            let eth=ipv4_tcp(&frame(123),vlan);
            let p=wifi_packet(&wifi_snap(&eth),ds,qos,ht,false);
            let segments=capture_file::packet_monitor_tcp(&p,2,987).unwrap();
            assert_eq!(segments.len(),1);
            let actual=&segments[0];assert_eq!(actual.flow,expected.flow);
            assert_eq!(actual.timestamp_ns,987);assert_eq!(actual.seq,1000);
            assert_eq!(actual.payload,expected.payload);assert!(actual.ack);
            let mut replay=Replay::new(true);replay.feed(Input::Segment(actual.clone())).unwrap();
            assert_eq!(replay.meter.snapshot().summary.damage,123);
        }
    }}
    for (p,kind) in [(&eth[..],1),(&eth[14..],3)] {
        let s=capture_file::packet_monitor_tcp(p,kind,987).unwrap();
        assert_eq!(s.len(),1);assert_eq!(s[0].payload,expected.payload);
    }
    assert!(capture_file::packet_monitor_tcp(&eth,0,0).unwrap().is_empty());
    assert!(capture_file::packet_monitor_tcp(&eth,5,0).unwrap().is_empty());
}
#[test] fn packet_monitor_wifi_ipv6_and_plaintext_protected_flag() {
    let mut ip=vec![0;60];ip[0]=0x60;ip[4..6].copy_from_slice(&20u16.to_be_bytes());ip[6]=6;
    ip[23]=1;ip[39]=2;ip[40..42].copy_from_slice(&1234u16.to_be_bytes());
    ip[42..44].copy_from_slice(&2345u16.to_be_bytes());ip[52]=0x50;
    let mut snap=vec![0xaa,0xaa,3,0,0,0xf8,0x86,0xdd];snap.extend(ip);
    let mut p=wifi_packet(&snap,0x0200,true,true,false);p[1]|=0x40;
    let s=capture_file::packet_monitor_tcp(&p,2,123).unwrap();
    assert_eq!(s.len(),1);assert_eq!(s[0].flow.src,"::1");assert_eq!(s[0].flow.dst,"::2");
    assert_eq!(s[0].flow.dport,2345);
}
#[test] fn packet_monitor_wifi_aggregate_and_truncated_tail() {
    let mut body=Vec::new();
    for amount in [12,34] {
        if !body.is_empty(){while body.len()%4!=0 {body.push(0);}}
        let snap=wifi_snap(&ipv4_tcp(&frame(amount),false));
        body.extend([0;12]);body.extend((snap.len() as u16).to_be_bytes());body.extend(snap);
    }
    let p=wifi_packet(&body,0x0200,true,false,true);
    let segments=capture_file::packet_monitor_tcp(&p,2,123).unwrap();
    assert_eq!(segments.len(),2);assert_eq!(segments[0].payload,frame(12));assert_eq!(segments[1].payload,frame(34));
    // A valid first subframe must not escape when the final subframe is cut.
    for end in 27..p.len() {
        let result=capture_file::packet_monitor_tcp(&p[..end],2,123);
        if let Ok(s)=result {assert_eq!(s.len(),1);} // Exact first-subframe boundary is valid.
    }
    assert!(capture_file::packet_monitor_tcp(&p[..p.len()-1],2,123).is_err());
}
#[test] fn packet_monitor_wifi_rejects_truncation_and_fragments_ignores_non_ip() {
    let p=wifi_packet(&wifi_snap(&ipv4_tcp(&frame(12),false)),0x0300,true,true,false);
    for end in 0..p.len() {assert!(capture_file::packet_monitor_tcp(&p[..end],2,0).is_err(),"accepted prefix {end}");}
    for (offset,flag) in [(1,4),(22,1)] {
        let mut bad=p.clone();bad[offset]|=flag;
        assert!(capture_file::packet_monitor_tcp(&bad,2,0).is_err());
    }
    for fc in [0x0080u16,0x00d4,0x0048,0x00c8] {
        assert!(capture_file::packet_monitor_tcp(&fc.to_le_bytes(),2,0).unwrap().is_empty());
    }
    for body in [&[0u8;40][..],&[0xaa,0xaa,3,0,0,0,8,6,0,0][..]] {
        let p=wifi_packet(body,0x0200,false,false,false);
        assert!(capture_file::packet_monitor_tcp(&p,2,0).unwrap().is_empty());
    }
    let mut bad=p;bad[0]|=1;assert!(capture_file::packet_monitor_tcp(&bad,2,0).is_err());
}
struct TestFile(std::path::PathBuf);
impl Drop for TestFile { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }
fn test_file(bytes: &[u8]) -> TestFile {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let p = std::env::temp_dir().join(format!("mabi-rust-test-{}-{n}.pcap",std::process::id()));
    let mut f = std::fs::OpenOptions::new().create_new(true).write(true).open(&p).unwrap();
    f.write_all(bytes).unwrap();TestFile(p)
}
#[test] fn pcap_byte_orders_and_timestamp_precision() {
    for le in [false,true] { for nano in [false,true] {
        let word = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let mut pcap = word(if nano { 0xa1b23c4d } else { 0xa1b2c3d4 }).to_vec();
        pcap.extend_from_slice(if le { &[2,0,4,0] } else { &[0,2,0,4] });
        for v in [0,0,262144,1] { pcap.extend(word(v)); }
        let packet = ipv4_tcp(&frame(17),true);
        for v in [1,123,packet.len() as u32,packet.len() as u32] { pcap.extend(word(v)); }
        pcap.extend(packet);let file = test_file(&pcap);let mut r = Replay::new(true);
        capture_file::read(&file.0,Some(2345),|s| r.feed(s)).unwrap();r.finish();
        assert_eq!(r.meter.snapshot().summary.damage,17);
        assert_eq!(r.meter.snapshot().summary.start,Some(1_000_000_000+123*if nano { 1 } else { 1000 }));
        assert!(capture_file::read(&file.0,None,|_| Ok(())).is_err());
    }}
}
#[test] fn synthetic_demo_baseline() {
    let r = replay::demo().unwrap();let s = r.meter.snapshot();assert_eq!(s.summary.damage,8900);
    assert_eq!(s.summary.hits,6);assert_eq!(s.summary.duration(),5.0);assert_eq!(s.summary.dps(),Some(1780.));
    let audit = r.audit();assert_eq!(audit["frame_count"].as_u64(),Some(6));
    assert_eq!(audit["frames"].as_array().unwrap().len(),6);
}

// Private fixtures stay outside the crate and are never embedded or committed.
#[test] #[ignore = "requires private current-game profile-panel capture"]
fn current_game_profile_action_preserves_numeric_parity_without_inventing_self() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-fixtures");
    let r = Replay::load(&root.join("rust-identity-profile-20261105-205033.jsonl"),None,false).unwrap();
    let s = r.meter.snapshot();
    // Gap recovery retains 32 additional explicit records (371,158 damage).
    // The original 430 explicit records are unchanged in the multiset audit.
    assert_eq!(r.frames,12264);assert_eq!(s.summary.damage,7_846_849);assert_eq!(s.summary.hits,462);
    assert_eq!(r.diagnostics.0["tcp_gap_recovered"],1);
    assert_eq!(s.summary.end.unwrap()-s.summary.start.unwrap(),44_367_453_700);
    assert_eq!(s.source_count,2);assert!(s.partial);assert_eq!(r.meter.registry.self_key,None);
    assert_eq!(r.meter.registry.profile_names.len(),5);assert!(r.meter.registry.tracking.personal.is_none());
    assert!(!r.diagnostics.0.contains_key("frame_rejected"));
}

#[test] #[ignore = "requires existing private fixture; run via scripts/test-rust.ps1 -LocalFixtures"]
fn real_fixture_07_numeric_parity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-fixtures");
    let r = Replay::load(&root.join("live-native-07.jsonl"),None,false).unwrap();let s = r.meter.snapshot();
    assert_eq!(s.summary.damage,1_592_235);assert_eq!(s.summary.hits,99);
    assert_eq!(s.summary.end.unwrap()-s.summary.start.unwrap(),9_330_202_000);
    assert!((s.summary.dps().unwrap()-170_653.8615).abs() < 0.001);
}
#[test] #[ignore = "requires existing private entry fixture"]
fn real_entry_numeric_parity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-fixtures");
    let r = Replay::load(&root.join("ui-name-entry-02.jsonl"),None,false).unwrap();let s = r.meter.snapshot();
    // Retained suffixes add 189 records / 3,746,166 damage after six holes.
    assert_eq!(s.summary.damage,100_043_267);assert_eq!(s.summary.hits,6026);
    assert_eq!(r.diagnostics.0["tcp_gap_recovered"],6);
    assert_eq!(s.summary.end.unwrap()-s.summary.start.unwrap(),581_169_047_500);
    assert!((s.summary.dps().unwrap()-172_141.4232749551).abs() < 0.000001);
    assert_eq!(s.target_count,4);
    assert!(s.targets.iter().all(|(_,label)| !label.starts_with("Name pending")));
}

#[test] #[ignore = "requires private current-game dummy capture from 2026-11-05"]
fn current_game_dummy_numeric_parity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-fixtures");
    let r = Replay::load(&root.join("rust-current-dummy-20261105-195023.jsonl"),None,false).unwrap();
    let s = r.meter.snapshot();
    // Retained suffixes add 68 records / 978,881 damage after three holes.
    assert_eq!(r.frames,7986);assert_eq!(s.summary.damage,4_837_662);assert_eq!(s.summary.hits,303);
    assert_eq!(r.diagnostics.0["tcp_gap_recovered"],3);
    assert_eq!(s.summary.end.unwrap()-s.summary.start.unwrap(),29_574_129_000);
    assert_eq!(s.source_count,2);assert_eq!(s.target_count,4);assert!(s.partial);
    assert_eq!(r.evidence.len(),1);assert_eq!(r.evidence[0].wire,Some(170631));
    assert_eq!(r.meter.registry.profile_names.len(),1);assert_eq!(r.meter.registry.self_key,None);
    assert!(s.rows.iter().all(|row| !row.named));
    assert!(!r.diagnostics.0.contains_key("frame_rejected"));
    let audit = r.audit();assert_eq!(audit["frame_count"].as_u64(),Some(7986));
    assert_eq!(audit["frames"].as_array().unwrap().len(),4096);
}
