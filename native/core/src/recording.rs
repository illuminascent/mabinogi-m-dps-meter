//! Lossless, append-only meter recordings. This layer never decodes game messages.
//! Independently compressed, checksummed blocks allow recovery of a crashed tail.
use crate::{transport::{Flow, Segment}, Result};
use serde_json::{json, Value};
use std::{fs::{File, OpenOptions}, io::{BufReader, Read, Write}, path::{Path,PathBuf}};

pub const MAGIC: &[u8; 8] = b"MABIREC\x01";
const ENCOUNTER_MAGIC: &[u8; 8] = b"MABIENC\x01";
const TARGET_BLOCK: usize = 256 * 1024;
pub const MAX_BLOCK: usize = 17 * 1024 * 1024;
const HEADER: usize = 32;

pub enum Record {
    Segment { ownership_epoch: u64, segment: Segment },
    Event(Value),
    Tick(u64),
}

#[derive(Default, Debug)]
pub struct Summary {
    pub blocks: u64,
    pub records: u64,
    pub segments: u64,
    pub payload_bytes: u64,
    pub ticks: u64,
    pub stored_bytes: u64,
    pub decoded_bytes: u64,
    pub incomplete_tail: bool,
}
impl Summary {
    pub fn json(&self) -> Value {
        json!({"blocks":self.blocks,"records":self.records,"segments":self.segments,
            "payload_bytes":self.payload_bytes,"ticks":self.ticks,"stored_bytes":self.stored_bytes,
            "decoded_bytes":self.decoded_bytes,"incomplete_tail":self.incomplete_tail})
    }
}

