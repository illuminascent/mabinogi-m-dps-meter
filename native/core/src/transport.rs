use crate::{identity, protocol::{self, Frame}, Diagnostics, Result};

pub const MAX_BUFFER: usize = 8 * 1024 * 1024;
pub const MAX_PENDING: usize = 256;
pub const GLOBAL_BUFFER: usize = 128 * 1024 * 1024;
const GAP_NS: u64 = 3_000_000_000;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Flow { pub src: String, pub sport: u16, pub dst: String, pub dport: u16 }
impl Flow {
    pub fn connection(&self) -> [(String,u16); 2] {
        let mut v = [(self.src.clone(),self.sport),(self.dst.clone(),self.dport)]; v.sort(); v
    }
}
#[derive(Clone, Debug)]
pub struct Segment {
    pub timestamp_ns: u64, pub flow: Flow, pub seq: u32, pub payload: Vec<u8>,
    pub syn: bool, pub fin: bool, pub rst: bool, pub ack: bool,
}
pub fn distance(seq: u32, expected: u32) -> i64 { seq.wrapping_sub(expected) as i32 as i64 }

#[derive(Default)]
pub struct Framer { pub buffer: Vec<u8>, pub aligned: bool,pub consumed:u64, gap_pending:bool }
impl Framer {
    fn discontinue(&mut self,missing:u64) {
        self.consumed+=self.buffer.len() as u64+missing;
        self.buffer.clear();self.aligned=false;self.gap_pending=true;
    }
    pub fn feed(&mut self, data: &[u8], diag: &mut Diagnostics) -> Result<Vec<Frame>> {
        if self.buffer.len()+data.len() > MAX_BUFFER { return Err("framing capacity reached".into()); }
        self.buffer.extend_from_slice(data);
        let mut frames = Vec::new(); let mut offset = 0; let mut skipped = false;
        let mut decoded_bytes = 0;
        while self.buffer.len()-offset >= 8 {
            let p = &self.buffer[offset..];
            let control = &p[..4] == b"MMCG";
            if !control && p.len() < 9 { break; }
            let (header, len, wire, compression) = if control {
                (8,protocol::u32le(p,4) as usize,None,0)
            } else { (9,protocol::u32le(p,4) as usize,Some(protocol::u32le(p,0) as i32),p[8]) };
            let valid = if let Some(w) = wire {
                (100..=500000).contains(&protocol::normalize(w)) && len <= protocol::MAX_WIRE && compression <= 1
            } else { len <= 65536 };
            let mut candidate = None;
            let anchor = if self.aligned { true }
            else if control { valid }
            else if matches!(wire,Some(20132 | 20316)) { len == 0 && compression == 0 }
            else if matches!(wire,Some(20054 | 20517)) && valid && (1..=262144).contains(&len) {
                if p.len() < header+len { break; }
                let f = protocol::read_frame(p)?.unwrap();
                let anchored = f.decoded && f.payload.len() <= 262144
                    && (wire != Some(20517) || f.payload.len() >= 128)
                    && identity::parse(wire.unwrap(), &f.payload).is_some();
                if anchored { candidate = Some(f); } anchored
            } else { false };
            if !valid || !anchor {
                if self.aligned { diag.report("alignment_lost");self.gap_pending=true; }
                self.aligned = false; offset += 1; skipped = true; continue;
            }
            if p.len() < header+len { break; }
            if candidate.is_some() { diag.report("identity_alignment"); }
            let mut f = match candidate { Some(v) => v, None => protocol::read_frame(p)?.unwrap() };
            f.stream_byte=self.consumed+offset as u64;
            decoded_bytes += f.payload.len();
            if decoded_bytes > protocol::MAX_DECODED { return Err("decoded delivery capacity reached".into()); }
            if !f.decoded { diag.report("frame_rejected");self.gap_pending=true; }
            f.gap_before=std::mem::take(&mut self.gap_pending);
            self.aligned = true; offset += f.consumed; frames.push(f);
        }
        self.buffer.drain(..offset);self.consumed+=offset as u64;
        if skipped { diag.report("unaligned_bytes"); }
        Ok(frames)
    }
}

