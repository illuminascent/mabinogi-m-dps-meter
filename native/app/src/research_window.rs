//! Standalone native epoch controls. The raw recorder runs on a separate thread.
use crate::{live,platform::{wide,Handle}};
use mabi_meter_core::Result;
use serde_json::{json,Value};
use std::{cell::Cell,fs::{File,OpenOptions},io::Write,path::PathBuf,ptr::{null,null_mut},
    sync::mpsc::{self,Receiver,SyncSender},thread::{self,JoinHandle},time::{Instant,SystemTime,UNIX_EPOCH}};

#[repr(C)] #[derive(Default)] struct Point { x:i32,y:i32 }
#[repr(C)] #[derive(Default)] struct Rect { left:i32,top:i32,right:i32,bottom:i32 }
#[repr(C)] struct Message { hwnd:Handle,message:u32,wparam:usize,lparam:isize,time:u32,pt:Point,private:u32 }
#[repr(C)] struct Class {
    size:u32,style:u32,procedure:Option<unsafe extern "system" fn(Handle,u32,usize,isize)->isize>,
    class_extra:i32,window_extra:i32,instance:Handle,icon:Handle,cursor:Handle,background:Handle,
    menu:*const u16,name:*const u16,small_icon:Handle,
}
#[repr(C)] struct MinMax { reserved:Point,max_size:Point,max_position:Point,min_track:Point,max_track:Point }
#[link(name="user32")] extern "system" {
    fn RegisterClassExW(class:*const Class)->u16;
    fn CreateWindowExW(ex:u32,class:*const u16,title:*const u16,style:u32,x:i32,y:i32,w:i32,h:i32,parent:Handle,menu:Handle,instance:Handle,param:Handle)->Handle;
    fn DefWindowProcW(h:Handle,m:u32,w:usize,l:isize)->isize;
    fn GetMessageW(message:*mut Message,h:Handle,min:u32,max:u32)->i32;
    fn TranslateMessage(message:*const Message)->i32;
    fn DispatchMessageW(message:*const Message)->isize;
    fn IsDialogMessageW(h:Handle,message:*mut Message)->i32;
    fn ShowWindow(h:Handle,show:i32)->i32;
    fn DestroyWindow(h:Handle)->i32;
    fn PostQuitMessage(code:i32);
    fn SetWindowLongPtrW(h:Handle,index:i32,value:isize)->isize;
    fn GetWindowLongPtrW(h:Handle,index:i32)->isize;
    fn GetClientRect(h:Handle,rect:*mut Rect)->i32;
    fn MoveWindow(h:Handle,x:i32,y:i32,w:i32,height:i32,repaint:i32)->i32;
    fn SetWindowPos(h:Handle,after:Handle,x:i32,y:i32,w:i32,height:i32,flags:u32)->i32;
    fn LoadCursorW(instance:Handle,name:*const u16)->Handle;
    fn LoadIconW(instance:Handle,name:*const u16)->Handle;
    fn SetTimer(h:Handle,id:usize,ms:u32,callback:*const std::ffi::c_void)->usize;
    fn KillTimer(h:Handle,id:usize)->i32;
    fn SendMessageW(h:Handle,m:u32,w:usize,l:isize)->isize;
    fn PostMessageW(h:Handle,m:u32,w:usize,l:isize)->i32;
    fn GetWindowTextLengthW(h:Handle)->i32;
    fn GetWindowTextW(h:Handle,text:*mut u16,count:i32)->i32;
    fn SetWindowTextW(h:Handle,text:*const u16)->i32;
    fn EnableWindow(h:Handle,enabled:i32)->i32;
    fn GetDpiForWindow(h:Handle)->u32;
    fn GetDpiForSystem()->u32;
    fn SetProcessDpiAwarenessContext(context:Handle)->i32;
}
#[link(name="kernel32")] extern "system" { fn GetModuleHandleW(name:*const u16)->Handle; }
#[link(name="gdi32")] extern "system" {
    fn CreateFontW(height:i32,width:i32,escape:i32,orientation:i32,weight:i32,italic:u32,underline:u32,strike:u32,charset:u32,output:u32,clip:u32,quality:u32,family:u32,face:*const u16)->Handle;
    fn DeleteObject(h:Handle)->i32;
}
#[link(name="shell32")] extern "system" {
    fn ShellExecuteW(h:Handle,verb:*const u16,file:*const u16,parameters:*const u16,directory:*const u16,show:i32)->isize;
}

