//! Reconstructed Score2 2.1.2 grammar and a separately gated observed JP profile.
//!
//! Static helper labels below are disassembly labels, never packet opcodes.
//! Native variants and the observed JP20054 layout are never interchangeable.
use super::Character;
use crate::Result;

#[derive(Clone,Debug)]
pub struct ReferenceAppearance {
    pub actor: u32,pub outer_reference: u64,pub identity: Character,pub display_name: String,
    pub uid_offset: usize,pub embedded_timestamp_ms: u64,pub variant: u8,
    pub consumed: usize,pub fully_consumed: bool,
}

struct Cursor<'a> { data: &'a [u8],pos: usize,jp:bool }
impl<'a> Cursor<'a> {
    fn take(&mut self,n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|end| *end <= self.data.len()).ok_or_else(|| format!("appearance bounds at {} need {n}",self.pos))?;
        let p = &self.data[self.pos..end];self.pos=end;Ok(p)
    }
    fn skip(&mut self,n: usize) -> Result<()> { self.take(n).map(|_| ()) }
    fn u8(&mut self) -> Result<u8> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> Result<u16> { Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap())) }
    fn u32(&mut self) -> Result<u32> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn i32(&mut self) -> Result<i32> { Ok(self.u32()? as i32) }
    fn u64(&mut self) -> Result<u64> { Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
    fn i64(&mut self) -> Result<i64> { Ok(self.u64()? as i64) }
    fn finite(&mut self) -> Result<f32> {
        let n=f32::from_bits(self.u32()?);if n.is_finite() { Ok(n) } else { Err("nonfinite appearance float".into()) }
    }
    fn string(&mut self) -> Result<String> {
        let n=self.count(4096)?;let p=self.take(n)?;
        let s=if n%2==0 { String::from_utf16_lossy(&p.chunks_exact(2).map(|s| u16::from_le_bytes([s[0],s[1]])).collect::<Vec<_>>()) }
            else { String::from_utf8_lossy(p).into_owned() };
        Ok(s.replace('\0',"").trim().to_owned())
    }
    fn count(&mut self,max: usize) -> Result<usize> {
        let n=self.u32()? as usize;if n>max { Err(format!("appearance count {n} exceeds {max}")) } else { Ok(n) }
    }
    fn array(&mut self,size: usize,max: usize) -> Result<()> {
        let n=self.count(max)?;self.skip(n.checked_mul(size).ok_or("appearance array overflow")?)
    }
    fn array4(&mut self) -> Result<()> { self.array(4,4096) }
    fn array8(&mut self) -> Result<()> { self.array(8,4096) }
    fn job(&mut self,tail: usize) -> Result<()> {
        if self.u32()?>200 || self.u16()?>2 { return Err("appearance job/type outside bounds".into()); }
        self.finite()?;self.skip(tail)
    }
    fn probe_four(&self) -> bool {
        let Some(p)=self.data.get(self.pos..self.pos.saturating_add(52)) else { return false; };
        u32::from_le_bytes(p[..4].try_into().unwrap())<=64 && p[4..].chunks_exact(8)
            .all(|s| u64::from_le_bytes(s.try_into().unwrap())<=100_000_000)
    }
    fn block(&mut self,label: u32,extra: bool,extra_byte: bool) -> Result<()> {
        let e=usize::from(extra);
        match label {
            0xBD2F0 => { self.skip(9)?;for _ in 0..2 { let n=self.i32()?;if n<0 { return Err("negative appearance blob".into()); }self.skip(n as usize)?; }self.skip(5)?; }
            0xBD440 => self.array4()?,
            0xBD4D0 => { self.string()?;self.skip(45)?; }
            0xBFE20 => { self.array4()?;self.array8()?; }
            0xBF9A0 => { for _ in 0..self.count(4096)? { self.block(0xBFE20,false,false)?; } }
            0xBFA30 | 0xC02C0 => {
                for _ in 0..self.count(256)? {
                    self.string()?;let v=self.i32()?;
                    if label==0xC02C0 && !(-4096..=4096).contains(&v) { return Err("appearance signed field outside bounds".into()); }
                    self.block(0xBFE20,false,false)?;
                }
            }
            0xBFB40 => { self.skip(21)?;self.finite()?;self.string()?;self.string()?;self.skip(7+e)?; }
            0xC03F0 | 0xC0690 => {
                if self.u32()?>100 { return Err("appearance trait integer outside bounds".into()); }
                if label==0xC0690 { self.string()?; }
                self.skip(17)?;let scale=self.finite()?;self.string()?;self.string()?;self.skip(7+e)?;
                if !(scale>0.0 && scale<10.0) { return Err("appearance trait scale outside bounds".into()); }
            }
            0xBD6A0 | 0xBD8B0 => {
                self.job(if label==0xBD6A0 { 16 } else { 20 })?;self.array4()?;
                for _ in 0..3 { self.block(0xBF9A0,false,false)?; }
                self.block(0xC02C0,false,false)?;
                self.block(if label==0xBD6A0 { 0xC0690 } else { 0xC03F0 },extra,false)?;
            }
            0xBDBA0 => { self.skip(13)?;self.string()?;self.skip(31)?; }
            0xBDDA0 => { self.skip(4)?;self.array4()?; }
            0xBDE50 => {
                for _ in 0..3 { self.finite()?; }self.skip(4)?;let scale=self.finite()?;
                self.skip(1+usize::from(extra_byte))?;self.finite()?;self.finite()?;
                self.skip(16)?;self.array8()?;self.skip(8)?;self.finite()?;self.skip(3)?;
                if extra { self.skip(8)?;self.finite()?;self.skip(8)?;self.finite()?;self.skip(7)?; }
                if scale<0.0 { return Err("negative appearance scale".into()); }
            }
            0xBE220 => self.skip(41+e)?,
            0xBE2D0 => self.skip(21)?,
            0xBE390 => {
                self.array4()?;for _ in 0..self.count(4096)? {
                    self.skip(16)?;if self.u32()?>1_000_000 { return Err("appearance record value outside bounds".into()); }self.skip(8)?;
                }
            }
            0xBE5B0 => { self.string()?;self.skip(4)?; }
            0xBE630 => { self.finite()?; }
            0xBE6A0 => self.skip(28)?,
            0xBE730 => { self.skip(25)?;self.array8()?;self.skip(1)?;self.string()?; }
            0xBE8B0 => { self.skip(2)?;self.string()?;self.skip(1)?; }
            0xBE940 => self.skip(19)?,
            0xBE9E0 => { for _ in 0..self.count(4096)? { self.string()?; }self.array4()?; }
            0xBEB30 => { for _ in 0..self.count(4096)? { self.string()?;self.string()?; } }
            0xBEE60 => { self.skip(1)?;self.array(8,128)?; }
            0xBEF40 => {
                self.skip(12)?;for _ in 0..3 { if self.u8()? !=0 { self.skip(8)?; } }
                self.string()?;self.skip(17)?;
            }
            0xBF120 => { self.skip(8)?;self.array4()?;self.skip(5)?; }
            0xBF200 => {
                let identity=self.i64()?;let name=self.string()?;self.skip(8)?;
                let level=self.i32()?;let another=self.i64()?;self.skip(5)?;
                if identity>0 && !name.is_empty() && (!(1..=100).contains(&level) || another<0 || !valid_name(&name,self.jp)) {
                    return Err("optional personal appearance record outside bounds".into());
                }
            }
            0xBF3F0 => {
                self.skip(6)?;self.finite()?;self.skip(20)?;self.array4()?;
                for _ in 0..3 { self.block(0xBF9A0,false,false)?; }
                self.block(0xBFA30,false,false)?;self.block(0xBFB40,extra,false)?;
            }
            0xBF5D0 => self.skip(17)?,
            0xBF670 => self.skip(49)?,
            0xBF760 => { self.skip(8)?;self.string()?; }
            0xBF7E0 => { self.skip(8)?;let n=self.count(1_048_576)?;self.skip(n)?;self.skip(6)?; }
            _ => return Err("unsupported reference appearance block".into()),
        }
        Ok(())
    }
}
fn valid_name(name: &str,jp:bool) -> bool {
    !name.trim().is_empty() && name.encode_utf16().count()<=32 && name.chars()
        .all(|c| c.is_alphabetic() || c.is_numeric() || matches!(c,' ' | '-' | '_') || (jp && c=='\u{30fb}'))
}
pub fn walk_reference(data: &[u8],variant: u8) -> Result<ReferenceAppearance> {
    walk(data,variant,false)
}
/// Strict JP20054 profile: bounded full grammar plus observed fingerprint.
/// Unsupported regional layouts return an error; no scanning fallback here.
pub fn parse_jp_observed(data:&[u8])->Result<ReferenceAppearance> { walk(data,0,true) }
fn walk(data: &[u8],variant: u8,jp:bool) -> Result<ReferenceAppearance> {
    if data.len()>crate::protocol::MAX_DECODED || variant>3 { return Err("reference appearance input limit/variant".into()); }
    let mut c=Cursor { data,pos:0,jp };let header=c.u64()?;c.skip(5)?;
    let extra=variant!=3;if extra { c.skip(4)?;let prefix=c.string()?;
        if jp && (c.pos!=21 || !prefix.is_empty()) { return Err("unsupported JP appearance prefix".into()); }
    }
    c.skip(if jp {402}else{458})?;for _ in 0..if jp {4}else{9} { c.finite()?; }
    let flag=c.u8()?;let probe=flag!=0 && c.probe_four();
    if jp && (flag!=1 || probe) { return Err("unsupported JP appearance common branch".into()); }
    if probe { c.skip(4)?; }
    c.skip(72)?;for _ in 0..3 { c.array8()?; }
    c.skip(if extra { 36 } else { 20 })?;let level=c.i32()?;c.skip(8)?;
    for label in [0xBD2F0,0xBD440,0xBD4D0,0xBD6A0,0xBD8B0,0xBDBA0] {
        if jp && label==0xBD8B0 && c.take(4)?!=[0;4] { return Err("unsupported JP first-job suffix".into()); }
        if jp && label==0xBDBA0 && c.take(8)?!=[0,0,0,0,0,0,0xc0,0x27] { return Err("unsupported JP second-job suffix".into()); }
        c.block(label,extra,false)?;
    }
    c.skip(5)?;c.block(0xBDDA0,false,false)?;c.block(0xBDE50,extra,variant==0)?;c.block(0xBE220,extra,false)?;
    c.skip(29+usize::from(variant==0))?;
    for label in [0xBE2D0,0xBE390,0xBE5B0,0xBE630] { c.block(label,extra,false)?; }
    c.skip(16)?;c.block(0xBE6A0,false,false)?;c.skip(1)?;c.block(0xBE730,false,false)?;c.skip(13)?;
    for label in [0xBE8B0,0xBE940,0xBE9E0,0xBEB30] { c.block(label,extra,false)?; }
    if extra { c.array4()?; }
    if jp && c.u8()?!=0 { return Err("unsupported JP pre-identity field".into()); }
    let uid_offset=c.pos;let uid=c.u64()?;let kind=c.u32()?;let display_name=c.string()?;
    let timestamp=c.u64()?;c.skip(4)?;
    if uid==0 || !(1..=128).contains(&kind) || !(1_500_000_000_000..=2_200_000_000_000).contains(&timestamp)
        || !valid_name(&display_name,jp) { return Err("invalid structured appearance identity".into()); }
    c.block(0xBEE60,false,false)?;c.skip(4)?;c.block(0xBEF40,false,false)?;c.skip(13)?;
    c.block(0xBF120,false,false)?;c.block(0xBF200,false,false)?;c.skip(if extra { 9 } else { 8 })?;
    c.block(0xBF3F0,extra,false)?;c.skip(5)?;c.block(0xBF3F0,extra,false)?;c.skip(22)?;
    c.block(0xBF5D0,false,false)?;c.block(0xBF670,false,false)?;c.skip(14+if variant<=1 { 4 } else { 0 })?;
    c.block(0xBF760,false,false)?;
    if jp {
        let opaque=c.take(24)?;
        if opaque[8..]!=[0,0,0x80,0x3f,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0xff,0,0,0,0] {
            return Err("unsupported JP final opaque region".into());
        }
    }
    c.block(0xBF7E0,false,false)?;c.block(0xBF7E0,false,false)?;
    if level<=0 || header as u32==0 { return Err("invalid appearance level/actor".into()); }
    if jp && c.pos!=data.len() { return Err("unconsumed JP appearance tail".into()); }
    Ok(ReferenceAppearance { actor:header as u32,outer_reference:header,identity:Character::score2_appearance_uid(uid)?,
        display_name,uid_offset,embedded_timestamp_ms:timestamp,variant,consumed:c.pos,fully_consumed:c.pos==data.len() })
}
/// Match the native choice: first full success, otherwise first partial success.
pub fn parse_reference(data: &[u8]) -> Result<ReferenceAppearance> {
    let mut partial=None;
    for variant in 0..4 {
        if let Ok(a)=walk_reference(data,variant) {
            if a.fully_consumed { return Ok(a); }if partial.is_none() { partial=Some(a); }
        }
    }
    partial.ok_or_else(|| "no supported reference appearance layout".into())
}

/// Reference semantic self-message body. No Japanese opcode is assigned here.
pub fn self_vote_reference(data: &[u8]) -> Result<u32> {
    let mut c=Cursor { data,pos:0,jp:false };let actor=c.u32()?;
    if actor==0 { Err("zero reference self vote".into()) } else { Ok(actor) }
}
