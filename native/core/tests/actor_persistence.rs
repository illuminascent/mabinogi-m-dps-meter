use mabi_meter_core::{capture_file::{self,Input},combat::Stamp,identity::{self,Character,Registry,tracking::{Evidence,EvidenceOrigin}},meter::{Action,Command,Event,Meter},protocol,replay::Replay,transport::{Flow,Segment}};
fn put32(p:&mut[u8],n:usize,v:u32){p[n..n+4].copy_from_slice(&v.to_le_bytes());}
fn put64(p:&mut[u8],n:usize,v:u64){p[n..n+8].copy_from_slice(&v.to_le_bytes());}
fn string(p:&mut Vec<u8>,s:&str){let bytes:Vec<_>=s.encode_utf16().flat_map(u16::to_le_bytes).collect();p.extend((bytes.len() as u32).to_le_bytes());p.extend(bytes);}
fn pet(p:&mut Vec<u8>,key:u64,actor:u32,name:&str){
    p.extend(key.to_le_bytes());p.extend((actor as u64).to_le_bytes());p.extend(key.to_le_bytes());p.extend(9999999u32.to_le_bytes());p.extend(1_790_000_000_000u64.to_le_bytes());p.extend(3u32.to_le_bytes());string(p,name);
    let at=p.len();p.resize(at+71,0);put32(p,at+8,15);
    p.extend(1u32.to_le_bytes());p.extend(3u32.to_le_bytes());p.extend(10f32.to_le_bytes());
    let at=p.len();p.resize(at+48,0);put32(p,at,3);put32(p,at+20,5);put32(p,at+25,4);
}
fn personal(actor:u32,uid:u64,prefix:usize)->Vec<u8>{
    let mut p=vec![0;prefix];p[..6].copy_from_slice(&uid.to_le_bytes()[..6]);p.extend(2u32.to_le_bytes());
    pet(&mut p,0x123456789abc,actor,"Pet");pet(&mut p,0x23456789abcd,actor,"");
    p.extend(0u32.to_le_bytes());p.extend(0x123456789abcu64.to_le_bytes());p.extend(0x123456789abcu64.to_le_bytes());p.extend(1u32.to_le_bytes());p.extend([0;16]);p.extend([0;32]);p
}
fn profile(actor:u32,uid:u64,name:&str)->Vec<u8>{let mut p=(actor as u64).to_le_bytes().to_vec();p.extend(uid.to_le_bytes());string(&mut p,name);string(&mut p,"Hello");string(&mut p,"opaque");p.extend([0;9]);p.extend(4u32.to_le_bytes());p.extend([255,216,255,217]);p.extend(0u32.to_le_bytes());p}
fn ev(n:u64)->Evidence{Evidence{frame:n,timestamp_ns:n,origin:EvidenceOrigin::Packet{wire:20054}}}
fn char_id(uid:u64)->Character{Character::score2_appearance_uid(uid).unwrap()}
fn feed(r:&mut Replay,seq:&mut u32,port:u16,wire:i32,p:&[u8],t:u64){let payload=protocol::encode_frame(wire,p);let len=payload.len() as u32;r.feed(Input::Segment(Segment{timestamp_ns:t,seq:*seq,flow:Flow{src:"192.0.2.1".into(),dst:"192.0.2.2".into(),sport:16000,dport:port},payload,syn:false,fin:false,rst:false,ack:true})).unwrap();*seq+=len;}

