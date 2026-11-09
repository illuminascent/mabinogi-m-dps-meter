//! Recorded session replay and parser-independent research export.
use crate::{capture_file::{self, Input}, identity::{self, EntityKey}, meter::{Action, Command},
    recording::{self, Record, Writer}, replay::Replay, session::Session, Result};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs::{File, OpenOptions}, io::{BufRead, BufReader, BufWriter, Read, Write}, path::Path};

fn uint(v: &Value, field: &str) -> Result<u64> { v[field].as_u64().ok_or_else(||format!("recording needs integer {field}")) }
fn boolean(v: &Value, field: &str) -> Result<bool> {v[field].as_bool().ok_or_else(||format!("recording needs boolean {field}"))}
fn key(v: &Value) -> Result<Option<EntityKey>> {
    if v.is_null() { return Ok(None); }
    Ok(Some(EntityKey {process:uint(v,"process")?,binding:uint(v,"binding")?,
        actor:u32::try_from(uint(v,"actor")?).map_err(|e|e.to_string())?,
        generation:u32::try_from(uint(v,"generation")?).map_err(|e|e.to_string())?}))
}
pub fn command_json(c: &Command) -> Value {
    let (action,value)=match &c.action {
        Action::Target(k)=>("target",k.map(crate::replay::key_json).unwrap_or(Value::Null)),
        Action::TargetSearch(q)=>("target_search",json!(q)),
        Action::Encounter(n)=>("encounter",json!(n)),Action::IdleSeconds(n)=>("idle_seconds",json!(n)),
        Action::AssignSelf(k)=>("assign_self",k.map(crate::replay::key_json).unwrap_or(Value::Null)),
        Action::Reset=>("reset",Value::Null),
    };
    json!({"id":c.id,"observed_revision":c.observed_revision,"encounter":c.encounter,"action":action,"value":value})
}
fn command(v: &Value) -> Result<Command> {
    let n=||v["value"].as_u64().ok_or_else(||"invalid recorded command value".to_string());
    let action=match v["action"].as_str() {
        Some("target")=>Action::Target(key(&v["value"])?),
        Some("assign_self")=>Action::AssignSelf(key(&v["value"])?),
        Some("encounter")=>Action::Encounter(n()?),Some("idle_seconds")=>Action::IdleSeconds(n()?),
        Some("target_search")=>Action::TargetSearch(v["value"].as_str().ok_or("invalid recorded search")?.into()),
        Some("reset")=>Action::Reset,_=>return Err("unsupported recorded command".into()),
    };
    Ok(Command {id:uint(v,"id")?,observed_revision:uint(v,"observed_revision")?,encounter:uint(v,"encounter")?,action})
}

pub fn start(replay: &Replay, now: u64, version: &str, source: &str) -> Value {
    json!({"type":"session_start","format":"mabi-meter-session-v1","mode":"session",
        "started_ns":now,"app_version":version,"source_fingerprint":source,"protocol_profile":crate::protocol::PROFILE,
        "scope":"approved game TCP, both directions; before application framing and combat decoding",
        "live_scope":replay.live_scope,"automatic_encounters":replay.meter.automatic_encounters,
        "idle_ns":replay.meter.idle_ns,"identity_cache":identity::cache::export(&replay.meter.registry)})
}

