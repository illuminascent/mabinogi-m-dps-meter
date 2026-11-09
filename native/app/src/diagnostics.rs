//! Append-only application diagnostics beside the raw recordings, never UI state.
use std::{fs::{File,OpenOptions},io::Write,path::{Path,PathBuf},sync::{Mutex,OnceLock},time::{SystemTime,UNIX_EPOCH}};

fn now_ns()->u128{SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()}
struct Journal{file:File}
impl Journal{
    fn open(directory:&Path)->std::io::Result<Self>{
        std::fs::create_dir_all(directory)?;
        let path=directory.join(format!("meter-{}-{}.log.jsonl",now_ns(),std::process::id()));
        Ok(Self{file:OpenOptions::new().write(true).create_new(true).open(path)?})
    }
    fn record(&mut self,level:&str,event:&str,detail:&str)->std::io::Result<()>{
        let mut line=serde_json::to_vec(&serde_json::json!({"timestamp_ns":now_ns().to_string(),"process_id":std::process::id(),"level":level,"event":event,"detail":detail}))?;
        line.push(b'\n');self.file.write_all(&line)?;self.file.flush()
    }
}
fn directory()->std::io::Result<PathBuf>{
    let exe=std::env::current_exe()?;
    Ok(exe.parent().ok_or_else(||std::io::Error::other("No application directory"))?.join("recordings"))
}
pub fn record(level:&str,event:&str,detail:&str){
    static JOURNAL:OnceLock<Mutex<Option<Journal>>>=OnceLock::new();
    let journal=JOURNAL.get_or_init(||Mutex::new(directory().and_then(|p|Journal::open(&p)).ok()));
    let saved=journal.lock().ok().and_then(|mut log|log.as_mut().map(|log|log.record(level,event,detail).is_ok())).unwrap_or(false);
    // Diagnostic I/O failure must not stop raw recording or damage delivery.
    if !saved{eprintln!("{level}: {event}: {detail}");}
}
pub fn error(event:&str,detail:&str){record("error",event,detail);}
#[cfg(windows)]
pub fn command(ack:&mabi_meter_core::meter::Ack){
    if !ack.applied{error("command_rejected",&format!("command {}: {}",ack.id,ack.reason.as_deref().unwrap_or("Unspecified rejection")));}
    if let Some(Err(e))=&ack.persistence{error("identity_cache_save",&format!("command {}: {e}",ack.id));}
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn appends_flushed_unicode_errors_and_keeps_multiline_details_in_one_record(){
        let directory=std::env::temp_dir().join(format!("mabi-log-test-{}-{}",std::process::id(),now_ns()));
        let mut log=Journal::open(&directory).unwrap();
        log.record("error","decoder_paused","日本語\nsecond line").unwrap();log.record("error","command_rejected","reason").unwrap();
        let path=std::fs::read_dir(&directory).unwrap().next().unwrap().unwrap().path();
        let text=std::fs::read_to_string(&path).unwrap();let rows:Vec<serde_json::Value>=text.lines().map(|line|serde_json::from_str(line).unwrap()).collect();
        assert_eq!(rows.len(),2);assert_eq!(rows[0]["detail"],"日本語\nsecond line");assert_eq!(rows[1]["event"],"command_rejected");assert!(rows[0]["timestamp_ns"].as_str().unwrap().parse::<u128>().unwrap()>0);
        drop(log);std::fs::remove_file(path).unwrap();std::fs::remove_dir(directory).unwrap();
    }
}
