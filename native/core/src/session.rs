//! UI-independent command/storage owner; the native desktop uses this same path.
use crate::{identity::cache,meter::{Ack,Command},replay::Replay};
use std::path::PathBuf;

pub struct Session { pub replay:Replay,pub cache_path:Option<PathBuf>,pub cache_warning:Option<String> }
impl Session {
    pub fn new(mut replay:Replay,cache_path:Option<PathBuf>)->Self {
        let mut cache_warning=None;
        if let Some(path)=cache_path.as_ref().filter(|p|p.exists()) {
            match cache::load(&mut replay.meter.registry,path) {
                Ok(restored) if restored.operator_receipt.is_some()=>cache_warning=Some("Saved marker awaits current identity evidence".into()),
                Ok(_)=>(),Err(e)=>cache_warning=Some(format!("Saved identities rejected: {e}")),
            }
        }
        Self { replay,cache_path,cache_warning }
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