pub fn load(path: &Path, aligned: bool) -> Result<Replay> {
    let (_,reference)=recording::resolve(path)?;
    let prefix=reference.as_ref().filter(|v|v["view_evicted"]==true)
        .map(|v|uint(v,"through_record")).transpose()?;
    let mut record_index=0;
    let mut session=Session::new(Replay::new(aligned),None);
    let mut started=false;let mut ended=false;let mut finished=false;let mut exact=false;
    let mut epoch=0;let mut had_ownership=false;
    let mut pending_ack=None;
    let summary=recording::read(path,|r| {
        record_index+=1;
        if prefix.is_some_and(|last|record_index>last) {return Ok(());}
        if ended { return Err("recording has records after session end".into()); }
        if !started && !matches!(&r,Record::Event(v) if v["type"]=="session_start") {
            return Err("recording must begin with session_start".into());
        }
        match r {
            Record::Segment {ownership_epoch,segment}=>{
                if finished {return Err("recording has traffic after decoder finish".into());}
                if exact && ownership_epoch!=epoch {return Err("recorded packet ownership epoch mismatch".into());}
                session.feed_recorded(Input::Segment(segment));
            },
            Record::Tick(ns)=>{
                if finished {return Err("recording has tick after decoder finish".into());}
                if exact {session.tick_recorded(ns);}
            },
            Record::Event(v)=>match v["type"].as_str().unwrap_or("") {
                "session_start"=>{
                    if started || v["format"]!="mabi-meter-session-v1" {return Err("unsupported/duplicate recording session header".into());}
                    started=true;
                    exact=match v["mode"].as_str() {Some("session")=>true,Some("traffic")=>false,_=>return Err("unknown recording session mode".into())};
                    if exact {
                        session.replay.live_scope=boolean(&v,"live_scope")?;
                        session.replay.meter.automatic_encounters=boolean(&v,"automatic_encounters")?;
                        let idle=uint(&v,"idle_ns")?;
                        if !(1_000_000_000..=300_000_000_000).contains(&idle) {return Err("invalid recorded idle timeout".into());}
                        session.replay.meter.idle_ns=idle;
                        identity::cache::restore(&mut session.replay.meter.registry,&v["identity_cache"])?;
                    }
                },
                "ownership" if exact=>{
                    let next=uint(&v["data"],"epoch")?;
                    if finished || next<=epoch {return Err("non-increasing recorded ownership epoch".into());}
                    if had_ownership {session.process_recorded(next);}had_ownership=true;epoch=next;
                },
                "loss"=>{
                    if finished {return Err("recording has loss after decoder finish".into());}
                    session.feed_recorded(capture_file::json_record(&serde_json::to_vec(&v).map_err(|e|e.to_string())?)?);
                },
                "command" if exact=>{
                    if finished || pending_ack.is_some() {return Err("recorded command order is invalid".into());}
                    let ack=session.command(command(&v["command"])?);
                    pending_ack=Some((ack.id,ack.applied,ack.reason));
                },
                "command_result" if exact=>{
                    let (id,applied,reason)=pending_ack.take().ok_or("recorded command result without command")?;
                    if uint(&v,"id")?!=id {return Err("recorded command result ID mismatch".into());}
                    // A changed parser may legitimately produce a different result. Retain a visible diagnostic.
                    if v["applied"].as_bool()!=Some(applied) || v["reason"]!=json!(reason) {
                        session.replay.diagnostics.report("recorded_command_result_diverged");session.replay.meter.partial=true;
                    }
                },
                "decoder_finish" if exact=>{
                    if finished {return Err("duplicate recorded decoder finish".into());}
                    if boolean(&v,"reset_inference")? {session.replay.reset_inference();}
                    session.replay.finish();finished=true;
                },
                "session_end"=>{
                    ended=true;
                    if !boolean(&v,"completed")? {session.replay.meter.partial=true;session.replay.diagnostics.report("recording_interrupted");}
                },
                // Descriptive evidence is retained/exported but cannot alter combat interpretation.
                "capture_status"|"capture_error"|"decoder_paused"|"ownership_summary"|"final_snapshot"|"marker"|"epoch_start"|"epoch_end"|"imported_row_metadata"|"scenario_observed"=>(),
                "ownership"|"command"|"command_result"|"decoder_finish" if !exact=>(),
                _=>return Err(format!("unsupported session event {}",v["type"])),
            }
        }
        Ok(())
    })?;
    if !started {return Err("recording has no session header".into());}
    if (!ended && prefix.is_none()) || summary.incomplete_tail || pending_ack.is_some() {
        session.replay.meter.partial=true;session.replay.diagnostics.report("recording_incomplete");
    }
    if !finished {session.replay.finish();}
    if prefix.is_some() {session.replay.diagnostics.report("recorded_encounter_prefix");}
    if let Some(reference)=reference {
        let id=uint(&reference,"encounter")?;
        if !session.replay.meter.encounters.contains_key(&id) {
            return Err(format!("encounter {id} is no longer retained in this session's bounded meter views; raw traffic remains exportable"));
        }
        session.replay.meter.viewing=Some(id);session.replay.meter.selected=None;
    }
    Ok(session.replay)
}