pub struct Writer<W: Write> { out: W, pending: Vec<u8>, blocks: u64, failed: bool }
impl Writer<File> {
    pub fn create(path: &Path) -> Result<Self> {
        Self::new(OpenOptions::new().write(true).create_new(true).open(path).map_err(|e|e.to_string())?)
    }
    pub fn sync(&mut self) -> Result<()> {
        self.flush()?;
        self.out.sync_data().map_err(|e|e.to_string())
    }
}
impl<W: Write> Writer<W> {
    pub fn new(mut out: W) -> Result<Self> {
        out.write_all(MAGIC).map_err(|e|e.to_string())?;
        Ok(Self {out,pending:Vec::with_capacity(TARGET_BLOCK),blocks:0,failed:false})
    }
    fn append(&mut self, tag: u8, bytes: &[u8]) -> Result<()> {
        if self.failed { return Err("recording writer failed; cannot append".into()); }
        if bytes.len()+5>MAX_BLOCK { return Err("recording record exceeds block limit".into()); }
        if self.pending.len()+bytes.len()+5>TARGET_BLOCK { self.flush()?; }
        self.pending.push(tag);
        self.pending.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        self.pending.extend_from_slice(bytes);
        if self.pending.len()>=TARGET_BLOCK { self.flush()?; }
        Ok(())
    }
    pub fn segment(&mut self, epoch: u64, s: &Segment) -> Result<()> {
        if s.flow.src.is_empty() || s.flow.dst.is_empty() || s.flow.src.len()>256 || s.flow.dst.len()>256
            || s.flow.sport==0 || s.flow.dport==0 || s.payload.len()+550>MAX_BLOCK {
            return Err("invalid recording segment".into());
        }
        let mut bytes=Vec::with_capacity(s.payload.len()+64);
        bytes.extend_from_slice(&epoch.to_le_bytes());
        bytes.extend_from_slice(&s.timestamp_ns.to_le_bytes());
        bytes.extend_from_slice(&s.seq.to_le_bytes());
        bytes.extend_from_slice(&s.flow.sport.to_le_bytes());
        bytes.extend_from_slice(&s.flow.dport.to_le_bytes());
        bytes.push(u8::from(s.syn)|u8::from(s.fin)<<1|u8::from(s.rst)<<2|u8::from(s.ack)<<3);
        for address in [&s.flow.src,&s.flow.dst] {
            bytes.extend_from_slice(&(address.len() as u16).to_le_bytes());
            bytes.extend_from_slice(address.as_bytes());
        }
        bytes.extend_from_slice(&s.payload);
        self.append(1,&bytes)
    }
    pub fn event(&mut self, value: &Value) -> Result<()> {
        if value["type"].as_str().is_none() { return Err("recording event needs type".into()); }
        self.append(2,&serde_json::to_vec(value).map_err(|e|e.to_string())?)
    }
    pub fn tick(&mut self, ns: u64) -> Result<()> { self.append(3,&ns.to_le_bytes()) }
    pub fn flush(&mut self) -> Result<()> {
        if self.failed { return Err("recording writer failed; cannot flush".into()); }
        let result=(||->Result<()> {
            if !self.pending.is_empty() {
                let mut compressed=Vec::new();
                {
                    let mut encoder=brotli::CompressorWriter::new(&mut compressed,4096,3,20);
                    encoder.write_all(&self.pending).map_err(|e|e.to_string())?;
                }
                let use_compressed=compressed.len()<self.pending.len();
                let stored=if use_compressed {&compressed}else{&self.pending};
                let mut header=Vec::with_capacity(HEADER);
                header.extend_from_slice(b"BLK1");
                header.extend_from_slice(&self.blocks.to_le_bytes());
                header.extend_from_slice(&(self.pending.len() as u32).to_le_bytes());
                header.extend_from_slice(&(stored.len() as u32).to_le_bytes());
                // Cover the sequence/length/codec header as well as stored bytes.
                header.extend_from_slice(&u32::from(use_compressed).to_le_bytes());
                let crc=checksum(&[],stored);
                header.extend_from_slice(&crc.to_le_bytes());
                header.extend_from_slice(&checksum(&header[4..],&[]).to_le_bytes());
                self.out.write_all(&header).and_then(|_|self.out.write_all(stored)).map_err(|e|e.to_string())?;
                self.blocks+=1;
                self.pending.clear();
                if self.pending.capacity()>TARGET_BLOCK*2 { self.pending=Vec::with_capacity(TARGET_BLOCK); }
            }
            self.out.flush().map_err(|e|e.to_string())
        })();
        if result.is_err() { self.failed=true; }
        result
    }
}

fn checksum(header: &[u8], body: &[u8]) -> u32 {
    use std::sync::OnceLock;
    static TABLE: OnceLock<[u32;256]>=OnceLock::new();
    let table=TABLE.get_or_init(|| std::array::from_fn(|i| {
        let mut c=i as u32;
        for _ in 0..8 { c=if c&1!=0 {0xedb88320^(c>>1)}else{c>>1}; }
        c
    }));
    let mut c=u32::MAX;
    for b in header.iter().chain(body) { c=table[((c^u32::from(*b))&255) as usize]^(c>>8); }
    !c
}

fn decode_segment(p: &[u8]) -> Result<Record> {
    if p.len()<29 || p[24]&!15!=0 { return Err("invalid recording segment header".into()); }
    let mut offset=25;
    let mut address=||->Result<String> {
        if p.len()<offset+2 { return Err("short recording address".into()); }
        let n=u16::from_le_bytes(p[offset..offset+2].try_into().unwrap()) as usize;offset+=2;
        if n==0 || n>256 || p.len()<offset+n { return Err("invalid recording address length".into()); }
        let value=std::str::from_utf8(&p[offset..offset+n]).map_err(|e|e.to_string())?.to_owned();offset+=n;
        Ok(value)
    };
    let src=address()?;let dst=address()?;
    let sport=u16::from_le_bytes(p[20..22].try_into().unwrap());
    let dport=u16::from_le_bytes(p[22..24].try_into().unwrap());
    if sport==0 || dport==0 { return Err("zero recording port".into()); }
    Ok(Record::Segment {ownership_epoch:u64::from_le_bytes(p[..8].try_into().unwrap()),
        segment:Segment {timestamp_ns:u64::from_le_bytes(p[8..16].try_into().unwrap()),
            seq:u32::from_le_bytes(p[16..20].try_into().unwrap()),flow:Flow {src,dst,sport,dport},
            syn:p[24]&1!=0,fin:p[24]&2!=0,rst:p[24]&4!=0,ack:p[24]&8!=0,payload:p[offset..].to_vec()}})
}

