//! Same-EXE elevated capture worker and unprivileged Rust controller.
use crate::{platform::*,wire};
use mabi_meter_core::{Result,capture_file::{self,Input},transport::Segment,replay::Replay,session::Session,meter::{Ack,Command,Snapshot}};
use std::{ptr::{null,null_mut},collections::{BTreeSet,VecDeque},time::{Duration,Instant,SystemTime,UNIX_EPOCH},sync::{mpsc::{Receiver,SyncSender,TryRecvError},Arc,Mutex},io::Write,path::PathBuf};
pub type Updates=Arc<Mutex<Option<Result<(Snapshot,String)>>>>;
fn now_ns()->u64{SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().min(u64::MAX as u128) as u64}
fn sleep(){std::thread::sleep(Duration::from_millis(3));}
fn hex(b:&[u8])->String{b.iter().map(|v|format!("{v:02x}")).collect()}
pub fn data_dir()->Result<PathBuf>{let exe=std::env::current_exe().map_err(|e|e.to_string())?;Ok(exe.parent().ok_or("No application directory")?.join("recordings"))}
struct Pipe {handle:Owned,buffer:Vec<u8>}
impl Pipe {
    fn new(handle:Handle)->Result<Self>{if handle as isize == -1 || handle.is_null(){Err(error("Create private capture pipe"))}else{Ok(Self{handle:Owned(handle),buffer:Vec::new()})}}
    fn send(&mut self,data:&[u8])->Result<()> {unsafe {
        if data.len()>wire::MAX{return Err("IPC message size limit".into());}
        let mut bytes=(data.len() as u32).to_le_bytes().to_vec();bytes.extend_from_slice(data);let mut offset=0;let deadline=Instant::now()+Duration::from_secs(10);
        while offset<bytes.len(){let mut n=0;let count=(bytes.len()-offset).min(16384);
            if WriteFile(self.handle.0,bytes[offset..].as_ptr(),count as u32,&mut n,null_mut())==0{return Err(error("Capture pipe write"));}
            offset+=n as usize;if n==0{if Instant::now()>deadline{return Err("Capture pipe backpressure timeout".into());}sleep();}
        }Ok(())
    }}
    fn recv(&mut self)->Result<Option<Vec<u8>>> {unsafe {
        let mut available=0;
        if PeekNamedPipe(self.handle.0,null_mut(),0,null_mut(),&mut available,null_mut())==0{return Err(error("Capture pipe disconnected"));}
        if available>0{let capacity=(available as usize).min(wire::MAX+4-self.buffer.len());
            if capacity==0{return Err("Capture IPC buffered length limit".into());}
            let mut bytes=vec![0;capacity];let mut n=0;
            if ReadFile(self.handle.0,bytes.as_mut_ptr(),capacity as u32,&mut n,null_mut())==0{return Err(error("Capture pipe read"));}
            self.buffer.extend_from_slice(&bytes[..n as usize]);
        }
        if self.buffer.len()<4{return Ok(None);}let n=u32::from_le_bytes(self.buffer[..4].try_into().unwrap()) as usize;
        if !(10..=wire::MAX).contains(&n){return Err("Invalid capture IPC length".into());}
        if self.buffer.len()<n+4{return Ok(None);}let result=self.buffer[4..4+n].to_vec();self.buffer.drain(..4+n);Ok(Some(result))
    }}
    fn wait(&mut self)->Result<Vec<u8>>{let deadline=Instant::now()+Duration::from_secs(90);loop{if let Some(bytes)=self.recv()?{return Ok(bytes);}if Instant::now()>deadline{return Err("Capture worker handshake timed out".into());}sleep();}}
}
#[repr(C)] struct ShellInfo {size:u32,mask:u32,window:Handle,verb:*const u16,file:*const u16,parameters:*const u16,directory:*const u16,show:i32,instance:Handle,id_list:Handle,class:*const u16,class_key:Handle,hotkey:u32,icon:Handle,process:Handle}
#[link(name="shell32")] extern "system" {fn ShellExecuteExW(info:*mut ShellInfo)->i32;}
#[link(name="kernel32")] extern "system" {fn GetProcessId(h:Handle)->u32;}
fn launch(pipe_name:&str,key:&[u8;32],seconds:u64)->Result<Owned>{unsafe {
    let exe=std::env::current_exe().map_err(|e|e.to_string())?;let file=wide(&exe.to_string_lossy());let verb=wide("runas");
    let parameters=wide(&format!("--capture-worker {pipe_name} {} {} {seconds}",hex(key),std::process::id()));let directory=wide(&exe.parent().ok_or("Missing application directory")?.to_string_lossy());
    let mut info:ShellInfo=std::mem::zeroed();info.size=std::mem::size_of::<ShellInfo>() as u32;info.mask=0x140;info.verb=verb.as_ptr();info.file=file.as_ptr();info.parameters=parameters.as_ptr();info.directory=directory.as_ptr();info.show=0;
    if ShellExecuteExW(&mut info)==0{return Err(error("Administrator approval for capture"));}Ok(Owned(info.process))
}}
fn server(seconds:u64)->Result<(Pipe,Owned)> {unsafe {
    mabi_capture::win::capability_check().map_err(|e|format!("Windows capture API unavailable: {e}"))?;
    let key=random()?;let name=format!("\\\\.\\pipe\\mabi-rust-{}",hex(&key[..16]));
    let sddl=wide(&format!("D:P(A;;GA;;;SY)(A;;GA;;;{})",user_sid()?));let mut descriptor=null_mut();
    if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(),1,&mut descriptor,null_mut())==0{return Err(error("Capture pipe ACL"));}
    let security=Security{length:std::mem::size_of::<Security>() as u32,descriptor,inherit:0};
    let handle=CreateNamedPipeW(wide(&name).as_ptr(),3|0x80000,8|1,1,wire::MAX as u32,wire::MAX as u32,0,&security);
    LocalFree(descriptor);let mut pipe=Pipe::new(handle)?;let child=launch(&name,&key,seconds)?;
    let deadline=Instant::now()+Duration::from_secs(90);let pid=GetProcessId(child.0);
    loop {ConnectNamedPipe(pipe.handle.0,null_mut());
        let mut actual=0;if GetNamedPipeClientProcessId(pipe.handle.0,&mut actual)!=0 && actual!=0 {
            if actual!=pid{return Err("Capture worker PID mismatch".into());}break;
        }
        if Instant::now()>deadline || WaitForSingleObject(child.0,0)==0{return Err("Capture worker did not connect".into());}sleep();
    }
    let mut actual=0;if GetNamedPipeClientProcessId(pipe.handle.0,&mut actual)==0 || actual!=pid{return Err("Capture worker PID mismatch".into());}
    pipe.send(&wire::envelope(1,1,&key)?)?;
    let reply=pipe.wait()?;let mut expected=1;let (kind,data)=wire::header(&reply,&mut expected)?;
    if kind!=1 || data!=key{return Err("Capture worker authentication failed".into());}Ok((pipe,child))
}}
fn client(name:&str,key:&[u8],parent:u32)->Result<(Pipe,Owned)> {unsafe {
    if !name.starts_with("\\\\.\\pipe\\mabi-rust-") || name.len()>128 || key.len()!=32 || parent==0{return Err("Invalid private worker parameters".into());}
    let parent_handle=Owned(OpenProcess(0x100000|0x1000,0,parent));if parent_handle.0.is_null(){return Err(error("Capture controller identity"));}
    let mut pipe=Pipe::new(CreateFileW(wide(name).as_ptr(),0xc0000000,0,null_mut(),3,0,null_mut()))?;
    let mode=1;if SetNamedPipeHandleState(pipe.handle.0,&mode,null(),null())==0{return Err(error("Private pipe mode"));}
    let mut actual=0;if GetNamedPipeServerProcessId(pipe.handle.0,&mut actual)==0 || actual!=parent{return Err("Capture controller PID mismatch".into());}
    let hello=pipe.wait()?;let mut expected=1;let (kind,data)=wire::header(&hello,&mut expected)?;
    if kind!=1 || data.len()!=32 || !data.iter().zip(key).fold(true,|same,(a,b)|same & (a==b)){return Err("Capture controller authentication failed".into());}
    pipe.send(&wire::envelope(1,1,key)?)?;Ok((pipe,parent_handle))
}}
struct Native(*mut mabi_capture::win::Capture);
impl Drop for Native {fn drop(&mut self){unsafe{mabi_capture::win::mabi_capture_close(self.0)}}}
impl Native {
    fn open(flows:&[mabi_capture::Flow])->Result<Self>{unsafe{let mut capture=null_mut();let mut message=[0u8;2048];
        if mabi_capture::win::mabi_capture_open_tcp_nics(flows.as_ptr(),flows.len() as u32,&mut capture,message.as_mut_ptr(),message.len() as u32)!=0 {let n=message.iter().position(|v|*v==0).unwrap_or(message.len());return Err(String::from_utf8_lossy(&message[..n]).into_owned());}Ok(Self(capture))}}
}
fn send(pipe:&mut Pipe,sequence:&mut u64,kind:u8,data:&[u8])->Result<()>{pipe.send(&wire::envelope(*sequence,kind,data)?)?;*sequence+=1;Ok(())}
fn send_segments(pipe:&mut Pipe,sequence:&mut u64,epoch:u64,records:Vec<Segment>)->Result<()> {
    let mut batch=Vec::new();let mut bytes=22;for s in records{if !batch.is_empty() && (bytes+54+s.payload.len()>wire::MAX || batch.len()>=1024){send(pipe,sequence,3,&wire::encode_segments(epoch,&batch)?)?;batch.clear();bytes=22;}bytes+=54+s.payload.len();batch.push(s);}
    if !batch.is_empty(){send(pipe,sequence,3,&wire::encode_segments(epoch,&batch)?)?;}Ok(())
}
pub fn worker(args:Vec<std::ffi::OsString>)->Result<()> {
    if args.len()!=4{return Err("Invalid capture worker invocation".into());}
    let name=args[0].to_str().ok_or("Invalid pipe")?;let key=capture_file::unhex(args[1].to_str().ok_or("Invalid secret")?)?;
    let parent=args[2].to_str().ok_or("Invalid parent")?.parse::<u32>().map_err(|e|e.to_string())?;
    let seconds=args[3].to_str().ok_or("Invalid duration")?.parse::<u64>().map_err(|e|e.to_string())?;
    if !(1..=3600).contains(&seconds){return Err("Capture duration outside 1..3600 seconds".into());}
    let (mut pipe,parent_handle)=client(name,&key,parent)?;let mut sequence=2;let mut expected=2;
    let outcome=(||->Result<()>{
        let clock=Instant::now();let mut discovery=0;let mut capture:Option<Native>=None;let mut owners=Vec::new();let mut nics=BTreeSet::new();let mut epoch=0;
        let mut pending=wire::OwnershipBuffer::default();let mut seen=BTreeSet::new();let mut recent=VecDeque::new();
        loop {
            let millis=clock.elapsed().as_millis() as u64;if millis>=seconds*1000 || unsafe{WaitForSingleObject(parent_handle.0,0)}==0{break;}
            if let Some(control)=pipe.recv()?{let (kind,data)=wire::header(&control,&mut expected)?;if kind==8 && data.is_empty(){break;}return Err("Unsupported worker command".into());}
            if millis>=discovery {
                let plan=discover()?;let changed=plan.owners!=owners;
                if changed {capture=None;pending.clear();owners=plan.owners.clone();epoch+=1;seen.clear();recent.clear();
                    let metadata=serde_json::json!({"epoch":epoch,"owners":owners,"observed_ns":now_ns()});send(&mut pipe,&mut sequence,2,metadata.to_string().as_bytes())?;
                }
                pending.approve(&plan.flows,millis);
                let mut flows=platform_flows(&plan.flows)?;
                let selected:BTreeSet<_>=flows.iter().map(|f|(f.adapter_guid,f.adapter_mac)).collect();
                if !selected.is_empty() && (capture.is_none() || selected!=nics) {
                    if capture.is_some(){send_segments(&mut pipe,&mut sequence,epoch,pending.drain(millis,true))?;drop(capture.take());pending.clear();pending.approve(&plan.flows,millis);send(&mut pipe,&mut sequence,5,b"capture_handover")?;}
                    let mut unique=BTreeSet::new();flows.retain(|f|unique.insert((f.adapter_guid,f.adapter_mac)));nics=selected;
                    capture=Some(Native::open(&flows)?);send(&mut pipe,&mut sequence,4,b"Capture running; learning identities")?;
                }
                if owners.is_empty(){capture=None;nics.clear();send(&mut pipe,&mut sequence,4,b"Waiting for game")?;}
                discovery=millis+500;
            }
            if let Some(c)=&capture {unsafe {
                let fault=mabi_capture::win::mabi_capture_fault(c.0);if fault!=0{return Err(format!("Windows capture stream error {fault}"));}
                let lost=mabi_capture::win::mabi_capture_loss(c.0);
                let oversized=mabi_capture::win::mabi_capture_oversized(c.0);let mut queue=[0u64;2];mabi_capture::win::mabi_capture_queue_reports(c.0,queue.as_mut_ptr(),2);
                if lost!=0{send(&mut pipe,&mut sequence,5,format!("native_capture_reports:{lost},oversized={oversized},contended={},full={}",queue[0],queue[1]).as_bytes())?;}
                for _ in 0..256 {
                    let mut p: mabi_capture::Packet=std::mem::zeroed();let result=mabi_capture::win::mabi_capture_poll(c.0,&mut p);
                    if result<0{return Err("Native capture queue failed".into());}if result==0{break;}
                    let marker=(p.component,p.group,p.packet_count,p.timestamp,p.direction);
                    if !seen.insert(marker){continue;}recent.push_back(marker);if recent.len()>4096{seen.remove(&recent.pop_front().unwrap());}
                    if p.missed_read!=0 || p.missed_write!=0{send(&mut pipe,&mut sequence,5,b"packet_stream_reports")?;}
                    let link=match p.kind{1=>1,3=>101,_=>continue};
                    let timestamp=p.timestamp.checked_sub(116444736000000000).and_then(|v|v.checked_mul(100)).ok_or("Unsupported Packet Monitor timestamp layout")?;
                    if timestamp.abs_diff(now_ns())>600_000_000_000{return Err("Unsupported Packet Monitor timestamp layout".into());}
                    if p.length as usize>p.data.len(){return Err("Invalid native packet length".into());}
                    match capture_file::tcp_packet(&p.data[..p.length as usize],link,timestamp){Ok(Some(s))=>pending.push(s,millis),Ok(None)=>(),Err(_)=>send(&mut pipe,&mut sequence,5,b"packet_rejected")?}
                }
            }}
            send_segments(&mut pipe,&mut sequence,epoch,pending.drain(clock.elapsed().as_millis() as u64,false))?;
            if pending.lost!=0{send(&mut pipe,&mut sequence,5,format!("ownership_buffer_reports:{}",pending.lost).as_bytes())?;pending.lost=0;}
            sleep();
        }
        send_segments(&mut pipe,&mut sequence,epoch,pending.drain(clock.elapsed().as_millis() as u64,true))?;
        send(&mut pipe,&mut sequence,6,&pending.unowned.to_le_bytes())?;Ok(())
    })();
    if let Err(e)=&outcome{let _=send(&mut pipe,&mut sequence,7,e.as_bytes());}outcome
}
fn platform_flows(flows:&[mabi_meter_core::transport::Flow])->Result<Vec<mabi_capture::Flow>> {if flows.is_empty(){Ok(Vec::new())}else{native_flows(flows)}}
pub fn controller(commands:Receiver<Command>,acks:SyncSender<Ack>,updates:Updates,seconds:u64,wait_after:bool)->Result<()> {
    let mut replay=Replay::new(false);replay.live_scope=true;replay.meter.automatic_encounters=true;
    let directory=data_dir()?;std::fs::create_dir_all(&directory).map_err(|e|e.to_string())?;
    let stem=format!("live-{}-{}",now_ns()/1_000_000_000,hex(&random()?[..4]));let path=directory.join(format!("{stem}.jsonl"));
    let mut record=std::io::BufWriter::new(std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e|e.to_string())?);
    let mut session=Session::new(replay,Some(directory.join("identity-cache.json")));let mut metadata=Vec::new();
    *updates.lock().unwrap()=Some(Ok((session.replay.meter.snapshot(),"Waiting for administrator approval".into())));
    let (mut pipe,worker_handle)=server(seconds)?;let mut expected=2;let mut epoch=0;let mut first=true;let mut terminal=false;
    let mut last_publish=Instant::now()-Duration::from_secs(1);let mut status="Learning identities".to_string();let start=Instant::now();let mut stopping=false;
    let outcome=(||->Result<()>{loop {
        loop {match commands.try_recv(){
            Ok(c)=>{let ack=session.command(c);acks.try_send(ack).map_err(|_|"UI acknowledgement queue is full")?;},
            Err(TryRecvError::Empty)=>break,Err(TryRecvError::Disconnected)=>{if !stopping{pipe.send(&wire::envelope(2,8,&[])?)?;stopping=true;}break;}
        }}
        let mut received=false;
        for _ in 0..64 {let Some(message)=pipe.recv()? else{break;};received=true;let (kind,data)=wire::header(&message,&mut expected)?;
            match kind {
                2=>{let v:serde_json::Value=serde_json::from_slice(data).map_err(|e|e.to_string())?;let next=v["epoch"].as_u64().ok_or("Missing ownership epoch")?;
                    if next<=epoch{return Err("Non-increasing ownership epoch".into());}
                    if !first{session.replay.process_replacement(next)?;}first=false;epoch=next;metadata.push(v);
                },
                3=>{let (batch_epoch,segments)=wire::decode_segments(data)?;if batch_epoch!=epoch{return Err("Packet ownership epoch mismatch".into());}
                    for s in segments {
                        let line=serde_json::json!({"timestamp_ns":s.timestamp_ns,"src":s.flow.src,"sport":s.flow.sport,"dst":s.flow.dst,"dport":s.flow.dport,"seq":s.seq,"syn":s.syn,"fin":s.fin,"rst":s.rst,"ack":s.ack,"payload_hex":hex(&s.payload)});
                        serde_json::to_writer(&mut record,&line).map_err(|e|e.to_string())?;record.write_all(b"\n").map_err(|e|e.to_string())?;
                        session.replay.feed(Input::Segment(s))?;
                    }
                },
                4=>status=String::from_utf8(data.to_vec()).map_err(|e|e.to_string())?,
                5=>{let detail=String::from_utf8(data.to_vec()).map_err(|e|e.to_string())?;let code=detail.split(':').next().unwrap_or("capture_report").to_string();
                    let reset=code=="capture_handover"||code=="packet_rejected";
                    let line=serde_json::json!({"type":"loss","code":code,"detail":detail,"reset_streams":reset});
                    serde_json::to_writer(&mut record,&line).map_err(|e|e.to_string())?;record.write_all(b"\n").map_err(|e|e.to_string())?;session.replay.feed(Input::Loss{code,reset})?;
                },
                6=>{terminal=true;status="Capture finished; recordings saved".into();break;},
                7=>return Err(String::from_utf8_lossy(data).into_owned()),_=>return Err("Unsupported capture message".into())
            }
        }
        session.replay.tick(now_ns().saturating_sub(1_250_000_000))?;
        if last_publish.elapsed()>=Duration::from_millis(200)||terminal {
            let snapshot=session.replay.meter.snapshot();let learned=session.replay.meter.registry.tracking.appearances.len();
            let self_status=if snapshot.self_binding.key.is_some(){"Self identified"}else{"Learning self identity"};
            *updates.lock().unwrap()=Some(Ok((snapshot,format!("{status}  ·  {self_status}  ·  {learned} players observed"))));last_publish=Instant::now();record.flush().map_err(|e|e.to_string())?;
        }
        if terminal{break;}
        if unsafe{WaitForSingleObject(worker_handle.0,0)}==0 && !received{return Err("Capture worker exited without final acknowledgement".into());}
        if start.elapsed()>Duration::from_secs(seconds+100){return Err("Capture controller deadline exceeded".into());}sleep();
    }Ok(())})();
    if outcome.is_err(){session.replay.reset_inference();}session.replay.finish();record.flush().map_err(|e|e.to_string())?;
    let cache_error=session.cache_path.as_ref().and_then(|path|mabi_meter_core::identity::cache::save(&session.replay.meter.registry,path).err());
    *updates.lock().unwrap()=Some(Ok((session.replay.meter.snapshot(),if terminal{"Capture finished; recordings saved".into()}else{"Capture interrupted; recording saved".into()})));
    let meta=serde_json::json!({"format":"mabi-dps-rust-live-v1","ownership_epochs":metadata,"completed":terminal,"error":outcome.as_ref().err(),"cache_error":cache_error,"capture_file":path.file_name().map(|p|p.to_string_lossy())});
    std::fs::write(directory.join(format!("{stem}.meta.json")),serde_json::to_vec_pretty(&meta).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    std::fs::write(directory.join(format!("{stem}.audit.json")),serde_json::to_vec_pretty(&session.replay.audit()).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if outcome.is_ok() && wait_after {while let Ok(c)=commands.recv(){if acks.send(session.command(c)).is_err(){break;}}}outcome
}
