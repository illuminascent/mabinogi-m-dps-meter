//! Recording-side encounter entries. No filename or scenario hint participates
//! in packet acceptance, encounter detection, or damage allocation.
use mabi_meter_core::{meter::EncounterState, recording::{self,Writer}, replay::Replay, transport::Segment, Result};
use serde_json::{json,Value};
use std::{collections::BTreeMap, fs::{File,OpenOptions}, path::{Path,PathBuf}};

/// Observed JP171057 prefix, deliberately separate from the damage parser. The
/// remaining body is uninterpreted and always remains in the raw recording.
fn scenario(p:&[u8])->Option<String> {
    let n=u32::from_le_bytes(p.get(..4)?.try_into().ok()?) as usize;
    if n==0 || n>512 || n%2!=0 {return None;}
    let words:Vec<u16>=p.get(4..4+n)?.chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();
    let s=String::from_utf16(&words).ok()?;
    if !s.starts_with("Scenario_") || s.len()>240 || !s.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_') {return None;}
    Some(s)
}
fn instance_hint(s:&str)->String {
    // Rooms of one scenario share a filename label; the exact original string
    // and its frame/context remain in scenario_observed events.
    s.split_once("_Room_").map_or(s,|(prefix,_)|prefix).chars().take(96).collect()
}
struct Entry { path:PathBuf, local:String, started_ns:u64, scenario:Option<String>, state:String, last_record:u64, summary:Value }
pub struct Recorder {
    pub directory:PathBuf,
    writer:Writer<File>,
    entries:BTreeMap<u64,Entry>,
    frame:u64,
    scenarios:BTreeMap<(u64,u64),String>,
    hint:Option<String>,
    records:u64,
    revision:Option<u64>,
}
impl Recorder {
    pub fn new(directory:&Path,start:&Value)->Result<Self> {
        std::fs::create_dir(directory).map_err(|e|e.to_string())?;
        let mut out=Self {directory:directory.into(),writer:Writer::create(&directory.join("context.mrec"))?,
            entries:BTreeMap::new(),frame:0,scenarios:BTreeMap::new(),hint:None,records:0,revision:None};
        out.event(start)?;out.sync()?;Ok(out)
    }
    pub fn event(&mut self,v:&Value)->Result<()> {self.writer.event(v)?;self.records+=1;Ok(())}
    pub fn segment(&mut self,epoch:u64,s:&Segment)->Result<()> {self.writer.segment(epoch,s)?;self.records+=1;Ok(())}
    pub fn tick(&mut self,ns:u64)->Result<()> {self.writer.tick(ns)?;self.records+=1;Ok(())}
    pub fn flush(&mut self)->Result<()> {self.writer.flush()}
    pub fn sync(&mut self)->Result<()> {self.writer.sync()}
    fn save(&self,id:u64,entry:&Entry,completed:bool,evicted:bool)->Result<()> {
        let temporary=self.directory.join(format!(".encounter-{id}.tmp"));
        let mut f=OpenOptions::new().write(true).create(true).truncate(true).open(&temporary).map_err(|e|e.to_string())?;
        recording::write_reference(&mut f,&json!({"format":"mabi-encounter-v1","source":"context.mrec","encounter":id,
            "started_ns":entry.started_ns,"os_local_timestamp":entry.local,"scenario_hint":entry.scenario,
            "scenario_meaning":"observed internal scenario prefix; not a localized or universally proven current-instance name",
            "state":entry.state,"capture_completed":completed,"view_evicted":evicted,"through_record":entry.last_record,
            "observed_summary":entry.summary}))?;
        f.sync_data().map_err(|e|e.to_string())?;drop(f);
        std::fs::rename(temporary,&entry.path).map_err(|e|e.to_string())
    }
    pub fn observe(&mut self,r:&Replay,local:&str,now:u64)->Result<()> {
        let changed_revision=self.revision!=Some(r.meter.revision);
        if self.frame==r.frames && !changed_revision {return Ok(());}
        // Only new bounded diagnostic frames are inspected; raw input was already
        // recorded, including messages the production parser failed to interpret.
        let frames:Vec<_>=r.frame_history.iter().rev().take_while(|e|e.frame>self.frame).collect();
        if let Some(newest)=frames.first() {self.frame=newest.frame;}
        for e in frames.into_iter().rev() {
            let context=(e.at.process,e.context);
            if matches!(e.wire,Some(20344|20552)) {
                self.scenarios.remove(&context);self.hint=None;
            }
            if e.decoded && e.wire==Some(171057) {
                if let Some(raw)=scenario(&e.payload) {
                    if self.scenarios.get(&context)!=Some(&raw) {
                        if self.scenarios.len()>=64 && !self.scenarios.contains_key(&context) {
                            self.scenarios.clear();
                        }
                        self.scenarios.insert(context,raw.clone());
                        self.event(&json!({"type":"scenario_observed","scenario":raw,"process":e.at.process,
                            "context":e.context,"frame":e.frame,"capture_ns":e.timestamp_ns,"wire":171057}))?;
                    }
                    self.hint=Some(instance_hint(&raw));
                }
            }
            if matches!(e.wire,Some(20526|21196|170205|170206|170207|170208|170209|170210)) {
                self.hint=self.scenarios.get(&context).map(|s|instance_hint(s));
            }
        }
        let stale:Vec<_>=self.entries.keys().filter(|id|!r.meter.encounters.contains_key(id)).copied().collect();
        for id in stale {
            if let Some(entry)=self.entries.remove(&id) {self.save(id,&entry,false,true)?;}
        }
        for (&id,encounter) in &r.meter.encounters {
            if encounter.views.is_empty() {continue;}
            let state=format!("{:?}",encounter.state);
            if !self.entries.contains_key(&id) {
                let hint=if id==r.meter.current {self.hint.clone()}else{None};
                let label=hint.as_deref().unwrap_or("unknown-instance");
                let entry=Entry {path:self.directory.join(format!("{local}_{label}_encounter-{id:04}.mrec")),
                    local:local.into(),started_ns:now,scenario:hint,state:state.clone(),last_record:self.records,summary:Value::Null};
                self.save(id,&entry,false,false)?;self.entries.insert(id,entry);
            }
            let mut entry=self.entries.remove(&id).unwrap();let old_path=entry.path.clone();let mut changed=entry.state!=state;
            entry.state=state;entry.last_record=self.records;
            if let Some(view)=encounter.views.get(&None).filter(|_|changed_revision || entry.summary.is_null()) {
                entry.summary=json!({"explicit_damage":view.total.damage.to_string(),"estimated_damage":view.estimated_total.damage.to_string(),
                    "explicit_hits":view.total.hits,"explicit_flags":view.total.flags,
                    "explicit_start_ns":view.total.start,"explicit_end_ns":view.total.end,
                    "estimated_start_ns":view.estimated_total.start,"estimated_end_ns":view.estimated_total.end});
            }
            if id==r.meter.current && entry.scenario.is_none() && encounter.state==EncounterState::Active {
                if let Some(hint)=&self.hint {
                    entry.scenario=Some(hint.clone());entry.path=self.directory.join(format!("{}_{}_encounter-{id:04}.mrec",entry.local,hint));changed=true;
                }
            }
            if changed {
                self.save(id,&entry,false,false)?;
                if old_path!=entry.path {std::fs::remove_file(old_path).map_err(|e|e.to_string())?;}
            }
            self.entries.insert(id,entry);
        }
        self.revision=Some(r.meter.revision);
        Ok(())
    }
    pub fn finish(&mut self,r:&Replay,local:&str,now:u64,completed:bool)->Result<()> {
        self.observe(r,local,now)?;self.sync()?;
        for (&id,entry) in &self.entries {self.save(id,entry,completed,false)?;}
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mabi_meter_core::{capture_file::Input,meter::{Action,Command},protocol,recording_session,session::Session,transport::Flow};
    #[test]
    fn scenario_prefix_is_bounded_and_does_not_guess_names_from_other_strings() {
        let encode=|s:&str| {let words:Vec<_>=s.encode_utf16().flat_map(|c|c.to_le_bytes()).collect();
            [(words.len() as u32).to_le_bytes().as_slice(),words.as_slice(),&[255,42]].concat()};
        let raw="Scenario_Test_Raid_Hard";assert_eq!(scenario(&encode(raw)).as_deref(),Some(raw));
        assert_eq!(instance_hint("Scenario_Test_5_1_Room_03"),"Scenario_Test_5_1");
        for text in ["Player Name","Scenario_../wrong","Scenario_A\0","Scenario_A/B"] {assert!(scenario(&encode(text)).is_none());}
        let p=encode(raw);for end in 0..p.len()-2 {assert!(scenario(&p[..end]).is_none());}
        assert!(scenario(&u32::MAX.to_le_bytes()).is_none());
    }
    #[test]
    fn encounter_files_share_context_preserve_internal_names_and_replay_evicted_views() {
        let base=std::env::temp_dir().join(format!("mabi-encounter-test-{}-{}",std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&base).unwrap();let directory=base.join("session");
        let mut s=Session::new(Replay::new(true),None);
        let mut record=Recorder::new(&directory,&recording_session::start(&s.replay,1,"test","test")).unwrap();
        let local="2026-11-09_19-30-00.123";let mut seq=1;
        let mut feed=|wire,p:&[u8],s:&mut Session,record:&mut Recorder,ns| {
            let payload=protocol::encode_frame(wire,p);let begin=seq;seq+=payload.len() as u32;
            let segment=Segment {timestamp_ns:ns,seq:begin,payload,flow:Flow {src:"server".into(),dst:"client".into(),sport:16000,dport:23},syn:false,fin:false,rst:false,ack:true};
            record.segment(0,&segment).unwrap();s.feed_recorded(Input::Segment(segment));record.observe(&s.replay,local,ns).unwrap();
        };
        let name="Scenario_Test_Raid_Hard_Room_01";
        let name_bytes:Vec<_>=name.encode_utf16().flat_map(|c|c.to_le_bytes()).collect();
        let body=[(name_bytes.len() as u32).to_le_bytes().as_slice(),&name_bytes].concat();
        feed(171057,&body,&mut s,&mut record,1);
        for id in 1..=35 {
            let mut body=vec![0;53];body[..4].copy_from_slice(&101u32.to_le_bytes());body[8..12].copy_from_slice(&999u32.to_le_bytes());body[16..24].copy_from_slice(&10u64.to_le_bytes());
            feed(21196,&body,&mut s,&mut record,id*100);
            if id<35 {
                let c=Command {id,observed_revision:s.replay.meter.revision,encounter:s.replay.meter.current,action:Action::Reset};
                record.event(&json!({"type":"command","command":recording_session::command_json(&c)})).unwrap();
                let ack=s.command(c);assert!(ack.applied);
                record.event(&json!({"type":"command_result","id":ack.id,"applied":ack.applied,"reason":ack.reason})).unwrap();
                record.observe(&s.replay,local,id*100+1).unwrap();
            }
        }
        record.event(&json!({"type":"decoder_finish","reset_inference":false})).unwrap();s.replay.finish();
        record.event(&json!({"type":"session_end","completed":true})).unwrap();record.finish(&s.replay,local,9999,true).unwrap();drop(record);
        let entries:Vec<_>=std::fs::read_dir(&directory).unwrap().map(|e|e.unwrap().path()).collect();assert_eq!(entries.len(),36);
        for id in [1,35] {
            let path=directory.join(format!("{local}_Scenario_Test_Raid_Hard_encounter-{id:04}.mrec"));
            let replay=Replay::load(&path,None,true).unwrap();assert_eq!(replay.meter.snapshot().encounter,id);
            assert_eq!(replay.meter.snapshot().summary.damage,10);
            let info=recording_session::inspect(&path).unwrap();assert_eq!(info["encounter"]["scenario_hint"],"Scenario_Test_Raid_Hard");
            assert_eq!(info["counts"]["segments"],36);
            assert_eq!(info["encounter"]["observed_summary"]["explicit_damage"],"10");
        }
        let missing=base.join("missing");std::fs::create_dir(&missing).unwrap();
        let original=entries.iter().find(|p|p.file_name().unwrap().to_string_lossy().contains("encounter-0035")).unwrap();
        let copied=missing.join("entry.mrec");std::fs::copy(original,&copied).unwrap();assert!(Replay::load(&copied,None,true).is_err());
        let mut unsafe_ref=File::create(missing.join("bad.mrec")).unwrap();
        recording::write_reference(&mut unsafe_ref,&json!({"format":"mabi-encounter-v1","source":"../context.mrec","encounter":1})).unwrap();drop(unsafe_ref);
        assert!(recording::resolve(&missing.join("bad.mrec")).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }
}