fn decode(tag: u8, p: &[u8]) -> Result<Record> {
    match tag {
        1=>decode_segment(p),
        2=>{
            let v:Value=serde_json::from_slice(p).map_err(|e|e.to_string())?;
            if v["type"].as_str().is_none() { return Err("recording event needs type".into()); }
            Ok(Record::Event(v))
        },
        3 if p.len()==8=>Ok(Record::Tick(u64::from_le_bytes(p.try_into().unwrap()))),
        _=>Err("unsupported recording record type/length".into()),
    }
}

/// An EOF inside the final block is recoverable, but no part of that block is
/// delivered. Checksums, bad lengths and unsupported versions always fail closed.
pub fn read(path: &Path, consume: impl FnMut(Record)->Result<()>) -> Result<Summary> {
    let (source,_)=resolve(path)?;
    read_from(BufReader::new(File::open(source).map_err(|e|e.to_string())?),consume)
}

/// Encounter entries share a sibling context.mrec. A fixed sibling name prevents
/// references from traversing directories, chaining, or selecting arbitrary files.
pub fn resolve(path: &Path) -> Result<(PathBuf,Option<Value>)> {
    let mut f=File::open(path).map_err(|e|e.to_string())?;
    let mut magic=[0;8];f.read_exact(&mut magic).map_err(|e|format!("recording header: {e}"))?;
    if &magic==MAGIC {return Ok((path.to_path_buf(),None));}
    if &magic!=ENCOUNTER_MAGIC {return Err("unsupported meter recording version".into());}
    let mut header=[0;8];f.read_exact(&mut header).map_err(|e|e.to_string())?;
    let n=u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
    if n>64*1024 {return Err("encounter reference exceeds limit".into());}
    let mut bytes=vec![0;n];f.read_exact(&mut bytes).map_err(|e|e.to_string())?;
    if f.read(&mut [0]) .map_err(|e|e.to_string())?!=0 || checksum(&[],&bytes)!=u32::from_le_bytes(header[4..].try_into().unwrap()) {
        return Err("encounter reference checksum/length mismatch".into());
    }
    let v:Value=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
    if v["format"]!="mabi-encounter-v1" || v["source"]!="context.mrec" || v["encounter"].as_u64().is_none_or(|n|n==0) {
        return Err("unsupported encounter reference".into());
    }
    let parent=path.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let source=parent.join("context.mrec");
    if source.canonicalize().map_err(|e|format!("missing shared context.mrec: {e}"))?.parent()!=Some(parent.canonicalize().map_err(|e|e.to_string())?.as_path()) {
        return Err("shared recording context must stay in its session directory".into());
    }
    let mut shared=File::open(&source).map_err(|e|e.to_string())?;
    shared.read_exact(&mut magic).map_err(|e|e.to_string())?;
    if &magic!=MAGIC {return Err("encounter context must be a raw recording, not another reference".into());}
    Ok((source,Some(v)))
}

