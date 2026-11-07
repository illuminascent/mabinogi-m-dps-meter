//! Count actual allocator requests, including resolver/output temporaries, in
//! a dedicated test binary. This is not process RSS or allocator private pages.
use std::{alloc::{GlobalAlloc,Layout,System},sync::atomic::{AtomicUsize,Ordering},time::Instant};
use mabi_meter_core::{attribution::{Candidate,MAX_ACTORS,MAX_RELATIONSHIPS,MAX_CREDITS,MAX_OWED,MAX_CREDIT_FRAMES},combat::{self,Attack,Combat,Execution,Stamp,Timeline},damage_wire::{Fields,Survey},health::{Health,MAX_FACTS,MAX_PENDING,MAX_SLOTS},identity::EntityKey,numeric::{Numeric,MAX_BASELINES},replay::Replay};
struct Counted;
static LIVE:AtomicUsize=AtomicUsize::new(0);
static PEAK:AtomicUsize=AtomicUsize::new(0);
fn add(n:usize){let live=LIVE.fetch_add(n,Ordering::Relaxed)+n;PEAK.fetch_max(live,Ordering::Relaxed);}
unsafe impl GlobalAlloc for Counted{
    unsafe fn alloc(&self,l:Layout)->*mut u8{let p=System.alloc(l);if !p.is_null(){add(l.size());}p}
    unsafe fn dealloc(&self,p:*mut u8,l:Layout){System.dealloc(p,l);LIVE.fetch_sub(l.size(),Ordering::Relaxed);}
    unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8{let q=System.realloc(p,l,n);if !q.is_null(){if n>=l.size(){add(n-l.size());}else{LIVE.fetch_sub(l.size()-n,Ordering::Relaxed);}}q}
}
#[global_allocator]static ALLOCATOR:Counted=Counted;
fn key(actor:u32)->EntityKey{EntityKey{process:0,binding:1,actor,generation:1}}
fn at(order:u64)->Stamp{Stamp{process:0,context:1,continuity:0,frame:order,order,timestamp_ns:0,tcp_byte:order*60}}

