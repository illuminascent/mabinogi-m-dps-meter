use mabi_meter_core::{attribution::{Attribution,MAX_CREDITS},capture_file::Input,combat::Stamp,
    health::{LOCAL_NS,WAIT_NS},identity::EntityKey,protocol,replay::Replay,transport::{Flow,Segment}};
fn key(actor:u32)->EntityKey{EntityKey{process:0,binding:1,actor,generation:1}}
fn at(frame:u64)->Stamp{Stamp{process:0,continuity:0,context:1,frame,order:frame,timestamp_ns:0,tcp_byte:frame*60}}
struct Run{r:Replay,seq:u32}
impl Run{
    fn new()->Self{let mut r=Replay::new(true);r.live_scope=true;Self{r,seq:1}}
    fn feed(&mut self,wire:i32,body:Vec<u8>,time:u64){let payload=protocol::encode_frame(wire,&body);let n=payload.len() as u32;self.r.feed(Input::Segment(Segment{timestamp_ns:time,flow:Flow{src:"192.0.2.1".into(),dst:"192.0.2.2".into(),sport:16000,dport:1234},seq:self.seq,payload,syn:false,fin:false,rst:false,ack:true})).unwrap();self.seq+=n;}
    fn hp(&mut self,hp:u64,time:u64){let mut p=vec![0;21];p[..8].copy_from_slice(&999u64.to_le_bytes());p[8..12].copy_from_slice(&8u32.to_le_bytes());p[12..20].copy_from_slice(&hp.to_le_bytes());self.feed(170205,p,time);}
    fn damage(&mut self,raw:u64,aux:u64,time:u64){let mut p=vec![0;53];for(offset,n)in[(0,101),(8,999),(16,raw),(24,aux)]{p[offset..offset+8].copy_from_slice(&n.to_le_bytes());}p[32]=7;self.feed(21196,p,time);}
    fn attack(&mut self,source:u64,time:u64){let mut p=vec![0;35];p[..8].copy_from_slice(&source.to_le_bytes());p[8..16].copy_from_slice(&999u64.to_le_bytes());p[16..20].copy_from_slice(&7000001u32.to_le_bytes());p[24]=7;self.feed(20526,p,time);}
    fn finish(&mut self){self.r.finish();assert!(self.r.semantic_error.is_none());let a=self.r.health.attribution.audit();assert_eq!(a["invariant"],true);assert_eq!(a["by_origin"]["explicit"]["invariant"],true);assert_eq!(a["by_origin"]["inferred"]["invariant"],true);assert_eq!(self.r.numeric.detected,self.r.health.loss);assert_eq!(self.r.health.loss,self.r.health.explicit+self.r.health.allocated+self.r.health.unassigned);}
}

#[test]
fn terminal_explicit_cannot_publish_before_reconciliation_and_raw_is_retained(){
    let mut run=Run::new();run.hp(17,0);run.hp(0,1_000_000);run.damage(900,0,1_000_000);run.attack(101,1_000_000);
    assert_eq!(run.r.meter.session.total.damage,0);
    assert_eq!(run.r.health.attribution.queued_amount(),900);
    run.r.tick(WAIT_NS+1_000_001).unwrap();
    assert_eq!(run.r.meter.session.total.damage,17);
    assert_eq!(run.r.health.attribution.clipped_explicit_overkill,883);
    assert_eq!(run.r.damage_wire.observations().next().unwrap().fields.raw_amount,900);
    assert_eq!(run.r.meter.session.total.hits,1);run.finish();
    assert_eq!(run.r.meter.session.total.damage,17);
}

#[test]
fn explicit_before_terminal_hp_remains_held_until_that_hp_budget_resolves(){
    let mut run=Run::new();run.hp(17,0);run.damage(900,0,1_000_000);run.attack(101,1_000_000);run.hp(0,50_000_000);
    run.r.tick(WAIT_NS+1_000_001).unwrap();
    assert_eq!(run.r.meter.session.total.damage,0,"a pre-HP claim must not escape at its own earlier deadline");
    run.r.tick(WAIT_NS+50_000_001).unwrap();run.finish();
    assert_eq!(run.r.meter.session.total.damage,17);
    assert_eq!(run.r.health.attribution.clipped_explicit_overkill,883);
}

#[test]
fn clipping_removes_only_terminal_impact_excess_and_preserves_auxiliary_interpretation(){
    for(raw,aux,credited,overkill)in[(900,20,37,863),(900,883,900,0)]{
        let mut run=Run::new();run.hp(17,0);run.hp(0,1);run.damage(raw,aux,1);run.attack(101,1);run.finish();
        assert_eq!(run.r.meter.session.total.damage,credited);
        assert_eq!(run.r.health.attribution.clipped_explicit_overkill,overkill);
        assert_eq!(run.r.health.explicit,17);
        assert_eq!(run.r.combat.damage_model.anchors,if overkill==0{1}else{0});
    }
}

