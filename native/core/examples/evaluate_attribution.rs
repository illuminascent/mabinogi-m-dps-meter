//! Offline ablation, never starts capture. Output is private when input is private.
use mabi_meter_core::{replay::{Replay,key_json},capture_file,Result};
use serde_json::json;
use std::{collections::BTreeMap,path::Path};
fn main(){if let Err(e)=run(){eprintln!("{e}");std::process::exit(1);}}
fn run()->Result<()>{
    let args:Vec<_>=std::env::args().collect();if args.len()!=4{return Err("usage: evaluate_attribution CAPTURE SELF_ACTOR OUTPUT_JSON".into());}
    let actor=args[2].parse::<u32>().map_err(|e|e.to_string())?;
    let started=std::time::Instant::now();
    let mut normal=Replay::new(false);normal.live_scope=true;
    capture_file::read(Path::new(&args[1]),None,|input|normal.feed(input))?;normal.finish();let normal_elapsed=started.elapsed().as_secs_f64();
    let mut hidden=Replay::new(false);hidden.live_scope=true;hidden.evaluation_withhold_actor=Some(actor);
    capture_file::read(Path::new(&args[1]),None,|input|hidden.feed(input))?;hidden.finish();
    let actual:u128=hidden.withheld.iter().map(|h|h.damage.amount as u128).sum();
    let recovered:u128=hidden.meter.inferred_hits.iter().filter(|h|h.source.actor==actor).map(|h|h.damage.amount as u128).sum();
    let mut targets:BTreeMap<u32,(u128,u128)>=BTreeMap::new();
    for h in &hidden.withheld{targets.entry(h.target.actor).or_default().0+=h.damage.amount as u128;}
    for h in hidden.meter.inferred_hits.iter().filter(|h|h.source.actor==actor){targets.entry(h.target.actor).or_default().1+=h.damage.amount as u128;}
    let mut capture_only=Replay::new(false);capture_only.live_scope=true;capture_only.use_clock_bounds=false;
    capture_file::read(Path::new(&args[1]),None,|input|capture_only.feed(input))?;capture_only.finish();
    let summary=|r:&Replay|json!({"explicit":r.meter.session.total.damage.to_string(),"explicit_hits":r.meter.session.total.hits,"estimated":r.meter.session.estimated_total.damage.to_string(),"allocations":r.meter.session.estimated_total.allocations,
        "unassigned":r.health.unassigned.to_string(),"hp_loss":r.health.loss.to_string(),"hp_explicit":r.health.explicit.to_string(),"hp_allocated":r.health.allocated.to_string(),"outcomes":r.health.outcomes,
        "equal_share":r.health.audit.iter().flat_map(|a|&a.allocations).filter(|a|a.method=="equal share").map(|a|a.amount as u128).sum::<u128>().to_string(),
        "skill_known_explicit":r.meter.skills.iter().filter(|((_,_,_,s,e),_)|s.is_some() && !e).map(|(_,v)|v).sum::<u128>().to_string(),
        "skill_known_estimated":r.meter.skills.iter().filter(|((_,_,_,s,e),_)|s.is_some() && *e).map(|(_,v)|v).sum::<u128>().to_string(),
        "sources":r.meter.session.sources.iter().map(|(k,a)|json!({"key":key_json(*k),"explicit":a.damage.to_string(),"estimated":r.meter.session.estimates.get(k).map_or(0,|a|a.damage).to_string()})).chain(r.meter.session.estimates.iter().filter(|(k,_)|!r.meter.session.sources.contains_key(k)).map(|(k,a)|json!({"key":key_json(*k),"explicit":"0","estimated":a.damage.to_string()}))).collect::<Vec<_>>()});
    let report=json!({"format":"mabi-attribution-ablation-v1","self_actor":actor,"normal":summary(&normal),"capture_time_only":summary(&capture_only),"normal_elapsed_seconds":normal_elapsed,"segments":normal.segments,"frames":normal.frames,"withheld":summary(&hidden),"withheld_amount":actual.to_string(),"reconstructed_amount":recovered.to_string(),"relative_error":if actual==0{None}else{Some((recovered as f64-actual as f64)/actual as f64)},
        "targets":targets.iter().map(|(actor,(actual,recovered))|json!({"actor":actor,"actual":actual.to_string(),"reconstructed":recovered.to_string()})).collect::<Vec<_>>(),
        "limitations":["same capture ablation, not independent damage ground truth","HP clipping, healing, omissions and amount/HP mismatches remain separate","no timing parameter fitting or raid acceptance"],"withheld_history_evictions":hidden.withheld_evictions,"normal_audit":normal.audit()});
    std::fs::write(&args[3],serde_json::to_vec(&report).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    println!("withheld {actual}; reconstructed {recovered}; report {}",args[3]);Ok(())
}
