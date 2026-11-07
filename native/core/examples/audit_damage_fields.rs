//! Read-only, streaming field census. No allocator, spill store or live capture.
use mabi_meter_core::{capture_file,replay::Replay,Result};
use serde_json::json;
use std::path::Path;
fn main(){if let Err(e)=run(){eprintln!("{e}");std::process::exit(1);}}
fn run()->Result<()>{let inputs:Vec<_>=std::env::args().skip(1).collect();if inputs.is_empty(){return Err("usage: audit_damage_fields CAPTURE...".into());}for input in inputs{let start=std::time::Instant::now();let mut r=Replay::new(false);r.live_scope=true;capture_file::read(Path::new(&input),None,|i|r.feed(i))?;r.finish();if let Some(e)=r.semantic_error{return Err(e);}println!("{}",json!({"input":input,"frames":r.frames,"native_explicit_records":r.meter.session.total.hits,"native_raw_amount":r.meter.session.total.damage.to_string(),"fields":r.damage_wire.summary(),"elapsed_seconds":start.elapsed().as_secs_f64(),"release":"blocked","scope":"released framing plus raw known-prefix survey; not causal or all-wire completeness"}));}Ok(())}
