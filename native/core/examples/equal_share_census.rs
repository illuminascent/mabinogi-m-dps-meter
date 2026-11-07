//! Read-only measurement of the current replay policy, including intervals that
//! leave the bounded audit deque. No allocation policy or application behavior changes.
use mabi_meter_core::{capture_file,replay::Replay,health::{Health,Accounting},Result};
use serde_json::{json,Value};
use std::path::Path;
#[derive(Default)]
struct Tally{intervals:u64,inferred_intervals:u64,equal_intervals:u64,allocations:u64,equal_allocations:u64,loss:u128,matched:u128,inferred:u128,equal:u128,single:u128,unassigned:u128}
impl Tally{
    fn add(&mut self,a:&Accounting){let n=a.allocations.iter().map(|a|a.amount as u128).sum::<u128>();self.intervals+=1;self.loss+=a.loss as u128;self.matched+=a.explicit as u128;self.inferred+=n;self.unassigned+=a.unassigned as u128;self.allocations+=a.allocations.len() as u64;if n>0{self.inferred_intervals+=1;}if a.reason=="equal share"{self.equal_intervals+=1;self.equal+=n;self.equal_allocations+=a.allocations.len() as u64;}if a.reason=="single immediate source"{self.single+=n;}}
    fn json(&self)->Value{json!({"intervals":self.intervals,"inferred_intervals":self.inferred_intervals,"equal_intervals":self.equal_intervals,"allocations":self.allocations,"equal_allocations":self.equal_allocations,"loss":self.loss.to_string(),"matched_hp_explicit":self.matched.to_string(),"inferred":self.inferred.to_string(),"equal_share_amount":self.equal.to_string(),"single_source_amount":self.single.to_string(),"unassigned":self.unassigned.to_string(),"equal_pct_of_inferred":pct(self.equal,self.inferred),"equal_pct_of_hp_loss":pct(self.equal,self.loss)})}
}
fn pct(n:u128,d:u128)->Option<f64>{(d!=0).then(||100.0*n as f64/d as f64)}
struct Collector{seen:u64,max_batch:u64,all:Tally,bosses:[Tally;3]}
impl Default for Collector{fn default()->Self{Self{seen:0,max_batch:0,all:Tally::default(),bosses:std::array::from_fn(|_|Tally::default())}}}
const BOSSES:[u32;3]=[2948385,3387349,4425809];
impl Collector{
    fn observe(&mut self,h:&Health)->Result<()>{
        // Evictions+retained length counts APPENDS, not creation IDs: different
        // targets can close out of order. Never assume monotonic closing IDs.
        let total=h.audit_evictions+h.audit.len() as u64;let new=total.checked_sub(self.seen).ok_or("audit append counter moved backwards")?;
        if new>h.audit.len() as u64{return Err("audit overflow within one input: exact fallback amount unavailable".into());}
        self.max_batch=self.max_batch.max(new);
        for a in h.audit.iter().rev().take(new as usize){self.all.add(a);for (i,target) in BOSSES.into_iter().enumerate(){if a.target.actor==target{self.bosses[i].add(a);}}}
        self.seen=total;Ok(())
    }
}
fn main(){if let Err(e)=run(){eprintln!("{e}");std::process::exit(1);}}
fn run()->Result<()>{
    let inputs:Vec<_>=std::env::args().skip(1).collect();if inputs.is_empty(){return Err("usage: equal_share_census CAPTURE...".into());}
    for input in inputs{let start=std::time::Instant::now();let mut r=Replay::new(false);let mut c=Collector::default();capture_file::read(Path::new(&input),None,|i|{r.feed(i)?;c.observe(&r.health)})?;r.finish();c.observe(&r.health)?;
        if let Some(e)=r.semantic_error{return Err(e);}
        if c.all.loss!=r.health.loss || c.all.inferred!=r.health.allocated || c.all.matched!=r.health.explicit || c.all.unassigned!=r.health.unassigned{return Err("streaming tally differs from lifetime HP totals".into());}
        if c.all.equal_intervals!=r.health.outcomes.get("equal share").copied().unwrap_or(0){return Err("equal-share interval count mismatch".into());}
        let displayed=r.meter.session.total.damage+r.meter.session.estimated_total.damage;
        println!("{}",json!({"input":input,"scope":"current default file replay; all capture targets; raw explicit plus inferred totals are not physical-damage truth","frames":r.frames,"explicit":r.meter.session.total.damage.to_string(),"meter_inferred":r.meter.session.estimated_total.damage.to_string(),"capture_combined":displayed.to_string(),"equal_pct_of_capture_combined":pct(c.all.equal,displayed),"all":c.all.json(),"bosses":BOSSES.into_iter().enumerate().map(|(i,target)|json!({"target":target,"tally":c.bosses[i].json()})).collect::<Vec<_>>(),"audit_rows_seen":c.seen,"audit_rows_evicted":r.health.audit_evictions,"max_rows_closed_per_input":c.max_batch,"elapsed_seconds":start.elapsed().as_secs_f64()}));
    }Ok(())
}
