//! Default production-path attribution evaluation. Input ablations belong in
//! independent fixture generation, never production detection/finalization.
use mabi_meter_core::replay::Replay;
fn main(){let path=std::env::args().nth(1).expect("usage: evaluate_attribution CAPTURE");match Replay::load(std::path::Path::new(&path),None,false){Ok(r)=>println!("{}",r.audit()),Err(e)=>{eprintln!("{e}");std::process::exit(1);}}}