pub fn write_reference(out: &mut impl Write, value: &Value) -> Result<()> {
    let bytes=serde_json::to_vec(value).map_err(|e|e.to_string())?;
    if bytes.len()>64*1024 {return Err("encounter reference exceeds limit".into());}
    out.write_all(ENCOUNTER_MAGIC)
        .and_then(|_|out.write_all(&(bytes.len() as u32).to_le_bytes()))
        .and_then(|_|out.write_all(&checksum(&[],&bytes).to_le_bytes()))
        .and_then(|_|out.write_all(&bytes)).map_err(|e|e.to_string())
}
pub fn read_from(mut input: impl Read, mut consume: impl FnMut(Record)->Result<()>) -> Result<Summary> {
    let mut magic=[0;8];input.read_exact(&mut magic).map_err(|e|format!("recording header: {e}"))?;
    if &magic!=MAGIC { return Err("unsupported meter recording version".into()); }
    let mut s=Summary {stored_bytes:8,..Summary::default()};
    loop {
        let mut h=[0;HEADER];
        match input.read(&mut h[..1]) {Ok(0)=>break,Ok(_)=>(),Err(e)=>return Err(e.to_string())}
        if let Err(e)=input.read_exact(&mut h[1..]) {
            if e.kind()==std::io::ErrorKind::UnexpectedEof {s.incomplete_tail=true;break;}
            return Err(e.to_string());
        }
        let sequence=u64::from_le_bytes(h[4..12].try_into().unwrap());
        let raw=u32::from_le_bytes(h[12..16].try_into().unwrap()) as usize;
        let stored=u32::from_le_bytes(h[16..20].try_into().unwrap()) as usize;
        let codec=u32::from_le_bytes(h[20..24].try_into().unwrap());
        if checksum(&h[4..28],&[])!=u32::from_le_bytes(h[28..32].try_into().unwrap()) {
            return Err(format!("recording block {} header checksum mismatch",s.blocks));
        }
        if &h[..4]!=b"BLK1" || sequence!=s.blocks || raw==0 || raw>MAX_BLOCK || stored==0
            || stored>MAX_BLOCK || codec>1 || (codec==0 && raw!=stored) {
            return Err(format!("invalid recording block {} header",s.blocks));
        }
        let mut bytes=vec![0;stored];
        if let Err(e)=input.read_exact(&mut bytes) {
            if e.kind()==std::io::ErrorKind::UnexpectedEof {s.incomplete_tail=true;break;}
            return Err(e.to_string());
        }
        if checksum(&[],&bytes)!=u32::from_le_bytes(h[24..28].try_into().unwrap()) {
            return Err(format!("recording block {} checksum mismatch",s.blocks));
        }
        let bytes=if codec==1 {crate::protocol::decompress(&bytes,raw)?}else{bytes};
        if bytes.len()!=raw { return Err("recording block decoded length mismatch".into()); }
        // Validate the entire block before allowing any of its records to mutate a consumer.
        let mut offset=0;
        while offset<bytes.len() {
            if bytes.len()-offset<5 { return Err("short recording record header".into()); }
            let n=u32::from_le_bytes(bytes[offset+1..offset+5].try_into().unwrap()) as usize;
            if n>bytes.len()-offset-5 { return Err("short recording record body".into()); }
            decode(bytes[offset],&bytes[offset+5..offset+5+n])?;
            offset+=5+n;
        }
        offset=0;
        while offset<bytes.len() {
            let n=u32::from_le_bytes(bytes[offset+1..offset+5].try_into().unwrap()) as usize;
            let record=decode(bytes[offset],&bytes[offset+5..offset+5+n])?;
            match &record {
                Record::Segment {segment,..}=>{s.segments+=1;s.payload_bytes+=segment.payload.len() as u64;},
                Record::Tick(_)=>s.ticks+=1,
                Record::Event(_)=>(),
            }
            consume(record)?;s.records+=1;offset+=5+n;
        }
        s.blocks+=1;s.stored_bytes+=(HEADER+stored) as u64;s.decoded_bytes+=raw as u64;
    }
    Ok(s)
}