pub struct TcpStream {
    pub framer: Framer,
    expected: Option<u32>, syn_seq: Option<u32>, fin_seq: Option<u32>,
    pending: Vec<(u32,Vec<u8>,u64)>, gap_since: Option<u64>, pub closed: bool,
    buffer_time:u64,
}
impl TcpStream {
    pub fn new(aligned: bool) -> Self {
        Self { framer: Framer { aligned,..Framer::default() }, expected: None, syn_seq: None,
            fin_seq: None, pending: Vec::new(), gap_since: None, closed: false,buffer_time:0 }
    }
    pub fn buffered(&self) -> usize {
        self.framer.buffer.len() + self.pending.iter().map(|(_,p,_)| p.len()).sum::<usize>()
    }
    pub fn reset(&mut self, aligned: bool) {
        let prior=self.expected.is_some();
        self.expected = None; self.fin_seq = None; self.pending.clear(); self.gap_since = None;
        self.framer.discontinue(0); self.framer.aligned = aligned;self.buffer_time=0;
        self.framer.gap_pending=prior;
    }
    pub fn finish(&self, diag: &mut Diagnostics) {
        if !self.pending.is_empty() || self.fin_seq.is_some() { diag.report("tcp_gap_at_eof"); }
        if !self.framer.buffer.is_empty() { diag.report("frame_tail_at_eof"); }
    }
    pub fn starts_after_fin(&self,seq:u32)->bool{self.closed && self.expected.is_some_and(|next|distance(seq,next)>=0)}
    pub fn feed(&mut self, s: &Segment, diag: &mut Diagnostics) -> Result<Vec<Frame>> {
        if s.rst {
            if self.buffered() > 0 || self.fin_seq.is_some() { diag.report("reset_with_buffer"); }
            self.reset(false); self.closed = true; return Ok(Vec::new());
        }
        if self.closed { if !s.syn { return Ok(Vec::new()); } self.closed = false; }
        if s.syn && self.syn_seq != Some(s.seq) {
            if self.expected.is_some() { diag.report("tcp_restart"); }
            self.reset(true); self.syn_seq = Some(s.seq); self.expected = Some(s.seq.wrapping_add(1));
        }
        let original_seq = s.seq.wrapping_add(u32::from(s.syn));
        if self.expected.is_none() { self.expected = Some(original_seq); }
        let expected = self.expected.unwrap(); let mut seq = original_seq;
        let delta = distance(seq,expected); let mut p = s.payload.as_slice();
        if delta < 0 {
            p = &p[(-delta as usize).min(p.len())..]; seq = expected; diag.report("retransmission");
        }
        if !p.is_empty() {
            // Subtract all earlier pending intervals before accepting new bytes.
            let mut pieces = vec![(seq,p.to_vec())];
            for (old_seq,old,_) in &self.pending {
                let start = distance(*old_seq,expected); let end = start+old.len() as i64;
                let mut remaining = Vec::new();
                for (seq,p) in pieces {
                    let a = distance(seq,expected); let b = a+p.len() as i64;
                    if b <= start || a >= end { remaining.push((seq,p)); }
                    else {
                        if a < start { remaining.push((seq,p[..(start-a) as usize].to_vec())); }
                        if b > end { remaining.push((expected.wrapping_add(end as u32),p[(end-a) as usize..].to_vec())); }
                    }
                }
                pieces = remaining;
            }
            self.pending.extend(pieces.into_iter().map(|(seq,p)|(seq,p,s.timestamp_ns)));
        }
        // FIN uses original sequence space, even when retransmitted bytes were trimmed.
        if s.fin { self.fin_seq = Some(original_seq.wrapping_add(s.payload.len() as u32)); }
        if self.pending.len() > MAX_PENDING || self.buffered() > MAX_BUFFER {
            diag.report("tcp_pending_overflow"); self.reset(false); return Ok(Vec::new());
        }
        self.pending.sort_by_key(|(seq,_,_)| distance(*seq,expected));
        let mut frames = Vec::new();
        let mut delivery_bytes = 0;
        while !self.pending.is_empty() {
            let missing=distance(self.pending[0].0,self.expected.unwrap());
            if missing>0 {
                // Skip only the missing range, never the retained suffix. Each
                // further hole gets its own bounded wait. Framing must reacquire
                // a validated anchor, and replay sees the boundary before data.
                let since=self.gap_since.get_or_insert(self.pending[0].2);
                if s.timestamp_ns.saturating_sub(*since)<=GAP_NS{break;}
                diag.report("tcp_gap_timeout");diag.report("tcp_gap_recovered");
                self.framer.discontinue(missing as u64);self.buffer_time=0;
                self.expected=Some(self.pending[0].0);self.gap_since=None;
            }
            let (seq,p,time) = self.pending.remove(0);
            let trim = (-distance(seq,self.expected.unwrap())) as usize;
            if trim < p.len() {
                self.expected = Some(self.expected.unwrap().wrapping_add((p.len()-trim) as u32));
                let old_end=self.framer.consumed+self.framer.buffer.len() as u64;
                let mut decoded = self.framer.feed(&p[trim..],diag)?;
                for f in &mut decoded {f.timestamp_ns=Some(if f.stream_byte<old_end{self.buffer_time.max(time)}else{time});}
                self.buffer_time=if self.framer.consumed<old_end{self.buffer_time.max(time)}else{time};
                delivery_bytes += decoded.iter().map(|f| f.payload.len()).sum::<usize>();
                if delivery_bytes > protocol::MAX_DECODED { return Err("decoded delivery capacity reached".into()); }
                frames.extend(decoded);
            }
            self.gap_since=None;
        }
        let waiting_fin = self.fin_seq.is_some_and(|seq| distance(seq,self.expected.unwrap()) > 0);
        if !self.pending.is_empty() || waiting_fin { self.gap_since.get_or_insert(s.timestamp_ns); }
        else { self.gap_since = None; }
        if self.fin_seq == self.expected && self.pending.is_empty() {
            self.expected = self.expected.map(|v| v.wrapping_add(1)); self.fin_seq = None; self.closed = true;
        }
        Ok(frames)
    }
}