const START:usize=101;
const END:usize=102;
const FOLDER:usize=103;
const QUIT:usize=104;
fn now_ns()->u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().min(u64::MAX as u128) as u64 }
unsafe fn set(h:Handle,text:&str) { SetWindowTextW(h,wide(text).as_ptr()); }
unsafe fn text(h:Handle)->String {
    let mut buffer=vec![0;GetWindowTextLengthW(h).max(0) as usize+1];
    let count=GetWindowTextW(h,buffer.as_mut_ptr(),buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..count.max(0) as usize])
}
struct Smoke { file:File,directory:PathBuf,stage:u8,started:Instant }
struct State {
    controls:Vec<Handle>,font:Handle,dpi:u32,commands:Option<SyncSender<Result<Value>>>,events:Receiver<Value>,
    worker:Option<JoinHandle<Result<()>>>,directory:Option<PathBuf>,active:Option<Value>,completed:u64,
    ready:bool,pending:bool,closing:bool,done:bool,failed:bool,finished:bool,log:String,smoke:Option<Smoke>,
}
impl State {
    fn px(&self,n:i32)->i32 { n*self.dpi as i32/96 }
    unsafe fn layout(&self,h:Handle) {
        let mut r=Rect::default();GetClientRect(h,&mut r);
        let width=r.right-self.px(32);
        // State, explanation, stats, timer, label caption/edit, notes caption/edit,
        // Start / End / folder / Quit, recording path, bounded event log.
        for (i,y,height) in [(0,16,42),(1,60,48),(2,114,24),(3,142,24),(4,178,24),(5,205,28),
            (6,244,24),(7,272,90),(12,414,50)] {
            MoveWindow(self.controls[i],self.px(16),self.px(y),width,self.px(height),1);
        }
        for (i,x,w) in [(8,16,115),(9,139,115),(10,262,150)] {
            MoveWindow(self.controls[i],self.px(x),self.px(378),self.px(w),self.px(30),1);
        }
        MoveWindow(self.controls[11],r.right-self.px(141),self.px(378),self.px(125),self.px(30),1);
        MoveWindow(self.controls[13],self.px(16),self.px(470),width,(r.bottom-self.px(486)).max(self.px(30)),1);
    }
    unsafe fn fonts(&mut self) {
        let old=self.font;
        self.font=CreateFontW(-self.px(15),0,0,0,400,0,0,0,1,0,0,5,0,wide("Segoe UI").as_ptr());
        for &control in &self.controls { SendMessageW(control,0x30,self.font as usize,1); }
        if !old.is_null() { DeleteObject(old); }
    }
    unsafe fn append(&mut self,line:&str) {
        self.log.push_str(line);self.log.push_str("\r\n");
        if self.log.len()>24*1024 {
            let cut=self.log.char_indices().find(|(i,_)|*i>=8*1024).map(|(i,_)|i).unwrap_or(0);
            self.log.drain(..cut);
        }
        set(self.controls[13],&self.log);
        SendMessageW(self.controls[13],0xb1,usize::MAX,-1); // select end
        SendMessageW(self.controls[13],0xb7,0,0); // scroll caret
    }
    unsafe fn buttons(&self) {
        let enabled=self.ready && !self.pending && !self.closing && !self.done && !self.failed;
        EnableWindow(self.controls[8],(enabled && self.active.is_none()) as i32);
        EnableWindow(self.controls[9],(enabled && self.active.is_some()) as i32);
        EnableWindow(self.controls[10],self.directory.is_some() as i32);
        EnableWindow(self.controls[11],(!self.closing || self.done) as i32);
        set(self.controls[11],if self.done {"Close"} else {"Quit and save"});
    }
    unsafe fn command(&mut self,action:&str) {
        if !self.ready || self.pending || self.closing || self.done || self.failed { return; }
        let value=json!({"action":action,"label":text(self.controls[5]),"notes":text(self.controls[7])});
        let result=crate::research::Recorder::validate_command(&value).map(|_|()).and_then(|_| {
            self.commands.as_ref().ok_or("Capture controls disconnected")?.try_send(Ok(value)).map_err(|e|e.to_string())
        });
        match result { Ok(())=>self.pending=true,Err(e)=>self.append(&format!("Command not sent: {e}")) }
        self.buttons();
    }
    unsafe fn event(&mut self,event:Value) {
        match event["type"].as_str().unwrap_or("") {
            "status"=>{
                let status=event["status"].as_str().unwrap_or("Capture status unavailable");
                self.ready=status!="Waiting for administrator approval";
                set(self.controls[0],status);
                self.directory=event["directory"].as_str().map(PathBuf::from);
                if let Some(path)=&self.directory { set(self.controls[12],&path.to_string_lossy()); }
                set(self.controls[2],&format!("{} TCP segments   |   {:.2} MiB payload   |   {} capture reports",
                    event["segments"].as_u64().unwrap_or(0),event["payload_bytes"].as_u64().unwrap_or(0) as f64/1048576.0,
                    event["capture_reports"].as_u64().unwrap_or(0)));
                self.active=event.get("active_epoch").filter(|v|v.is_object()).cloned();
                self.completed=event["completed_epochs"].as_u64().unwrap_or(0);
            },
            "ack"=>{self.pending=false;self.append(match event["action"].as_str() {
                Some("start")=>"Epoch started.",Some("end")=>"Epoch ended; warm-up capture continues.",_=>"Saving and draining capture...",
            });},
            "rejected"=>{self.pending=false;self.append(&format!("Command rejected: {}",event["error"]));},
            "finished"=>{
                if let Some(summary)=event.get("summary") { self.event(summary.clone()); }
                self.finished=true;self.ready=false;self.active=None;
                self.failed=event.get("error").is_some_and(|v|!v.is_null());
                let message=if self.failed {format!("Capture interrupted: {}",event["error"])} else {"Capture finished; session files saved.".into()};
                set(self.controls[0],&message);self.append(&message);
            },
            _=>{},
        }
    }
    unsafe fn close(&mut self,h:Handle) {
        if self.done { DestroyWindow(h);return; }
        self.closing=true;
        // Channel EOF follows queued commands. The shared session sends stop,
        // drains the worker and finalizes files; no process is terminated.
        self.commands.take();
        set(self.controls[0],"Stopping and saving... Finish or dismiss any pending UAC prompt.");
        self.buttons();
    }
    unsafe fn poll(&mut self,h:Handle) {
        for _ in 0..64 { match self.events.try_recv() { Ok(event)=>self.event(event),Err(_)=>break } }
        if self.worker.as_ref().is_some_and(|worker|worker.is_finished()) {
            let result=self.worker.take().unwrap().join().unwrap_or_else(|_|Err("Capture controller panicked".into()));
            // Join precedes the last drain: completion cannot overtake final status.
            while let Ok(event)=self.events.try_recv() { self.event(event); }
            self.done=true;self.active=None;self.commands.take();
            if let Err(e)=result {
                self.failed=true;set(self.controls[0],&format!("Capture interrupted: {e}"));self.append(&e);
            } else if !self.finished {
                self.failed=true;self.append("Capture ended without a completion summary. Session may be incomplete.");
                set(self.controls[0],"Session may be incomplete; see messages and recording files.");
            }
        }
        if let Some(active)=&self.active {
            let seconds=now_ns().saturating_sub(active["start_ns"].as_u64().unwrap_or(now_ns()))/1_000_000_000;
            set(self.controls[3],&format!("Epoch {}: {}   |   {:02}:{:02}",active["id"],active["label"].as_str().unwrap_or(""),seconds/60,seconds%60));
        } else { set(self.controls[3],if self.done {"Capture stopped"} else {"No active epoch; warm-up continues"}); }
        self.buttons();
        self.smoke_step(h);
        if self.closing && self.done && self.smoke.is_none() {
            if self.failed {self.closing=false;self.buttons();} else {DestroyWindow(h);}
        }
    }
    unsafe fn smoke_step(&mut self,h:Handle) {
        let Some(mut smoke)=self.smoke.take() else {return;};
        let timed_out=smoke.started.elapsed().as_secs()>15;
        match smoke.stage {
            0 if self.ready=>{
                set(self.controls[5],"比較 / two players");set(self.controls[7],"Warm-up retained\r\nPlayer A attacks only");
                PostMessageW(h,0x111,START,0);smoke.stage=1;
            },
            1 if self.active.is_some() && !self.pending=>{
                set(self.controls[7],"End note: Player B observed 123 HP");PostMessageW(h,0x111,END,0);smoke.stage=2;
            },
            2 if self.active.is_none() && self.completed==1 && !self.pending=>{PostMessageW(h,0x111,START,0);smoke.stage=3;},
            3 if self.active.is_some()=>{PostMessageW(h,0x10,0,0);smoke.stage=4;},
            _=>{},
        }
        if timed_out || self.done {
            let check=(||->Result<Value> {
                if timed_out || self.failed || smoke.stage!=4 {return Err("Research UI smoke did not complete the command/save sequence".into());}
                let meta:Value=serde_json::from_slice(&std::fs::read(smoke.directory.join("session.meta.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
                if meta["completed"]!=true || meta["epochs"].as_array().map(Vec::len)!=Some(2)
                    || meta["epochs"][0]["label"]!="比較 / two players"
                    || meta["epochs"][0]["notes"]!="Warm-up retained\r\nPlayer A attacks only"
                    || meta["epochs"][0]["end_reason"]!="manual"
                    || meta["epochs"][0]["end_notes"]!="End note: Player B observed 123 HP"
                    || meta["epochs"][1]["end_reason"]!="session_stop" {return Err("Research UI smoke metadata mismatch".into());}
                Ok(meta)
            })();
            let report=json!({"passed":check.is_ok(),"capture_started":false,"stage":smoke.stage,
                "error":check.as_ref().err(),"session":check.as_ref().ok()});
            let written=smoke.file.write_all(report.to_string().as_bytes());
            if check.is_err() || written.is_err() {self.failed=true;}
            self.commands.take();self.closing=false;DestroyWindow(h);
        } else {self.smoke=Some(smoke);}
    }
}

thread_local! { static DISPATCHING:Cell<bool>=const {Cell::new(false)}; }
struct Guard;
impl Drop for Guard { fn drop(&mut self) { DISPATCHING.with(|v|v.set(false)); } }
unsafe extern "system" fn procedure(h:Handle,message:u32,w:usize,l:isize)->isize {
    // Destruction can be synchronous while a timer/close handler owns State.
    if message==0x2 {KillTimer(h,1);PostQuitMessage(0);return 0;}
    let pointer=GetWindowLongPtrW(h,-21) as *mut State;
    // Child text/font/layout operations can synchronously reenter this callback.
    if pointer.is_null() || DISPATCHING.with(|v|v.replace(true)) {return DefWindowProcW(h,message,w,l);}
    let _guard=Guard;let state=&mut *pointer;
    match message {
        0x5=>{state.layout(h);0},
        0x24=>{let size=&mut *(l as *mut MinMax);size.min_track=Point{x:state.px(680),y:state.px(620)};0},
        0x2e0=>{
            state.dpi=(w&0xffff) as u32;state.fonts();let r=&*(l as *const Rect);
            SetWindowPos(h,null_mut(),r.left,r.top,r.right-r.left,r.bottom-r.top,0x14);state.layout(h);0
        },
        0x113=>{state.poll(h);0},
        0x111 if w>>16==0=>{
            match w&0xffff {
                START=>state.command("start"),END=>state.command("end"),QUIT=>state.close(h),
                FOLDER=>if let Some(path)=&state.directory {
                    let opened=ShellExecuteW(h,wide("open").as_ptr(),wide(&path.to_string_lossy()).as_ptr(),null(),null(),1);
                    if opened<=32 {state.append(&format!("Could not open recordings folder (Windows shell error {opened})."));}
                },_=>{},
            } 0
        },
        0x10=>{state.close(h);0},
        _=>DefWindowProcW(h,message,w,l),
    }
}

/// The smoke uses the real Recorder and command controls, with a synthetic
/// backend that never calls discovery, UAC, the capture API or the live worker.
fn smoke_backend(directory:PathBuf,commands:Receiver<Result<Value>>,events:SyncSender<Value>)->Result<()> {
    let mut recorder=crate::research::Recorder::new(&directory,now_ns())?;
    let emit=|value|events.send(value).map_err(|e|e.to_string());
    emit(recorder.status("Synthetic research UI check; no capture"))?;
    while let Ok(value)=commands.recv() {
        let value=value?;recorder.command(&value,now_ns())?;
        emit(json!({"type":"ack","action":value["action"]}))?;
        emit(recorder.status("Synthetic research UI check; no capture"))?;
    }
    recorder.command(&json!({"action":"stop"}),now_ns())?;
    recorder.finish(now_ns(),true,None)?;
    emit(json!({"type":"finished","error":null,"summary":recorder.status("Saved")}))
}

pub fn run(output:Option<PathBuf>,report:Option<PathBuf>)->Result<()> {
    let smoke_mode=report.is_some();
    let smoke=if let Some(path)=report {
        let file=OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e|e.to_string())?;
        let directory=path.parent().unwrap_or(std::path::Path::new(".")).join(format!("research-ui-smoke-{}-{}",std::process::id(),now_ns()));
        Some(Smoke{file,directory,stage:0,started:Instant::now()})
    } else {None};
    unsafe {
        SetProcessDpiAwarenessContext(-4isize as Handle);
        let instance=GetModuleHandleW(null());let name=wide("MabiResearchEpochs");
        let class=Class {size:std::mem::size_of::<Class>() as u32,style:3,procedure:Some(procedure),class_extra:0,window_extra:0,
            instance,icon:LoadIconW(instance,1usize as *const u16),cursor:LoadCursorW(null_mut(),32512usize as *const u16),
            background:16usize as Handle,menu:null(),name:name.as_ptr(),small_icon:null_mut()};
        if RegisterClassExW(&class)==0 {return Err(crate::platform::error("Register research window"));}
        let dpi=GetDpiForSystem().max(96);
        let title=if smoke.is_some() {"Mabinogi research - synthetic UI check (no capture)"} else {"Mabinogi research - capture epochs"};
        let h=CreateWindowExW(0x10000,name.as_ptr(),wide(title).as_ptr(),0x00cf0000,i32::MIN,i32::MIN,
            800*dpi as i32/96,680*dpi as i32/96,null_mut(),null_mut(),instance,null_mut());
        if h.is_null() {return Err(crate::platform::error("Create research window"));}
        let child=|class:&str,title:&str,id:usize,style:u32,ex:u32|CreateWindowExW(ex,wide(class).as_ptr(),wide(title).as_ptr(),
            0x50000000|style,0,0,50,30,h,id as Handle,instance,null_mut());
        let controls=vec![
            child("STATIC","Starting capture backend...",0,0,0),
            child("STATIC","Warm-up is retained from launch. Start / End only mark an epoch.\r\nAll approved game TCP is saved in both directions, including unknown messages.",0,0,0),
            child("STATIC","0 TCP segments   |   0.00 MiB payload   |   0 capture reports",0,0,0),
            child("STATIC","No active epoch",0,0,0),
            child("STATIC","Epoch label (for example: 2 players, teammate attacks only)",0,0,0),
            child("EDIT","",0,0x10080,0x200),
            child("STATIC","Notes / ground truth (party, target, actions, HP, measured damage or DPS)",0,0,0),
            child("EDIT","",0,0x110044,0x200),
            child("BUTTON","Start epoch",START,0x10000,0),child("BUTTON","End epoch",END,0x10000,0),
            child("BUTTON","Open recordings",FOLDER,0x10000,0),child("BUTTON","Quit and save",QUIT,0x10000,0),
            child("STATIC","Waiting for session folder...",0,0,0),child("EDIT","",0,0x310844,0x200),
        ];
        if controls.iter().any(|h|h.is_null()) {DestroyWindow(h);return Err("Create research controls failed".into());}
        let (tx,commands)=mpsc::sync_channel(16);let (events_tx,events)=mpsc::sync_channel(64);
        let mut state=Box::new(State {controls,font:null_mut(),dpi:GetDpiForWindow(h).max(96),commands:Some(tx),events,
            worker:None,directory:None,active:None,completed:0,ready:false,pending:false,closing:false,done:false,failed:false,finished:false,log:String::new(),smoke});
        state.fonts();state.layout(h);state.buttons();
        SendMessageW(state.controls[5],0xc5,4096,0);SendMessageW(state.controls[7],0xc5,4096,0);
        SetWindowLongPtrW(h,-21,(&mut *state as *mut State) as isize);
        if SetTimer(h,1,100,null_mut())==0 {
            SetWindowLongPtrW(h,-21,0);DestroyWindow(h);DeleteObject(state.font);
            return Err("Create research UI timer failed".into());
        }
        let synthetic=state.smoke.as_ref().map(|s|s.directory.clone());
        state.worker=Some(thread::spawn(move ||match synthetic {
            Some(directory)=>smoke_backend(directory,commands,events_tx),
            None=>live::research_session(output,commands,|value|events_tx.try_send(value).map_err(|e|format!("Research UI status delivery failed: {e}"))),
        }));
        ShowWindow(h,5);
        let mut message:Message=std::mem::zeroed();let mut result=Ok(());
        loop {
            let status=GetMessageW(&mut message,null_mut(),0,0);
            if status==0 {break;}
            if status<0 {result=Err("Research UI message loop failed".into());break;}
            if IsDialogMessageW(h,&mut message)==0 {TranslateMessage(&message);DispatchMessageW(&message);}
        }
        state.commands.take();
        // An unexpected message-loop failure still lets the recorder drain.
        // Drain status while joining so synthetic bounded sends cannot deadlock.
        if let Some(worker)=state.worker.take() {
            while !worker.is_finished() {while state.events.try_recv().is_ok() {} thread::sleep(std::time::Duration::from_millis(10));}
            let _=worker.join();
        }
        SetWindowLongPtrW(h,-21,0);DeleteObject(state.font);
        if smoke_mode && state.failed {Err("Research UI smoke failed; see report".into())} else {result}
    }
}
