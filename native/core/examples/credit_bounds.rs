//! Offline storage-contract inspection; no capture or historical index.
use mabi_meter_core::{combat::{Combat,Timeline},health::Health,replay::Replay};
fn main(){let r=Replay::new(false);println!("{}",serde_json::json!({"health":Health::default().resource_summary(),"combat_ceiling":Combat::default().assignment_ceiling_bytes(),"timeline_ceiling":Timeline::default().assignment_ceiling_bytes(),"replay":r.assignment_storage()}));}
