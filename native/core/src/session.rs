//! UI-independent command/storage owner; the native desktop uses this same path.
use crate::{identity::cache,meter::{Ack,Command},replay::Replay};
use std::path::PathBuf;

pub struct Session { pub replay:Replay,pub cache_path:Option<PathBuf>,pub cache_warning:Option<String>,pub decoding_error:Option<String> }
impl Session {
    pub fn new(mut replay:Replay,cache_path:Option<PathBuf>)->Self {
        let mut cache_warning=None;
        if let Some(path)=cache_path.as_ref().filter(|p|p.exists()) {
            match cache::load(&mut replay.meter.registry,path) {
                Ok(restored) if restored.operator_receipt.is_some()=>cache_warning=Some("Saved marker awaits current identity evidence".into()),
                Ok(_)=>(),Err(e)=>cache_warning=Some(format!("Saved identities rejected: {e}")),
            }
        }
        Self { replay,cache_path,cache_warning,decoding_error:None }
    }
    fn decoded(&mut self,result:crate::Result<()>){
        if let Err(error)=result{
            self.replay.reset_inference();self.replay.meter.partial=true;
            self.replay.diagnostics.report("semantic_decoder_paused");self.decoding_error=Some(error);
        }
    }
    /// The caller durably records input first. Semantic failure freezes this
    /// decoder but never aborts the recording loop or retries partial mutations.
    pub fn feed_recorded(&mut self,input:crate::capture_file::Input){
        if self.decoding_error.is_none(){let result=self.replay.feed(input);self.decoded(result);}
    }
    pub fn tick_recorded(&mut self,now:u64){
        if self.decoding_error.is_none(){let result=self.replay.tick(now);self.decoded(result);}
    }
    pub fn process_recorded(&mut self,epoch:u64){
        if self.decoding_error.is_none(){let result=self.replay.process_replacement(epoch);self.decoded(result);}
    }
    pub fn command(&mut self,command:Command)->Ack {
        let reset=matches!(command.action,crate::meter::Action::Reset);
        let persist=matches!(command.action,crate::meter::Action::AssignSelf(_));
        let mut ack=self.replay.meter.command(command);
        if reset && ack.applied {self.replay.reset_inference();}
        if ack.applied && persist { if let Some(path)=&self.cache_path {
            ack.persistence=Some(cache::save(&self.replay.meter.registry,path));
        }}
        ack
    }
}
