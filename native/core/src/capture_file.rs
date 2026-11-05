//! Streaming readers. Filenames never imply a protocol boundary or game port.
use crate::{transport::{Flow,Segment}, Result};
use serde_json::Value;
use std::{fs::File, io::{BufRead, BufReader, Read}, net::{Ipv4Addr,Ipv6Addr}, path::Path};

pub enum Input { Segment(Segment), Loss { code: String, reset: bool } }
const MAX_LINE: usize = 17 * 1024 * 1024;
fn integer(row: &Value, key: &str, max: u64) -> Result<u64> {
    row.get(key).and_then(Value::as_u64).filter(|v| *v <= max).ok_or_else(|| format!("invalid {key}"))
}
fn text(row: &Value, key: &str) -> Result<String> {
    row.get(key).and_then(Value::as_str).filter(|v| !v.is_empty() && v.len() <= 256)
        .map(str::to_owned).ok_or_else(|| format!("invalid {key}"))
}
fn flag(row: &Value, key: &str, default: bool) -> Result<bool> {
    match row.get(key) { None => Ok(default), Some(v) => v.as_bool().ok_or_else(|| format!("invalid {key}")) }
}
pub fn unhex(v: &str) -> Result<Vec<u8>> {
    if v.len()%2 != 0 || v.len() > 16*1024*1024 { return Err("invalid hex length".into()); }
    fn nibble(b: u8) -> Result<u8> {
        match b { b'0'..=b'9' => Ok(b-b'0'), b'a'..=b'f' => Ok(b-b'a'+10),
            b'A'..=b'F' => Ok(b-b'A'+10), _ => Err("invalid hex byte".into()) }
    }
    v.as_bytes().chunks_exact(2).map(|p| Ok(nibble(p[0])?*16+nibble(p[1])?)).collect()
}
pub fn json_record(line: &[u8]) -> Result<Input> {
    let row: Value = serde_json::from_slice(line).map_err(|e| e.to_string())?;
    if row.get("type").and_then(Value::as_str) == Some("loss") {
        if row.get("detail").and_then(Value::as_str).is_none() { return Err("loss detail required".into()); }
        return Ok(Input::Loss { code: text(&row,"code")?, reset: flag(&row,"reset_streams",true)? });
    }
    let sport = integer(&row,"sport",65535)? as u16; let dport = integer(&row,"dport",65535)? as u16;
    if sport == 0 || dport == 0 { return Err("zero port".into()); }
    let payload = match row.get("payload_hex") { None => Vec::new(),
        Some(v) => unhex(v.as_str().ok_or("invalid payload_hex")?)? };
    Ok(Input::Segment(Segment { timestamp_ns: integer(&row,"timestamp_ns",u64::MAX)?,
        flow: Flow { src: text(&row,"src")?, dst: text(&row,"dst")?,sport,dport },
        seq: integer(&row,"seq",u32::MAX as u64)? as u32, payload,
        syn: flag(&row,"syn",false)?, fin: flag(&row,"fin",false)?, rst: flag(&row,"rst",false)?,
        ack: flag(&row,"ack",true)? }))
}