#[test]
fn nonterminal_raw_amount_is_not_scaled(){
    let mut run=Run::new();run.hp(1000,0);run.hp(100,1);run.damage(900,0,1);run.attack(101,1);run.finish();
    assert_eq!(run.r.meter.session.total.damage,900);assert_eq!(run.r.health.attribution.clipped_explicit_overkill,0);
}

#[test]
fn missing_or_ambiguous_or_disagreeing_presentation_does_not_prove_overkill(){
    for sources in [vec![],vec![202],vec![101,202]]{
        let mut run=Run::new();run.hp(17,0);run.hp(0,1);run.damage(900,0,1);for s in sources{run.attack(s,1);}run.finish();
        assert_eq!(run.r.meter.session.total.damage,900);assert_eq!(run.r.health.attribution.clipped_explicit_overkill,0);
    }
}

#[test]
fn initial_zero_and_gap_uncertainty_cannot_supply_a_terminal_cap(){
    let mut run=Run::new();run.hp(0,0);run.damage(900,0,1);run.attack(101,1);run.finish();
    assert_eq!(run.r.health.loss,0);assert_eq!(run.r.meter.session.total.damage,900);
    let mut run=Run::new();run.hp(17,0);run.r.feed(Input::Loss{code:"synthetic_gap".into(),reset:false}).unwrap();run.hp(0,1);run.damage(900,0,1);run.attack(101,1);run.finish();
    assert_eq!(run.r.health.loss,17);assert_eq!(run.r.meter.session.total.damage,900);assert_eq!(run.r.health.attribution.clipped_explicit_overkill,0);
}

#[test]
fn bounded_hold_without_new_hp_expires_and_eof_flush_also_drains_it(){
    let mut run=Run::new();run.hp(17,0);run.damage(900,0,1);run.attack(101,1);
    run.r.tick(WAIT_NS+LOCAL_NS+2).unwrap();assert_eq!(run.r.meter.session.total.damage,900);run.finish();
    let mut run=Run::new();run.hp(17,0);run.damage(900,0,1);run.finish();assert_eq!(run.r.meter.session.total.damage,900);assert_eq!(run.r.health.attribution.queued_len(),0);
}

#[test]
fn a_leased_credit_and_arithmetic_retry_cannot_be_rewritten_by_later_clipping(){
    let mut a=Attribution::default();a.submit_explicit(key(101),key(999),900,[0;7],at(1));
    let original=a.front_credit().unwrap();assert!(a.prepare_skill(original.id,Some(7)));
    assert!(!a.finalize_explicit(key(999),at(1),900,17));
    let retry=a.front_credit().unwrap();assert_eq!(retry.id,original.id);assert_eq!(retry.amount(),900);assert_eq!(retry.prepared_skill,Some(Some(7)));
    assert_eq!(a.overkill_clip_unavailable,883);assert!(a.ack_credit(original.id));assert_eq!(a.audit()["invariant"],true);
}

#[test]
fn pressure_coarsening_counts_unavailable_clipping_without_rewriting_another_packet(){
    let mut a=Attribution::default();for frame in 1..=MAX_CREDITS as u64{a.submit_explicit(key(101),key(999),1,[0;7],at(frame));}
    let packet=at(MAX_CREDITS as u64+1);a.submit_explicit_held(key(101),key(999),900,[0;7],packet,WAIT_NS);
    assert!(!a.finalize_explicit(key(999),packet,900,17));assert_eq!(a.overkill_clip_unavailable,883);assert_eq!(a.clipped_explicit_overkill,0);
    assert!(a.front_ready_credit(0).is_some(),"coarsening cannot keep extending a front hold under continuous pressure");while let Some(c)=a.front_ready_credit(0){assert!(a.ack_credit(c.id));}
    assert_eq!(a.published_explicit,MAX_CREDITS as u128+900);assert_eq!(a.audit()["invariant"],true);
}

#[test]
fn clipping_a_preselected_split_claim_conserves_integer_credit(){
    let mut a=Attribution::default();a.note_relationship(key(101),key(999),at(1));a.note_relationship(key(202),key(999),at(2));
    a.submit_unbound_explicit_held(key(999),900,[0;7],at(3),WAIT_NS);
    assert!(a.finalize_explicit(key(999),at(3),900,17));let mut n=0;
    while let Some(c)=a.front_ready_credit(0){n+=c.amount();assert!(a.ack_credit(c.id));}
    assert_eq!(n,17);assert_eq!(a.clipped_explicit_overkill,883);assert_eq!(a.audit()["invariant"],true);
}
