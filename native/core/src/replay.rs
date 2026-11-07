use crate::{capture_file::{self,Input}, identity, meter::{Event,Hit,Meter,Snapshot},
    protocol, transport::{Flow,Segment,TcpStream,GLOBAL_BUFFER}, Diagnostics, Result};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,VecDeque}, path::Path};
use crate::combat::{self,Stamp,Attack,Execution,Effect};
pub const PENDING_SKILLS:usize=512;
pub const HP_PUBLICATION_CHAINS:usize=1024;

#[derive(Clone)]
pub struct Evidence {
    pub at:Stamp,
    pub tcp_byte:u64,pub context:u64,
    pub timestamp_ns: u64,pub frame: u64,pub wire: Option<i32>,pub compression: u8,
    pub flow: Flow,pub payload: Vec<u8>,pub payload_length: usize,pub decoded: bool,
}
pub struct Replay {
    pub damage_wire:crate::damage_wire::Survey,
    pub semantic_error:Option<String>,
    pub hp_evidence:crate::hp_evidence::Ledger,
    pub protected_hp:crate::protected_hp::Decoder,
    pub protected_hp_samples:VecDeque<(identity::EntityKey,Stamp,u64)>,pub protected_hp_evictions:u64,
    pub protected_hp_seeds:VecDeque<(identity::EntityKey,Stamp,usize,u64,bool)>,
    pub use_clock_bounds:bool,
    pub combat:combat::Combat,pub timeline:combat::Timeline,
    last_clock:BTreeMap<(u64,u32),u64>,
    roster_revision:u64,
    pending_skills:VecDeque<(Hit,Stamp,crate::meter::HitAssignment)>,
    pub explicit_origins:BTreeMap<&'static str,u64>,
    pub explicit_origin_examples:VecDeque<Value>,
    pub numeric:crate::numeric::Numeric,
    pub meter: Meter, pub diagnostics: Diagnostics, pub segments: u64, pub frames: u64,
    streams: BTreeMap<Flow,TcpStream>, connections: BTreeMap<[(String,u16);2],u64>,
    next_lineage:u64,fins:BTreeMap<u64,u8>,
    syns: BTreeMap<[(String,u16);2],(Flow,u32)>,
    last_timestamp: Option<u64>, pub evidence: VecDeque<Evidence>,
    pub frame_history: VecDeque<Evidence>, history_bytes: usize,
    aligned: bool, finished: bool, buffered: usize,
    pub health:crate::health::Health,
    process:u64,continuities:BTreeMap<u64,u64>,context_orders:BTreeMap<u64,u64>,
    pub live_scope:bool,
}
impl Replay {
    pub fn new(aligned: bool) -> Self {
        Self { damage_wire:crate::damage_wire::Survey::default(),semantic_error:None,hp_evidence:crate::hp_evidence::Ledger::default(),protected_hp:crate::protected_hp::Decoder::default(),protected_hp_samples:VecDeque::new(),protected_hp_evictions:0,protected_hp_seeds:VecDeque::new(),use_clock_bounds:true,explicit_origins:BTreeMap::new(),explicit_origin_examples:VecDeque::new(),numeric:crate::numeric::Numeric::default(),combat:combat::Combat::default(),timeline:combat::Timeline::default(),last_clock:BTreeMap::new(),roster_revision:u64::MAX,pending_skills:VecDeque::new(),meter: Meter::default(),diagnostics: Diagnostics::default(),segments: 0,frames: 0,
            streams: BTreeMap::new(),connections: BTreeMap::new(),next_lineage:0,fins:BTreeMap::new(),syns: BTreeMap::new(),
            last_timestamp: None,evidence: VecDeque::new(),frame_history: VecDeque::new(),history_bytes: 0,
            aligned,finished: false,buffered: 0,health:crate::health::Health::default(),process:0,continuities:BTreeMap::new(),context_orders:BTreeMap::new(),live_scope:false }
    }
    fn invalidate_context(&mut self,context:u64){
        self.revalidate_damage_support();
        *self.continuities.entry(context).or_default()+=1;
        self.protected_hp.mark_conditional(context);
        self.health.clear_context_with_model(context,&mut self.combat.damage_model);self.combat.clear_context(context);self.timeline.clear_context(context);self.last_clock.retain(|(c,_),_|*c!=context);
    }
    pub fn feed(&mut self, input: Input) -> Result<()> {
        if self.semantic_error.is_some(){return Err("replay decoder paused after semantic error".into());}
        let result=self.feed_inner(input);
        if let Err(error)=&result{self.semantic_error=Some(error.clone());self.meter.partial=true;}
        result
    }
    fn feed_inner(&mut self, input: Input) -> Result<()> {
        if self.finished { return Err("replay already finished".into()); }
        self.revalidate_damage_support();
        let s = match input {
            Input::Loss { code,reset } => {
                self.diagnostics.report(&code); self.meter.event(Event::Gap)?;
                for epoch in self.continuities.values_mut(){*epoch+=1;}self.health.clear_with_model(&mut self.combat.damage_model);self.drain_assignments()?;self.combat.clear();self.timeline.clear();self.last_clock.clear();
                for context in self.continuities.keys(){self.protected_hp.mark_conditional(*context);}
                self.finalize_skills(None)?;
                if reset { self.streams.clear(); self.buffered = 0; self.syns.clear(); } return Ok(());
            }
            Input::Segment(s) => s,
        };
        if self.last_timestamp.is_some_and(|t| s.timestamp_ns < t) { return Err("capture timestamps moved backwards".into()); }
        self.last_timestamp = Some(s.timestamp_ns); self.segments += 1;
        // Availability is monotonic input delivery time. Original per-frame
        // capture stamps remain intact, including recovered TCP suffixes.
        self.health.set_available(s.timestamp_ns);
        // Finish already mature decisions before new downstream combat can
        // advance encounter membership. Facts released together below all get
        // this same availability and are never expired between batch frames.
        let _=self.health.tick_with_model(s.timestamp_ns,&mut self.combat.damage_model);
        let connection = s.flow.connection();
        let new_syn=s.syn && !s.ack && self.syns.get(&connection)!=Some(&(s.flow.clone(),s.seq));
        let previous=self.connections.get(&connection).copied();
        if new_syn {
            if let Some(old)=previous{
                // A fresh handshake reusing a tuple supersedes its previous TCP
                // incarnation. Sequence/framing, identity context and HP keys
                // must not share a lifetime merely because the tuple is equal.
                self.invalidate_context(old*2);self.invalidate_context(old*2+1);
                self.meter.registry.tracking.transport_closed((self.process,(old<<32)|self.meter.binding),identity::tracking::Evidence{
                    frame:self.frames,timestamp_ns:s.timestamp_ns,origin:identity::tracking::EvidenceOrigin::Semantic
                })?;
                self.protected_hp.clear_context(old*2);self.protected_hp.clear_context(old*2+1);self.fins.remove(&old);
                self.hp_evidence.retire_context(old*2);self.hp_evidence.retire_context(old*2+1);
            }
            let keys: Vec<_> = self.streams.keys().filter(|k| k.connection() == connection).cloned().collect();
            if !keys.is_empty() {
                self.diagnostics.report("tcp_restart"); self.meter.registry.cache_metadata();
                for k in keys { let stream = self.streams.remove(&k).unwrap();
                    self.buffered -= stream.buffered(); stream.finish(&mut self.diagnostics); }
            }
            self.syns.insert(connection.clone(),(s.flow.clone(),s.seq));
        }
        if previous.is_none() || new_syn {
            if self.next_lineage>=4096{return Err("connection lifetime capacity reached".into());}
            self.next_lineage+=1;self.connections.insert(connection.clone(),self.next_lineage);
        }
        let lineage=self.connections[&connection];
        let direction=u64::from((s.flow.src.clone(),s.flow.sport)>(s.flow.dst.clone(),s.flow.dport));
        let context=lineage*2+direction;
        // Never invent another TCP lifetime from a late payload after close.
        // Retained streams can still fill holes before their FIN position;
        // TcpStream itself rejects duplicates once that direction has drained.
        if !s.syn && !s.payload.is_empty() && !self.streams.contains_key(&s.flow)
            && self.meter.registry.tracking.context_closed((self.process,(lineage<<32)|self.meter.binding)){
            self.diagnostics.report("closed_connection_payload");self.meter.partial=true;return Ok(());
        }
        if !self.streams.contains_key(&s.flow) {
            if self.streams.len() >= 4096 { return Err("directional stream capacity reached".into()); }
            self.streams.insert(s.flow.clone(),TcpStream::new(self.aligned));
        }
        if self.streams[&s.flow].closed && !s.syn && !s.payload.is_empty(){
            if !self.streams[&s.flow].starts_after_fin(s.seq){self.diagnostics.report("retransmission");return Ok(());}
            if self.meter.registry.tracking.context_closed((self.process,(lineage<<32)|self.meter.binding)){
                self.diagnostics.report("closed_connection_payload");self.meter.partial=true;return Ok(());
            }
            // Preserve the existing half-close refresh path for genuinely new
            // sequence space, while rejecting already consumed bytes above.
            self.invalidate_context(context);
            let old=self.streams.remove(&s.flow).unwrap();self.buffered-=old.buffered();old.finish(&mut self.diagnostics);
            self.meter.registry.cache_metadata();self.streams.insert(s.flow.clone(),TcpStream::new(self.aligned));
        }
        // Reserve worst-case incoming bytes before allocating pending/framing buffers.
        if self.buffered+s.payload.len() > GLOBAL_BUFFER { return Err("global transport capacity reached".into()); }
        let stream = self.streams.get_mut(&s.flow).unwrap();
        let loss_before=["tcp_gap_timeout","tcp_pending_overflow","alignment_lost","frame_rejected"].iter().map(|k|self.diagnostics.0.get(*k).copied().unwrap_or(0)).sum::<u64>();
        let before = stream.buffered(); let decoded = stream.feed(&s,&mut self.diagnostics)?;
        self.buffered = self.buffered-before+stream.buffered();
        let loss_after=["tcp_gap_timeout","tcp_pending_overflow","alignment_lost","frame_rejected"].iter().map(|k|self.diagnostics.0.get(*k).copied().unwrap_or(0)).sum::<u64>();
        if loss_after!=loss_before && decoded.is_empty() {self.invalidate_context(context);}
        // Acknowledge the whole extracted batch before any semantic handler can
        // fail. Unknown opcodes and non-runtime actor widths are included.
        let mut continuity=*self.continuities.entry(context).or_default();
        let order=*self.context_orders.entry(context).or_default();
        for (i,frame) in decoded.iter().enumerate(){
            if frame.gap_before{continuity+=1;}
            let at=Stamp{process:self.process,continuity,context,frame:self.frames+i as u64+1,order:order+i as u64+1,
                timestamp_ns:frame.timestamp_ns.unwrap_or(s.timestamp_ns),tcp_byte:frame.stream_byte};
            self.hp_evidence.acknowledge(at,frame.wire,&frame.payload,frame.decoded);
            if frame.wire==Some(21196){self.damage_wire.observe(at,s.timestamp_ns,&frame.payload,frame.decoded);}
            if !frame.decoded{continuity+=1;}
        }
        for frame in decoded {
            if frame.gap_before {self.invalidate_context(context);}
            let timestamp_ns=frame.timestamp_ns.unwrap_or(s.timestamp_ns);
            self.frames += 1;
            let order=self.context_orders.entry(context).or_default();*order+=1;
            let at=Stamp{process:self.process,continuity:*self.continuities.entry(context).or_default(),context,frame:self.frames,order:*order,timestamp_ns,tcp_byte:frame.stream_byte};
            self.hp_evidence.reached(at);

            let record = Evidence { at,tcp_byte:frame.stream_byte,context,timestamp_ns,frame: self.frames,wire: frame.wire,
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
            if !frame.decoded { self.invalidate_context(context);continue; }
            if let Some(w)=frame.wire{if self.timeline.observe(w,&frame.payload,at).is_err(){self.diagnostics.report("clock_metadata_degraded");}if self.use_clock_bounds && matches!(w,170066|171124){self.health.clock_context(context);}}
            self.timeline.event(at);
            let mut status_binding_failure=None;
            let status_previous=if let Some(w @ (20051|20054|20517))=frame.wire{
                let base=if w==20517{8}else{0};
                let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
                frame.payload.get(base..base+8).map(|p|protocol::u64le(p,0)).filter(|a|*a>0 && *a<=u32::MAX as u64)
                    .and_then(|actor|self.meter.registry.current_key(self.process,binding,actor as u32))
            }else{None};
            let handled=(||->Result<()>{
            match frame.wire {
                Some(21196) => {self.damage_fact(&frame.payload,at,lineage)?;}
                Some(20526) => { match protocol::attack(&frame.payload) {
                    Ok(a) if a.actor!=0 && a.target!=0=>{
                        let binding=if self.live_scope {self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let source=self.observed_entity(binding,a.actor,at);
                        let target=self.observed_entity(binding,a.target,at);
                        self.note_pair_roles(source,target,at);
                        if !self.meter.registry.damage_eligible(source,target){self.diagnostics.report("friendly_fire_attack_claim_rejected");return Ok(());}
                        let attack=Attack{source,target,key1:a.key1,key2:a.key2,meta:a.meta,at};
                        self.health.attack(attack.clone())?;self.combat.attack(attack);self.meter.activity(source,target,timestamp_ns);
                    },_=>self.diagnostics.report("payload_rejected")
                }}
                Some(w @ 170205..=170210)=>{self.hp_update(w,&frame.payload,at,lineage)?;}
                Some(20590)=>{
                    let evidence=identity::tracking::Evidence{frame:self.frames,timestamp_ns,origin:identity::tracking::EvidenceOrigin::Packet{wire:20590}};
                    if let Some(p)=identity::personal::parse(&frame.payload){
                        let previous=self.meter.registry.personal_bindings.back().map(|p|p.key);
                        let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        if let Some(key)=self.meter.registry.personal_actor(self.process,binding,(self.process,(lineage<<32)|self.meter.binding),p,evidence,at)?{
                            if previous.is_some_and(|old|old!=key){self.invalidate_context(context);self.diagnostics.report("actor_binding_replaced");}
                            self.diagnostics.report("personal_actor_binding");self.diagnostics.report("personal_identity");
                        }else{self.diagnostics.report("stale_identity_ignored");}
                    }else if let Some(personal)=identity::personal_prefix(&frame.payload) {
                        self.meter.registry.wire_personal(self.process,(lineage<<32)|self.meter.binding,personal,identity::tracking::Evidence {
                            frame:self.frames,timestamp_ns,origin:identity::tracking::EvidenceOrigin::Packet {wire:20590}
                        })?;self.diagnostics.report("personal_identity");
                    }else {self.diagnostics.report("identity_unresolved");}
                }
                Some(20559)=>{
                    if let Some(p)=identity::profile::parse(&frame.payload){
                        let character=identity::Character::score2_appearance_uid(p.uid)?;
                        let evidence=identity::tracking::Evidence{frame:self.frames,timestamp_ns,origin:identity::tracking::EvidenceOrigin::Packet{wire:20559}};
                        if self.meter.registry.tracking.stale_identity(&character,evidence,(self.process,(lineage<<32)|self.meter.binding)){self.diagnostics.report("stale_identity_ignored");return Ok(());}
                        let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        if self.meter.registry.stale_actor_identity(self.process,binding,p.actor,evidence){self.diagnostics.report("stale_identity_ignored");return Ok(());}
                        let key=self.meter.registry.appearance(self.process,binding,identity::Appearance{actor:p.actor,name:p.name.clone(),uid:Some(p.uid),template:None,offset:8,record_reference:None})?;
                        self.meter.registry.wire_named_appearance(key,character,Some(p.name),evidence,(self.process,(lineage<<32)|self.meter.binding))?;
                        self.meter.registry.note_observed(key,at);self.diagnostics.report("profile_identity");
                    }else{self.diagnostics.report("identity_unresolved");}
                }
                Some(170631) => {
                    if let Some(n)=identity::name_record(&frame.payload) {
                        self.meter.registry.name_metadata(n.uid,n.name,identity::tracking::Evidence {
                            frame:self.frames,timestamp_ns,
                            origin:identity::tracking::EvidenceOrigin::Packet { wire:170631 }
                        },false)?;
                        self.diagnostics.report("name_metadata");
                    } else { self.diagnostics.report("identity_unresolved"); }
                }
                Some(20054) => {
                    if let Ok(a)=identity::score2_layout::parse_jp_observed(&frame.payload) {
                        if self.meter.registry.tracking.stale_identity(&a.identity,identity::tracking::Evidence{frame:self.frames,timestamp_ns,origin:identity::tracking::EvidenceOrigin::Packet{wire:20054}},(self.process,(lineage<<32)|self.meter.binding)){self.diagnostics.report("stale_identity_ignored");status_binding_failure=Some("stale identity");return Ok(());}
                        let binding=if self.live_scope {self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let source=identity::tracking::Evidence{frame:self.frames,timestamp_ns,origin:identity::tracking::EvidenceOrigin::Packet{wire:20054}};
                        if self.meter.registry.stale_actor_identity(self.process,binding,a.actor,source){self.diagnostics.report("stale_identity_ignored");status_binding_failure=Some("stale identity");return Ok(());}
                        let key=self.meter.registry.appearance(self.process,binding,identity::Appearance {
                            actor:a.actor,name:a.display_name.clone(),uid:a.identity.uid,template:None,
                            offset:a.uid_offset,record_reference:Some(a.outer_reference)
                        })?;
                        let source=identity::tracking::Evidence {
                            frame:self.frames,timestamp_ns,
                            origin:identity::tracking::EvidenceOrigin::Packet { wire:20054 }
                        };
                        self.meter.registry.note_observed(key,at);
                        self.meter.registry.wire_named_appearance(key,a.identity,Some(a.display_name),source,(self.process,(lineage<<32)|self.meter.binding))?;
                        self.diagnostics.report("structured_player_appearance");
                    } else if let Some(a)=identity::parse(20054,&frame.payload) {
                        let binding=if self.live_scope {self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        if self.meter.registry.current_key(self.process,binding,a.actor).is_some_and(|key|
                            self.meter.registry.tracking.appearances.contains_key(&key) || self.meter.registry.owners.links.contains_key(&key)){
                            self.diagnostics.report("legacy_player_metadata_only");status_binding_failure=Some("weak identity");return Ok(());
                        }
                        self.meter.registry.appearance(self.process,binding,a)?;
                        self.diagnostics.report("legacy_player_metadata_only");
                    } else { self.diagnostics.report("identity_unresolved");status_binding_failure=Some("identity unresolved"); }
                }
                Some(w @ (20517|20051)) => {
                    let base=if w==20517{8}else{0};
                    let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
                    let evidence=identity::tracking::Evidence{frame:self.frames,timestamp_ns,origin:identity::tracking::EvidenceOrigin::Packet{wire:w}};
                    if let Some(raw)=frame.payload.get(base..base+8).map(|p|protocol::u64le(p,0)){
                        if raw>0 && raw<=u32::MAX as u64 && self.meter.registry.stale_actor_identity(self.process,binding,raw as u32,evidence){
                            self.diagnostics.report("stale_identity_ignored");status_binding_failure=Some("stale identity");return Ok(());
                        }
                    }
                    if let Some(spawn)=identity::ownership::parse(w,&frame.payload){
                        let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let key=self.meter.registry.owned_spawn(self.process,binding,spawn,identity::tracking::Evidence{frame:self.frames,timestamp_ns,origin:identity::tracking::EvidenceOrigin::Packet{wire:w}})?;
                        self.meter.registry.note_observed(key,at);if let Some(owner)=self.meter.registry.owner(key){self.meter.registry.note_observed(owner,at);}
                        self.diagnostics.report("pet_ownership");
                    }else if let Some(a) = identity::parse(w,&frame.payload) {
                        let binding=if self.live_scope {self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let key=self.meter.registry.appearance(self.process,binding,a)?;self.meter.registry.note_observed(key,at);
                    } else { self.diagnostics.report("identity_unresolved");status_binding_failure=Some("identity unresolved"); }
                }
                Some(170064)=>{
                    if let Some(a)=combat::action(&frame.payload){
                        let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let source=self.observed_entity(binding,a.actor,at);
                        self.combat.execution(Execution{source,skill:a.skill,action:Some(a.name),variant:Some(a.tail),at,confirmed:false})?;
                        self.meter.skill_names=self.combat.names.clone();
                    }else{self.diagnostics.report("payload_rejected");}
                }
                Some(170066) if frame.payload.len()==24=>{
                    let a=protocol::u64le(&frame.payload,0);
                    if a>0 && a<=u32::MAX as u64{
                        if !self.last_clock.contains_key(&(context,a as u32)) && self.last_clock.len()>=32768{return Err("action clock capacity reached".into());}
                        self.last_clock.insert((context,a as u32),at.order);
                    }
                }
                Some(171117) if frame.payload.len()==20=>{
                    let a=protocol::u64le(&frame.payload,0);
                    if a>0 && a<=u32::MAX as u64{
                        let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let source=self.meter.registry.observe_at(self.process,binding,a as u32,at)?;
                        self.combat.state(source,protocol::u32le(&frame.payload,12),at,self.last_clock.get(&(context,a as u32)).copied());
                    }
                }
                Some(170069|170071) if frame.payload.len()==36=>{
                    let p=&frame.payload;let affected=protocol::u64le(p,0);let source=protocol::u64le(p,28);
                    if affected>0 && affected<=u32::MAX as u64 && (source<=u32::MAX as u64 || source==u64::MAX){
                        let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let affected=self.meter.registry.observe_at(self.process,binding,affected as u32,at)?;
                        let source=if source>0 && source<=u32::MAX as u64{Some(self.meter.registry.observe_at(self.process,binding,source as u32,at)?)}else{None};
                        match self.combat.effect(Effect{affected,instance:protocol::u64le(p,8),id:protocol::u32le(p,16),time_bits:protocol::u32le(p,20),stacks:protocol::u32le(p,24),source,first:at,last:at}){
                            Ok(())=>(),Err(e) if e=="effect instance conflict"=>self.diagnostics.report("effect_instance_conflict"),Err(e)=>return Err(e),
                        }
                    }
                }
                Some(170070) if frame.payload.len()==16=>{
                    let a=protocol::u64le(&frame.payload,0);
                    if a>0 && a<=u32::MAX as u64{
                        let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let affected=self.meter.registry.observe(self.process,binding,a as u32)?;
                        self.combat.remove_at(affected,protocol::u64le(&frame.payload,8),at);
                    }
                }
                Some(20684|20686|20687|171124|170197)=>{},
                Some(170822) if frame.payload.len()==20=>{
                    let actor=protocol::u64le(&frame.payload,0);
                    if actor>0 && actor<=u32::MAX as u64 && [8,12,16].iter().all(|n|f32::from_bits(protocol::u32le(&frame.payload,*n)).is_finite()){
                        let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        self.meter.registry.observe_at(self.process,binding,actor as u32,at)?;
                    }else{self.diagnostics.report("payload_rejected");}
                }
                Some(20344 | 20552) => {
                    self.diagnostics.report("scene_marker"); self.meter.event(Event::Rebroadcast)?;
                }
                Some(_) => self.diagnostics.report("unknown_opcode"), None => (),
            }
                Ok(())
            })();
            if let Err(error)=handled{self.diagnostics.report("semantic_enrichment_degraded");self.hp_evidence.outcome(at.frame,&format!("enrichment failed; numeric/credit processing continues: {error}"));self.meter.partial=true;}
            self.revalidate_damage_support();
            // Appearance/ownership adjudication, including generation changes,
            // completes before binding the common status from this same frame.
            if let Some(old)=status_previous.filter(|old|!self.meter.registry.is_current(*old)){
                // A proven retired generation is never selected by update
                // dispatch again. Raw/stale status evidence remains in ledger.
                self.protected_hp.forget(old);self.health.retire_target(old);self.hp_evidence.retire_target(old);self.numeric.retire(old);
            }
            if let Some(w @ (20051|20054|20517))=frame.wire{self.hp_status(w,&frame.payload,at,lineage,status_binding_failure)?;}
        }
        if s.fin{*self.fins.entry(lineage).or_default()|=1<<direction;}
        if s.rst || self.fins.get(&lineage)==Some(&3) {
            self.meter.registry.tracking.transport_closed((self.process,(lineage<<32)|self.meter.binding),identity::tracking::Evidence {
                frame:self.frames,timestamp_ns:s.timestamp_ns,origin:identity::tracking::EvidenceOrigin::Semantic
            })?;
        }
        if s.rst {
            self.invalidate_context(lineage*2);self.invalidate_context(lineage*2+1);
            let keys: Vec<_> = self.streams.keys().filter(|k| k.connection() == connection).cloned().collect();
            for k in keys { let stream = self.streams.remove(&k).unwrap();
                self.buffered -= stream.buffered(); stream.finish(&mut self.diagnostics); }
            self.syns.remove(&connection); self.meter.registry.cache_metadata();
        }
        // Closure summaries alone are insufficient: a FIN can arrive before
        // older hole-filled bytes. Reclaim only after both directions drain,
        // or RST has abandoned their buffers, or a fresh SYN supersedes them.
        if s.rst || (self.fins.get(&lineage)==Some(&3) && self.streams.iter().filter(|(flow,_)|flow.connection()==connection).all(|(_,stream)|stream.closed && stream.buffered()==0)){
            for c in [lineage*2,lineage*2+1]{self.protected_hp.clear_context(c);self.hp_evidence.retire_context(c);}
        }
        // Process companions from an entire released batch before committing.
        // Expiry is indexed by availability, not a sweep per unrelated frame.
        if self.use_clock_bounds{self.health.constrain(&self.timeline);}
        let _=self.health.tick_with_model(s.timestamp_ns,&mut self.combat.damage_model);
        self.finalize_skills(Some(s.timestamp_ns))?;
        self.drain_assignments()?;
        self.meter.partial |= self.diagnostics.partial(); Ok(())
    }
    fn hp_quarantine(&mut self,context:u64,binding:u64,actor:Option<u64>,discard_iv:bool){
        if let Some(actor)=actor.filter(|a|*a>0 && *a<=u32::MAX as u64){
            if let Some(target)=self.meter.registry.current_key(self.process,binding,actor as u32){
                self.health.clear_target_with_model(context,target,&mut self.combat.damage_model);
                self.hp_evidence.break_target(context,target);
                if discard_iv{self.protected_hp.forget_at(context,target);self.hp_evidence.forget(context,target);}
            }
        }else{self.invalidate_context(context);}
    }
    fn hp_numeric(&mut self,target:identity::EntityKey,hp:u64,at:Stamp,role:&'static str)->Result<()> {
        let conditional=self.protected_hp.conditional(at.context,target);
        let may_train=!role.starts_with("protected") || !conditional;
        self.hp_numeric_fact(target,target.actor as u64,hp,at,role,may_train)
    }
    fn hp_numeric_fact(&mut self,target:identity::EntityKey,wire:u64,hp:u64,at:Stamp,role:&'static str,may_train:bool)->Result<()> {
        self.sync_roster(at);
        let kind=self.actor_kind(target);
        self.health.note_actor_kind(target,kind,at);
        self.hp_evidence.observe(target,hp,at,role,!may_train);
        let local_prior=self.hp_evidence.observations.back().and_then(|o|o.previous);
        let decision=self.numeric.observe(target,wire,hp,at,local_prior);
        match decision.disposition{
            crate::numeric::Disposition::Sentinel=>{self.hp_evidence.outcome(at.frame,"MAX sentinel; finite comparisons and obligations retained");self.health.sentinel(target,at);return Ok(());}
            crate::numeric::Disposition::ParallelOverlap=>{self.diagnostics.report("parallel_hp_observation_already_covered");self.hp_evidence.outcome(at.frame,"parallel overlapping HP observation; unique lower endpoints remain attributable");return Ok(());}
            _=>(),
        }
        self.hp_evidence.outcome(at.frame,"numeric HP admitted to mandatory attribution seam");
        self.health.accept_numeric(target,hp,at,decision.previous,may_train && !decision.uncertain);
        Ok(())
    }
    fn hp_update(&mut self,wire:i32,p:&[u8],at:Stamp,lineage:u64)->Result<()> {
        let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
        let actor=p.get(..8).map(|p|protocol::u64le(p,0));
        let Some(envelope)=crate::hp_wire::envelope(p) else{
            self.hp_evidence.outcome(at.frame,"unsupported HP envelope; raw retained");self.diagnostics.report("payload_rejected");
            self.hp_quarantine(at.context,binding,actor,false);return Ok(());
        };
        if envelope.actor==0 || envelope.actor>u32::MAX as u64{
            if envelope.blob.len()==8{let hp=protocol::u64le(envelope.blob,0);if hp<=i64::MAX as u64{
                self.hp_evidence.unbound(at,envelope.actor,hp,"numeric target identity outside registry detail domain");
                let target=identity::EntityKey{process:self.process,binding,actor:0,generation:0};
                return self.hp_numeric_fact(target,envelope.actor,hp,at,"wire-unbound plain update",false);
            }}
            self.hp_evidence.outcome(at.frame,"protected target outside supported codec domain; no numeric value decoded");return Ok(());
        }
        if self.hp_evidence.blocked(at.context,envelope.actor as u32) && envelope.blob.len()==8{
            let hp=protocol::u64le(envelope.blob,0);
            if hp<=i64::MAX as u64{
                self.hp_evidence.unbound(at,envelope.actor,hp,"identity enrichment unresolved; numeric obligation retained");
                let target=self.observed_entity(binding,envelope.actor as u32,at);
                return self.hp_numeric_fact(target,envelope.actor,hp,at,"unbound plain update",false);
            }
        }
        // Codec selection follows the shared blob geometry, not six separate
        // opcode layouts. Unknown opcodes are acknowledged but not asserted HP.
        if !matches!(envelope.blob.len(),8|16){
            self.hp_evidence.outcome(at.frame,"unsupported HP scalar size; raw retained");
            self.hp_quarantine(at.context,binding,actor,false);return Ok(());
        }
        let protected=envelope.blob.len()==16;
        let plain=if !protected{
            let hp=protocol::u64le(envelope.blob,0);
            if hp<=i64::MAX as u64{Ok(hp)}else{Err(crate::protected_hp::DecodeError::OutOfRange)}
        }else{Ok(0)};
        if let Err(error)=plain{
            self.hp_evidence.outcome(at.frame,&format!("HP decode {error:?}; raw retained"));self.diagnostics.report("protected_hp_rejected");
            self.hp_quarantine(at.context,binding,actor,false);return Ok(());
        }
        let target=if protected{
            let Some(target)=self.meter.registry.current_key(self.process,binding,envelope.actor as u32) else{
                self.hp_evidence.outcome(at.frame,"HP decode MissingIv; raw retained");self.diagnostics.report("protected_hp_missing_iv");return Ok(());
            };target
        }else{self.observed_entity(binding,envelope.actor as u32,at)};
        let hp=if protected{self.protected_hp.decode(at.context,target,wire,p)}else{plain};
        match hp{
            Ok(hp)=>{
                if protected{
                    if self.protected_hp_samples.len()==combat::HISTORY{self.protected_hp_samples.pop_front();self.protected_hp_evictions+=1;}
                    self.protected_hp_samples.push_back((target,at,hp));self.diagnostics.report("protected_hp_decoded");
                }
                self.hp_numeric(target,hp,at,if protected{"protected update"}else{"plain update"})?;
            }
            Err(error)=>{
                self.health.clear_target_with_model(at.context,target,&mut self.combat.damage_model);
                self.hp_evidence.break_target(at.context,target);
                self.hp_evidence.outcome(at.frame,&format!("HP decode {error:?}; raw retained"));
                self.diagnostics.report(if error==crate::protected_hp::DecodeError::MissingIv{"protected_hp_missing_iv"}else{"protected_hp_rejected"});
            }
        }Ok(())
    }
    fn hp_status(&mut self,wire:i32,p:&[u8],at:Stamp,lineage:u64,binding_failure:Option<&'static str>)->Result<()> {
        let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
        let base=if wire==20517{8}else{0};
        let outer=p.get(base..base+8).map(|p|protocol::u64le(p,0));
        let status=match crate::hp_wire::status(wire,p){
            Ok(status)=>status,
            Err(error)=>{
                self.hp_evidence.outcome(at.frame,&format!("status prefix {error:?}; raw retained"));self.diagnostics.report("protected_hp_seed_rejected");
                if binding_failure!=Some("stale identity"){self.hp_quarantine(at.context,binding,outer,true);}return Ok(());
            }
        };
        if status.actor==0 || status.actor>u32::MAX as u64{
            if status.mode==0 && status.hp.len()==8{let hp=protocol::u64le(status.hp,0);if hp<=i64::MAX as u64{
                self.hp_evidence.unbound(at,status.actor,hp,"numeric status target outside registry detail domain");
                let target=identity::EntityKey{process:self.process,binding,actor:0,generation:0};
                return self.hp_numeric_fact(target,status.actor,hp,at,"wire-unbound plain status",false);
            }}
            self.hp_evidence.outcome(at.frame,"protected target outside supported codec domain; no numeric value decoded");return Ok(());
        }
        let value=if status.mode==0{
            if status.hp.len()==8{let n=protocol::u64le(status.hp,0);if n<=i64::MAX as u64{Ok(n)}else{Err(crate::protected_hp::DecodeError::OutOfRange)}}
            else{Err(crate::protected_hp::DecodeError::Unsupported)}
        }else{crate::protected_hp::spawn_seed(wire,p).and_then(|s|s.map(|s|s.initial_hp).ok_or(crate::protected_hp::DecodeError::Unsupported))};
        let hp=match value{
            Ok(hp)=>hp,
            Err(error)=>{
                self.hp_evidence.outcome(at.frame,&format!("status scalar {error:?}; raw retained"));self.diagnostics.report("protected_hp_seed_rejected");
                if binding_failure!=Some("stale identity"){self.hp_quarantine(at.context,binding,Some(status.actor),true);}return Ok(());
            }
        };
        if let Some(reason)=binding_failure{
            self.hp_evidence.unbound(at,status.actor,hp,reason);
            if reason!="stale identity"{self.hp_evidence.block(at.context,status.actor as u32);self.protected_hp.forget_at(at.context,self.meter.registry.current_key(self.process,binding,status.actor as u32).unwrap_or(identity::EntityKey{process:self.process,binding,actor:status.actor as u32,generation:0}));}
            let actor=status.actor as u32;
            let target=if reason=="stale identity"{
                self.meter.registry.records.keys().filter(|key|key.process==self.process && key.binding==binding && key.actor==actor && !self.meter.registry.is_current(**key)).max_by_key(|key|key.generation).copied()
                    .unwrap_or(identity::EntityKey{process:self.process,binding,actor,generation:0})
            }else{self.observed_entity(binding,actor,at)};
            return self.hp_numeric_fact(target,status.actor,hp,at,"identity-unbound numeric status",false);
        }
        self.hp_evidence.resolve(at.context,status.actor as u32);
        let target=self.observed_entity(binding,status.actor as u32,at);
        if let Ok(iv)=status.iv.try_into(){
            // Plain status also supplies an IV. A later protected update can
            // use it without assuming mode is a permanent property of actor.
            let seed=crate::protected_hp::SpawnSeed{actor:status.actor as u32,iv,initial_hp:hp,iv_offset:status.iv_offset};
            let registration=if status.mode==0{self.protected_hp.register_declared(at.context,target,&seed)}else{self.protected_hp.register(at.context,target,&seed)};
            let changed=match registration{
                Ok(changed)=>changed,
                Err(_)=>{self.diagnostics.report("hp_key_capacity");return self.hp_numeric_fact(target,status.actor,hp,at,"numeric status without retained key",false);}
            };
            if changed{self.health.clear_target_with_model(at.context,target,&mut self.combat.damage_model);}
            if status.mode!=0{
                if self.protected_hp_seeds.len()==128{self.protected_hp_seeds.pop_front();self.diagnostics.report("protected_hp_seed_eviction");}
                self.protected_hp_seeds.push_back((target,at,status.iv_offset,hp,changed));self.diagnostics.report("protected_hp_seed");
            }
        }else{
            self.hp_quarantine(at.context,binding,Some(status.actor),true);
        }
        self.hp_numeric(target,hp,at,if status.mode==0{"plain status"}else{"protected status"})
    }
    pub fn finish(&mut self) {
        self.revalidate_damage_support();
        if self.finished {
            if self.publish_credits(true).is_err(){self.diagnostics.report("credit_delivery_pending_retry");}
            return;
        }
        if !self.finished {
            if self.use_clock_bounds{self.health.constrain(&self.timeline);}
            let _=self.health.flush_with_model(&mut self.combat.damage_model);if self.publish_credits(true).is_err(){self.diagnostics.report("credit_delivery_pending_retry");}
            if self.finalize_skills(None).is_err(){self.diagnostics.report("skill_capacity");}
            for stream in self.streams.values() { stream.finish(&mut self.diagnostics); }
            self.meter.partial |= self.diagnostics.partial(); self.finished = true;
        }
    }
    fn observed_entity(&mut self,binding:u64,actor:u32,at:Stamp)->identity::EntityKey {
        match self.meter.registry.observe_at(self.process,binding,actor,at){
            Ok(key)=>key,
            Err(_)=>{self.diagnostics.report("identity_detail_degraded_credit_retained");identity::EntityKey{process:self.process,binding,actor,generation:0}}
        }
    }
    fn sync_roster(&mut self,at:Stamp){
        self.revalidate_damage_support();
        if self.roster_revision==self.meter.registry.revision{return;}
        let roster:Vec<_>=self.meter.registry.records.iter().rev().filter(|(key,_)|key.process==self.process && self.meter.registry.is_current(**key)).take(crate::attribution::MAX_ACTORS).map(|(key,_)|(*key,self.actor_kind(*key),self.meter.registry.timing.get(key).and_then(|t|t.last).unwrap_or(at))).collect();
        for(key,kind,observed)in roster{self.health.note_actor_kind(key,kind,observed);}
        self.roster_revision=self.meter.registry.revision;
    }
    fn revalidate_damage_support(&mut self){
        self.health.attribution.active_idle_ns=self.meter.idle_ns;
        self.health.revalidate_registry(&self.meter.registry);
    }
    fn actor_kind(&self,key:identity::EntityKey)->&'static str{
        if self.meter.registry.damage_side(key)==identity::DamageSide::Player{return "player";}
        match self.meter.registry.records.get(&key).and_then(|m|m.kind){Some("pet")|None=>"unknown",Some(kind)=>kind}
    }
    fn note_pair_roles(&mut self,source:identity::EntityKey,target:identity::EntityKey,at:Stamp){
        self.revalidate_damage_support();
        for key in [source,target]{
            let kind=self.actor_kind(key);
            self.health.note_actor_kind(key,kind,at);
        }
    }
    fn damage_fact(&mut self,p:&[u8],at:Stamp,lineage:u64)->Result<()> {
        let Ok(fields)=crate::damage_wire::Fields::parse(p)else{self.diagnostics.report("payload_rejected");return Ok(());};
        let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
        let target_actor=u32::try_from(fields.target_prefix).ok().filter(|n|*n!=0);
        let target=target_actor.map(|actor|self.observed_entity(binding,actor,at)).unwrap_or(identity::EntityKey{process:self.process,binding,actor:0,generation:0});
        if fields.raw_amount==u64::MAX{self.health.sentinel(target,at);self.diagnostics.report("damage_non_numeric_sentinel");return Ok(());}
        self.sync_roster(at);
        let source=u32::try_from(fields.source_prefix).ok().filter(|n|*n!=0).map(|actor|self.observed_entity(binding,actor,at));
        if let Some(source)=source{
            self.note_pair_roles(source,target,at);
            let role=if self.meter.registry.self_key==Some(source){"main character"}else if self.meter.registry.is_self_source(source){"owned main-character entity"}else if self.meter.registry.self_key.is_none(){"self unresolved at packet time"}else{"other source at packet time"};
            *self.explicit_origins.entry(role).or_default()+=1;
            self.health.explicit_fields(source,target,fields.raw_amount,fields.hp_impact_candidate(),fields.raw_flags,at)?;
        }else{
            *self.explicit_origins.entry("raw source requires real actor fallback").or_default()+=1;
            self.health.unbound_explicit_fields(target,fields.raw_amount,fields.hp_impact_candidate(),fields.raw_flags,at)?;
        }
        Ok(())
    }
    fn drain_assignments(&mut self)->Result<()>{self.publish_credits(false)}
    /// Retry acknowledged delivery independently of decoder/capture lifetime.
    /// Rejected credits stay owned by Attribution, including after EOF/errors.
    pub fn retry_pending_credits(&mut self)->Result<()>{self.publish_credits(true)}
    fn publish_credits(&mut self,_force:bool)->Result<()> {
        self.revalidate_damage_support();
        // The outbox owns the immutable lease until downstream commit succeeds.
        // Errors retain this credit and every following obligation for retry.
        for _ in 0..crate::attribution::MAX_CREDITS{
            let Some(credit)=self.health.front_credit()else{break;};
            let (hit,inferred,at,computed)=match &credit.kind{
                crate::attribution::CreditKind::Explicit{hit,at}=>(hit.clone(),false,*at,if credit.detail_coarsened{None}else{self.linked_explicit_skill(hit,*at).skill}),
                crate::attribution::CreditKind::Inferred(e)=>{
                    let skill=e.skill.or_else(||self.combat.attacks.iter().find(|a|a.source==e.source && a.target==e.target && e.frames.contains(&a.at.frame)).and_then(|a|self.combat.linked_skill(a).skill));
                    (Hit{timestamp_ns:e.timestamp_ns,source:e.source,target:e.target,damage:protocol::Damage{actor:e.source.actor,target:e.target.actor,amount:e.amount,flags:[false;5],raw_flags:[0;7]}},true,e.at,if credit.detail_coarsened{None}else{skill})
                }
            };
            let already_committed=self.meter.receipt_committed(credit.id);
            if !already_committed && !self.meter.registry.damage_eligible(hit.source,hit.target){
                if !self.health.attribution.reject_leased(credit.id){return Err("friendly-fire credit rejection receipt mismatch".into());}
                self.diagnostics.report("friendly_fire_credit_rejected");self.meter.partial=true;continue;
            }
            if !already_committed && self.meter.credit_admission(hit.source,inferred)==crate::meter::CreditAdmission::SourceCapacity{
                let accepted:Vec<_>=self.health.attribution.active_candidates(hit.target,at).into_iter().filter(|c|self.meter.registry.damage_eligible(c.source,hit.target) && self.meter.credit_admission(c.source,inferred)==crate::meter::CreditAdmission::Ready).collect();
                if !self.health.attribution.retry_credit(credit.id,&accepted){return Err("credit capacity retry receipt mismatch".into());}
                self.diagnostics.report("credit_source_capacity_forced_inference");self.meter.partial=true;continue;
            }
            let skill=credit.prepared_skill.unwrap_or(computed);
            if credit.prepared_skill.is_none() && !self.health.attribution.prepare_skill(credit.id,skill){return Err("credit preparation receipt mismatch".into());}
            match self.meter.commit_credit(credit.id,hit.clone(),inferred,skill){
                Ok(status)=>{
                    if !inferred && skill.is_none() && !credit.detail_coarsened && status==crate::meter::CreditCommit::Committed{
                        if self.pending_skills.len()==PENDING_SKILLS{self.pending_skills.pop_front();self.diagnostics.report("skill_evidence_wait_detail_coarsened");}
                        self.pending_skills.push_back((hit,at,self.meter.last_hit_assignment(false).expect("committed raw credit")));
                    }
                    if !self.health.attribution.ack_credit(credit.id){return Err("attribution receipt acknowledgment mismatch".into());}
                }
                Err(_)=>{self.diagnostics.report("credit_delivery_pending_retry");self.meter.partial=true;break;}
            }
        }
        Ok(())
    }
    fn linked_explicit_skill(&self,hit:&Hit,at:Stamp)->combat::SkillMatch{
        let mut links=self.combat.attacks.iter().filter(|a|a.source==hit.source && a.target==hit.target && a.at.process==at.process && a.at.context==at.context && a.at.continuity==at.continuity && a.meta[..7]==hit.damage.raw_flags && a.at.timestamp_ns.abs_diff(at.timestamp_ns)<=crate::health::LOCAL_NS);
        let first=links.next();first.filter(|_|links.next().is_none()).map(|a|self.combat.linked_skill(a)).unwrap_or(combat::SkillMatch{skill:None,frames:Vec::new(),outcome:"explicit component linkage unresolved"})
    }
    fn finalize_skills(&mut self,now:Option<u64>)->Result<()>{
        while let Some((hit,at,_))=self.pending_skills.front(){
            let matched=self.linked_explicit_skill(hit,*at);
            if matched.skill.is_none() && now.is_some_and(|n|n.saturating_sub(at.timestamp_ns)<=crate::health::WAIT_NS){break;}
            let(hit,at,assignment)=self.pending_skills.pop_front().unwrap();
            if self.meter.refine_recorded_skill(&hit,&assignment,matched.skill,false).is_err(){self.diagnostics.report("skill_detail_retained_unknown");}
            self.meter.skill_evidence(at,hit.source,hit.target,matched);
        }Ok(())
    }
    pub fn tick(&mut self,now:u64)->Result<()> {
        self.revalidate_damage_support();
        if self.use_clock_bounds{self.health.constrain(&self.timeline);}
        let _=self.health.tick_with_model(now,&mut self.combat.damage_model);
        self.finalize_skills(Some(now))?;
        self.publish_credits(false)?;
        if self.meter.last_self_activity.is_some_and(|t|now.saturating_sub(t)>=self.meter.idle_ns){let _=self.health.flush_with_model(&mut self.combat.damage_model);self.publish_credits(true)?;}
        self.meter.tick(now);Ok(())
    }
    pub fn commit_pending(&mut self)->Result<()> {
        self.revalidate_damage_support();
        let _=self.health.flush_with_model(&mut self.combat.damage_model);self.publish_credits(true)?;self.finalize_skills(None)?;
        if self.health.attribution.queued_len()==0 && self.health.attribution.owed_amount()>0{self.publish_credits(true)?;}
        if self.health.attribution.queued_len()>0 || self.health.attribution.owed_amount()>0{return Err("credit delivery remains pending; measurement boundary retained".into());}Ok(())
    }
    pub fn reset_inference(&mut self){self.health.clear_with_model(&mut self.combat.damage_model);if self.publish_credits(true).is_err(){self.diagnostics.report("credit_delivery_pending_retry");}self.hp_evidence.partition();self.combat.attacks.clear();}
    /// For an independently verified room transition in the same run. No JP
    /// scene marker is sufficient to invoke this. Preserve actors/participants,
    /// ownership and decryption keys while ending transient causal continuity.
    pub fn internal_transition(&mut self,evidence:u64)->Result<()>{
        if self.meter.transition_seen(evidence){return Ok(());}
        self.commit_pending()?;
        self.finalize_skills(None)?;
        let contexts:Vec<_>=self.continuities.keys().copied().collect();
        for context in contexts{self.invalidate_context(context);}
        self.meter.event(Event::InternalTransition{evidence})
    }
    pub fn process_replacement(&mut self,epoch:u64)->Result<()> {
        self.commit_pending()?;

        self.protected_hp.clear();
        self.hp_evidence.clear();
        self.continuities.clear();self.context_orders.clear();self.health.clear();self.finalize_skills(None)?;self.combat.clear();self.timeline.clear();self.last_clock.clear();self.streams.clear();self.syns.clear();self.connections.clear();self.next_lineage=0;self.fins.clear();self.buffered=0;
        self.process=epoch;self.meter.event(Event::ProcessReplacement{evidence:epoch})
    }
    pub fn load(path: &Path, port: Option<u16>, aligned: bool) -> Result<Self> {
        let mut replay = Self::new(aligned); capture_file::read(path,port,|input| replay.feed(input))?;
        replay.finish(); Ok(replay)
    }
    pub fn audit(&self) -> Value {
        let s = self.meter.snapshot();
        let mut audit=json!({ "format": "mabi-dps-rust-replay-v5", "protocol_profile": protocol::PROFILE,"semantic_error":self.semantic_error,
            "attribution_policy":{"algorithm":"uniform packet attribution with eligibility v3","local_capture_ns":crate::health::LOCAL_NS,"availability_retention_ns":crate::health::WAIT_NS,"candidate_frames":null,"fallback":"one resolver; current packet support then eligible active attackers within idle split; no support stays unattributed","active_idle_ns":self.meter.idle_ns,"friendly_fire":"same actor and confirmed player/pet side excluded from every attribution path; generic NPC type does not prove faction","clock_policy":"partial ordering only; no exact HP server timestamp","raw_explicit_policy":"retain raw meter amount; checked raw-minus-aux used for HP reconciliation; raw source is a role claim"},
            "hp_accounting":{"refinement_evictions":self.health.refinement_evictions,"refinements":self.health.refinements.iter().map(|r|json!({"interval":r.interval,"source":key_json(r.source),"amount":r.amount,"at":stamp_json(r.at),"attack_frames":r.attack_frames,"method":"late explicit refines provisional allocation before commit"})).collect::<Vec<_>>(),"loss":self.health.loss.to_string(),"matched_explicit":self.health.explicit.to_string(),"allocated":self.health.allocated.to_string(),"unassigned":self.health.unassigned.to_string(),"outcomes":self.health.outcomes,"audit_evictions":self.health.audit_evictions,
                "intervals":self.health.audit.iter().map(|a|json!({"id":a.id,"target":key_json(a.target),"at":stamp_json(a.at),"baseline":stamp_json(a.baseline),"hp_before":a.hp_before,"hp_after":a.hp_after,"loss":a.loss,"explicit":a.explicit,"raw_explicit_claims":a.raw_explicit.to_string(),"residual":a.residual,"unassigned":a.unassigned,"reason":a.reason,"explicit_mismatch":a.explicit_mismatch,"candidates":a.candidates.iter().map(|k|key_json(*k)).collect::<Vec<_>>(),"evidence_frames":a.evidence,"allocations":a.allocations.iter().map(|e|json!({"source":key_json(e.source),"amount":e.amount,"skill":e.skill,"method":e.method,"evidence_frames":e.frames})).collect::<Vec<_>>()})).collect::<Vec<_>>()},
            "ownership":self.meter.registry.owners.links.values().map(|l|json!({"pet":key_json(l.pet),"owner":key_json(l.owner),"template":l.template,"kind":l.kind,"outer_reference":l.outer,"conflicting":l.conflicting,"conflict_evidence":l.conflict_evidence.map(|(owner,e)|json!({"owner":key_json(owner),"frame":e.frame,"timestamp_ns":e.timestamp_ns})),"frame":l.evidence.frame,"wire":format!("{:?}",l.evidence.origin)})).collect::<Vec<_>>(),
            "identity_timing":self.meter.registry.timing.iter().map(|(k,t)|json!({"entity":key_json(*k),"first":t.first.map(stamp_json),"last":t.last.map(stamp_json),"identity_received_ns":t.received.map(|e|e.timestamp_ns),"binding_resolved_ns":t.resolved.map(|e|e.timestamp_ns)})).collect::<Vec<_>>(),
            "timeline":{"bounds":self.timeline.bounds.values().map(|b|json!({"at":stamp_json(b.at),"lower":b.lower.map(|p|json!({"epoch":p.epoch,"ticks":p.ticks,"frame":p.frame})),"upper":b.upper.map(|p|json!({"epoch":p.epoch,"ticks":p.ticks,"frame":p.frame})),"interpretation":"partial clock/order bracket; not exact event time"})).collect::<Vec<_>>(),"evictions":self.timeline.evictions,"samples":self.timeline.samples.iter().map(|t|json!({"at":stamp_json(t.at),"wire":t.wire,"raw":t.raw,"unwrapped_ticks":t.ticks,"epoch":t.epoch,"interpretation":t.interpretation})).collect::<Vec<_>>()},
            "combat_evidence":{"confirmed_crosswalks":self.combat.confirmed_executions,"ended_effect_evictions":self.combat.ended_evictions,"ended_effects":self.combat.ended_effects.iter().map(|(e,at)|json!({"affected":key_json(e.affected),"source":e.source.map(key_json),"instance":e.instance.to_string(),"id":e.id,"first":stamp_json(e.first),"last_snapshot":stamp_json(e.last),"removed":stamp_json(*at)})).collect::<Vec<_>>(),"evictions":self.combat.evictions,"effect_updates":self.combat.effect_updates,"effect_removals":self.combat.removals,
                "attacks":self.combat.attacks.iter().map(|a|json!({"source":key_json(a.source),"target":key_json(a.target),"key1":a.key1,"key2":a.key2,"meta":hex(&a.meta),"at":stamp_json(a.at)})).collect::<Vec<_>>(),
                "executions":self.combat.executions.iter().map(|e|json!({"source":key_json(e.source),"skill":e.skill,"action":e.action,"variant":e.variant.as_ref().map(|v|hex(v)),"confirmed_crosswalk":e.confirmed,"at":stamp_json(e.at)})).collect::<Vec<_>>(),
                "effects":self.combat.effects.values().map(|e|json!({"affected":key_json(e.affected),"source":e.source.map(key_json),"instance":e.instance.to_string(),"id":e.id,"remaining_time_bits":e.time_bits,"stacks":e.stacks,"first":stamp_json(e.first),"last":stamp_json(e.last)})).collect::<Vec<_>>()},
            "skill_amounts":self.meter.skills.iter().map(|((encounter,source,target,skill,estimated),amount)|json!({"encounter":encounter,"source":key_json(*source),"target":key_json(*target),"skill":skill,"amount":amount.to_string(),"estimated":estimated,"label_provenance":"JP numeric key; packet internal names when observed"})).collect::<Vec<_>>(),
            "skill_matches":self.meter.skill_matches.iter().map(|(at,source,target,m)|json!({"at":stamp_json(*at),"source":key_json(*source),"target":key_json(*target),"skill":m.skill,"outcome":m.outcome,"evidence_frames":m.frames})).collect::<Vec<_>>(),
            "experimental": true,"live_acceptance": false,"partial": s.partial,
            "segments": self.segments,"frame_count": self.frames,"issues": self.diagnostics.0,
            "assume_aligned": self.aligned,"summary": snapshot_json(&s),
            "entity_scope_policy": if self.live_scope {"verified game process epoch; transport independent"}else{"offline connection lineage; no process ownership inferred"},
            "capture_totals":{"explicit_damage":self.meter.session.total.damage.to_string(),"explicit_hits":self.meter.session.total.hits,"estimated_damage":self.meter.session.estimated_total.damage.to_string(),"estimated_allocations":self.meter.session.estimated_total.allocations},
            "encounters":self.meter.encounters.iter().map(|(id,e)|json!({"id":id,"state":format!("{:?}",e.state),"views":e.views.iter().map(|(target,v)|json!({"target":target.map(key_json),"explicit_damage":v.total.damage.to_string(),"explicit_hits":v.total.hits,"estimated_damage":v.estimated_total.damage.to_string(),"estimated_allocations":v.estimated_total.allocations,"start_ns":v.total.start.into_iter().chain(v.estimated_total.start).min(),"end_ns":v.total.end.into_iter().chain(v.estimated_total.end).max(),"sources":v.sources.iter().map(|(key,a)|json!({"key":key_json(*key),"damage":a.damage.to_string(),"hits":a.hits})).collect::<Vec<_>>(),"estimates":v.estimates.iter().map(|(key,a)|json!({"key":key_json(*key),"damage":a.damage.to_string(),"allocations":a.allocations})).collect::<Vec<_>>() })).collect::<Vec<_>>() })).collect::<Vec<_>>(),
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
            "estimated_allocations":self.meter.inferred_hits.iter().map(|h|json!({"timestamp_ns":h.timestamp_ns,"source":key_json(h.source),"target":key_json(h.target),"amount":h.damage.amount})).collect::<Vec<_>>(),
            "disabled_adapters": ["JP20344/20552 transitions","unseen JP appearance profiles","unvalidated periodic damage ticks","universal VFX damage matching"] });
        audit["hp_evidence"]=self.hp_evidence.audit();
        audit["damage_wire_fields"]=self.damage_wire.audit();
        audit["assignment_resources"]=self.health.resource_summary();
        audit["assignment_dependencies"]=self.assignment_storage();
        audit["numeric_observations"]=self.numeric.audit();
        audit["attribution_obligations"]=self.health.attribution.audit();
        audit["damage_model"]=self.combat.damage_model.summary();
        audit["explicit_source_roles"]=json!({"counts":self.explicit_origins,"examples":self.explicit_origin_examples,"interpretation":"packet-time identity classification, not proof of source role; receiver-local player-plus-pet model under review"});
        audit["protected_hp"]=json!({"key_count":self.protected_hp.key_count(),"sample_evictions":self.protected_hp_evictions,
            "seeds":self.protected_hp_seeds.iter().map(|(key,at,offset,hp,changed)|json!({"target":key_json(*key),"at":stamp_json(*at),"iv_offset":offset,"initial_hp":hp,"changed":changed,"method":"AES-128-CBC actor key; spawn IV; strict padding"})).collect::<Vec<_>>(),
            "samples":self.protected_hp_samples.iter().map(|(key,at,hp)|json!({"target":key_json(*key),"at":stamp_json(*at),"hp":hp})).collect::<Vec<_>>()});
        audit["participants"]=json!(self.meter.registry.records.iter().map(|(key,m)|json!({"entity":key_json(*key),"participant":self.meter.registry.participant(*key),"state":self.meter.registry.binding_state(*key),"kind":m.kind,"uid":m.uid,"name":m.name})).collect::<Vec<_>>());
        audit["personal_bindings"]=json!({"evictions":self.meter.registry.personal_binding_evictions,"proofs":self.meter.registry.personal_bindings.iter().map(|p|json!({"entity":key_json(p.key),"frame":p.evidence.frame,"capture_ns":p.evidence.timestamp_ns,"offset":p.offset,"end":p.end,"pets":p.pets.iter().map(|p|json!({"persistent":p.persistent.to_string(),"template":p.template})).collect::<Vec<_>>(),"method":"complete personal pet collection and selected-key suffix"})).collect::<Vec<_>>()});
        audit["identity_candidates"]=json!(self.meter.registry.tracking.candidates.values().map(|c|json!({"old":key_json(c.old),"new":key_json(c.new),"frame":c.source.frame,"reason":c.reason})).collect::<Vec<_>>());
        audit["identity_transitions"]=json!({"evictions":self.meter.registry.tracking.event_evictions,
            "closed_contexts":self.meter.registry.tracking.closed_contexts().map(|((process,context),e)|json!({"process":process,"context":context,"frame":e.frame,"capture_ns":e.timestamp_ns})).collect::<Vec<_>>(),
            "events":self.meter.registry.tracking.events.iter().map(|e|json!({"revision":e.revision,"reason":e.reason,"entity":e.key.map(key_json),"source":e.source.map(|s|json!({"frame":s.frame,"capture_ns":s.timestamp_ns,"origin":format!("{:?}",s.origin)}))})).collect::<Vec<_>>()});
        audit
    }
    pub fn assignment_storage(&self)->Value{
        let health=self.health.resource_summary();
        let health_ceiling=health["conservative_owned_ceiling_bytes"].as_u64().unwrap_or(0) as usize;
        let scratch=health["scratch_ceiling_bytes"].as_u64().unwrap_or(0) as usize;
        let output=health["returned_output_ceiling_bytes"].as_u64().unwrap_or(0) as usize;
        let allocation_headroom=256*1024;
        let pending_skill_ceiling=PENDING_SKILLS*(std::mem::size_of::<(Hit,Stamp,crate::meter::HitAssignment)>()+128);
        let ceiling=health_ceiling+scratch+output+self.combat.assignment_ceiling_bytes()+self.timeline.assignment_ceiling_bytes()+self.damage_wire.owned_bytes()+crate::numeric::Numeric::ceiling_bytes()+pending_skill_ceiling+allocation_headroom;
        json!({"conservative_total_ceiling_bytes":ceiling,"allocation_headroom_bytes":allocation_headroom,"budget_bytes":10*1024*1024,"within_budget":health_ceiling>0 && ceiling<=10*1024*1024,"bound_scope":"finite inference/obligation/publication capacities; independent meter results and raw recording excluded","combat_owned_bytes":self.combat.assignment_owned_bytes(),"combat_ceiling_bytes":self.combat.assignment_ceiling_bytes(),"timeline_conservative_bytes":self.timeline.assignment_owned_bytes(),"timeline_ceiling_bytes":self.timeline.assignment_ceiling_bytes(),"raw_damage_ring_owned_bytes":self.damage_wire.owned_bytes(),"pending_skill_ceiling_bytes":pending_skill_ceiling,"pending_skill_owned_bytes":self.pending_skills.capacity()*(std::mem::size_of::<(Hit,Stamp,crate::meter::HitAssignment)>()+128),"numeric_owned_bytes":self.numeric.owned_bytes(),"numeric_ceiling_bytes":crate::numeric::Numeric::ceiling_bytes(),"attribution_owned_bytes":self.health.attribution.owned_bytes(),"attribution_ceiling_bytes":crate::attribution::Attribution::ceiling_bytes()})
    }
}
pub(crate) fn hex(p: &[u8]) -> String { p.iter().map(|b| format!("{b:02X}")).collect() }
pub(crate) fn stamp_json(s:Stamp)->Value{json!({"process":s.process,"continuity":s.continuity,"context":s.context,"frame":s.frame,"order":s.order,"capture_ns":s.timestamp_ns,"tcp_byte":s.tcp_byte})}
fn evidence_json(e: &Evidence) -> Value {
    json!({"timestamp_ns": e.timestamp_ns,"frame": e.frame,"wire_opcode": e.wire,"tcp_byte":e.tcp_byte,"context":e.context,"process":e.at.process,"continuity":e.at.continuity,"order":e.at.order,
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
        "dps": s.summary.dps(),"source_count": s.source_count,"target_count": s.target_count,"remembered_players":s.remembered_players,
        "revision": s.revision,"identity_revision": s.identity_revision,"last_command": s.last_command,
        "self_identity": {"key": s.self_binding.key.map(key_json),
            "evidence": format!("{:?}",s.self_binding.evidence),"primary_locked": s.self_binding.primary_locked,
            "voter_locked":s.self_binding.voter_locked,
            "origin":s.self_binding.source.map(|e| format!("{:?}",e.origin)),
            "source_frame": s.self_binding.source.filter(|e|matches!(e.origin,identity::tracking::EvidenceOrigin::Packet { .. })).map(|e| e.frame)},
        "actors": s.rows.iter().map(|r| json!({"key": key_json(r.key),"label": r.label,
            "damage": r.damage.to_string(),"hits": r.hits,"dps": r.dps,"share": r.share,
            "named": r.named,"is_self": r.is_self,"assignment_key": r.assignment_key.map(key_json),"estimated_damage":r.estimated_damage.to_string(),"pet_damage":r.pet_damage.to_string(),"skills":r.skills.iter().map(|s|json!({"id":s.id,"label":s.label,"explicit":s.explicit.to_string(),"estimated":s.estimated.to_string()})).collect::<Vec<_>>()})).collect::<Vec<_>>() })
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
