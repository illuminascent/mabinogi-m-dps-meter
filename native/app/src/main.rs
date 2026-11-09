#![cfg_attr(all(windows,not(debug_assertions)),windows_subsystem="windows")]
use mabi_meter_core::{replay::{self,Replay}, Result};
use std::path::PathBuf;
mod wire;
mod research;
mod diagnostics;
#[cfg(windows)] mod platform;
#[cfg(windows)] mod live;
#[cfg(windows)] mod research_window;

#[cfg(windows)]
mod window;

#[derive(Clone)]
pub enum Source { Empty, Demo, File { path: PathBuf,port: Option<u16>,aligned: bool } }
pub fn load(source: Source) -> Result<Replay> {
    match source { Source::Empty => Ok(Replay::new(false)),Source::Demo => replay::demo(),
        Source::File { path,port,aligned } => Replay::load(&path,port,aligned) }
}
fn run() -> Result<i32> {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next().unwrap_or_else(|| if cfg!(feature="research-ui") { "research-ui".into() } else { "ui".into() });
    if mode == "--version" || mode == "-V" {
        println!("{} {}",if cfg!(feature="research-ui") {"mabi-research"} else {"mabi-meter"},env!("CARGO_PKG_VERSION"));return Ok(0);
    }
    if mode == "research-ui" || mode == "--research-ui-smoke" {
        let smoke = mode == "--research-ui-smoke";
        let mut output = None;
        let report = if smoke { Some(PathBuf::from(args.next().ok_or("Research UI smoke requires a new report path")?)) } else { None };
        while let Some(arg) = args.next() {
            if smoke || arg != "--output" || output.is_some() { return Err("research-ui accepts --output DIRECTORY".into()); }
            output = Some(PathBuf::from(args.next().ok_or("--output requires DIRECTORY")?));
        }
        #[cfg(windows)] { research_window::run(output,report)?; return Ok(0); }
        #[cfg(not(windows))] { let _ = (output,report); return Err("Research UI requires Windows".into()); }
    }
    if mode == "research" {
        let mut directory = None;
        while let Some(arg) = args.next() {
            if arg != "--output" || directory.is_some() { return Err("research accepts --output DIRECTORY".into()); }
            directory = Some(PathBuf::from(args.next().ok_or("--output requires DIRECTORY")?));
        }
        #[cfg(windows)] { live::research_controller(directory)?; return Ok(0); }
        #[cfg(not(windows))] { let _ = directory; return Err("Research capture requires Windows".into()); }
    }
    if mode == "--capture-worker" {
        #[cfg(windows)] {live::worker(args.collect())?;return Ok(0);}
        #[cfg(not(windows))] {return Err("Capture requires Windows".into());}
    }
    if mode == "--check-system" {
        #[cfg(windows)] {
            mabi_capture::win::capability_check()?;let plan=platform::discover()?;
            println!("{}",serde_json::json!({"packet_monitor_api":true,"architecture":std::env::consts::ARCH,"game_process_count":plan.owners.len(),"owned_tcp_count":plan.flows.len(),"capture_started":false}));return Ok(0);
        }
        #[cfg(not(windows))] {return Err("Windows system check requires Windows".into());}
    }
    if mode=="live" {
        #[cfg(windows)] {
            if args.next().as_deref()!=Some(std::ffi::OsStr::new("--seconds")){return Err("live requires --seconds N".into());}
            let seconds=args.next().and_then(|s|s.to_str().and_then(|s|s.parse::<u64>().ok())).filter(|s|(1..=3600).contains(s)).ok_or("Capture duration must be 1..3600 seconds")?;
            if args.next().is_some(){return Err("Unexpected live option".into());}
            let (_keep,commands)=std::sync::mpsc::sync_channel(1);let (acks,_ack_rx)=std::sync::mpsc::sync_channel(1);
            let updates=std::sync::Arc::new(std::sync::Mutex::new(None));live::controller(commands,acks,updates.clone(),seconds,false)?;
            if let Some(snapshot)=updates.lock().unwrap().take(){println!("{}",serde_json::json!({"summary":replay::snapshot_json(&snapshot),"recordings":live::data_dir()?}));}return Ok(0);
        }
        #[cfg(not(windows))] {return Err("Live capture requires Windows".into());}
    }
    if mode == "--help" || mode == "-h" {
        println!("mabi-meter ui [--demo | --replay FILE] [--port N] [--assume-aligned]\n\
                  mabi-meter replay FILE [--port N] [--assume-aligned] [--audit FILE]\n\
                  mabi-meter demo\n\
                  mabi-meter --check-system\n\
                  mabi-meter --version\n\
                  mabi-meter research [--output DIRECTORY] (JSON commands on stdin)\n\
                  mabi-research research-ui [--output DIRECTORY] (native capture controls)\n\
                  UI starts live capture with administrator approval. Replay needs no elevation.");
        return Ok(0);
    }
    if mode == "demo" { println!("{}",replay::demo()?.audit()); return Ok(0); }
    if mode=="--ui-smoke" {
        let report=PathBuf::from(args.next().ok_or("UI smoke requires report path")?);
        #[cfg(windows)] {window::smoke(report)?;return Ok(0);}
        #[cfg(not(windows))] {let _=report;return Err("UI smoke requires Windows".into());}
    }
    if mode != "ui" && mode != "replay" { return Err("unknown command; use --help".into()); }
    let mut path = if mode == "replay" { Some(PathBuf::from(args.next().ok_or("replay requires FILE")?)) } else { None };
    let mut port = None; let mut aligned = false; let mut demo = false; let mut audit = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--replay") if mode == "ui" => path = Some(args.next().ok_or("--replay requires FILE")?.into()),
            Some("--demo") if mode == "ui" => demo = true,
            Some("--assume-aligned") => aligned = true,
            Some("--port") => {
                let value = args.next().ok_or("--port requires N")?;
                port = Some(value.to_str().ok_or("invalid port")?.parse::<u16>().map_err(|e| e.to_string())?);
                if port == Some(0) { return Err("port must be nonzero".into()); }
            }
            Some("--audit") if mode == "replay" => audit = Some(PathBuf::from(args.next().ok_or("--audit requires FILE")?)),
            _ => return Err(format!("unknown option: {}",arg.to_string_lossy())),
        }
    }
    if demo && path.is_some() { return Err("choose --demo or --replay".into()); }
    let source = if demo { Source::Demo } else if let Some(path) = path { Source::File { path,port,aligned } }
        else { Source::Empty };
    if mode == "ui" {
        #[cfg(windows)] { return window::run(source).map(|_| 0); }
        #[cfg(not(windows))] { return Err("native desktop requires Windows; replay works on this platform".into()); }
    }
    let replay = load(source)?; let output = serde_json::to_string_pretty(&replay.audit()).map_err(|e| e.to_string())?;
    if let Some(path) = audit {
        // Never overwrite the supplied capture by a mistaken --audit argument.
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
        file.write_all(output.as_bytes()).map_err(|e| e.to_string())?;
    }
    println!("{output}"); Ok(if replay.meter.partial { 2 } else { 0 })
}
fn main() {
    #[cfg(windows)]
    // Direct rlib linkage. This query has no capture/session side effects.
    assert_eq!(mabi_capture::mabi_capture_abi_version(),2);
    let exit = match run() { Ok(n) => n,Err(e) => { diagnostics::error("application_error",&e);eprintln!("{e}"); 1 } };
    std::process::exit(exit);
}
