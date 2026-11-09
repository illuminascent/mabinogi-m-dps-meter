use mabi_meter_core::{capture_file::{self,Input}, identity::cache, meter::{Action,Command}, protocol,
    recording::{self,Record,Writer}, recording_session, replay::{self,Replay}, session::Session, transport::{Flow,Segment}};
use serde_json::json;
use std::{path::PathBuf,time::{SystemTime,UNIX_EPOCH}};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let p=std::env::temp_dir().join(format!("mabi-recording-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&p).unwrap();Self(p)
    }
}
impl Drop for Directory {fn drop(&mut self) {std::fs::remove_dir_all(&self.0).unwrap();}}
fn damage(seq:&mut u32,ns:u64,source:u32,target:u32,amount:u64) -> Segment {
    let mut p=vec![0;53];p[..4].copy_from_slice(&source.to_le_bytes());p[8..12].copy_from_slice(&target.to_le_bytes());
    p[16..24].copy_from_slice(&amount.to_le_bytes());let payload=protocol::encode_frame(21196,&p);
    let start=*seq;*seq=seq.wrapping_add(payload.len() as u32);
    Segment {timestamp_ns:ns,seq:start,payload,flow:Flow {src:"192.0.2.1".into(),dst:"192.0.2.2".into(),sport:12345,dport:23456},
        syn:false,fin:false,rst:false,ack:true}
}
fn logged_command(w:&mut Writer<std::fs::File>,s:&mut Session,id:u64,action:Action) {
    let command=Command {id,observed_revision:s.replay.meter.revision,encounter:s.replay.meter.snapshot().encounter,action};
    w.event(&json!({"type":"command","command":recording_session::command_json(&command)})).unwrap();
    let ack=s.command(command);
    w.event(&json!({"type":"command_result","id":ack.id,"applied":ack.applied,"reason":ack.reason})).unwrap();
}

#[test]
fn replay_reconstructs_commands_ticks_cache_process_replacement_and_entire_audit() {
    let dir=Directory::new();let path=dir.0.join("run.mrec");let mut w=Writer::create(&path).unwrap();
    let mut r=Replay::new(true);r.live_scope=true;r.meter.automatic_encounters=true;
    let mut initial=cache::export(&r.meter.registry);
    initial["names"]=json!([{"uid":"12345","name":"保存された名前","conflicting":false}]);
    cache::restore(&mut r.meter.registry,&initial).unwrap();
    w.event(&recording_session::start(&r,1,"test","test-content")).unwrap();
    let mut s=Session::new(r,None);let mut seq=1000;
    w.event(&json!({"type":"ownership","data":{"epoch":7,"owners":[[123,456]]}})).unwrap();
    for i in 0..6u64 {
        if i==4 {
            w.event(&json!({"type":"ownership","data":{"epoch":9}})).unwrap();s.process_recorded(9);seq=1000;
        }
        let p=damage(&mut seq,i*1_000_000_000,101,999,100+i);
        w.segment(if i<4 {7}else{9},&p).unwrap();s.feed_recorded(Input::Segment(p));
        let tick=i*1_000_000_000+350_000_000;w.tick(tick).unwrap();s.tick_recorded(tick);
        match i {
            0=>logged_command(&mut w,&mut s,1,Action::IdleSeconds(2)),
            1=>{
                let me=s.replay.meter.registry.records.keys().find(|k|k.actor==101).copied();
                logged_command(&mut w,&mut s,2,Action::AssignSelf(me));
                logged_command(&mut w,&mut s,3,Action::TargetSearch("日本語".into()));
            },
            2=>{logged_command(&mut w,&mut s,4,Action::Reset);logged_command(&mut w,&mut s,5,Action::IdleSeconds(0));},
            3=>{
                let loss=json!({"type":"loss","code":"test_capture_loss","detail":"synthetic loss","reset_streams":false});
                w.event(&loss).unwrap();s.feed_recorded(capture_file::json_record(loss.to_string().as_bytes()).unwrap());
            },_=>(),
        }
    }
    w.event(&json!({"type":"decoder_finish","reset_inference":false})).unwrap();s.replay.finish();
    w.event(&json!({"type":"session_end","completed":true})).unwrap();w.sync().unwrap();drop(w);
    let actual=Replay::load(&path,None,true).unwrap();
    assert_eq!(actual.audit(),s.replay.audit());
    let output=dir.0.join("research-export");let meta=recording_session::export(&path,&output).unwrap();
    assert_eq!(meta["format"],"mabi-research-v1");assert_eq!(meta["completed"],true);assert_eq!(meta["segments"],6);
    let mut raw=Vec::new();capture_file::read(&path,None,|i|{if let Input::Segment(p)=i {raw.push(recording::segment_json(0,&p));}Ok(())}).unwrap();
    let mut exported=Vec::new();capture_file::read(&output.join("traffic.jsonl"),None,|i|{if let Input::Segment(p)=i {exported.push(recording::segment_json(0,&p));}Ok(())}).unwrap();
    assert_eq!(raw,exported);
    assert!(recording_session::export(&path,&output).is_err());
}

#[test]
fn parser_failure_never_removes_subsequent_raw_observations_and_crashes_are_partial() {
    let dir=Directory::new();let path=dir.0.join("crash.mrec");let mut w=Writer::create(&path).unwrap();
    let r=Replay::new(true);w.event(&recording_session::start(&r,1,"test","test")).unwrap();let mut seq=1;
    for ns in [5,4,6] {w.segment(0,&damage(&mut seq,ns,101,999,10)).unwrap();}
    w.flush().unwrap();drop(w);
    let replay=Replay::load(&path,None,true).unwrap();assert!(replay.semantic_error.is_some());assert!(replay.meter.partial);
    assert_eq!(replay.segments,1);
    let meta=recording_session::export(&path,&dir.0.join("recovered")).unwrap();
    assert_eq!(meta["segments"],3);assert_eq!(meta["completed"],false);
    let mut count=0;recording::read(&path,|r|{if let Record::Segment {..}=r {count+=1;}Ok(())}).unwrap();assert_eq!(count,3);
}

#[test]
fn packed_legacy_traffic_preserves_bytes_duplicate_observations_metadata_and_audit() {
    let dir=Directory::new();let path=dir.0.join("old.jsonl");let packed=dir.0.join("new.mrec");let mut seq=1000;
    let mut rows=Vec::new();
    for (sec,source,target,amount) in [(0,101,999,1200),(1,102,999,800),(2,101,999,1500),(3,102,998,1000),(4,101,998,2000),(5,101,999,2400)] {
        let p=damage(&mut seq,sec*1_000_000_000,source,target,amount);rows.push(recording::segment_json(42,&p));
    }
    rows[0]["extra_research_field"]=json!({"uninterpreted":true});
    std::fs::write(&path,rows.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();
    recording_session::pack(&path,&packed).unwrap();
    assert!(recording_session::pack(&path,&packed).is_err());
    assert_eq!(Replay::load(&packed,None,true).unwrap().audit(),replay::demo().unwrap().audit());
    let info=recording_session::inspect(&packed).unwrap();assert_eq!(info["events"]["imported_row_metadata"],1);
    let mut actual=Vec::new();recording::read(&packed,|r|{if let Record::Segment {ownership_epoch,segment}=r {actual.push(recording::segment_json(ownership_epoch,&segment));}Ok(())}).unwrap();
    rows[0].as_object_mut().unwrap().remove("extra_research_field");assert_eq!(actual,rows);
}