pub fn segment_json(epoch: u64, s: &Segment) -> Value {
    let hex:String=s.payload.iter().map(|b|format!("{b:02x}")).collect();
    json!({"ownership_epoch":epoch,"timestamp_ns":s.timestamp_ns,"src":s.flow.src,"dst":s.flow.dst,
        "sport":s.flow.sport,"dport":s.flow.dport,"seq":s.seq,"syn":s.syn,"fin":s.fin,
        "rst":s.rst,"ack":s.ack,"payload_hex":hex})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn segment() -> Segment {
        Segment {timestamp_ns:91,flow:Flow {src:"2001:db8::1".into(),dst:"2001:db8::2".into(),sport:12,dport:34},
            seq:u32::MAX,payload:(0..=255).collect(),syn:true,fin:true,rst:true,ack:false}
    }
    #[test]
    fn roundtrip_preserves_unknown_bytes_flags_epochs_order_and_unicode() {
        let mut bytes=Vec::new();let mut w=Writer::new(&mut bytes).unwrap();let p=segment();
        w.event(&json!({"type":"session_start","notes":"日本語\nunknown fields"})).unwrap();
        w.segment(42,&p).unwrap();w.tick(123).unwrap();w.flush().unwrap();
        w.segment(43,&p).unwrap();w.flush().unwrap();drop(w);
        let mut order=Vec::new();
        let summary=read_from(bytes.as_slice(),|r|{match r {
            Record::Event(e)=>{assert_eq!(e["notes"],"日本語\nunknown fields");order.push(0);},
            Record::Segment {ownership_epoch,segment:s}=>{assert_eq!(segment_json(ownership_epoch,&s),segment_json(if order.len()==1 {42}else{43},&p));order.push(1);},
            Record::Tick(n)=>{assert_eq!(n,123);order.push(2);},
        }Ok(())}).unwrap();
        assert_eq!(order,[0,1,2,1]);assert_eq!(summary.blocks,2);assert!(!summary.incomplete_tail);
    }
    #[test]
    fn every_truncated_tail_recovers_only_complete_blocks_and_corruption_fails() {
        let mut bytes=Vec::new();let mut w=Writer::new(&mut bytes).unwrap();
        w.segment(1,&segment()).unwrap();w.flush().unwrap();drop(w);let boundary=bytes.len();
        // A second block without another file header.
        let mut second=Vec::new();let mut w=Writer::new(&mut second).unwrap();w.tick(42).unwrap();w.flush().unwrap();drop(w);
        second[12..20].copy_from_slice(&1u64.to_le_bytes());
        let crc=checksum(&second[12..36],&[]);second[36..40].copy_from_slice(&crc.to_le_bytes());
        bytes.extend_from_slice(&second[8..]);
        for cut in boundary+1..bytes.len() {
            let mut count=0;let s=read_from(&bytes[..cut],|_|{count+=1;Ok(())}).unwrap();
            assert_eq!(count,1);assert!(s.incomplete_tail);
        }
        assert_eq!(read_from(bytes.as_slice(),|_|Ok(())).unwrap().records,2);
        for index in [12,20,24,28,32,36,40,bytes.len()-1] {
            let mut bad=bytes.clone();bad[index]^=1;assert!(read_from(bad.as_slice(),|_|Ok(())).is_err());
        }
        let mut bad=bytes.clone();bad[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_from(bad.as_slice(),|_|Ok(())).is_err());
        let mut bad=bytes;bad[7]=99;assert!(read_from(bad.as_slice(),|_|Ok(())).is_err());
    }
    #[test]
    fn compression_reduces_repeated_wire_data_without_collapsing_observations() {
        let mut bytes=Vec::new();let mut w=Writer::new(&mut bytes).unwrap();
        for _ in 0..1000 {w.segment(9,&segment()).unwrap();}w.flush().unwrap();drop(w);
        let s=read_from(bytes.as_slice(),|_|Ok(())).unwrap();
        assert_eq!(s.segments,1000);assert!(s.stored_bytes<s.decoded_bytes/5);
    }
}
