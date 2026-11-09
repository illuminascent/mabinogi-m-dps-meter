//! Frameless native desktop. Capture and aggregation run outside the UI thread.
use crate::{load,Source};
use mabi_meter_core::{identity::EntityKey,meter::{Ack,Action,Command,Snapshot},session::Session,Result};
use std::{cell::Cell,ffi::c_void,ptr::{null,null_mut},sync::{mpsc::{self,Receiver,SyncSender},Arc,Mutex},thread};

thread_local! {
    static DISPATCHING: Cell<bool> = const { Cell::new(false) };
    // Native child painting can reenter while State is borrowed. Its shared
    // brush lives independently of that borrow, on this single UI thread.
    static CONTROL_BRUSH: Cell<Handle> = const { Cell::new(null_mut()) };
}
struct DispatchGuard;
impl Drop for DispatchGuard { fn drop(&mut self) { DISPATCHING.with(|v| v.set(false)); } }

type Handle = *mut c_void;
type WndProc = unsafe extern "system" fn(Handle,u32,usize,isize)->isize;
#[repr(C)] #[derive(Clone,Copy,Default)] struct Point { x: i32,y: i32 }
#[repr(C)] #[derive(Clone,Copy,Default)] struct Rect { left: i32,top: i32,right: i32,bottom: i32 }
#[repr(C)] struct Message { hwnd: Handle,message: u32,wparam: usize,lparam: isize,time: u32,pt: Point,private: u32 }
#[repr(C)] struct WindowClass {
    size: u32,style: u32,procedure: Option<WndProc>,class_extra: i32,window_extra: i32,instance: Handle,
    icon: Handle,cursor: Handle,background: Handle,menu: *const u16,name: *const u16,small_icon: Handle,
}
#[repr(C)] struct Paint { dc: Handle,erase: i32,rect: Rect,restore: i32,update: i32,reserved: [u8;32] }
#[repr(C)] struct MinMax { reserved: Point,max_size: Point,max_position: Point,min_track: Point,max_track: Point }

#[link(name="user32")]
extern "system" {
    fn RegisterClassExW(c: *const WindowClass)->u16;
    fn CreateWindowExW(ex: u32,class: *const u16,title: *const u16,style: u32,x: i32,y: i32,w: i32,h: i32,
        parent: Handle,menu: Handle,instance: Handle,param: *mut c_void)->Handle;
    fn DefWindowProcW(h: Handle,m: u32,w: usize,l: isize)->isize;
    fn GetMessageW(m: *mut Message,h: Handle,min: u32,max: u32)->i32;
    fn TranslateMessage(m: *const Message)->i32;
    fn DispatchMessageW(m: *const Message)->isize;
    fn ShowWindow(h: Handle,show: i32)->i32;
    fn DestroyWindow(h: Handle)->i32;
    fn PostQuitMessage(code: i32);
    fn SetWindowLongPtrW(h: Handle,index: i32,value: isize)->isize;
    fn GetWindowLongPtrW(h: Handle,index: i32)->isize;
    fn GetClientRect(h: Handle,r: *mut Rect)->i32;
    fn ScreenToClient(h: Handle,p: *mut Point)->i32;
    fn GetCursorPos(p: *mut Point)->i32;
    fn LoadCursorW(instance: Handle,name: *const u16)->Handle;
    fn MoveWindow(h: Handle,x: i32,y: i32,w: i32,height: i32,repaint: i32)->i32;
    fn InvalidateRect(h: Handle,r: *const Rect,erase: i32)->i32;
    fn BeginPaint(h: Handle,p: *mut Paint)->Handle;
    fn EndPaint(h: Handle,p: *const Paint)->i32;
    fn FillRect(dc: Handle,r: *const Rect,brush: Handle)->i32;
    fn DrawTextW(dc: Handle,t: *const u16,n: i32,r: *mut Rect,format: u32)->i32;
    fn SetTimer(h: Handle,id: usize,ms: u32,callback: *const c_void)->usize;
    fn KillTimer(h: Handle,id: usize)->i32;
    fn SendMessageW(h: Handle,m: u32,w: usize,l: isize)->isize;
    fn CreatePopupMenu()->Handle;
    fn AppendMenuW(h: Handle,flags: u32,id: usize,text: *const u16)->i32;
    fn TrackPopupMenu(h: Handle,flags: u32,x: i32,y: i32,reserved: i32,owner: Handle,r: *const Rect)->i32;
    fn DestroyMenu(h: Handle)->i32;
    fn SetWindowPos(h: Handle,after: Handle,x: i32,y: i32,w: i32,height: i32,flags: u32)->i32;
    fn SetProcessDpiAwarenessContext(context: Handle)->i32;
    fn GetDpiForWindow(h: Handle)->u32;
    fn GetDpiForSystem()->u32;
    fn PostMessageW(h:Handle,message:u32,w:usize,l:isize)->i32;
    fn LoadIconW(instance:Handle,name:*const u16)->Handle;
    fn GetWindowTextW(h:Handle,text:*mut u16,count:i32)->i32;
    fn SetWindowTextW(h:Handle,text:*const u16)->i32;
    fn GetWindowRect(h:Handle,r:*mut Rect)->i32;
    fn MonitorFromRect(r:*const Rect,flags:u32)->Handle;
    fn GetMonitorInfoW(h:Handle,info:*mut MonitorInfo)->i32;
}
#[link(name="gdi32")]
extern "system" {
    fn CreateCompatibleDC(dc:Handle)->Handle;
    fn CreateCompatibleBitmap(dc:Handle,width:i32,height:i32)->Handle;
    fn DeleteDC(dc:Handle)->i32;
    fn BitBlt(destination:Handle,x:i32,y:i32,width:i32,height:i32,source:Handle,source_x:i32,source_y:i32,operation:u32)->i32;
    fn CreateSolidBrush(color: u32)->Handle;
    fn DeleteObject(h: Handle)->i32;
    fn SelectObject(dc: Handle,obj: Handle)->Handle;
    fn SetTextColor(dc: Handle,color: u32)->u32;
    fn SetBkMode(dc: Handle,mode: i32)->i32;
    fn SetBkColor(dc:Handle,color:u32)->u32;
    fn CreatePen(style:i32,width:i32,color:u32)->Handle;
    fn Polyline(dc:Handle,points:*const Point,count:i32)->i32;
    fn CreateFontW(height: i32,width: i32,escape: i32,orientation: i32,weight: i32,italic: u32,
        underline: u32,strike: u32,charset: u32,output: u32,clip: u32,quality: u32,family: u32,face: *const u16)->Handle;
}
#[link(name="kernel32")]
extern "system" { fn GetModuleHandleW(name: *const u16)->Handle; }

