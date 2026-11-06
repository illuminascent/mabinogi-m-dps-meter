//! Raw research storage. No opcode, identity, HP or damage acceptance filters.
//! Epoch bounds use capture time, since worker ownership holdback delays delivery.
use mabi_meter_core::{transport::Segment, Result};
use serde_json::{json, Value};
use std::{fs::{File, OpenOptions}, io::{BufWriter, Write}, path::{Path, PathBuf}};

fn line(file: &mut BufWriter<File>, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *file, value).map_err(|e| e.to_string())?;
    file.write_all(b"\n").map_err(|e| e.to_string())
}

pub struct Recorder {
    pub directory: PathBuf,
    tcp: BufWriter<File>,
    journal: BufWriter<File>,
    started_ns: u64,
    pub segments: u64,
    pub payload_bytes: u64,
    pub reports: u64,
    active: Option<Value>,
    epochs: Vec<Value>,
    next_epoch: u64,
    last_packet_ns: Option<u64>,
}

impl Recorder {
    pub fn new(directory: &Path, started_ns: u64) -> Result<Self> {
        // Caller supplies a unique session folder. Never append to old evidence.
        std::fs::create_dir(directory).map_err(|e| e.to_string())?;
        let open = |name: &str| OpenOptions::new().write(true).create_new(true)
            .open(directory.join(name)).map(BufWriter::new).map_err(|e| e.to_string());
        let mut out = Self { directory: directory.into(), tcp: open("traffic.jsonl")?,
            journal: open("events.jsonl")?, started_ns, segments: 0, payload_bytes: 0,
            reports: 0, active: None, epochs: Vec::new(), next_epoch: 1, last_packet_ns: None };
        out.event(json!({"type":"session_start", "observed_ns":started_ns,
            "format":"mabi-research-v1", "scope":"all approved game TCP, both directions",
            "epoch_interval":"start_ns <= packet timestamp_ns < end_ns",
            "warmup":"retain all preceding traffic when replaying an epoch"}))?;
        out.save_meta(false, None, None)?;
        out.flush()?;
        Ok(out)
    }

    pub fn event(&mut self, value: Value) -> Result<()> { line(&mut self.journal, &value) }

    pub fn segment(&mut self, ownership_epoch: u64, s: &Segment) -> Result<()> {
        let payload: String = s.payload.iter().map(|b| format!("{b:02x}")).collect();
        line(&mut self.tcp, &json!({"timestamp_ns":s.timestamp_ns,
            "ownership_epoch":ownership_epoch, "src":s.flow.src, "sport":s.flow.sport,
            "dst":s.flow.dst, "dport":s.flow.dport, "seq":s.seq, "syn":s.syn,
            "fin":s.fin, "rst":s.rst, "ack":s.ack, "payload_hex":payload}))?;
        self.segments += 1;
        self.payload_bytes += s.payload.len() as u64;
        self.last_packet_ns = Some(s.timestamp_ns);
        Ok(())
    }

    pub fn loss(&mut self, detail: &str, observed_ns: u64) -> Result<()> {
        self.reports += 1;
        let code = detail.split(':').next().unwrap_or("capture_report");
        let value = json!({"type":"loss", "code":code, "detail":detail,
            "reset_streams":matches!(code,"capture_handover"|"packet_rejected"),
            "observed_ns":observed_ns, "after_segment":self.segments});
        line(&mut self.tcp, &value)?;
        self.event(value)
    }

    pub fn validate_command(value: &Value) -> Result<&str> {
        let action = value["action"].as_str().ok_or("Command requires action")?;
        if !matches!(action, "start" | "end" | "stop") { return Err("Unknown research action".into()); }
        for field in ["label", "notes"] {
            if let Some(text) = value.get(field) {
                if !text.is_string() || text.as_str().unwrap().len() > 4096 {
                    return Err(format!("{field} must be text up to 4096 UTF-8 bytes"));
                }
            }
        }
        Ok(action)
    }

    pub fn command(&mut self, value: &Value, now: u64) -> Result<()> {
        match Self::validate_command(value)? {
            "start" => {
                if self.active.is_some() { return Err("An epoch is already recording".into()); }
                if self.epochs.len() >= 4096 { return Err("Session epoch limit reached".into()); }
                let epoch = json!({"id":self.next_epoch,"start_ns":now,
                    "label":value.get("label").and_then(Value::as_str).unwrap_or(""),
                    "notes":value.get("notes").and_then(Value::as_str).unwrap_or(""),
                    "after_segment":self.segments});
                self.event(json!({"type":"epoch_start","epoch":epoch}))?;
                self.active = Some(epoch);
                self.next_epoch += 1;
            }
            "end" => {
                if self.active.is_none() { return Err("No epoch is recording".into()); }
                self.end_epoch(now, "manual", value.get("notes").and_then(Value::as_str))?;
            }
            "stop" => { self.end_epoch(now, "session_stop", None)?; }
            _ => unreachable!(),
        }
        self.save_meta(false, None, None)?;
        self.flush()
    }