fn line(out: &mut impl Write, v: &Value) -> Result<()> {
    serde_json::to_writer(&mut *out,v).map_err(|e|e.to_string())?;
    out.write_all(b"\n").map_err(|e|e.to_string())
}
fn create(path: &Path) -> Result<BufWriter<File>> {
    OpenOptions::new().write(true).create_new(true).open(path).map(BufWriter::new).map_err(|e|e.to_string())
}

pub fn dump(input:&Path,out:&mut impl Write)->Result<recording::Summary> {
    let mut segments=0;
    recording::read(input,|r| {
        match r {
            Record::Segment {ownership_epoch,segment}=>{
                let mut v=recording::segment_json(ownership_epoch,&segment);v["type"]=json!("segment");segments+=1;line(out,&v)?;
            },
            Record::Tick(ns)=>line(out,&json!({"type":"tick","watermark_ns":ns,"after_segment":segments}))?,
            Record::Event(mut v)=>{v["after_segment"]=json!(segments);line(out,&v)?;},
        }Ok(())
    })
}

/// No combat parser is used by inspect, pack, or export.
pub fn inspect(path: &Path) -> Result<Value> {
    let (source,reference)=recording::resolve(path)?;
    let mut types=BTreeMap::<String,u64>::new();let mut start=Value::Null;let mut end=Value::Null;let mut snapshot=Value::Null;
    let summary=recording::read(path,|r| {
        if let Record::Event(v)=r {
            let kind=v["type"].as_str().unwrap().to_owned();
            if kind=="session_start" {start=v.clone();}if kind=="session_end" {end=v.clone();}
            if kind=="final_snapshot" {snapshot=v["snapshot"].clone();}
            *types.entry(kind).or_default()+=1;
        }Ok(())
    })?;
    Ok(json!({"format":"mabi-recording-v1","counts":summary.json(),"events":types,
        "session":start,"end":end,"encounter":reference,"recorded_final_snapshot":snapshot,
        "complete":!summary.incomplete_tail && end["completed"]==true,
        "context_bytes":std::fs::metadata(source).map_err(|e|e.to_string())?.len(),
        "entry_bytes":std::fs::metadata(path).map_err(|e|e.to_string())?.len()}))
}

