//! Two receiver viewpoints of the same synthetic run. Peer explicit records are
//! validation labels only: each Replay sees its own explicit packets and the
//! shared HP/attack stream, and estimates must never become training labels.
use mabi_meter_core::{capture_file::Input, protocol, replay::Replay, transport::{Flow,Segment}};

fn put64(p:&mut [u8],offset:usize,n:u64){p[offset..offset+8].copy_from_slice(&n.to_le_bytes());}
struct View { replay:Replay, seq:u32, receiver:u64 }
impl View {
    fn new(receiver:u64)->Self{let mut replay=Replay::new(true);replay.live_scope=true;Self{replay,seq:1,receiver}}
    fn feed(&mut self,wire:i32,body:&[u8],time:u64){
        let payload=protocol::encode_frame(wire,body);let n=payload.len() as u32;
        self.replay.feed(Input::Segment(Segment{timestamp_ns:time,flow:Flow{src:"192.0.2.1".into(),dst:"192.0.2.2".into(),sport:16000,dport:1234},seq:self.seq,payload,syn:false,fin:false,rst:false,ack:true})).unwrap();self.seq+=n;
    }
    fn hp(&mut self,n:u64,time:u64){let mut p=vec![0;21];put64(&mut p,0,999);p[8..12].copy_from_slice(&8u32.to_le_bytes());put64(&mut p,12,n);self.feed(170205,&p,time);}
    fn event(&mut self,source:u64,raw:u64,aux:u64,hp:u64,time:u64){
        self.hp(hp,time);
        if source==self.receiver{let mut p=vec![0;53];put64(&mut p,0,source);put64(&mut p,8,999);put64(&mut p,16,raw);put64(&mut p,24,aux);p[32]=7;self.feed(21196,&p,time);}
        let mut p=vec![0;35];put64(&mut p,0,source);put64(&mut p,8,999);p[16..20].copy_from_slice(&7000001u32.to_le_bytes());p[24]=7;self.feed(20526,&p,time);
    }
    fn finish(&mut self){self.replay.finish();assert!(self.replay.semantic_error.is_none());assert_eq!(self.replay.numeric.detected,self.replay.health.loss);assert_eq!(self.replay.health.loss,self.replay.health.explicit+self.replay.health.allocated+self.replay.health.unassigned);assert_eq!(self.replay.health.attribution.audit()["invariant"],true);}
}

#[test]
fn peer_explicit_labels_confirm_nonterminal_hp_impact_in_both_directions(){
    let mut views=[View::new(101),View::new(202)];
    for v in &mut views{v.hp(1000,0);for (source,raw,aux,hp,time) in [(101,200,0,800,1),(202,100,0,700,2),(101,80,20,640,3)]{v.event(source,raw,aux,hp,time*1_000_000_000);}v.finish();}
    assert_eq!(views[0].replay.meter.session.total.damage,280);
    assert_eq!(views[0].replay.meter.session.estimated_total.damage,100);
    assert_eq!(views[1].replay.meter.session.total.damage,100);
    assert_eq!(views[1].replay.meter.session.estimated_total.damage,260);
    // Only the receiver's structurally concordant explicit/HP anchors train.
    assert_eq!(views[0].replay.combat.damage_model.anchors,2);
    assert_eq!(views[1].replay.combat.damage_model.anchors,1);
    assert_eq!(views[0].replay.health.unassigned,0);
    assert_eq!(views[1].replay.health.unassigned,0);
}

#[test]
fn killing_blow_clips_explicit_credit_to_peer_hp_but_preserves_raw_observation(){
    for receiver in [101,202]{
        let attacker=if receiver==101{202}else{101};
        let mut peer=View::new(receiver);let mut own=View::new(attacker);
        for v in [&mut peer,&mut own]{v.hp(17,0);v.event(attacker,900,0,0,1_000_000_000);v.finish();}
        assert_eq!(own.replay.meter.session.total.damage,17);
        assert_eq!(own.replay.damage_wire.observations().next().unwrap().fields.raw_amount,900);
        assert_eq!(own.replay.health.attribution.clipped_explicit_overkill,883);
        assert_eq!(own.replay.health.attribution.detected_explicit,17);
        assert_eq!(own.replay.health.explicit,17);
        assert_eq!(peer.replay.meter.session.estimated_total.damage,17);
        assert_eq!(peer.replay.health.loss,17);
        assert_eq!(own.replay.combat.damage_model.anchors,0);
        assert_eq!(peer.replay.combat.damage_model.anchors,0);
    }
}

#[test]
fn zero_impact_peer_component_does_not_gain_credit_or_train_from_raw_amount(){
    let mut views=[View::new(101),View::new(202)];
    for v in &mut views{v.hp(100,0);v.event(202,50,50,100,1_000_000_000);v.event(101,20,0,80,2_000_000_000);v.finish();}
    assert_eq!(views[0].replay.meter.session.estimated_total.damage,0);
    assert_eq!(views[1].replay.meter.session.total.damage,50);
    assert_eq!(views[1].replay.health.explicit,0);
    assert_eq!(views[1].replay.meter.session.estimated_total.damage,20);
    assert_eq!(views[1].replay.combat.damage_model.anchors,0);
}