    fn end_epoch(&mut self, now: u64, reason: &str, notes: Option<&str>) -> Result<()> {
        if let Some(mut epoch) = self.active.take() {
            epoch["end_ns"] = json!(now);
            epoch["end_reason"] = json!(reason);
            epoch["end_after_segment"] = json!(self.segments);
            if let Some(notes) = notes { epoch["end_notes"] = json!(notes); }
            self.event(json!({"type":"epoch_end", "epoch":epoch}))?;
            self.epochs.push(epoch);
        }
        Ok(())
    }

    pub fn status(&self, status: &str) -> Value {
        json!({"type":"status", "status":status, "directory":self.directory,
            "segments":self.segments,"payload_bytes":self.payload_bytes,
            "capture_reports":self.reports, "last_packet_ns":self.last_packet_ns,
            "active_epoch":self.active,"completed_epochs":self.epochs.len()})
    }

    pub fn flush(&mut self) -> Result<()> {
        self.tcp.flush().and_then(|_| self.journal.flush()).map_err(|e| e.to_string())
    }

    fn save_meta(&self, completed: bool, ended_ns: Option<u64>, error: Option<&str>) -> Result<()> {
        let meta = json!({"format":"mabi-research-v1","completed":completed,
            "started_ns":self.started_ns,"ended_ns":ended_ns,"error":error,
            "traffic_file":"traffic.jsonl","event_file":"events.jsonl",
            "epoch_interval":"start_ns <= packet timestamp_ns < end_ns",
            "segments":self.segments,"payload_bytes":self.payload_bytes,
            "capture_reports":self.reports,"epochs":self.epochs,"active_epoch":self.active});
        let temporary = self.directory.join("session.meta.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&meta).map_err(|e|e.to_string())?)
            .map_err(|e|e.to_string())?;
        // Windows std::fs::rename replaces existing files without a remove window.
        std::fs::rename(temporary, self.directory.join("session.meta.json")).map_err(|e|e.to_string())
    }

    pub fn finish(&mut self, now: u64, completed: bool, error: Option<&str>) -> Result<()> {
        self.end_epoch(now, if completed {"session_finished"} else {"interrupted"}, None)?;
        self.event(json!({"type":"session_end","observed_ns":now,"completed":completed,"error":error}))?;
        self.flush()?;
        self.tcp.get_ref().sync_data().and_then(|_|self.journal.get_ref().sync_data()).map_err(|e|e.to_string())?;
        self.save_meta(completed, Some(now), error)
    }
}

#[cfg(test)] mod tests {
    use super::*;
    use mabi_meter_core::{capture_file, transport::Flow};
    #[test]
    fn epochs_retain_warmup_delayed_packets_unknown_payload_and_losses() {
        let dir = std::env::temp_dir().join(format!("mabi-research-test-{}-{}",std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let mut r = Recorder::new(&dir, 1).unwrap();
        let mut s = Segment {timestamp_ns:10,flow:Flow {src:"192.0.2.1".into(),sport:1234,
            dst:"192.0.2.2".into(),dport:5678},seq:42,payload:vec![0xff,0,0x80],
            syn:false,fin:false,rst:false,ack:true};
        r.segment(7,&s).unwrap(); // warmup
        r.command(&json!({"action":"start","label":"日本語 raid","notes":"2 players"}),20).unwrap();
        assert!(r.command(&json!({"action":"start"}),21).is_err());
        s.timestamp_ns=19;r.segment(7,&s).unwrap(); // delayed pre-start traffic
        r.command(&json!({"action":"end","notes":"expected 123 DPS"}),30).unwrap();
        s.timestamp_ns=29;
        std::mem::swap(&mut s.flow.src,&mut s.flow.dst);
        std::mem::swap(&mut s.flow.sport,&mut s.flow.dport);
        r.segment(7,&s).unwrap(); // delayed in-epoch outbound
        r.loss("native_capture_reports:2",31).unwrap();
        r.command(&json!({"action":"start","label":"next"}),40).unwrap();
        r.finish(50,false,Some("test disconnect")).unwrap();
        assert_eq!(r.segments,3);
        let meta:Value=serde_json::from_slice(&std::fs::read(dir.join("session.meta.json")).unwrap()).unwrap();
        assert_eq!(meta["epochs"][0]["start_ns"],20);
        assert_eq!(meta["epochs"][0]["end_ns"],30);
        assert_eq!(meta["epochs"][0]["end_notes"],"expected 123 DPS");
        assert_eq!(meta["epochs"][1]["end_reason"],"interrupted");
        assert_eq!(meta["completed"],false);
        let mut count=0;
        capture_file::read(&dir.join("traffic.jsonl"),None,|input| {
            if let capture_file::Input::Segment(s)=input {assert_eq!(s.payload,vec![0xff,0,0x80]);count+=1;} Ok(())
        }).unwrap();
        assert_eq!(count,3);
        let events=std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
        assert!(events.contains("epoch_start") && events.contains("native_capture_reports"));
        drop(r);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn reject_invalid_commands() {
        for value in [json!({}),json!({"action":"reset"}),json!({"action":"start","label":3}),
            json!({"action":"end","notes":"x".repeat(4097)})] {
            assert!(Recorder::validate_command(&value).is_err());
        }
    }
}
