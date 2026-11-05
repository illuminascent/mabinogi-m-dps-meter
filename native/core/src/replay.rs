use crate::{capture_file::{self,Input}, identity, meter::{Event,Hit,Meter,Snapshot},
    protocol, transport::{Flow,Segment,TcpStream,GLOBAL_BUFFER}, Diagnostics, Result};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,VecDeque}, path::Path};

#[derive(Clone)]
pub struct Evidence {
    pub timestamp_ns: u64,pub frame: u64,pub wire: Option<i32>,pub compression: u8,
    pub flow: Flow,pub payload: Vec<u8>,pub payload_length: usize,pub decoded: bool,
}
pub struct Replay {
    pub meter: Meter, pub diagnostics: Diagnostics, pub segments: u64, pub frames: u64,
    streams: BTreeMap<Flow,TcpStream>, connections: BTreeMap<[(String,u16);2],u64>,
    syns: BTreeMap<[(String,u16);2],(Flow,u32)>,
    last_timestamp: Option<u64>, pub evidence: VecDeque<Evidence>,
    pub frame_history: VecDeque<Evidence>, history_bytes: usize,
    aligned: bool, finished: bool, buffered: usize,
    health:crate::health::Health,
    process:u64,
    pub live_scope:bool,
}
impl Replay {
    pub fn new(aligned: bool) -> Self {
        Self { meter: Meter::default(),diagnostics: Diagnostics::default(),segments: 0,frames: 0,
            streams: BTreeMap::new(),connections: BTreeMap::new(),syns: BTreeMap::new(),
            last_timestamp: None,evidence: VecDeque::new(),frame_history: VecDeque::new(),history_bytes: 0,
            aligned,finished: false,buffered: 0,health:crate::health::Health::default(),process:0,live_scope:false }
    }
    pub fn feed(&mut self, input: Input) -> Result<()> {
        if self.finished { return Err("replay already finished".into()); }
        let s = match input {
            Input::Loss { code,reset } => {
                self.diagnostics.report(&code); self.meter.event(Event::Gap)?;
                if reset { self.streams.clear(); self.buffered = 0; self.syns.clear();self.health.clear(); } return Ok(());
            }
            Input::Segment(s) => s,
        };
        if self.last_timestamp.is_some_and(|t| s.timestamp_ns < t) { return Err("capture timestamps moved backwards".into()); }
        self.last_timestamp = Some(s.timestamp_ns); self.segments += 1;
        let connection = s.flow.connection();
        if !self.connections.contains_key(&connection) {
            if self.connections.len() >= 4096 { return Err("connection capacity reached".into()); }
            self.connections.insert(connection.clone(),self.connections.len() as u64+1);
        }
        let lineage = self.connections[&connection];
        if s.syn && !s.ack && self.syns.get(&connection) != Some(&(s.flow.clone(),s.seq)) {
            self.health.clear_context(lineage);
            let keys: Vec<_> = self.streams.keys().filter(|k| k.connection() == connection).cloned().collect();
            if !keys.is_empty() {
                self.diagnostics.report("tcp_restart"); self.meter.registry.cache_metadata();
                for k in keys { let stream = self.streams.remove(&k).unwrap();
                    self.buffered -= stream.buffered(); stream.finish(&mut self.diagnostics); }
            }
            self.syns.insert(connection.clone(),(s.flow.clone(),s.seq));
        }
        if !self.streams.contains_key(&s.flow) {
            if self.streams.len() >= 4096 { return Err("directional stream capacity reached".into()); }
            self.streams.insert(s.flow.clone(),TcpStream::new(self.aligned));
        }
        if self.streams[&s.flow].closed && !s.syn && !s.payload.is_empty() {
            let old = self.streams.remove(&s.flow).unwrap();self.buffered -= old.buffered();
            old.finish(&mut self.diagnostics);self.meter.registry.cache_metadata();
            self.streams.insert(s.flow.clone(),TcpStream::new(self.aligned));
        }
        // Reserve worst-case incoming bytes before allocating pending/framing buffers.
        if self.buffered+s.payload.len() > GLOBAL_BUFFER { return Err("global transport capacity reached".into()); }
        let stream = self.streams.get_mut(&s.flow).unwrap();
        let loss_before=["tcp_gap_timeout","tcp_pending_overflow","alignment_lost","frame_rejected"].iter().map(|k|self.diagnostics.0.get(*k).copied().unwrap_or(0)).sum::<u64>();
        let before = stream.buffered(); let decoded = stream.feed(&s,&mut self.diagnostics)?;
        self.buffered = self.buffered-before+stream.buffered();
        let loss_after=["tcp_gap_timeout","tcp_pending_overflow","alignment_lost","frame_rejected"].iter().map(|k|self.diagnostics.0.get(*k).copied().unwrap_or(0)).sum::<u64>();
        if loss_after!=loss_before {self.health.clear_context(lineage);}
        for frame in decoded {
            self.frames += 1;
            let record = Evidence { timestamp_ns: s.timestamp_ns,frame: self.frames,wire: frame.wire,
                compression: frame.compression,flow: s.flow.clone(),payload_length: frame.payload.len(),
                payload: frame.payload.iter().take(65536).copied().collect(),decoded: frame.decoded };
            let cost = record.payload.len()+512;
            while self.history_bytes+cost > 32*1024*1024 || self.frame_history.len() >= 4096 {
                let old = self.frame_history.pop_front().unwrap(); self.history_bytes -= old.payload.len()+512;
                self.diagnostics.report("diagnostic_eviction");
            }
            self.history_bytes += cost; self.frame_history.push_back(record.clone());
            if matches!(frame.wire,Some(20054 | 20517 | 20590 | 170631)) {
                if self.evidence.len() == 128 { self.evidence.pop_front(); self.diagnostics.report("identity_evidence_eviction"); }
                self.evidence.push_back(record);
            }
            if !frame.decoded { continue; }
            match frame.wire {
                Some(21196) => match protocol::damage(&frame.payload) {
                    Ok(d) => {
                        let binding = if self.live_scope {self.meter.binding}else{(lineage << 32) | self.meter.binding};
                        let source = self.meter.registry.observe(self.process,binding,d.actor)?;
                        let target = self.meter.registry.observe(self.process,binding,d.target)?;
                        self.health.explicit_in(lineage,target);
                        self.meter.add(Hit { timestamp_ns: s.timestamp_ns,source,target,damage: d })?;
                    }
                    Err(_) => self.diagnostics.report("payload_rejected"),
                }
                Some(20526) => { match protocol::attack(&frame.payload) {
                    Ok(a) if a.actor!=0 && a.target!=0 && a.actor!=a.target=>{
                        let binding=if self.live_scope {self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let source=self.meter.registry.observe(self.process,binding,a.actor)?;
                        let target=self.meter.registry.observe(self.process,binding,a.target)?;
                        self.health.attack_in(lineage,source,target);self.meter.activity(source,target,s.timestamp_ns);
                    },_=>self.diagnostics.report("payload_rejected")
                }}
                Some(170205)=>{
                    if let Some((actor,hp))=crate::health::parse(&frame.payload) {
                        let binding=if self.live_scope {self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let target=self.meter.registry.observe(self.process,binding,actor)?;
                        if let Some(estimate)=self.health.update_in(lineage,target,hp,s.timestamp_ns)? {self.estimate(estimate)?;}
                    }else{self.diagnostics.report("payload_rejected");}
                }
                Some(20590)=>{
                    if let Some(personal)=identity::personal_prefix(&frame.payload) {
                        self.meter.registry.wire_personal(self.process,(lineage<<32)|self.meter.binding,personal,identity::tracking::Evidence {
                            frame:self.frames,timestamp_ns:s.timestamp_ns,origin:identity::tracking::EvidenceOrigin::Packet {wire:20590}
                        })?;self.diagnostics.report("personal_identity");
                    }else {self.diagnostics.report("identity_unresolved");}
                }
                Some(170631) => {
                    if let Some(n)=identity::name_record(&frame.payload) {
                        self.meter.registry.name_metadata(n.uid,n.name,identity::tracking::Evidence {
                            frame:self.frames,timestamp_ns:s.timestamp_ns,
                            origin:identity::tracking::EvidenceOrigin::Packet { wire:170631 }
                        },false)?;
                        self.diagnostics.report("name_metadata");
                    } else { self.diagnostics.report("identity_unresolved"); }
                }
                Some(20054) => {
                    if let Ok(a)=identity::score2_layout::parse_jp_observed(&frame.payload) {
                        let binding=if self.live_scope {self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let key=self.meter.registry.appearance(self.process,binding,identity::Appearance {
                            actor:a.actor,name:a.display_name.clone(),uid:a.identity.uid,template:None,
                            offset:a.uid_offset,record_reference:Some(a.outer_reference)
                        })?;
                        let source=identity::tracking::Evidence {
                            frame:self.frames,timestamp_ns:s.timestamp_ns,
                            origin:identity::tracking::EvidenceOrigin::Packet { wire:20054 }
                        };
                        self.meter.registry.wire_appearance_context(key,a.identity.clone(),source,(self.process,(lineage<<32)|self.meter.binding))?;
                        self.meter.registry.appearance_named_evidence(key,a.identity,Some(a.display_name),source)?;
                        self.diagnostics.report("structured_player_appearance");
                    } else if let Some(a)=identity::parse(20054,&frame.payload) {
                        let binding=if self.live_scope {self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        self.meter.registry.appearance(self.process,binding,a)?;
                        self.diagnostics.report("legacy_player_metadata_only");
                    } else { self.diagnostics.report("identity_unresolved"); }
                }
                Some(w @ (20517|20051)) => {
                    if let Some(a) = identity::parse(w,&frame.payload) {
                        let binding=if self.live_scope {self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        self.meter.registry.appearance(self.process,binding,a)?;
                    } else { self.diagnostics.report("identity_unresolved"); }
                }
                Some(20344 | 20552) => {
                    self.diagnostics.report("scene_marker"); self.meter.event(Event::Rebroadcast)?;
                }
                Some(_) => self.diagnostics.report("unknown_opcode"), None => (),
            }
        }
        if s.rst {
            self.health.clear_context(lineage);
            let keys: Vec<_> = self.streams.keys().filter(|k| k.connection() == connection).cloned().collect();
            for k in keys { let stream = self.streams.remove(&k).unwrap();
                self.buffered -= stream.buffered(); stream.finish(&mut self.diagnostics); }
            self.syns.remove(&connection); self.meter.registry.cache_metadata();
        }
        self.meter.partial |= self.diagnostics.partial(); Ok(())
    }
    pub fn finish(&mut self) {
        if !self.finished {
            for e in self.health.flush() {if self.estimate(e).is_err(){self.diagnostics.report("inference_capacity");}}
            for stream in self.streams.values() { stream.finish(&mut self.diagnostics); }
            self.meter.partial |= self.diagnostics.partial(); self.finished = true;
        }
    }
    fn estimate(&mut self,e:crate::health::Estimate)->Result<()> {
        if e.source==e.target{return Ok(());}
        self.meter.add_estimate(Hit {timestamp_ns:e.timestamp_ns,source:e.source,target:e.target,damage:protocol::Damage {
            actor:e.source.actor,target:e.target.actor,amount:e.amount,flags:[false;5],raw_flags:[0;7]
        }})
    }
    pub fn tick(&mut self,now:u64)->Result<()> {
        if self.meter.last_self_activity.is_some_and(|t|now.saturating_sub(t)>=self.meter.idle_ns) {
            for e in self.health.flush() {self.estimate(e)?;}
        }
        self.meter.tick(now);Ok(())
    }
    pub fn reset_inference(&mut self) {self.health.clear();}
    pub fn process_replacement(&mut self,epoch:u64)->Result<()> {
        self.health.clear();self.streams.clear();self.syns.clear();self.connections.clear();self.buffered=0;
        self.process=epoch;self.meter.event(Event::ProcessReplacement{evidence:epoch})
    }
    pub fn load(path: &Path, port: Option<u16>, aligned: bool) -> Result<Self> {
        let mut replay = Self::new(aligned); capture_file::read(path,port,|input| replay.feed(input))?;
        replay.finish(); Ok(replay)
    }
    pub fn audit(&self) -> Value {
        let s = self.meter.snapshot();
        json!({ "format": "mabi-dps-rust-replay-v1", "protocol_profile": protocol::PROFILE,
            "experimental": true,"live_acceptance": false,"partial": s.partial,
            "segments": self.segments,"frame_count": self.frames,"issues": self.diagnostics.0,
            "assume_aligned": self.aligned,"summary": snapshot_json(&s),
            "entity_scope_policy": if self.live_scope {"verified game process epoch; transport independent"}else{"offline connection lineage; no process ownership inferred"},
            "capture_totals":{"explicit_damage":self.meter.session.total.damage.to_string(),"explicit_hits":self.meter.session.total.hits,"estimated_damage":self.meter.session.estimated_total.damage.to_string(),"estimated_events":self.meter.session.estimated_total.hits},
            "encounters":self.meter.encounters.iter().map(|(id,e)|json!({"id":id,"state":format!("{:?}",e.state),"views":e.views.iter().map(|(target,v)|json!({"target":target.map(key_json),"explicit_damage":v.total.damage.to_string(),"explicit_hits":v.total.hits,"estimated_damage":v.estimated_total.damage.to_string(),"estimated_events":v.estimated_total.hits,"start_ns":v.total.start.or(v.estimated_total.start),"end_ns":v.total.end.or(v.estimated_total.end),"sources":v.sources.iter().map(|(key,a)|json!({"key":key_json(*key),"damage":a.damage.to_string(),"hits":a.hits})).collect::<Vec<_>>(),"estimates":v.estimates.iter().map(|(key,a)|json!({"key":key_json(*key),"damage":a.damage.to_string(),"events":a.hits})).collect::<Vec<_>>() })).collect::<Vec<_>>() })).collect::<Vec<_>>(),
            "raw_hits_retained": self.meter.raw_hits.len(),"raw_hits_evicted": self.meter.evicted_hits,
            "identity_evidence_retained": self.evidence.len(),
            "identity_metadata_count":self.meter.registry.profile_names.len(),
            "identity_metadata":self.meter.registry.profile_names.iter().map(|(uid,m)| json!({
                "uid":uid.to_string(),"name":m.name,"conflicting":m.conflicting,"cached":m.cached,
                "source_frame":m.source.frame,"role":"name metadata; not local-character confirmation"
            })).collect::<Vec<_>>(),
            "hits": self.meter.raw_hits.iter().map(|h| json!({"timestamp_ns": h.timestamp_ns,
                "source": key_json(h.source),"target": key_json(h.target),"amount": h.damage.amount,
                "flags": h.damage.flags,"raw_flags": hex(&h.damage.raw_flags)})).collect::<Vec<_>>(),
            "identity_evidence": self.evidence.iter().map(evidence_json).collect::<Vec<_>>(),
            "frames": self.frame_history.iter().map(evidence_json).collect::<Vec<_>>(),
            "estimated_hits":self.meter.inferred_hits.iter().map(|h|json!({"timestamp_ns":h.timestamp_ns,"source":key_json(h.source),"target":key_json(h.target),"amount":h.damage.amount,"evidence":"HP decrease and one subsequent same-target attack"})).collect::<Vec<_>>(),
            "disabled_adapters": ["JP20344/20552 transitions","unseen JP appearance profiles","skill matcher"] })
    }
}
fn hex(p: &[u8]) -> String { p.iter().map(|b| format!("{b:02X}")).collect() }
fn evidence_json(e: &Evidence) -> Value {
    json!({"timestamp_ns": e.timestamp_ns,"frame": e.frame,"wire_opcode": e.wire,
        "compression": e.compression,"payload_encoding": if e.decoded { "decoded" } else { "wire" },
        "failure": if e.decoded { None } else { Some("Brotli rejected or expansion limit") },
        "flow": [e.flow.src.clone(),e.flow.sport.to_string(),e.flow.dst.clone(),e.flow.dport.to_string()],
        "payload_length": e.payload_length,"payload_truncated": e.payload.len() < e.payload_length,
        "payload_hex": hex(&e.payload)})
}
pub fn key_json(k: identity::EntityKey) -> Value {
    json!({"process": k.process,"binding": k.binding,"actor": k.actor,"generation": k.generation})
}
pub fn snapshot_json(s: &Snapshot) -> Value {
    // Decimal strings preserve u128 values in JSON consumers without bigint support.
    json!({ "damage": s.summary.damage.to_string(),"hits": s.summary.hits,
        "start_ns": s.summary.start,"end_ns": s.summary.end,"duration_seconds": s.summary.duration(),
        "dps": s.summary.dps(),"source_count": s.source_count,"target_count": s.target_count,
        "revision": s.revision,"identity_revision": s.identity_revision,"last_command": s.last_command,
        "self_identity": {"key": s.self_binding.key.map(key_json),
            "evidence": format!("{:?}",s.self_binding.evidence),"primary_locked": s.self_binding.primary_locked,
            "voter_locked":s.self_binding.voter_locked,
            "origin":s.self_binding.source.map(|e| format!("{:?}",e.origin)),
            "source_frame": s.self_binding.source.filter(|e|matches!(e.origin,identity::tracking::EvidenceOrigin::Packet { .. })).map(|e| e.frame)},
        "actors": s.rows.iter().map(|r| json!({"key": key_json(r.key),"label": r.label,
            "damage": r.damage.to_string(),"hits": r.hits,"dps": r.dps,"share": r.share,
            "named": r.named,"is_self": r.is_self,"assignment_key": r.assignment_key.map(key_json)})).collect::<Vec<_>>() })
}
pub fn demo() -> Result<Replay> {
    let mut r = Replay::new(true); let mut seq = 1000u32;
    for (sec,source,target,amount) in [(0,101,999,1200u64),(1,102,999,800),(2,101,999,1500),
        (3,102,998,1000),(4,101,998,2000),(5,101,999,2400)] {
        let mut p = vec![0;53]; p[..4].copy_from_slice(&u32::to_le_bytes(source));
        p[8..12].copy_from_slice(&u32::to_le_bytes(target)); p[16..24].copy_from_slice(&amount.to_le_bytes());
        let payload = protocol::encode_frame(21196,&p); let n = payload.len() as u32;
        r.feed(Input::Segment(Segment { timestamp_ns: sec*1_000_000_000,
            flow: Flow { src: "192.0.2.1".into(),sport: 12345,dst: "192.0.2.2".into(),dport: 23456 },
            seq,payload,syn: false,fin: false,rst: false,ack: true }))?; seq = seq.wrapping_add(n);
    }
    r.finish(); Ok(r)
}