#[test]
fn allocator_requested_peak_includes_packet_dependencies_scratch_and_mass_output(){
    let baseline=LIVE.load(Ordering::Relaxed);PEAK.store(baseline,Ordering::Relaxed);
    let mut h=Health::default();let mut c=Combat::default();let mut timeline=Timeline::default();let mut wire=Survey::default();let mut numeric=Numeric::default();
    for actor in 1..=MAX_SLOTS as u32{h.update(key(actor),10000,at(0)).unwrap();numeric.observe(key(actor),actor as u64,10000,at(0),None);}
    // All global interval/fact slots active at once. Four separately observed
    // components per budget,32 budgets per target,32 targets.
    let mut frame=1;
    for target in 1..=32u32{for hit in 1..=32u64{
        let hp=10000-hit*4;numeric.observe(key(target),target as u64,hp,at(frame),None);h.update(key(target),hp,at(frame)).unwrap();frame+=1;
        for source in 5000..5004{h.attack(Attack{source:key(source),target:key(target),key1:77,key2:0,meta:[0;8],at:at(frame)}).unwrap();frame+=1;}
    }}
    let resources=h.resource_summary();assert_eq!(resources["pending"],MAX_PENDING);assert_eq!(resources["facts"],MAX_FACTS);
    for n in 0..combat::ATTACK_HISTORY{c.attack(Attack{source:key(1),target:key(2),key1:77,key2:0,meta:[0;8],at:at(n as u64)});}
    for n in 0..combat::EXECUTION_HISTORY{c.execution(Execution{source:key(1),skill:77,action:Some("a".repeat(combat::ACTION_BYTES)),variant:Some([0;29]),at:at(n as u64),confirmed:true}).unwrap();}
    let mut clock=vec![0;24];clock[8..12].copy_from_slice(&1u32.to_le_bytes());
    for n in 0..combat::CLOCK_HISTORY{timeline.observe(170066,&clock,at(n as u64)).unwrap();timeline.event(at(n as u64));}
    for n in 2..=combat::MAX_CLOCK_CONTEXTS{let mut stamp=at(n as u64);stamp.context=n as u64;timeline.observe(170066,&clock,stamp).unwrap();}
    let mut body=[0;53];body[..8].copy_from_slice(&1u64.to_le_bytes());body[8..16].copy_from_slice(&2u64.to_le_bytes());let fields=Fields::parse(&body).unwrap();assert_eq!(fields.hp_impact_candidate(),Some(0));
    for n in 0..1024{wire.observe(at(n),0,&body,true);}
    // Fill the same obligation owner used by Health, rather than measuring a
    // mostly empty second owner after taking the allocator peak. All semantic
    // lengths below are at their limits simultaneously with HP facts/budgets,
    // numeric baselines, combat, clock history and raw-field diagnostics.
    for n in 0..MAX_ACTORS{h.attribution.note_actor_kind(key(5000+n as u32),"Player",at(frame));}
    for n in 0..MAX_RELATIONSHIPS{h.attribution.note_relationship(key(5000+(n%MAX_ACTORS)as u32),key(100_000+n as u32),at(frame));}
    for n in 0..MAX_CREDITS{let support=[Candidate{source:key(5000+(n%4)as u32),skill:Some(77),weight:1.0,frames:vec![n as u64;MAX_CREDIT_FRAMES]}];h.attribution.resolve_budget(key(300_000+n as u32),at(frame),n as u64,1,&support,"pressure queued credit");}
    // Debt is delivery-only and retains an actual finite packet's exact
    // source. Missing attackers no longer create inference debt.
    for n in 0..MAX_OWED{let mut source=key(9000+n as u32);source.binding=99;let mut target=key(200_000+n as u32);target.binding=99;h.attribution.submit_explicit(source,target,7,[0;7],at(frame));}
    let full_pressure=h.attribution.audit();assert_eq!(full_pressure["actors"],MAX_ACTORS);assert_eq!(full_pressure["relationships"],MAX_RELATIONSHIPS);assert_eq!(full_pressure["queued_rows"],MAX_CREDITS);assert_eq!(full_pressure["owed_rows"],MAX_OWED);assert_eq!(full_pressure["invariant"],true);
    assert_eq!(numeric.audit()["retained"],MAX_BASELINES);assert_eq!(numeric.detected,4096);
    let mut delivered=0u128;let mut credit_count=0;
    // Deliver retained known-source claims while all existing packet/model
    // dependencies remain saturated; appearance data never supplies activity.
    let began=Instant::now();while let Some(credit)=h.attribution.front_credit(){delivered+=credit.amount()as u128;credit_count+=1;assert!(h.attribution.prepare_skill(credit.id,None));assert!(h.attribution.ack_credit(credit.id));}let settlement_ns=began.elapsed().as_nanos();
    assert_eq!(delivered,(MAX_CREDITS+7*MAX_OWED)as u128);assert_eq!(h.attribution.owed_amount(),0);
    // Restore a full compatible roster and exercise all 512 fallback shares.
    // Deques retain their saturated storage throughout this transient work.
    for n in 0..MAX_ACTORS{let source=key(5000+n as u32);h.attribution.note_actor_kind(source,"Player",at(frame));h.attribution.note_relationship(source,key(400_000),at(frame));}
    const WIDE_AMOUNT:u64=1_048_576;
    let began=Instant::now();let wide=h.attribution.resolve_budget(key(400_000),at(frame),0,WIDE_AMOUNT,&[],"pressure all compatible actors");let wide_ns=began.elapsed().as_nanos();assert_eq!(wide.len(),MAX_ACTORS);assert_eq!(wide.iter().map(|e|e.amount as u128).sum::<u128>(),WIDE_AMOUNT as u128);
    let began=Instant::now();let output=h.tick_with_model(300_000_000,&mut c.damage_model);let expiry_ns=began.elapsed().as_nanos();
    while let Some(credit)=h.attribution.front_credit(){delivered+=credit.amount() as u128;credit_count+=1;assert!(h.attribution.ack_credit(credit.id));}
    let expected=(MAX_CREDITS+7*MAX_OWED)as u128+WIDE_AMOUNT as u128+4096;assert_eq!(delivered,expected);assert_eq!(h.loss,4096);assert_eq!(h.allocated,MAX_CREDITS as u128+WIDE_AMOUNT as u128+4096);assert_eq!(h.unassigned,0);assert_eq!(h.attribution.owed_amount(),0);assert_eq!(h.attribution.audit()["invariant"],true);
    let measured=PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
    let contract=Replay::new(false).assignment_storage();assert_eq!(contract["within_budget"],true,"{contract}");
    println!("{}",serde_json::json!({"allocator_requested_peak_bytes":measured,"mass_1024_budget_expiry_service_ns":expiry_ns.to_string(),"full_1024_owed_settlement_service_ns":settlement_ns.to_string(),"wide_512_actor_fallback_service_ns":wide_ns.to_string(),"output_allocations":credit_count,"delivered_amount":delivered.to_string(),"numeric_baselines":numeric.audit()["retained"],"simultaneous_attribution_pressure":full_pressure,"reported_health_peak_bytes":h.resource_summary()["peak_conservative_resident_bytes"],"storage_contract":contract,"scope":"dedicated simultaneous inference and publication pressure; allocator-requested bytes including transient fallback/output allocations, not whole application RSS"}));
    assert!(measured<10*1024*1024,"actual allocator requests exceeded assignment budget: {measured}");
    std::hint::black_box((&h,&c,&timeline,&wire,&numeric,&wide,&output));
}
