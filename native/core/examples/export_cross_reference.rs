//! Offline evidence export for paired captures. Never feeds a peer's observations
//! into production inference. Keep exports and character metadata private.
use mabi_meter_core::{capture_file, combat::Stamp, replay::{Replay, key_json}, Result};
use serde_json::{json, Value};
use std::{fs::OpenOptions, io::{BufWriter, Write}, path::Path};

fn hex(p: &[u8]) -> String { p.iter().map(|b|format!("{b:02X}")).collect() }
fn stamp_json(s: Stamp) -> Value { json!({"process":s.process,"continuity":s.continuity,
    "context":s.context,"frame":s.frame,"order":s.order,"capture_ns":s.timestamp_ns,"tcp_byte":s.tcp_byte}) }

fn row(out: &mut impl Write, value: Value) -> Result<()> {
    serde_json::to_writer(&mut *out, &value).map_err(|e| e.to_string())?;
    out.write_all(b"\n").map_err(|e| e.to_string())
}
#[derive(Default)]
struct Cursor { frames: u64, hp: u64, intervals: u64 }
impl Cursor {
    fn collect(&mut self, r: &Replay, out: &mut impl Write) -> Result<()> {
        let frames = r.frames - self.frames;
        if frames > r.frame_history.len() as u64 { return Err("frame export overflow".into()); }
        for e in r.frame_history.iter().skip(r.frame_history.len() - frames as usize) {
            // Complete payloads for the evidenced combat/clock families only.
            // Other families retain length/opcode; no completeness assertion.
            let body = matches!(e.wire, Some(20526 | 21196 | 170066 | 171124));
            row(out, json!({"kind":"packet","at":stamp_json(e.at),"wire":e.wire,
                "inbound":e.flow.sport==16000,"length":e.payload_length,"decoded":e.decoded,
                "payload":body.then(||hex(&e.payload)),"truncated":body && e.payload.len()!=e.payload_length}))?;
        }
        self.frames = r.frames;
        let total = r.hp_evidence.observation_evictions + r.hp_evidence.observations.len() as u64;
        let count = total - self.hp;
        if count > r.hp_evidence.observations.len() as u64 { return Err("HP export overflow".into()); }
        for o in r.hp_evidence.observations.iter().skip(r.hp_evidence.observations.len() - count as usize) {
            row(out, json!({"kind":"hp","at":stamp_json(o.at),"target":key_json(o.target),
                "hp":o.hp,"role":o.role,"conditional":o.conditional_key,"comparison":o.comparison,
                "previous":o.previous.map(|(at,hp)|json!({"at":stamp_json(at),"hp":hp}))}))?;
        }
        self.hp = total;
        let total = r.health.audit_evictions + r.health.audit.len() as u64;
        let count = total - self.intervals;
        if count > r.health.audit.len() as u64 { return Err("interval export overflow".into()); }
        for a in r.health.audit.iter().skip(r.health.audit.len() - count as usize) {
            row(out, json!({"kind":"interval","id":a.id,"at":stamp_json(a.at),"baseline":stamp_json(a.baseline),
                "target":key_json(a.target),"hp_before":a.hp_before,"hp_after":a.hp_after,
                "loss":a.loss,"explicit":a.explicit,"raw_explicit":a.raw_explicit.to_string(),
                "residual":a.residual,"unassigned":a.unassigned,"reason":a.reason,
                "mismatch":a.explicit_mismatch,"evidence":a.evidence,
                "candidates":a.candidates.iter().map(|k|key_json(*k)).collect::<Vec<_>>(),
                "allocations":a.allocations.iter().map(|e|json!({"source":key_json(e.source),"amount":e.amount,
                    "skill":e.skill,"method":e.method,"frames":e.frames})).collect::<Vec<_>>()}))?;
        }
        self.intervals = total;
        Ok(())
    }
}
fn main() { if let Err(e) = run() { eprintln!("{e}"); std::process::exit(1); } }
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len()!=3 { return Err("usage: export_cross_reference CAPTURE NEW_EVENTS.jsonl NEW_AUDIT.json".into()); }
    let mut out = BufWriter::new(OpenOptions::new().write(true).create_new(true).open(&args[1]).map_err(|e|e.to_string())?);
    let mut audit = BufWriter::new(OpenOptions::new().write(true).create_new(true).open(&args[2]).map_err(|e|e.to_string())?);
    let mut r = Replay::new(false);
    let mut cursor = Cursor::default();
    capture_file::read(Path::new(&args[0]), None, |input| { r.feed(input)?; cursor.collect(&r, &mut out) })?;
    r.finish(); cursor.collect(&r, &mut out)?;
    if let Some(error) = &r.semantic_error { return Err(error.clone()); }
    if r.numeric.detected != r.health.loss || r.health.loss != r.health.explicit+r.health.allocated+r.health.unassigned {
        return Err("HP conservation failed".into());
    }
    let obligations=r.health.attribution.audit();
    if obligations["invariant"]!=true || obligations["by_origin"]["explicit"]["invariant"]!=true
        || obligations["by_origin"]["inferred"]["invariant"]!=true { return Err("credit conservation failed".into()); }
    row(&mut audit,r.audit())?;
    out.flush().map_err(|e|e.to_string())?;
    audit.flush().map_err(|e|e.to_string())?;
    println!("{}",json!({"frames":cursor.frames,"hp_observations":cursor.hp,"intervals":cursor.intervals}));
    Ok(())
}