fn wide(s: &str)->Vec<u16> { s.encode_utf16().chain(Some(0)).collect() }
fn rgb(r: u32,g: u32,b: u32)->u32 { r|(g<<8)|(b<<16) }
#[repr(C)] struct MonitorInfo {size:u32,monitor:Rect,work:Rect,flags:u32}
#[derive(Default)]
struct BackBuffer {dc:Handle,bitmap:Handle,previous:Handle,width:i32,height:i32}
impl BackBuffer {
    unsafe fn ensure(&mut self,reference:Handle,width:i32,height:i32)->bool {
        if width<=0 || height<=0{return false;}
        if !self.dc.is_null() && self.width==width && self.height==height{return true;}
        let dc=CreateCompatibleDC(reference);if dc.is_null(){return false;}
        let bitmap=CreateCompatibleBitmap(reference,width,height);
        if bitmap.is_null(){DeleteDC(dc);return false;}
        let previous=SelectObject(dc,bitmap);
        if previous.is_null() || previous as isize == -1 {DeleteObject(bitmap);DeleteDC(dc);return false;}
        *self=Self {dc,bitmap,previous,width,height};true
    }
}
impl Drop for BackBuffer {fn drop(&mut self){unsafe{
    if !self.dc.is_null(){SelectObject(self.dc,self.previous);DeleteObject(self.bitmap);DeleteDC(self.dc);}
}}}
struct Toolbar {encounter:Rect,idle_label:Rect,idle:Rect,seconds:Rect,split:Rect}
struct State {
    commands:SyncSender<Command>,acks:Receiver<Ack>,latest:crate::live::Updates,
    snapshot:Option<Snapshot>,command_id:u64,
    encounters:Vec<u64>,encounter_combo:Handle,search:Handle,idle:Handle,
    filter:String,target_scroll:usize,topmost:bool,dpi:u32,font:Handle,edit_brush:Handle,
    smoke:Option<Smoke>,
    buffer:BackBuffer,
}
struct Smoke {stage:u8,started:std::time::Instant,report:std::path::PathBuf,self_key:Option<EntityKey>}
impl State {
    fn px(&self,n:i32)->i32{n*self.dpi as i32/96}
    fn toolbar(&self,width:i32)->Toolbar {
        let p=|n|self.px(n);
        Toolbar {
            encounter:Rect{left:p(104),top:p(48),right:width-p(300),bottom:p(80)},
            idle_label:Rect{left:width-p(276),top:p(48),right:width-p(204),bottom:p(80)},
            idle:Rect{left:width-p(192),top:p(50),right:width-p(136),bottom:p(78)},
            seconds:Rect{left:width-p(128),top:p(48),right:width-p(112),bottom:p(80)},
            split:Rect{left:width-p(100),top:p(48),right:width-p(20),bottom:p(80)},
        }
    }
    fn send(&mut self,action:Action){
        let Some(s)=&self.snapshot else{return;};self.command_id+=1;
        let c=Command{id:self.command_id,observed_revision:s.revision,encounter:s.encounter,action};
        if let Err(e)=self.commands.try_send(c){crate::diagnostics::error("command_queue",&e.to_string());}
    }
    unsafe fn font(&mut self,h:Handle){
        let old=self.font;self.font=CreateFontW(-self.px(14),0,0,0,400,0,0,0,1,0,0,5,0,wide("Yu Gothic UI").as_ptr());
        for child in [self.encounter_combo,self.search,self.idle]{SendMessageW(child,0x30,self.font as usize,1);}
        if !old.is_null(){DeleteObject(old);}self.layout(h);
    }
    unsafe fn layout(&self,h:Handle){
        let mut r=Rect::default();GetClientRect(h,&mut r);let p=|v|self.px(v);
        let toolbar=self.toolbar(r.right);
        MoveWindow(self.encounter_combo,toolbar.encounter.left,toolbar.encounter.top,toolbar.encounter.right-toolbar.encounter.left,p(320),1);
        MoveWindow(self.search,p(22),p(162),p(176),p(25),1);
        MoveWindow(self.idle,toolbar.idle.left,toolbar.idle.top,toolbar.idle.right-toolbar.idle.left,toolbar.idle.bottom-toolbar.idle.top,1);
    }
    fn visible_targets(&self)->Vec<(Option<EntityKey>,String)>{
        let mut rows=vec![(None,"All targets".into())];
        if let Some(s)=&self.snapshot{rows.extend(s.targets.iter().filter(|(_,name)|name.to_lowercase().contains(&self.filter)).map(|(k,n)|(Some(*k),n.clone())));}
        rows
    }
    unsafe fn update(&mut self,s:Snapshot){
        if self.snapshot.as_ref().is_some_and(|old|s.last_command<old.last_command || s.revision<old.revision){return;}
        let changed=self.snapshot.as_ref().is_none_or(|old|old.encounters!=s.encounters || old.current_encounter!=s.current_encounter);
        if changed {
            SendMessageW(self.encounter_combo,0x14b,0,0);self.encounters.clear();
            for (id,state) in s.encounters.iter().rev(){
                let text=format!("Encounter {id}  /  {:?}{}",state,if *id==s.current_encounter{"  (current)"}else{""});
                SendMessageW(self.encounter_combo,0x143,0,wide(&text).as_ptr() as isize);self.encounters.push(*id);
            }
        }
        if self.snapshot.as_ref().is_some_and(|old|old.encounter!=s.encounter){self.filter.clear();self.target_scroll=0;SetWindowTextW(self.search,wide("").as_ptr());}
        let selection=self.encounters.iter().position(|id|*id==s.encounter).unwrap_or(0);
        if changed || SendMessageW(self.encounter_combo,0x147,0,0)!=selection as isize {
            SendMessageW(self.encounter_combo,0x14e,selection,0);
        }
        self.snapshot=Some(s);
    }
    unsafe fn poll(&mut self,h:Handle){
        let incoming=self.latest.lock().unwrap().take();let mut dirty=false;
        if let Some(snapshot)=incoming{self.update(snapshot);dirty=true;}
        while let Ok(ack)=self.acks.try_recv(){
            self.update(ack.snapshot);dirty=true;
        }
        if dirty{InvalidateRect(h,null(),0);}
        self.smoke_step(h);
    }
    unsafe fn smoke_step(&mut self,h:Handle){
        let Some(mut smoke)=self.smoke.take() else{return;};let mut failure=None;let mut done=false;
        if smoke.started.elapsed()>std::time::Duration::from_secs(15){failure=Some("UI smoke deadline exceeded");}
        else if let Some(snapshot)=&self.snapshot {
            match smoke.stage {
                0=>{if snapshot.summary.damage!=8900 || snapshot.targets.len()!=2 || snapshot.display_hits!=9 || !snapshot.rows.iter().any(|r|r.explicit_hits==4 && r.display_hits==7){failure=Some("Initial demo snapshot mismatch");}
                    else{PostMessageW(h,0x202,0,((self.px(250) as isize)<<16)|self.px(30) as isize);smoke.stage=1;}},
                1 if snapshot.target.is_some()=>{if snapshot.summary.damage!=3000{failure=Some("Target sidebar did not select its own totals");}
                    else {let key=snapshot.rows[0].assignment_key;smoke.self_key=key;self.send(Action::AssignSelf(key));smoke.stage=2;}},
                2 if snapshot.self_binding.key==smoke.self_key=>{let mut r=Rect::default();GetClientRect(h,&mut r);PostMessageW(h,0x202,0,((self.px(62) as isize)<<16)|(r.right-self.px(55)) as isize);smoke.stage=3;},
                3 if snapshot.encounter==2=>{if snapshot.summary.damage!=0 || snapshot.self_binding.key!=smoke.self_key{failure=Some("Split lost warmed self or retained combat totals");}
                    else{SendMessageW(self.encounter_combo,0x14e,1,0);PostMessageW(h,0x111,4|(1<<16),self.encounter_combo as isize);smoke.stage=4;}},
                4 if snapshot.encounter==1=>{if snapshot.summary.damage!=8900 || snapshot.display_hits!=9 || snapshot.target.is_some() || !snapshot.rows.iter().any(|r|r.is_self){failure=Some("Encounter history/All targets/marker mismatch");}else{done=true;}},_=>()
            }
        }
        if done||failure.is_some(){let mut r=Rect::default();GetClientRect(h,&mut r);let result=serde_json::json!({"passed":done,"error":failure,"stage":smoke.stage,"layout":{"client_height":r.bottom,"bottom_padding":self.px(16),"sidebar_bottom":self.content_bottom(r.bottom),"chart_bottom":self.content_bottom(r.bottom),"table_bottom":self.table_bottom(r.bottom),"chart_label_top":self.chart_top(r.bottom)-self.px(26),"footer_reserved":0},"snapshot":self.snapshot.as_ref().map(mabi_meter_core::replay::snapshot_json)});let _=std::fs::write(&smoke.report,result.to_string());PostMessageW(h,0x10,0,0);}else{self.smoke=Some(smoke);}
    }
    unsafe fn text(&self,dc:Handle,text:&str,mut rect:Rect,color:u32,right:bool){
        SetTextColor(dc,color);let text=wide(text);DrawTextW(dc,text.as_ptr(),(text.len()-1) as i32,&mut rect,0x20|4|0x8000|0x800|if right{2}else{0});
    }
    unsafe fn fill(&self,dc:Handle,rect:Rect,color:u32){let brush=CreateSolidBrush(color);FillRect(dc,&rect,brush);DeleteObject(brush);}
    fn content_bottom(&self,height:i32)->i32{height-self.px(16)}
    fn chart_top(&self,height:i32)->i32{self.content_bottom(height)-self.px(64)}
    fn table_bottom(&self,height:i32)->i32{self.chart_top(height)-self.px(30)}
    fn row_height(&self,height:i32)->i32 {((self.table_bottom(height)-self.px(164))/16).clamp(self.px(18),self.px(28))}
    unsafe fn paint(&mut self,h:Handle){
        let mut paint:Paint=std::mem::zeroed();let screen=BeginPaint(h,&mut paint);let mut r=Rect::default();GetClientRect(h,&mut r);
        let buffered=self.buffer.ensure(screen,r.right,r.bottom);
        let dc=if buffered {self.buffer.dc}else{screen};
        let p=|v|self.px(v);let fg=rgb(225,232,242);let muted=rgb(142,157,179);let accent=rgb(100,207,222);
        self.fill(dc,r,rgb(19,24,31));let old=SelectObject(dc,self.font);SetBkMode(dc,1);
        let rect=|x,y,w,height|Rect{left:p(x),top:p(y),right:p(x+w),bottom:p(y+height)};
        self.text(dc,"MABI / DPS",rect(20,6,220,30),fg,false);
        for (text,x,width,color) in [(if self.topmost{"Pinned"}else{"Pin"},r.right-p(158),64,muted),("_",r.right-p(88),36,muted),("×",r.right-p(46),36,fg)]{
            self.text(dc,text,Rect{left:x,top:p(4),right:x+p(width),bottom:p(36)},color,false);
        }
        self.text(dc,"Encounter",rect(20,46,80,32),muted,false);
        let toolbar=self.toolbar(r.right);
        self.text(dc,"Self idle",toolbar.idle_label,muted,false);
        self.text(dc,"s",toolbar.seconds,muted,false);
        self.text(dc,"Split",toolbar.split,accent,false);
        self.fill(dc,Rect{left:p(16),top:p(88),right:r.right-p(16),bottom:p(120)},rgb(28,36,46));
        self.fill(dc,Rect{left:p(16),top:p(132),right:p(208),bottom:self.content_bottom(r.bottom)},rgb(24,31,40));
        self.text(dc,"Search targets",rect(24,132,170,26),muted,false);
        let rows=self.visible_targets();let mut y=p(200);
        for (key,label) in rows.iter().take(1).chain(rows.iter().skip(1+self.target_scroll)){
            if y+p(30)>self.content_bottom(r.bottom)-p(4){break;}
            let selected=self.snapshot.as_ref().is_some_and(|s|s.target==*key);
            if selected{self.fill(dc,Rect{left:p(20),top:y,right:p(204),bottom:y+p(30)},rgb(36,62,75));}
            self.text(dc,label,Rect{left:p(28),top:y,right:p(196),bottom:y+p(30)},if selected{accent}else{fg},false);y+=p(32);
        }
        if let Some(s)=&self.snapshot {
            let dps=s.summary.dps().map_or_else(||"—".into(),|v|format!("{v:.1}"));
            self.text(dc,&format!("Damage {}     DPS {}     {:.2}s     {} hits",number(s.summary.damage),dps,s.summary.duration(),s.display_hits),
                Rect{left:p(28),top:p(88),right:r.right-p(24),bottom:p(120)},fg,false);
            let left=p(224);let table_width=r.right-left-p(22);let name_right=left+table_width*36/100;
            let damage_right=left+table_width*49/100;let dps_right=left+table_width*62/100;let share_right=left+table_width*70/100;let hits_right=left+table_width*76/100;let crit_right=left+table_width*85/100;let hp_right=r.right-p(22);
            let column=|a,b,y,height|Rect{left:a,top:y,right:b,bottom:y+height};
            self.text(dc,&format!("Sources  {}/{}",s.rows.len(),s.source_count),column(left,name_right,p(130),p(28)),muted,false);
            for (label,a,b) in [("Damage",name_right,damage_right),("DPS",damage_right,dps_right),("Share",dps_right,share_right),("Hits",share_right,hits_right),("Crit",hits_right,crit_right),("Estimated",crit_right,hp_right)]{
                self.text(dc,label,column(a,b,p(130),p(28)),muted,true);
            }
            let height=self.row_height(r.bottom);let max=s.rows.first().map_or(1,|r|r.damage.max(1));
            for (i,row) in s.rows.iter().enumerate(){
                let y=p(164)+height*i as i32;if y+height>self.table_bottom(r.bottom){break;}
                let color=if row.is_self{accent}else{fg};
                self.fill(dc,column(left,left+((table_width as f64*row.damage as f64/max as f64) as i32),y,height-p(2)),if row.is_self{rgb(27,48,59)}else{rgb(27,34,44)});
                self.text(dc,&format!("{}  {}",i+1,row.label),column(left+p(4),name_right-p(5),y,height),color,false);
                self.text(dc,&number(row.damage),column(name_right,damage_right,y,height),color,true);
                self.text(dc,&row.dps.map_or_else(||"—".into(),|v|format!("{v:.1}")),column(damage_right,dps_right,y,height),color,true);
                self.text(dc,&format!("{:.1}%",row.share*100.0),column(dps_right,share_right,y,height),muted,true);
                self.text(dc,&row.display_hits.to_string(),column(share_right,hits_right,y,height),muted,true);
                self.text(dc,&if row.explicit_hits==0{"—".into()}else{format!("{:.1}%",row.critical_rate*100.0)},column(hits_right,crit_right,y,height),muted,true);
                self.text(dc,&if row.estimated_damage==0{"—".into()}else{number(row.estimated_damage)},column(crit_right,hp_right,y,height),muted,true);
            }
            let chart_top=self.chart_top(r.bottom);let chart_bottom=self.content_bottom(r.bottom);let chart_left=left;let chart_right=r.right-p(22);
            self.text(dc,&format!("{} estimated  ·  cumulative / last 5 s",number(s.estimated_damage)),column(left,chart_right,chart_top-p(26),p(24)),muted,false);
            self.fill(dc,Rect{left:chart_left,top:chart_top,right:chart_right,bottom:chart_bottom},rgb(24,31,40));
            let duration=s.chart.last().map_or(1.0,|p|p.0.max(0.001));let peak=s.chart.iter().flat_map(|p|[p.1,p.2]).fold(1.0,f64::max);
            for (index,color) in [(1,rgb(82,166,226)),(2,rgb(92,211,164))]{
                let points:Vec<_>=s.chart.iter().map(|v|Point{x:chart_left+(v.0/duration*(chart_right-chart_left) as f64) as i32,y:chart_bottom-((if index==1{v.1}else{v.2})/peak*(chart_bottom-chart_top) as f64) as i32}).collect();
                if points.len()>1{let pen=CreatePen(0,p(2).max(1),color);let previous=SelectObject(dc,pen);Polyline(dc,points.as_ptr(),points.len() as i32);SelectObject(dc,previous);DeleteObject(pen);}
            }
        }
        SelectObject(dc,old);
        if buffered {BitBlt(screen,0,0,r.right,r.bottom,dc,0,0,0x00cc0020);}
        EndPaint(h,&paint);
    }
}
fn number(value:u128)->String {let digits=value.to_string();let mut s=String::new();for (i,c) in digits.chars().enumerate(){if i>0 && (digits.len()-i)%3==0{s.push(',');}s.push(c);}s}
fn settings_path()->Option<std::path::PathBuf>{crate::live::data_dir().ok().map(|p|p.join("window.json"))}
unsafe fn save_window(h:Handle){let mut r=Rect::default();if GetWindowRect(h,&mut r)!=0{if let Some(path)=settings_path(){if let Some(parent)=path.parent(){let _=std::fs::create_dir_all(parent);}let _=std::fs::write(path,serde_json::json!({"x":r.left,"y":r.top,"width":r.right-r.left,"height":r.bottom-r.top}).to_string());}}}
unsafe fn initial_rect()->Rect{
    let scale=GetDpiForSystem().max(96) as i32;let mut r=Rect{left:100,top:100,right:100+1000*scale/96,bottom:100+760*scale/96};
    if let Some(v)=settings_path().and_then(|p|std::fs::read(p).ok()).filter(|b|b.len()<4096).and_then(|b|serde_json::from_slice::<serde_json::Value>(&b).ok()){
        if let (Some(x),Some(y),Some(width),Some(height))=(v["x"].as_i64(),v["y"].as_i64(),v["width"].as_i64(),v["height"].as_i64()){
            if (-32768..=32768).contains(&x) && (-32768..=32768).contains(&y) && (860..=8192).contains(&width) && (640..=8192).contains(&height){r=Rect{left:x as i32,top:y as i32,right:(x+width) as i32,bottom:(y+height) as i32};}
        }
    }
    let monitor=MonitorFromRect(&r,2);let mut info:MonitorInfo=std::mem::zeroed();info.size=std::mem::size_of::<MonitorInfo>() as u32;
    if GetMonitorInfoW(monitor,&mut info)!=0{let width=(r.right-r.left).min(info.work.right-info.work.left);let height=(r.bottom-r.top).min(info.work.bottom-info.work.top);r.left=r.left.clamp(info.work.left,info.work.right-width);r.top=r.top.clamp(info.work.top,info.work.bottom-height);r.right=r.left+width;r.bottom=r.top+height;}r
}
unsafe extern "system" fn procedure(h:Handle,message:u32,w:usize,l:isize)->isize{
    if message==0x83{return 0;}
    // DestroyWindow synchronously reenters this procedure while State is borrowed.
    // Shutdown has no State access and must bypass the mutable dispatch guard.
    if message==2{KillTimer(h,1);PostQuitMessage(0);return 0;}
    if message==0x14{return 1;}
    if matches!(message,0x133|0x134|0x135|0x138) {
        let brush=CONTROL_BRUSH.with(Cell::get);
        if !brush.is_null(){SetTextColor(w as Handle,rgb(225,232,242));SetBkColor(w as Handle,rgb(28,38,48));return brush as isize;}
    }
    // DefWindowProc can run a nested move/size loop. Never hold State or the
    // dispatch guard across that loop: WM_SIZE/WM_PAINT must keep working.
    if !matches!(message,5|0xf|0x113|0x24|0x84|0x2e0|0x111|0x202|0x20a|0x7b|0x10) {
        return DefWindowProcW(h,message,w,l);
    }
    let ptr=GetWindowLongPtrW(h,-21) as *mut State;if ptr.is_null(){return DefWindowProcW(h,message,w,l);}
    if DISPATCHING.with(|v|v.replace(true)){return DefWindowProcW(h,message,w,l);}let _guard=DispatchGuard;let s=&mut *ptr;
    match message {
        5=>{s.layout(h);InvalidateRect(h,null(),0);0},0x14=>1,0xf=>{s.paint(h);0},0x113=>{s.poll(h);0},
        0x24=>{let m=&mut *(l as *mut MinMax);m.min_track=Point{x:s.px(860),y:s.px(640)};0},
        0x84=>{let mut pt=Point{x:l as i16 as i32,y:(l>>16) as i16 as i32};ScreenToClient(h,&mut pt);let mut r=Rect::default();GetClientRect(h,&mut r);let edge=s.px(6);
            let left=pt.x<edge;let right=pt.x>=r.right-edge;let top=pt.y<edge;let bottom=pt.y>=r.bottom-edge;
            if top&&left{13}else if top&&right{14}else if bottom&&left{16}else if bottom&&right{17}else if left{10}else if right{11}else if top{12}else if bottom{15}else if pt.y<s.px(40)&&pt.x<r.right-s.px(170){2}else{1}},
        0x2e0=>{s.dpi=(w&65535) as u32;let r=&*(l as *const Rect);SetWindowPos(h,null_mut(),r.left,r.top,r.right-r.left,r.bottom-r.top,0x14);s.font(h);InvalidateRect(h,null(),0);0},
        0x111=>{match w&65535 {
            4 if w>>16==1=>{let index=SendMessageW(s.encounter_combo,0x147,0,0);if index>=0{if let Some(id)=s.encounters.get(index as usize){s.send(Action::Encounter(*id));}}},
            6 if w>>16==0x300=>{let mut b=[0u16;256];let n=GetWindowTextW(s.search,b.as_mut_ptr(),256);s.filter=String::from_utf16_lossy(&b[..n.max(0) as usize]).to_lowercase();s.target_scroll=0;s.send(Action::TargetSearch(s.filter.clone()));},
            7 if w>>16==0x300=>{let mut b=[0u16;16];let n=GetWindowTextW(s.idle,b.as_mut_ptr(),16);if let Ok(seconds)=String::from_utf16_lossy(&b[..n.max(0) as usize]).parse::<u64>(){if (1..=300).contains(&seconds){s.send(Action::IdleSeconds(seconds));}}},_=>()}
            InvalidateRect(h,null(),0);0},
        0x202=>{let x=l as i16 as i32;let y=(l>>16) as i16 as i32;let mut r=Rect::default();GetClientRect(h,&mut r);
            if y<s.px(40){if x>r.right-s.px(52){save_window(h);DestroyWindow(h);}else if x>r.right-s.px(94){ShowWindow(h,6);}else if x>r.right-s.px(170){s.topmost=!s.topmost;SetWindowPos(h,if s.topmost{-1isize as Handle}else{-2isize as Handle},0,0,0,0,0x13);}}
            else if {let button=s.toolbar(r.right).split;x>=button.left&&x<button.right&&y>=button.top&&y<button.bottom}{s.send(Action::Reset);}
            else if (s.px(20)..s.px(208)).contains(&x)&&y>=s.px(200)&&y<s.content_bottom(r.bottom)-s.px(4){let row=((y-s.px(200))/s.px(32)) as usize;let index=if row==0{0}else{row+s.target_scroll};if let Some((key,_))=s.visible_targets().get(index){s.send(Action::Target(*key));}}
            InvalidateRect(h,null(),0);0},
        0x20a=>{let mut pt=Point::default();GetCursorPos(&mut pt);ScreenToClient(h,&mut pt);if pt.x<s.px(208){let delta=(w>>16) as i16;let max=s.visible_targets().len().saturating_sub(2);s.target_scroll=if delta<0{(s.target_scroll+3).min(max)}else{s.target_scroll.saturating_sub(3)};InvalidateRect(h,null(),0);}0},
        0x7b=>{let mut pt=Point::default();GetCursorPos(&mut pt);let screen=pt;ScreenToClient(h,&mut pt);let mut r=Rect::default();GetClientRect(h,&mut r);let row=(pt.y-s.px(164))/s.row_height(r.bottom);
            let detail=if pt.x>=s.px(224)&&pt.y>=s.px(164)&&pt.y<s.table_bottom(r.bottom){s.snapshot.as_ref().and_then(|v|v.rows.get(row as usize)).cloned()}else{None};
            if let Some(detail)=detail{
                let menu=CreatePopupMenu();let key=detail.assignment_key;
                AppendMenuW(menu,if key.is_some(){0}else{1},1,wide("Mark as me").as_ptr());AppendMenuW(menu,0,2,wide("Clear my marker").as_ptr());
                AppendMenuW(menu,0x800,0,null());
                AppendMenuW(menu,1,0,wide(&format!("Pet contribution: {}",number(detail.pet_damage))).as_ptr());
                let skills=CreatePopupMenu();
                AppendMenuW(skills,1,0,wide("Skill   |   Explicit   |   Estimated").as_ptr());
                for skill in detail.skills.iter().take(64){
                    AppendMenuW(skills,1,0,wide(&format!("{}   |   {}   |   {}",skill.label.replace('&',"&&"),number(skill.explicit),number(skill.estimated))).as_ptr());
                }
                if detail.skills.len()>64{AppendMenuW(skills,1,0,wide(&format!("{} further skills in audit",detail.skills.len()-64)).as_ptr());}
                AppendMenuW(menu,0x10,skills as usize,wide("Skill breakdown").as_ptr());
                let command=TrackPopupMenu(menu,0x102,screen.x,screen.y,0,h,null());DestroyMenu(menu);
                if command==1{if let Some(key)=key{s.send(Action::AssignSelf(Some(key)));}}if command==2{s.send(Action::AssignSelf(None));}InvalidateRect(h,null(),0);
            }0},
        0x10=>{save_window(h);DestroyWindow(h);0},2=>{KillTimer(h,1);PostQuitMessage(0);0},_=>DefWindowProcW(h,message,w,l)
    }
}
pub fn run(source:Source)->Result<()> {run_inner(source,None)}
pub fn smoke(report:std::path::PathBuf)->Result<()> {run_inner(Source::Demo,Some(report))}
fn run_inner(source:Source,smoke_report:Option<std::path::PathBuf>)->Result<()> {
    let (tx,rx)=mpsc::sync_channel::<Command>(64);let (ack_tx,ack_rx)=mpsc::sync_channel(64);let latest=Arc::new(Mutex::new(None));let output=latest.clone();
    let smoke_mode=smoke_report.is_some();
    let core=thread::Builder::new().name("meter-core".into()).spawn(move||{
        if matches!(source,Source::Empty){if let Err(e)=crate::live::controller(rx,ack_tx,output.clone(),3600,true){crate::diagnostics::error("capture_start_or_finalize",&e);}return;}
        let mut replay=match load(source){Ok(r)=>r,Err(e)=>{crate::diagnostics::error("replay_load",&e);return;}};
        if smoke_mode{
            // A zero-amount synthetic credit exercises the actual Hits renderer
            // without changing the six explicit demo hits or its damage totals.
            let mut h=replay.meter.raw_hits.front().unwrap().clone();h.timestamp_ns=5_000_000_000;h.damage.amount=0;
            replay.meter.add_estimate(h.clone()).unwrap();let tag=replay.meter.last_hit_assignment(true).unwrap();
            replay.meter.set_display_hits(h.source,h.target,&tag,3);
        }
        let mut session=Session::new(replay,None);
        *output.lock().unwrap()=Some(session.replay.meter.snapshot());
        while let Ok(c)=rx.recv(){let ack=session.command(c);crate::diagnostics::command(&ack);if ack_tx.send(ack).is_err(){break;}}
    }).map_err(|e|e.to_string())?;
    unsafe {
        SetProcessDpiAwarenessContext(-4isize as Handle);let instance=GetModuleHandleW(null());let name=wide("MabiRustMeter");let icon=LoadIconW(instance,1usize as *const u16);
        let class=WindowClass{size:std::mem::size_of::<WindowClass>() as u32,style:3,procedure:Some(procedure),class_extra:0,window_extra:0,instance,icon,cursor:LoadCursorW(null_mut(),32512usize as *const u16),background:null_mut(),menu:null(),name:name.as_ptr(),small_icon:icon};
        if RegisterClassExW(&class)==0{return Err("Register native window failed".into());}let r=initial_rect();
        let h=CreateWindowExW(0,name.as_ptr(),wide("Mabi DPS").as_ptr(),0x820c0000,r.left,r.top,r.right-r.left,r.bottom-r.top,null_mut(),null_mut(),instance,null_mut());if h.is_null(){return Err("Create native window failed".into());}
        let child=|class:&str,text:&str,id:usize,style:u32|CreateWindowExW(0,wide(class).as_ptr(),wide(text).as_ptr(),0x50010000|style,0,0,50,30,h,id as Handle,instance,null_mut());
        if icon.is_null(){DestroyWindow(h);return Err("Embedded application icon failed to load".into());}
        let smoke=smoke_report.map(|report|Smoke{stage:0,started:std::time::Instant::now(),report,self_key:None});
        let mut state=Box::new(State{commands:tx,acks:ack_rx,latest,snapshot:None,command_id:0,encounters:Vec::new(),encounter_combo:child("COMBOBOX","",4,0x00200003),search:child("EDIT","",6,0x80),idle:child("EDIT","15",7,0x2000),filter:String::new(),target_scroll:0,topmost:false,dpi:GetDpiForWindow(h).max(96),font:null_mut(),edit_brush:CreateSolidBrush(rgb(28,38,48)),smoke,buffer:BackBuffer::default()});
        CONTROL_BRUSH.with(|brush|brush.set(state.edit_brush));
        SendMessageW(state.search,0x1501,1,wide("Search targets").as_ptr() as isize);state.font(h);SetWindowLongPtrW(h,-21,(&mut *state as *mut State) as isize);SetTimer(h,1,40,null());ShowWindow(h,5);
        let mut message:Message=std::mem::zeroed();loop{let result=GetMessageW(&mut message,null_mut(),0,0);if result==0{break;}if result<0{return Err("Native message loop failed".into());}TranslateMessage(&message);DispatchMessageW(&message);}
        CONTROL_BRUSH.with(|brush|brush.set(null_mut()));DeleteObject(state.font);DeleteObject(state.edit_brush);drop(state);
    }
    // Dropping the command sender requests own-worker stop; let recording finish.
    core.join().map_err(|_|"Meter core thread panicked".to_string())?;Ok(())
}