/// Convert a legacy raw log without rewriting/deleting the input. Extra row fields
/// are retained as evidence events, so research metadata cannot silently disappear.
pub fn pack(input: &Path, output: &Path) -> Result<Value> {
    let mut reader=BufReader::new(File::open(input).map_err(|e|e.to_string())?);
    let mut out=Writer::create(output)?;
    out.event(&json!({"type":"session_start","format":"mabi-meter-session-v1","mode":"traffic",
        "source_file":input.file_name().map(|v|v.to_string_lossy()),"source_format":"tcp-jsonl",
        "scope":"imported TCP observations; original live session controls unavailable"}))?;
    loop {
        let mut bytes=Vec::new();
        (&mut reader).take((recording::MAX_BLOCK+1) as u64).read_until(b'\n',&mut bytes).map_err(|e|e.to_string())?;
        if bytes.is_empty() {break;}
        if bytes.len()>recording::MAX_BLOCK {return Err("input JSONL line exceeds recording limit".into());}
        if bytes.iter().all(u8::is_ascii_whitespace) {continue;}
        let row:Value=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
        match capture_file::json_record(&bytes)? {
            Input::Segment(s)=>{
                let epoch=match row.get("ownership_epoch") {None=>0,Some(v)=>v.as_u64().ok_or("invalid ownership_epoch")?};
                // Preserve any fields outside the established TCP JSONL schema.
                let extra:serde_json::Map<String,Value>=row.as_object().ok_or("expected input object")?.iter()
                    .filter(|(k,_)|!matches!(k.as_str(),"timestamp_ns"|"ownership_epoch"|"src"|"dst"|"sport"|"dport"|"seq"|"syn"|"fin"|"rst"|"ack"|"payload_hex"))
                    .map(|(k,v)|(k.clone(),v.clone())).collect();
                if !extra.is_empty() {out.event(&json!({"type":"imported_row_metadata","fields":extra}))?;}
                out.segment(epoch,&s)?;
            },
            Input::Loss {..}=>out.event(&row)?,
        }
    }
    out.event(&json!({"type":"session_end","completed":true,"meaning":"input file fully converted; original capture completeness unknown"}))?;
    out.sync()?;inspect(output)
}

pub fn export(input: &Path, directory: &Path) -> Result<Value> {
    let (_,reference)=recording::resolve(input)?;
    // A new directory prevents accidental destruction of accepted research evidence.
    std::fs::create_dir(directory).map_err(|e|e.to_string())?;
    let mut traffic=create(&directory.join("traffic.jsonl"))?;
    let mut events=create(&directory.join("events.jsonl"))?;
    let mut segments=0;let mut start=Value::Null;let mut end=Value::Null;
    let mut epochs=Vec::new();let mut active=Value::Null;let mut reports=0;
    let summary=recording::read(input,|r| {
        match r {
            Record::Segment {ownership_epoch,segment}=>{line(&mut traffic,&recording::segment_json(ownership_epoch,&segment))?;segments+=1;},
            Record::Tick(ns)=>line(&mut events,&json!({"type":"tick","watermark_ns":ns,"after_segment":segments}))?,
            Record::Event(mut v)=>{
                match v["type"].as_str().unwrap_or("") {
                    "session_start"=>start=v.clone(),"session_end"=>end=v.clone(),
                    "loss"=>{reports+=1;line(&mut traffic,&v)?;},
                    "epoch_start"=>active=v["epoch"].clone(),
                    "epoch_end"=>{epochs.push(v["epoch"].clone());active=Value::Null;},_=>(),
                }
                v["after_segment"]=json!(segments);line(&mut events,&v)?;
            }
        }Ok(())
    })?;
    let completed=!summary.incomplete_tail && end["completed"]==true;
    let meta=json!({"format":"mabi-research-v1","source_format":"mabi-recording-v1",
        "completed":completed,"started_ns":start["started_ns"],"ended_ns":end["observed_ns"],
        "traffic_file":"traffic.jsonl","event_file":"events.jsonl","segments":segments,
        "payload_bytes":summary.payload_bytes,"capture_reports":reports,"epochs":epochs,"active_epoch":active,
        "epoch_interval":"start_ns <= packet timestamp_ns < end_ns",
        "session":start,"end":end,"encounter":reference,"recording":summary.json(),
        "note":"Traffic is raw evidence; meter commands/ticks remain in events.jsonl. Keep all preceding traffic for warm-up."});
    if !completed {line(&mut traffic,&json!({"type":"loss","code":"recording_incomplete",
        "detail":"Missing successful session end or truncated final block","reset_streams":false}))?;}
    let mut metadata=create(&directory.join("session.meta.json"))?;
    line(&mut metadata,&meta)?;metadata.flush().map_err(|e|e.to_string())?;
    traffic.flush().and_then(|_|events.flush()).map_err(|e|e.to_string())?;
    Ok(meta)
}
