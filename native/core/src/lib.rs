//! Offline-first Rust meter core. No capture initialization or UI dependencies.
pub mod capture_file;
pub mod identity;
pub mod meter;
pub mod protocol;
pub mod replay;
pub mod transport;
pub mod session;
pub mod health;
pub mod protected_hp;
pub mod combat;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Default, Debug, Clone)]
pub struct Diagnostics(pub std::collections::BTreeMap<String, u64>);

impl Diagnostics {
    pub fn report(&mut self, code: &str) {
        *self.0.entry(code.to_owned()).or_default() += 1;
    }
    pub fn partial(&self) -> bool {
        self.0.keys().any(|s| !matches!(s.as_str(),
            "retransmission" | "scene_marker" | "identity_alignment" | "unknown_opcode" |
            "diagnostic_eviction" | "identity_evidence_eviction" | "tcp_restart" | "name_metadata" |
            "personal_identity" | "structured_player_appearance" | "legacy_player_metadata_only" | "pet_ownership" |
            "personal_actor_binding" | "profile_identity" | "protected_hp_decoded" | "protected_hp_seed" |
            "protected_hp_seed_eviction" | "tcp_gap_recovered" | "stale_identity_ignored" | "actor_binding_replaced"))
    }
}