pub fn read(path: &Path, port: Option<u16>, mut consume: impl FnMut(Input) -> Result<()>) -> Result<()> {
    let mut reader = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
    match path.extension().and_then(|s| s.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("jsonl") => {
            let mut number = 0;
            loop {
                let mut line = Vec::new();
                // take bounds allocation even when a malicious line has no newline.
                (&mut reader).take((MAX_LINE+1) as u64).read_until(b'\n',&mut line).map_err(|e| e.to_string())?;
                if line.is_empty() { break; }
                number += 1;
                if line.len() > MAX_LINE { return Err(format!("JSONL line {number}: size limit")); }
                if line.iter().all(u8::is_ascii_whitespace) { continue; }
                consume(json_record(&line).map_err(|e| format!("JSONL line {number}: {e}"))?)?;
            }
            Ok(())
        }
        Some("pcap" | "cap") => pcap(&mut reader,port.filter(|p| *p != 0).ok_or("PCAP requires --port")?,consume),
        _ => Err("expected .jsonl or classic .pcap".into()),
    }
}
fn exact(r: &mut impl Read, p: &mut [u8]) -> Result<()> { r.read_exact(p).map_err(|e| e.to_string()) }
fn pcap(r: &mut impl Read, port: u16, mut consume: impl FnMut(Input)->Result<()>) -> Result<()> {
    let mut h = [0;24]; exact(r,&mut h)?;
    let (le, scale) = match h[..4] {
        [0xd4,0xc3,0xb2,0xa1] => (true,1000), [0xa1,0xb2,0xc3,0xd4] => (false,1000),
        [0x4d,0x3c,0xb2,0xa1] => (true,1), [0xa1,0xb2,0x3c,0x4d] => (false,1),
        _ => return Err("expected classic PCAP (not PCAPNG)".into()),
    };
    let word = |p: &[u8]| if le { u32::from_le_bytes(p.try_into().unwrap()) }
        else { u32::from_be_bytes(p.try_into().unwrap()) };
    if h[4..8] != if le { [2,0,4,0] } else { [0,2,0,4] } { return Err("PCAP version".into()); }
    let snaplen = word(&h[16..20]); let link = word(&h[20..24]) & 65535;
    if ![1,101,228,229].contains(&link) { return Err("PCAP link type".into()); }
    loop {
        let mut head = [0;16];
        match r.read(&mut head[..1]) { Ok(0) => break, Ok(_) => (), Err(e) => return Err(e.to_string()) }
        exact(r,&mut head[1..])?;
        let sec = word(&head[..4]) as u64; let fraction = word(&head[4..8]) as u64;
        let size = word(&head[8..12]); let original = word(&head[12..]);
        if size > snaplen.min(8*1024*1024) || size > original || fraction >= 1_000_000_000/scale {
            return Err("invalid PCAP length/timestamp".into());
        }
        let mut p = vec![0;size as usize]; exact(r,&mut p)?;
        if size < original { consume(Input::Loss { code: "capture_truncated".into(),reset: true })?; continue; }
        match tcp_packet(&p,link,sec*1_000_000_000+fraction*scale) {
            Ok(Some(s)) if s.flow.sport == port || s.flow.dport == port => consume(Input::Segment(s))?,
            Ok(_) => (), Err(_) => consume(Input::Loss { code: "packet_rejected".into(), reset: true })?,
        }
    }
    Ok(())
}

pub fn tcp_packet(mut p: &[u8], link: u32, timestamp_ns: u64) -> Result<Option<Segment>> {
    fn be(p: &[u8], n: usize) -> u16 { u16::from_be_bytes([p[n],p[n+1]]) }
    if link == 1 {
        if p.len() < 14 { return Err("short Ethernet".into()); }
        let mut kind = be(p,12); let mut offset = 14;
        while kind == 0x8100 || kind == 0x88a8 {
            if p.len() < offset+4 { return Err("short VLAN".into()); }
            kind = be(p,offset+2); offset += 4;
        }
        if kind != 0x0800 && kind != 0x86dd { return Ok(None); } p = &p[offset..];
    } else if ![101,228,229].contains(&link) { return Err("link type".into()); }
    if p.is_empty() { return Err("empty IP".into()); }
    let (src,dst,tcp) = match p[0] >> 4 {
        4 => {
            if p.len() < 20 { return Err("short IPv4".into()); }
            let header = (p[0]&15) as usize*4; let len = be(p,2) as usize;
            if header < 20 || len < header || len > p.len() || be(p,6)&0x3fff != 0 { return Err("IPv4 length/fragment".into()); }
            if p[9] != 6 { return Ok(None); }
            (Ipv4Addr::from(<[u8;4]>::try_from(&p[12..16]).unwrap()).to_string(),
             Ipv4Addr::from(<[u8;4]>::try_from(&p[16..20]).unwrap()).to_string(),&p[header..len])
        }
        6 => {
            if p.len() < 40 { return Err("short IPv6".into()); }
            let len = 40+be(p,4) as usize;
            if len > p.len() || [0,43,44,50,51,60].contains(&p[6]) { return Err("IPv6 length/extension".into()); }
            if p[6] != 6 { return Ok(None); }
            (Ipv6Addr::from(<[u8;16]>::try_from(&p[8..24]).unwrap()).to_string(),
             Ipv6Addr::from(<[u8;16]>::try_from(&p[24..40]).unwrap()).to_string(),&p[40..len])
        }
        _ => return Err("IP version".into()),
    };
    if tcp.len() < 20 { return Err("short TCP".into()); }
    let header = (tcp[12] >> 4) as usize*4;
    if header < 20 || header > tcp.len() { return Err("TCP header length".into()); }
    let flags = tcp[13];
    Ok(Some(Segment { timestamp_ns, flow: Flow { src,dst,sport: be(tcp,0),dport: be(tcp,2) },
        seq: u32::from_be_bytes(tcp[4..8].try_into().unwrap()), payload: tcp[header..].to_vec(),
        syn: flags&2 != 0,fin: flags&1 != 0,rst: flags&4 != 0,ack: flags&16 != 0 }))
}
