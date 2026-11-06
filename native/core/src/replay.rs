use crate::{capture_file::{self,Input}, identity, meter::{Event,Hit,Meter,Snapshot},
    protocol, transport::{Flow,Segment,TcpStream,GLOBAL_BUFFER}, Diagnostics, Result};
use serde_json::{json,Value};
use std::{collections::{BTreeMap,VecDeque}, path::Path};
use crate::combat::{self,Stamp,Attack,Execution,Effect};

#[derive(Clone)]
pub struct Evidence {
    pub at:Stamp,
    pub tcp_byte:u64,pub context:u64,
    pub timestamp_ns: u64,pub frame: u64,pub wire: Option<i32>,pub compression: u8,
    pub flow: Flow,pub payload: Vec<u8>,pub payload_length: usize,pub decoded: bool,
}
pub struct Replay {
    /// Offline ablation only. Default live/replay never withholds accepted hits.
    pub use_clock_bounds:bool,pub evaluation_withhold_actor:Option<u32>,pub withheld:VecDeque<Hit>,pub withheld_evictions:u64,
    pub combat:combat::Combat,pub timeline:combat::Timeline,
    last_clock:BTreeMap<(u64,u32),u64>,
    pending_skills:VecDeque<(Hit,Stamp,crate::meter::HitAssignment)>,
    health_identity_revision:u64,
    pub meter: Meter, pub diagnostics: Diagnostics, pub segments: u64, pub frames: u64,
    streams: BTreeMap<Flow,TcpStream>, connections: BTreeMap<[(String,u16);2],u64>,
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
        Self { use_clock_bounds:true,evaluation_withhold_actor:None,withheld:VecDeque::new(),withheld_evictions:0,health_identity_revision:u64::MAX,combat:combat::Combat::default(),timeline:combat::Timeline::default(),last_clock:BTreeMap::new(),pending_skills:VecDeque::new(),meter: Meter::default(),diagnostics: Diagnostics::default(),segments: 0,frames: 0,
            streams: BTreeMap::new(),connections: BTreeMap::new(),syns: BTreeMap::new(),
            last_timestamp: None,evidence: VecDeque::new(),frame_history: VecDeque::new(),history_bytes: 0,
            aligned,finished: false,buffered: 0,health:crate::health::Health::default(),process:0,continuities:BTreeMap::new(),context_orders:BTreeMap::new(),live_scope:false }
    }
    fn invalidate_context(&mut self,context:u64){
        *self.continuities.entry(context).or_default()+=1;
        self.health.clear_context(context);self.combat.clear_context(context);self.timeline.clear_context(context);self.last_clock.retain(|(c,_),_|*c!=context);
    }
    pub fn feed(&mut self, input: Input) -> Result<()> {
        if self.finished { return Err("replay already finished".into()); }
        let s = match input {
            Input::Loss { code,reset } => {
                self.diagnostics.report(&code); self.meter.event(Event::Gap)?;
                for epoch in self.continuities.values_mut(){*epoch+=1;}self.health.clear();self.combat.clear();self.timeline.clear();self.last_clock.clear();
                self.finalize_skills(None)?;
                if reset { self.streams.clear(); self.buffered = 0; self.syns.clear(); } return Ok(());
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
        let context=lineage*2+u64::from((s.flow.src.clone(),s.flow.sport)>(s.flow.dst.clone(),s.flow.dport));
        if s.syn && !s.ack && self.syns.get(&connection) != Some(&(s.flow.clone(),s.seq)) {
            self.invalidate_context(lineage*2);self.invalidate_context(lineage*2+1);
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
            self.invalidate_context(context);
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
        if loss_after!=loss_before {self.invalidate_context(context);}
        for frame in decoded {
            self.frames += 1;
            let order=self.context_orders.entry(context).or_default();*order+=1;
            let at=Stamp{process:self.process,continuity:*self.continuities.entry(context).or_default(),context,frame:self.frames,order:*order,timestamp_ns:s.timestamp_ns,tcp_byte:frame.stream_byte};
            if self.health_identity_revision!=self.meter.registry.revision{
                self.health.self_sources(self.meter.registry.self_key.is_some(),self.meter.registry.records.keys().filter(|k|self.meter.registry.is_self_source(**k)).copied().collect());
                self.health_identity_revision=self.meter.registry.revision;
            }
            if self.use_clock_bounds{self.health.constrain(&self.timeline);}
            for e in self.health.expire(at){self.estimate(e)?;}
            self.finalize_skills(Some(at.timestamp_ns))?;
            let record = Evidence { at,tcp_byte:frame.stream_byte,context,timestamp_ns: s.timestamp_ns,frame: self.frames,wire: frame.wire,
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
            if let Some(w)=frame.wire{self.timeline.observe(w,&frame.payload,at)?;}
            self.timeline.event(at);
            match frame.wire {
                Some(21196) => match protocol::damage(&frame.payload) {
                    Ok(d) => {
                        let binding = if self.live_scope {self.meter.binding}else{(lineage << 32) | self.meter.binding};
                        let source = self.meter.registry.observe_at(self.process,binding,d.actor,at)?;
                        let target = self.meter.registry.observe_at(self.process,binding,d.target,at)?;
                        if self.evaluation_withhold_actor==Some(d.actor){
                            if self.withheld.len()==65536{self.withheld.pop_front();self.withheld_evictions+=1;}
                            self.withheld.push_back(Hit{timestamp_ns:s.timestamp_ns,source,target,damage:d});continue;
                        }
                        self.health.explicit(source,target,d.amount,at)?;
                        let hit=Hit { timestamp_ns: s.timestamp_ns,source,target,damage: d };
                        self.meter.add(hit.clone())?;
                        if self.pending_skills.len()>=8192{return Err("pending skill capacity reached".into());}
                        self.pending_skills.push_back((hit,at,self.meter.last_hit_assignment(false).expect("accepted hit")));
                    }
                    Err(_) => {
                        let p=&frame.payload;
                        if p.len()>=53 && protocol::u64le(p,16)>1_000_000_000_000 && protocol::u32le(p,0)!=0 && protocol::u32le(p,8)!=0 && protocol::u32le(p,0)!=protocol::u32le(p,8){
                            let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
                            let target=self.meter.registry.observe_at(self.process,binding,protocol::u32le(p,8),at)?;
                            self.health.sentinel(target,at);self.diagnostics.report("damage_sentinel_or_out_of_range");
                        }else{self.diagnostics.report("payload_rejected");}
                    }
                }
                Some(20526) => { match protocol::attack(&frame.payload) {
                    Ok(a) if a.actor!=0 && a.target!=0 && a.actor!=a.target=>{
                        let binding=if self.live_scope {self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let source=self.meter.registry.observe_at(self.process,binding,a.actor,at)?;
                        let target=self.meter.registry.observe_at(self.process,binding,a.target,at)?;
                        let attack=Attack{source,target,key1:a.key1,key2:a.key2,meta:a.meta,at};
                        self.health.attack(attack.clone())?;self.combat.attack(attack);self.meter.activity(source,target,s.timestamp_ns);
                    },_=>self.diagnostics.report("payload_rejected")
                }}
                Some(170205)=>{
                    if let Some((actor,hp))=crate::health::parse(&frame.payload) {
                        let binding=if self.live_scope {self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let target=self.meter.registry.observe_at(self.process,binding,actor,at)?;
                        for estimate in self.health.update(target,hp,at)? {self.estimate(estimate)?;}
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
                        self.meter.registry.note_observed(key,at);
                        self.meter.registry.wire_named_appearance(key,a.identity,Some(a.display_name),source,(self.process,(lineage<<32)|self.meter.binding))?;
                        self.diagnostics.report("structured_player_appearance");
                    } else if let Some(a)=identity::parse(20054,&frame.payload) {
                        let binding=if self.live_scope {self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        self.meter.registry.appearance(self.process,binding,a)?;
                        self.diagnostics.report("legacy_player_metadata_only");
                    } else { self.diagnostics.report("identity_unresolved"); }
                }
                Some(w @ (20517|20051)) => {
                    if let Some(spawn)=identity::ownership::parse(w,&frame.payload){
                        let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let key=self.meter.registry.owned_spawn(self.process,binding,spawn,identity::tracking::Evidence{frame:self.frames,timestamp_ns:s.timestamp_ns,origin:identity::tracking::EvidenceOrigin::Packet{wire:w}})?;
                        self.meter.registry.note_observed(key,at);if let Some(owner)=self.meter.registry.owner(key){self.meter.registry.note_observed(owner,at);}
                        self.diagnostics.report("pet_ownership");
                    }else if let Some(a) = identity::parse(w,&frame.payload) {
                        let binding=if self.live_scope {self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let key=self.meter.registry.appearance(self.process,binding,a)?;self.meter.registry.note_observed(key,at);
                    } else { self.diagnostics.report("identity_unresolved"); }
                }
                Some(170064)=>{
                    if let Some(a)=combat::action(&frame.payload){
                        let binding=if self.live_scope{self.meter.binding}else{(lineage<<32)|self.meter.binding};
                        let source=self.meter.registry.observe_at(self.process,binding,a.actor,at)?;
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
                Some(20344 | 20552) => {
                    self.diagnostics.report("scene_marker"); self.meter.event(Event::Rebroadcast)?;
                }
                Some(_) => self.diagnostics.report("unknown_opcode"), None => (),
            }
        }
        if s.rst {
            self.invalidate_context(lineage*2);self.invalidate_context(lineage*2+1);
            let keys: Vec<_> = self.streams.keys().filter(|k| k.connection() == connection).cloned().collect();
            for k in keys { let stream = self.streams.remove(&k).unwrap();
                self.buffered -= stream.buffered(); stream.finish(&mut self.diagnostics); }
            self.syns.remove(&connection); self.meter.registry.cache_metadata();
        }
        self.meter.partial |= self.diagnostics.partial(); Ok(())
    }
    pub fn finish(&mut self) {
        if !self.finished {
            if self.use_clock_bounds{self.health.constrain(&self.timeline);}
            for e in self.health.flush() {if self.estimate(e).is_err(){self.diagnostics.report("inference_capacity");}}
            if self.finalize_skills(None).is_err(){self.diagnostics.report("skill_capacity");}
            for stream in self.streams.values() { stream.finish(&mut self.diagnostics); }
            self.meter.partial |= self.diagnostics.partial(); self.finished = true;
        }
    }
    fn estimate(&mut self,mut e:crate::health::Estimate)->Result<()> {
        if e.source==e.target{return Ok(());}
        if e.skill.is_none() && e.method=="single immediate source"{
            let matched=self.combat.skill(e.source,e.target,e.at);
            if let Some(skill)=matched.skill{
                e.skill=Some(skill);e.frames.extend(&matched.frames);
                if let Some(a)=self.health.audit.iter_mut().rev().find(|a|a.id==e.interval).and_then(|a|a.allocations.iter_mut().find(|a|a.source==e.source)){
                    a.skill=e.skill;a.frames=e.frames.clone();
                }
            }
            self.meter.skill_evidence(e.at,e.source,e.target,matched);
        }
        let hit=Hit {timestamp_ns:e.timestamp_ns,source:e.source,target:e.target,damage:protocol::Damage {
            actor:e.source.actor,target:e.target.actor,amount:e.amount,flags:[false;5],raw_flags:[0;7]
        }};
        self.meter.add_estimate(hit.clone())?;
        let assignment=self.meter.last_hit_assignment(true).expect("accepted allocation");
        self.meter.attribute_recorded_skill(&hit,&assignment,e.skill,true)?;Ok(())
    }
    fn finalize_skills(&mut self,now:Option<u64>)->Result<()> {
        while self.pending_skills.front().is_some_and(|(_,at,_)|now.is_none_or(|n|n.saturating_sub(at.timestamp_ns)>combat::EVENT_NS)){
            let (hit,at,assignment)=self.pending_skills.pop_front().unwrap();let skill=self.combat.skill(hit.source,hit.target,at);
            self.meter.attribute_recorded_skill(&hit,&assignment,skill.skill,false)?;
            self.meter.skill_evidence(at,hit.source,hit.target,skill);
        }Ok(())
    }
    pub fn tick(&mut self,now:u64)->Result<()> {
        if self.use_clock_bounds{self.health.constrain(&self.timeline);}
        for e in self.health.tick(now){self.estimate(e)?;}
        self.finalize_skills(Some(now))?;
        if self.meter.last_self_activity.is_some_and(|t|now.saturating_sub(t)>=self.meter.idle_ns) {
            for e in self.health.flush() {self.estimate(e)?;}
        }
        self.meter.tick(now);Ok(())
    }
    pub fn reset_inference(&mut self) {self.health.clear();if self.finalize_skills(None).is_err(){self.diagnostics.report("skill_capacity");}self.combat.attacks.clear();}
    pub fn process_replacement(&mut self,epoch:u64)->Result<()> {
        self.continuities.clear();self.context_orders.clear();self.health.clear();self.finalize_skills(None)?;self.combat.clear();self.timeline.clear();self.last_clock.clear();self.streams.clear();self.syns.clear();self.connections.clear();self.buffered=0;
        self.process=epoch;self.meter.event(Event::ProcessReplacement{evidence:epoch})
    }
    pub fn load(path: &Path, port: Option<u16>, aligned: bool) -> Result<Self> {
        let mut replay = Self::new(aligned); capture_file::read(path,port,|input| replay.feed(input))?;
        replay.finish(); Ok(replay)
    }
    pub fn audit(&self) -> Value {
        let s = self.meter.snapshot();
        json!({ "format": "mabi-dps-rust-replay-v2", "protocol_profile": protocol::PROFILE,
            "attribution_policy":{"candidate_ns":combat::EVENT_NS,"evidence_wait_ns":crate::health::WAIT_NS,"candidate_frames":combat::EVENT_FRAMES,"fallback":"equal distinct attacker shares; rotating integer remainder","clock_policy":"partial ordering only; no exact HP server timestamp"},
            "hp_accounting":{"refinement_evictions":self.health.refinement_evictions,"refinements":self.health.refinements.iter().map(|r|json!({"interval":r.interval,"source":key_json(r.source),"amount":r.amount,"at":stamp_json(r.at),"attack_frames":r.attack_frames,"method":"late explicit refines provisional allocation before commit"})).collect::<Vec<_>>(),"loss":self.health.loss.to_string(),"matched_explicit":self.health.explicit.to_string(),"allocated":self.health.allocated.to_string(),"unassigned":self.health.unassigned.to_string(),"outcomes":self.health.outcomes,"audit_evictions":self.health.audit_evictions,
                "intervals":self.health.audit.iter().map(|a|json!({"id":a.id,"target":key_json(a.target),"at":stamp_json(a.at),"baseline":stamp_json(a.baseline),"hp_before":a.hp_before,"hp_after":a.hp_after,"loss":a.loss,"explicit":a.explicit,"residual":a.residual,"unassigned":a.unassigned,"reason":a.reason,"explicit_mismatch":a.explicit_mismatch,"candidates":a.candidates.iter().map(|k|key_json(*k)).collect::<Vec<_>>(),"evidence_frames":a.evidence,"allocations":a.allocations.iter().map(|e|json!({"source":key_json(e.source),"amount":e.amount,"skill":e.skill,"method":e.method,"evidence_frames":e.frames})).collect::<Vec<_>>()})).collect::<Vec<_>>()},
            "ownership":self.meter.registry.owners.links.values().map(|l|json!({"pet":key_json(l.pet),"owner":key_json(l.owner),"template":l.template,"kind":l.kind,"outer_reference":l.outer,"conflicting":l.conflicting,"conflict_evidence":l.conflict_evidence.map(|(owner,e)|json!({"owner":key_json(owner),"frame":e.frame,"timestamp_ns":e.timestamp_ns})),"frame":l.evidence.frame,"wire":format!("{:?}",l.evidence.origin)})).collect::<Vec<_>>(),
            "identity_timing":self.meter.registry.timing.iter().map(|(k,t)|json!({"entity":key_json(*k),"first":t.first.map(stamp_json),"identity_received_ns":t.received.map(|e|e.timestamp_ns),"binding_resolved_ns":t.resolved.map(|e|e.timestamp_ns)})).collect::<Vec<_>>(),
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
            "disabled_adapters": ["JP20344/20552 transitions","unseen JP appearance profiles","unvalidated periodic damage ticks","universal VFX damage matching"] })
    }
}
fn hex(p: &[u8]) -> String { p.iter().map(|b| format!("{b:02X}")).collect() }
fn stamp_json(s:Stamp)->Value{json!({"process":s.process,"continuity":s.continuity,"context":s.context,"frame":s.frame,"order":s.order,"capture_ns":s.timestamp_ns,"tcp_byte":s.tcp_byte})}
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
        "dps": s.summary.dps(),"source_count": s.source_count,"target_count": s.target_count,
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