#[test] fn personal_collection_handles_variable_prefixes_and_rejects_partial_ambiguous_or_conflicting_records(){
    let uid=0x0012_3456_0001;
    for prefix in [64,79,1001]{let p=personal(17,uid,prefix);let found=identity::personal::parse(&p).unwrap();assert_eq!((found.actor,found.offset,found.pets.len()),(17,prefix,2));assert_eq!(found.character.uid,None);}
    let p=personal(17,uid,96);let found=identity::personal::parse(&p).unwrap();
    for end in 0..found.end{assert!(identity::personal::parse(&p[..end]).is_none());}
    let mut bad=p.clone();put64(&mut bad,96+4+8,18);assert!(identity::personal::parse(&bad).is_none());
    let mut bad=p.clone();put32(&mut bad,96,u32::MAX);assert!(identity::personal::parse(&bad).is_none());
    let mut bad=p.clone();bad.extend(&p[96..]);assert!(identity::personal::parse(&bad).is_none());
    let mut bad=p;put64(&mut bad,100,0);assert!(identity::personal::parse(&bad).is_none());
}
#[test] fn self_rebinds_from_personal_state_without_a_name_or_appearance_and_keeps_participant_handle(){
    let mut r=Replay::new(true);r.live_scope=true;let mut a=1;let mut b=1;let uid=0x0012_3456_0001;
    feed(&mut r,&mut a,10,20590,&personal(50,uid,80),1);let old=r.meter.registry.self_key.unwrap();let participant=r.meter.registry.participant(old);
    assert_eq!(old.actor,50);assert_eq!(r.meter.registry.tracking.personal.as_ref().unwrap().uid,None);assert_eq!(r.meter.registry.remembered_players(),1);
    feed(&mut r,&mut b,20,20590,&personal(3,uid,128),2);let new=r.meter.registry.self_key.unwrap();
    assert_eq!(new.actor,3);assert!(!r.meter.registry.is_current(old));assert_eq!(r.meter.registry.participant(new),participant);
    assert_eq!(r.meter.registry.group(new),old);assert!(r.meter.registry.label(new).starts_with("<ME>"));assert_eq!(r.meter.registry.remembered_players(),1);
    assert_eq!(r.meter.registry.personal_bindings.len(),2);
}
#[test] fn split_gap_and_same_actor_room_refresh_retain_anonymous_participants_and_operator_assignment(){
    let mut m=Meter::default();let actor=m.registry.observe(1,1,10).unwrap();let unknown=m.registry.observe(1,1,20).unwrap();let id=m.registry.participant(unknown);
    m.registry.appearance_evidence(actor,char_id(0x0012_3456_0001),ev(1)).unwrap();m.registry.assign(Some(actor)).unwrap();
    m.event(Event::Gap).unwrap();m.event(Event::Rebroadcast).unwrap();m.event(Event::InternalTransition{evidence:4}).unwrap();
    assert_eq!(m.binding,1);assert_eq!(m.registry.binding_state(unknown),"carried");
    let ack=m.command(Command{id:1,observed_revision:m.revision,encounter:m.current,action:Action::Reset});assert!(ack.applied);
    assert_eq!(m.registry.self_key,Some(actor));assert_eq!(m.registry.observe(1,1,20).unwrap(),unknown);assert_eq!(m.registry.participant(unknown),id);
    m.registry.note_observed(unknown,Stamp{process:1,context:2,continuity:1,frame:3,order:1,timestamp_ns:4,tcp_byte:0});
    assert_eq!(m.registry.binding_state(unknown),"observed");assert_eq!(m.snapshot().remembered_players,1);assert!(m.snapshot().rows.is_empty());
}
#[test] fn direct_personal_owner_replaces_self_even_without_a_socket_or_room_announcement(){
    let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;let uid=0x0012_3456_0001;
    feed(&mut r,&mut seq,10,20590,&personal(50,uid,80),1);let old=r.meter.registry.self_key.unwrap();let id=r.meter.registry.participant(old);
    feed(&mut r,&mut seq,10,20559,&profile(50,uid,"Self"),2);
    let mut hp=vec![0;21];put64(&mut hp,0,99);put32(&mut hp,8,8);put64(&mut hp,12,200);feed(&mut r,&mut seq,10,170205,&hp,3);
    let mut attack=vec![0;35];put32(&mut attack,0,50);put32(&mut attack,8,99);feed(&mut r,&mut seq,10,20526,&attack,4);
    feed(&mut r,&mut seq,10,20590,&personal(3,uid,128),5);let new=r.meter.registry.self_key.unwrap();
    put64(&mut hp,12,190);feed(&mut r,&mut seq,10,170205,&hp,6);r.finish();assert_eq!(r.health.loss,10);assert_eq!(r.health.unassigned,0);
    assert_eq!(new.actor,3);assert!(!r.meter.registry.is_current(old));assert_eq!(r.meter.registry.participant(new),id);
    assert!(r.meter.registry.label(new).starts_with("Self"));assert_eq!(r.meter.registry.remembered_players(),1);
}
#[test] fn scoped_replacement_retires_only_the_matched_old_world_actor_and_concurrent_claims_stay_separate(){
    let mut r=Registry::default();let me=char_id(0x0012_3456_0001);let peer=char_id(0x0012_3466_0001);
    r.wire_personal(1,100,me.clone(),ev(1)).unwrap();let old=r.observe(1,1,50).unwrap();let other=r.observe(1,1,40).unwrap();
    r.wire_named_appearance(old,peer.clone(),Some("Peer".into()),ev(2),(1,100)).unwrap();let id=r.participant(old);
    r.wire_personal(1,200,me,ev(3)).unwrap();let new=r.observe(1,1,3).unwrap();r.wire_named_appearance(new,peer.clone(),None,ev(4),(1,200)).unwrap();
    assert!(!r.is_current(old));assert!(r.is_current(other));assert_eq!(r.participant(new),id);assert_eq!(r.group(new),old);assert!(r.label(new).starts_with("Peer"));
    let concurrent=r.observe(1,1,4).unwrap();r.wire_named_appearance(concurrent,peer,None,ev(5),(1,200)).unwrap();
    assert!(r.is_current(new));assert_ne!(r.group(new),r.group(concurrent));assert!(!r.tracking.candidates.is_empty());
    r.retire(concurrent).unwrap();assert!(r.tracking.candidates.is_empty());assert_eq!(r.participant(new),id);
}
#[test] fn profile_envelope_is_complete_and_names_do_not_override_uid_conflicts(){
    let p=profile(5,0x0012_3456_0001,"Peer");let parsed=identity::profile::parse(&p).unwrap();assert_eq!((parsed.actor,parsed.uid,parsed.name.as_str()),(5,0x0012_3456_0001,"Peer"));
    for end in 0..p.len(){assert!(identity::profile::parse(&p[..end]).is_none());}
    let mut bad=p.clone();bad.push(0);assert!(identity::profile::parse(&bad).is_none());
    let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;
    feed(&mut r,&mut seq,10,20559,&p,1);let first=r.meter.registry.observe(0,1,5).unwrap();
    feed(&mut r,&mut seq,10,20559,&profile(6,0x0012_3466_0001,"Peer"),2);let second=r.meter.registry.observe(0,1,6).unwrap();
    assert_ne!(r.meter.registry.participant(first),r.meter.registry.participant(second));assert_eq!(r.meter.registry.self_key,None);
    r.process_replacement(1).unwrap();assert_eq!(r.meter.registry.remembered_players(),0);assert_eq!(r.meter.registry.self_key,None);
}
fn close_direction(r:&mut Replay,seq:u32,port:u16,reverse:bool,time:u64){
    let flow=if reverse{Flow{src:"192.0.2.2".into(),dst:"192.0.2.1".into(),sport:port,dport:16000}}
        else{Flow{src:"192.0.2.1".into(),dst:"192.0.2.2".into(),sport:16000,dport:port}};
    r.feed(Input::Segment(Segment{timestamp_ns:time,seq,flow,payload:Vec::new(),syn:false,fin:true,rst:false,ack:true})).unwrap();
}
#[test] fn recovered_old_appearance_cannot_roll_back_a_reused_actor_or_its_name(){
    let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;
    let flow=Flow{src:"192.0.2.1".into(),dst:"192.0.2.2".into(),sport:16000,dport:10};
    let seg=|seq,payload,time|Input::Segment(Segment{timestamp_ns:time,seq,flow:flow.clone(),payload,syn:false,fin:false,rst:false,ack:true});
    r.feed(seg(1000,Vec::new(),0)).unwrap();
    let mut delayed=protocol::encode_frame(20132,&[]);delayed.extend(protocol::encode_frame(20559,&profile(50,0x0012_3456_0001,"Old")));
    r.feed(seg(1010,delayed,1)).unwrap();
    feed(&mut r,&mut seq,20,20559,&profile(50,0x0012_3466_0001,"Current"),2);
    let fresh=r.meter.registry.current_key(0,1,50).unwrap();
    r.feed(seg(1100,Vec::new(),3_000_000_002)).unwrap();
    assert_eq!(r.meter.registry.current_key(0,1,50),Some(fresh));
    assert_eq!(r.meter.registry.records[&fresh].uid,Some(0x0012_3466_0001));assert!(r.meter.registry.label(fresh).starts_with("Current"));
}
#[test] fn legacy_name_scan_cannot_retire_or_relabel_a_verified_binding(){
    let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;let uid=0x0012_3456_0001;
    feed(&mut r,&mut seq,10,20590,&personal(50,uid,80),1);feed(&mut r,&mut seq,10,20559,&profile(50,uid,"Self"),2);
    let key=r.meter.registry.self_key.unwrap();let mut weak=vec![0;100];put32(&mut weak,0,50);put64(&mut weak,16,0x0012_3466_0001);
    put32(&mut weak,24,1);put32(&mut weak,28,10);for (i,b) in "Other".encode_utf16().flat_map(u16::to_le_bytes).enumerate(){weak[32+i]=b;}
    put64(&mut weak,42,1_790_000_000_000);assert!(identity::score2_layout::parse_jp_observed(&weak).is_err());assert!(identity::player(&weak).is_some());
    feed(&mut r,&mut seq,10,20054,&weak,3);
    assert_eq!(r.meter.registry.self_key,Some(key));assert_eq!(r.meter.registry.current_key(0,1,50),Some(key));
    assert_eq!(r.meter.registry.records[&key].uid,Some(uid));assert!(r.meter.registry.label(key).starts_with("Self"));
}
#[test] fn operator_assignment_follows_proven_replacement_and_retracts_with_alias_ambiguity(){
    let mut r=Registry::default();let c=char_id(0x0012_3456_0001);let old=r.observe(1,1,50).unwrap();
    r.appearance_evidence(old,c.clone(),ev(1)).unwrap();r.assign_command(Some(old),17).unwrap();r.retire(old).unwrap();
    assert_eq!(r.self_key,None);let next=r.observe(1,1,3).unwrap();r.appearance_evidence(next,c.clone(),ev(2)).unwrap();
    assert_eq!(r.self_key,Some(next));assert!(r.operator_assigned);
    let concurrent=r.observe(1,1,4).unwrap();r.appearance_evidence(concurrent,c,ev(3)).unwrap();assert_eq!(r.self_key,None);
    r.retire(concurrent).unwrap();assert_eq!(r.self_key,Some(next));assert!(r.operator_assigned);
}
#[test] fn new_pet_spawn_cannot_overwrite_a_current_player_incarnation(){
    let mut r=Registry::default();let old=r.observe(1,1,50).unwrap();r.appearance_evidence(old,char_id(0x0012_3456_0001),ev(1)).unwrap();
    let pet=r.owned_spawn(1,1,identity::ownership::Spawn{actor:50,owner:10,template:123,outer:None,kind:4},ev(2)).unwrap();
    assert_ne!(pet,old);assert_eq!(r.records[&old].kind,Some("player"));assert_eq!(r.records[&pet].kind,Some("pet"));
    assert!(!r.is_current(old));assert_eq!(r.owner(pet).unwrap().actor,10);
}
#[test] fn four_instance_encounters_then_raid_without_personal_snapshot_starts_a_fifth_battle(){
    let mut r=Replay::new(true);r.live_scope=true;r.meter.automatic_encounters=true;let uid=0x0012_3456_0001;let mut prior=None;
    for n in 0..5u64{
        let mut seq=1;let port=10+n as u16;let actor=50+n as u32;let time=(n*40+1)*1_000_000_000;
        if let Some((old_seq,old_port))=prior{close_direction(&mut r,old_seq,old_port,false,time-2);close_direction(&mut r,1,old_port,true,time-1);}
        if n<4{feed(&mut r,&mut seq,port,20590,&personal(actor,uid,80),time);}
        feed(&mut r,&mut seq,port,20559,&profile(actor,uid,"Self"),time+1);
        assert_eq!(r.meter.registry.self_key.unwrap().actor,actor);
        let mut damage=vec![0;53];put32(&mut damage,0,actor);put32(&mut damage,8,900+n as u32);put64(&mut damage,16,100);
        feed(&mut r,&mut seq,port,21196,&damage,time+2);
        assert_eq!(r.meter.current,n+1);assert_eq!(r.meter.snapshot().summary.damage,100);
        r.tick(time+20_000_000_000).unwrap();prior=Some((seq,port));
    }
    assert!(r.meter.registry.tracking.events.iter().any(|e|e.reason=="self_context_handoff"));
    assert_eq!(r.meter.registry.remembered_players(),1);assert_eq!(r.meter.session.total.damage,500);
}
#[test] fn half_closed_tcp_does_not_handoff_but_complete_close_and_fresh_exact_uid_do(){
    let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;let mut next=1;let uid=0x0012_3456_0001;
    feed(&mut r,&mut seq,10,20590,&personal(50,uid,80),1);feed(&mut r,&mut seq,10,20559,&profile(50,uid,"Self"),2);
    close_direction(&mut r,seq,10,false,3);feed(&mut r,&mut next,20,20559,&profile(3,uid,"Self"),4);
    assert_eq!(r.meter.registry.self_key.unwrap().actor,50);
    close_direction(&mut r,1,10,true,5);feed(&mut r,&mut next,20,20559,&profile(3,uid,"Self"),6);
    assert_eq!(r.meter.registry.self_key.unwrap().actor,3);assert_eq!(r.meter.registry.remembered_players(),1);
}
#[test] fn both_fin_notifications_prove_closure_even_when_missing_bytes_prevent_stream_drain(){
    let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;let uid=0x0012_3456_0001;
    feed(&mut r,&mut seq,10,20590,&personal(50,uid,80),1);feed(&mut r,&mut seq,10,20559,&profile(50,uid,"Self"),2);
    close_direction(&mut r,seq+100,10,false,3);close_direction(&mut r,1,10,true,4);
    let mut next=1;feed(&mut r,&mut next,20,20559,&profile(3,uid,"Self"),5);
    assert_eq!(r.meter.registry.self_key.unwrap().actor,3);
}
#[test] fn new_syn_on_reused_tcp_tuple_has_a_new_identity_context_and_does_not_reuse_boss_keys(){
    let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;let uid=0x0012_3456_0001;
    feed(&mut r,&mut seq,10,20590,&personal(50,uid,80),1);feed(&mut r,&mut seq,10,20559,&profile(50,uid,"Self"),2);
    let old_context=r.frame_history.back().unwrap().context;
    let boss=r.meter.registry.observe(0,1,9).unwrap();
    r.protected_hp.register(old_context,boss,&mabi_meter_core::protected_hp::SpawnSeed{actor:9,iv:[0;16],initial_hp:100,iv_offset:0}).unwrap();
    r.feed(Input::Segment(Segment{timestamp_ns:3,seq:5000,flow:Flow{src:"192.0.2.1".into(),dst:"192.0.2.2".into(),sport:16000,dport:10},payload:Vec::new(),syn:true,fin:false,rst:false,ack:false})).unwrap();
    seq=5001;feed(&mut r,&mut seq,10,20559,&profile(3,uid,"Self"),4);
    assert_ne!(r.frame_history.back().unwrap().context,old_context);assert_eq!(r.meter.registry.self_key.unwrap().actor,3);
    let mut update=vec![0;29];put64(&mut update,0,9);put32(&mut update,8,16);
    assert_eq!(r.protected_hp.decode(r.frame_history.back().unwrap().context,boss,170206,&update),Err(mabi_meter_core::protected_hp::DecodeError::MissingIv));
}
#[test] fn recovered_personal_owner_cannot_replace_a_newer_actor_incarnation(){
    let mut r=Registry::default();let p=identity::personal::parse(&personal(50,0x0012_3456_0001,80)).unwrap();
    let at=|t|Stamp{process:1,context:1,continuity:0,frame:t,order:t,timestamp_ns:t,tcp_byte:0};
    r.personal_actor(1,1,(1,100),p.clone(),ev(10),at(10)).unwrap();
    let key=r.appearance(1,1,identity::Appearance{actor:50,name:"Other".into(),uid:Some(0x0012_3466_0001),template:None,offset:0,record_reference:None}).unwrap();
    r.wire_named_appearance(key,char_id(0x0012_3466_0001),Some("Other".into()),ev(20),(1,200)).unwrap();
    assert!(r.personal_actor(1,1,(1,100),p,ev(15),at(15)).unwrap().is_none());
    assert_eq!(r.current_key(1,1,50),Some(key));assert_eq!(r.records[&key].uid,Some(0x0012_3466_0001));
}
#[test] fn closed_context_handoff_rejects_code_only_conflicting_uid_stale_and_reopened_socket(){
    for mode in 0..5{
        let mut r=Registry::default();let uid=0x0012_3456_0001;let me=char_id(uid);
        r.wire_personal(1,100,me.clone(),ev(1)).unwrap();let old=r.observe(1,1,50).unwrap();
        r.wire_named_appearance(old,me.clone(),Some("Self".into()),ev(2),(1,100)).unwrap();
        r.tracking.transport_closed((1,100),ev(5)).unwrap();
        let mut identity=me;let mut evidence=ev(6);
        match mode{0=>identity.uid=None,1=>identity=char_id(uid|(1u64<<48)),2=>evidence=ev(4),
            3=>r.tracking.transport_open((1,100)),_=>r.tracking.transport_closed((1,200),ev(5)).unwrap()}
        let new=r.observe(1,1,3).unwrap();r.wire_named_appearance(new,identity,None,evidence,(1,200)).unwrap();
        assert_eq!(r.self_key,Some(old));assert!(r.is_current(old));assert_ne!(r.group(old),r.group(new));
    }
}
#[test] fn closed_socket_retransmission_cannot_duplicate_damage_or_restore_a_superseded_owner(){
    let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;let uid=0x0012_3456_0001;
    feed(&mut r,&mut seq,10,20590,&personal(50,uid,80),1);feed(&mut r,&mut seq,10,20559,&profile(50,uid,"Self"),2);
    let mut damage=vec![0;53];put32(&mut damage,0,50);put32(&mut damage,8,90);put64(&mut damage,16,100);
    let damage_seq=seq;feed(&mut r,&mut seq,10,21196,&damage,3);
    close_direction(&mut r,seq,10,false,4);
    let mut early_retransmit=damage_seq;feed(&mut r,&mut early_retransmit,10,21196,&damage,4);assert_eq!(r.meter.session.total.damage,100);
    close_direction(&mut r,1,10,true,5);
    let mut next=1;feed(&mut r,&mut next,20,20559,&profile(3,uid,"Self"),6);
    let mut retransmit_seq=damage_seq;feed(&mut r,&mut retransmit_seq,10,21196,&damage,7);
    assert_eq!(r.meter.session.total.damage,100);
    feed(&mut r,&mut seq,10,20590,&personal(50,uid,80),8);
    assert_eq!(r.meter.registry.self_key.unwrap().actor,3);
}
#[test] fn closed_personal_context_cannot_reassert_self_even_with_a_later_receipt_time(){
    let mut r=Registry::default();let uid=0x0012_3456_0001;let p=identity::personal::parse(&personal(50,uid,80)).unwrap();
    let at=|t|Stamp{process:1,context:1,continuity:0,frame:t,order:t,timestamp_ns:t,tcp_byte:0};
    r.personal_actor(1,1,(1,100),p.clone(),ev(10),at(10)).unwrap();let old=r.self_key.unwrap();
    r.wire_named_appearance(old,char_id(uid),Some("Self".into()),ev(11),(1,100)).unwrap();
    r.tracking.transport_closed((1,100),ev(12)).unwrap();let new=r.observe(1,1,3).unwrap();
    r.wire_named_appearance(new,char_id(uid),Some("Self".into()),ev(13),(1,200)).unwrap();
    assert!(r.personal_actor(1,1,(1,100),p,ev(20),at(20)).unwrap().is_none());assert_eq!(r.self_key,Some(new));
}
#[test] fn peer_appearance_before_self_handoff_keeps_the_same_participant_and_first_close_proof(){
    for personal_snapshot in [false,true]{
    let mut r=Registry::default();let me=char_id(0x0012_3456_0001);let peer=char_id(0x0012_3466_0001);
    r.wire_personal(1,100,me.clone(),ev(1)).unwrap();let old_self=r.observe(1,1,50).unwrap();
    r.wire_named_appearance(old_self,me.clone(),Some("Self".into()),ev(2),(1,100)).unwrap();
    let old_peer=r.observe(1,1,60).unwrap();r.wire_named_appearance(old_peer,peer.clone(),Some("Peer".into()),ev(3),(1,100)).unwrap();let participant=r.participant(old_peer);
    r.tracking.transport_closed((1,100),ev(4)).unwrap();r.tracking.transport_closed((1,100),ev(10)).unwrap();
    let new_peer=r.observe(1,1,6).unwrap();r.wire_named_appearance(new_peer,peer,None,ev(5),(1,200)).unwrap();
    if personal_snapshot{r.wire_personal(1,200,me.clone(),ev(6)).unwrap();}
    let new_self=r.observe(1,1,5).unwrap();r.wire_named_appearance(new_self,me,None,ev(6),(1,200)).unwrap();
    assert_eq!(r.self_key,Some(new_self));assert_eq!(r.participant(new_peer),participant);assert!(!r.is_current(old_peer));
    assert_eq!(r.remembered_players(),2);
    }
}
#[test] fn matching_six_byte_codes_do_not_override_two_conflicting_verified_full_uids(){
    let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;let uid=0x0012_3456_0001u64;let different=uid|(1u64<<48);
    feed(&mut r,&mut seq,10,20590,&personal(50,uid,80),1);feed(&mut r,&mut seq,10,20559,&profile(50,uid,"First"),2);
    let first=r.meter.registry.self_key.unwrap();
    feed(&mut r,&mut seq,10,20559,&profile(3,different,"Second"),3);
    feed(&mut r,&mut seq,10,20590,&personal(3,uid,128),4);let second=r.meter.registry.self_key.unwrap();
    assert_eq!(second.actor,3);assert_eq!(r.meter.registry.records[&second].uid,Some(different));
    assert!(!r.meter.registry.is_current(first));
    assert_ne!(r.meter.registry.participant(first),r.meter.registry.participant(second));assert!(r.meter.registry.label(second).starts_with("Second"));
}
#[test] fn new_personal_incarnation_does_not_inherit_a_full_uid_until_an_independent_join(){
    let mut r=Replay::new(true);r.live_scope=true;let uid=0x0012_3456_0001;let mut seq=1;let mut next=1;
    feed(&mut r,&mut seq,10,20590,&personal(50,uid,80),1);feed(&mut r,&mut seq,10,20559,&profile(50,uid,"Self"),2);
    feed(&mut r,&mut next,20,20590,&personal(3,uid,128),3);let key=r.meter.registry.self_key.unwrap();
    assert_eq!(r.meter.registry.records[&key].uid,None);assert!(r.meter.registry.label(key).starts_with("Self"));
    feed(&mut r,&mut next,20,20559,&profile(3,uid,"Self"),4);assert_eq!(r.meter.registry.records[&key].uid,Some(uid));
}
#[test] fn conflicting_fresh_label_cannot_fall_back_to_a_historical_alias_name(){
    let mut r=Registry::default();let c=char_id(0x0012_3456_0001);let old=r.observe(1,1,50).unwrap();
    r.wire_named_appearance(old,c.clone(),Some("First".into()),ev(1),(1,100)).unwrap();r.retire(old).unwrap();let new=r.observe(1,1,3).unwrap();
    r.wire_named_appearance(new,c.clone(),None,ev(2),(1,100)).unwrap();assert!(r.label(new).starts_with("First"));
    r.appearance(1,1,identity::Appearance{actor:3,name:"Different".into(),uid:c.uid,template:None,offset:0,record_reference:None}).unwrap();
    assert!(r.records[&new].conflicting);assert!(r.label(new).starts_with("Name pending"));
}
#[test]
fn recovered_old_personal_state_cannot_reactivate_a_superseded_actor(){
    let mut r=Registry::default();let uid=0x0012_3456_0001;
    let at=|time|Stamp{process:1,context:1,continuity:0,frame:time,order:time,timestamp_ns:time,tcp_byte:0};
    let p=identity::personal::parse(&personal(50,uid,80)).unwrap();r.personal_actor(1,1,(1,100),p,ev(10),at(10)).unwrap();
    let p=identity::personal::parse(&personal(3,uid,90)).unwrap();r.personal_actor(1,1,(1,200),p,ev(20),at(20)).unwrap();
    let latest=r.self_key.unwrap();let p=identity::personal::parse(&personal(50,uid,80)).unwrap();
    let stale=Evidence{frame:30,timestamp_ns:15,origin:EvidenceOrigin::Packet{wire:20590}};
    assert!(r.personal_actor(1,1,(1,100),p,stale,at(15)).unwrap().is_none());assert_eq!(r.self_key,Some(latest));assert_eq!(r.remembered_players(),1);
}
#[test] fn verified_room_transition_clears_matching_once_and_keeps_actor_keys(){
    let mut r=Replay::new(true);r.live_scope=true;let mut seq=1;let key=r.meter.registry.observe(0,1,10).unwrap();r.meter.registry.assign(Some(key)).unwrap();
    let mut hp=vec![0;21];put64(&mut hp,0,99);put32(&mut hp,8,8);put64(&mut hp,12,100);feed(&mut r,&mut seq,10,170205,&hp,0);
    let mut a=vec![0;35];put32(&mut a,0,10);put32(&mut a,8,99);feed(&mut r,&mut seq,10,20526,&a,0);
    r.internal_transition(100).unwrap();assert_eq!(r.meter.registry.self_key,Some(key));assert!(r.meter.registry.is_current(key));
    put64(&mut hp,12,90);feed(&mut r,&mut seq,10,170205,&hp,0);r.internal_transition(100).unwrap();
    put64(&mut hp,12,80);feed(&mut r,&mut seq,10,170205,&hp,0);feed(&mut r,&mut seq,10,20526,&a,0);r.finish();
    assert_eq!(r.health.loss,20);assert_eq!(r.meter.session.estimated_total.damage,20);assert_eq!(r.meter.binding,1);
}
#[test] fn changed_actor_in_same_socket_needs_a_verified_room_boundary_and_stable_identity(){
    let mut m=Meter::default();let c=char_id(0x0012_3456_0001);let old=m.registry.observe(1,1,50).unwrap();
    m.registry.wire_named_appearance(old,c.clone(),Some("Peer".into()),ev(1),(1,100)).unwrap();let participant=m.registry.participant(old);
    m.event(Event::InternalTransition{evidence:10}).unwrap();let new=m.registry.observe(1,1,3).unwrap();
    m.registry.wire_named_appearance(new,c.clone(),None,ev(2),(1,100)).unwrap();assert!(!m.registry.is_current(old));assert_eq!(m.registry.participant(new),participant);
    m.event(Event::InternalTransition{evidence:11}).unwrap();
    m.registry.note_observed(new,Stamp{process:1,context:1,continuity:0,frame:3,order:3,timestamp_ns:3,tcp_byte:0});
    let concurrent=m.registry.observe(1,1,4).unwrap();m.registry.wire_named_appearance(concurrent,c,None,ev(4),(1,100)).unwrap();
    assert!(m.registry.is_current(new));assert_ne!(m.registry.group(new),m.registry.group(concurrent));
}
#[test]
#[ignore="private eight personal snapshots, profile envelopes and raid-entry TCP"]
fn private_personal_grammars_and_real_gap_recover_all_raid_players_before_combat(){
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/test-fixtures/actor-continuity-20261106.json");
    let fixture:serde_json::Value=serde_json::from_slice(&std::fs::read(root).unwrap()).unwrap();let mut offsets=std::collections::BTreeSet::new();
    let mut without_appearances=Replay::new(true);without_appearances.live_scope=true;let mut participant=None;
    for (i,case) in fixture["personals"].as_array().unwrap().iter().enumerate(){
        let p=capture_file::unhex(case["payload_hex"].as_str().unwrap()).unwrap();let parsed=identity::personal::parse(&p).unwrap();
        assert_eq!(parsed.actor as u64,case["actor"].as_u64().unwrap());assert_eq!(parsed.pets.len(),2);offsets.insert(parsed.offset);
        feed(&mut without_appearances,&mut 1,100+i as u16,20590,&p,i as u64);
        let key=without_appearances.meter.registry.self_key.unwrap();assert_eq!(key.actor,parsed.actor);
        let id=without_appearances.meter.registry.participant(key);assert_eq!(id,*participant.get_or_insert(id));
        assert_eq!(without_appearances.meter.registry.tracking.personal.as_ref().unwrap().uid,None);
        assert_eq!(without_appearances.meter.registry.remembered_players(),1);
    }
    assert!(offsets.len()>=4);
    let mut peers=std::collections::BTreeMap::new();for case in fixture["players"].as_array().unwrap(){let a=identity::score2_layout::parse_jp_observed(&capture_file::unhex(case["payload_hex"].as_str().unwrap()).unwrap()).unwrap();peers.insert(a.identity.uid.unwrap(),a.actor);}
    for case in fixture["profiles"].as_array().unwrap(){let p=identity::profile::parse(&capture_file::unhex(case["payload_hex"].as_str().unwrap()).unwrap()).unwrap();assert_eq!(peers[&p.uid],p.actor);}
    let mut r=Replay::new(false);r.live_scope=true;let mut before_timeout=false;
    for s in fixture["entry_segments"].as_array().unwrap(){r.feed(capture_file::json_record(s.to_string().as_bytes()).unwrap()).unwrap();
        if s["timestamp_ns"].as_u64().unwrap()<1793958180000000000 && r.meter.registry.self_key.is_some(){before_timeout=true;assert_eq!(r.meter.registry.self_key.unwrap().actor,3386257);}}
    assert!(before_timeout);assert_eq!(r.diagnostics.0["tcp_gap_recovered"],1);assert_eq!(r.diagnostics.0["structured_player_appearance"],8);
    assert_eq!(r.meter.registry.remembered_players(),8);assert_eq!(r.meter.registry.self_key.unwrap().actor,3386257);
    let me=r.meter.registry.self_key.unwrap();assert_eq!(r.meter.registry.timing[&me].received.unwrap().timestamp_ns,1793958177415925900);
}
