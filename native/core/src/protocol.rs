use crate::Result;
use std::{collections::BTreeMap, io::Read, sync::OnceLock};

pub const MAX_WIRE: usize = 4 * 1024 * 1024;
pub const MAX_DECODED: usize = 32 * 1024 * 1024;
pub const PROFILE: &str = "mmh-jp.4-observed-v5";

pub fn normalize(wire: i32) -> i32 {
    static MAP: OnceLock<BTreeMap<i32, i32>> = OnceLock::new();
    *MAP.get_or_init(|| serde_json::from_str(include_str!(
        "../data/jp_protocol_map.json")).expect("embedded JP map"))
        .get(&wire).unwrap_or(&wire.saturating_add(200_000))
}

pub fn u32le(p: &[u8], n: usize) -> u32 {
    u32::from_le_bytes(p[n..n+4].try_into().unwrap())
}
pub fn u64le(p: &[u8], n: usize) -> u64 {
    u64::from_le_bytes(p[n..n+8].try_into().unwrap())
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub wire: Option<i32>,
    pub compression: u8,
    pub payload: Vec<u8>,
    pub decoded: bool,
    pub consumed: usize,
}

/// Stream output is bounded during reads, including the final limit probe.
pub fn decompress(input: &[u8], limit: usize) -> Result<Vec<u8>> {
    let mut reader = brotli::Decompressor::new(input, 4096);
    let mut output = Vec::new();
    let mut block = [0u8; 8192];
    loop {
        let allowance = block.len().min(limit.saturating_sub(output.len()).saturating_add(1));
        let n = reader.read(&mut block[..allowance]).map_err(|e| format!("Brotli: {e}"))?;
        if n == 0 { return Ok(output); }
        if output.len() + n > limit { return Err("Brotli expansion limit".into()); }
        output.extend_from_slice(&block[..n]);
    }
}

/// None is an incomplete frame; Err is an invalid header, at a known boundary.
pub fn read_frame(p: &[u8]) -> Result<Option<Frame>> {
    if p.len() < 8 { return Ok(None); }
    if &p[..4] == b"MMCG" {
        let n = u32le(p, 4) as usize;
        if n > 65536 { return Err("invalid MMCG length".into()); }
        return Ok((p.len() >= 8+n).then(|| Frame { wire: None, compression: 0,
            payload: p[8..8+n].to_vec(), decoded: true, consumed: 8+n }));
    }
    if p.len() < 9 { return Ok(None); }
    let wire = u32le(p, 0) as i32;
    let n = u32le(p, 4) as usize;
    if !(100..=500_000).contains(&normalize(wire)) || n > MAX_WIRE || p[8] > 1 {
        return Err("invalid frame header".into());
    }
    if p.len() < 9+n { return Ok(None); }
    let (payload, decoded) = if p[8] == 1 {
        match decompress(&p[9..9+n], MAX_DECODED) {
            Ok(v) => (v, true), Err(_) => (p[9..9+n].to_vec(), false),
        }
    } else { (p[9..9+n].to_vec(), true) };
    Ok(Some(Frame { wire: Some(wire), compression: p[8], payload, decoded, consumed: 9+n }))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Damage {
    pub actor: u32, pub target: u32, pub amount: u64,
    pub flags: [bool; 5], pub raw_flags: [u8; 7],
}
pub fn damage(p: &[u8]) -> Result<Damage> {
    if p.len() < 53 { return Err("short damage payload".into()); }
    let (actor, target, amount) = (u32le(p, 0), u32le(p, 8), u64le(p, 16));
    if actor == 0 || target == 0 || actor == target || amount > 1_000_000_000_000 {
        return Err("damage sanity check".into());
    }
    Ok(Damage { actor, target, amount, flags: [p[32]&1 != 0, p[33]&2 != 0,
        p[33]&4 != 0, p[35]&8 != 0, p[36]&16 != 0], raw_flags: p[32..39].try_into().unwrap() })
}

#[derive(Debug)]
pub struct Attack {
    pub actor: u32, pub target: u32, pub key1: u32, pub key2: u32, pub meta: [u8; 8],
}
pub fn attack(p: &[u8]) -> Result<Attack> {
    if p.len() < 35 { return Err("short attack payload".into()); }
    Ok(Attack { actor: u32le(p, 0), target: u32le(p, 8), key1: u32le(p, 16),
        key2: u32le(p, 20), meta: p[24..32].try_into().unwrap() })
}

pub fn encode_frame(wire: i32, p: &[u8]) -> Vec<u8> {
    let mut v = wire.to_le_bytes().to_vec();
    v.extend_from_slice(&(p.len() as u32).to_le_bytes()); v.push(0); v.extend_from_slice(p); v
}
