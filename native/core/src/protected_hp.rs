//! JP170206..170210 HP scalars. AES-128-CBC uses the actor's little-endian u64
//! twice as its key and a per-incarnation IV carried by common actor status. The
//! plaintext is one little-endian u64 followed by eight PKCS#7 padding bytes.
//! Keys survive a capture gap; temporal HP/action matching must not survive it.
use crate::{identity::EntityKey, Result};
use std::collections::BTreeMap;

const MAX_KEYS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    Unsupported,
    Malformed,
    MissingIv,
    ActorMismatch,
    InvalidPadding,
    OutOfRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnSeed {
    pub actor: u32,
    pub iv: [u8; 16],
    pub initial_hp: u64,
    /// Absolute offset in the original wire payload, including JP20517's prefix.
    pub iv_offset: usize,
}

/// Read the shared common-state prefix, independent of appearance suffixes.
pub fn spawn_seed(wire: i32, payload: &[u8]) -> std::result::Result<Option<SpawnSeed>, DecodeError> {
    if !matches!(wire,20051|20054|20517){return Ok(None);}
    let s=crate::hp_wire::status(wire,payload).map_err(|_|DecodeError::Malformed)?;
    if s.mode==0{return Ok(None);}
    if s.actor==0 || s.actor>u32::MAX as u64{return Err(DecodeError::ActorMismatch);}
    let iv=s.iv.try_into().map_err(|_|DecodeError::Unsupported)?;
    let cipher=s.hp.try_into().map_err(|_|DecodeError::Unsupported)?;
    Ok(Some(SpawnSeed{actor:s.actor as u32,iv,initial_hp:value(s.actor as u32,iv,cipher)?,iv_offset:s.iv_offset}))
}

/// An update must carry the entire observed byte-array grammar, including its
/// opaque trailer byte. Unsupported or malformed messages never create actors.
pub fn message_actor(wire: i32, p: &[u8]) -> Option<u32> {
    if !(170205..=170210).contains(&wire){return None;}
    let envelope=crate::hp_wire::envelope(p)?;
    if envelope.blob.len()!=16{return None;}
    let actor=envelope.actor;
    (actor > 0 && actor <= u32::MAX as u64).then_some(actor as u32)
}

struct Binding {
    iv: [u8; 16],
    round_keys: [u8; 176],
    conditional: bool,
}

#[derive(Default)]
pub struct Decoder {
    bindings: BTreeMap<(u64, EntityKey), Binding>,
}

impl Decoder {
    /// Returns true on a new binding or a changed IV. An exact IV repeat returns
    /// false and preserves the key; the caller can process its current HP as an
    /// ordinary observation. Changed IVs require clearing that target's old HP
    /// baseline before accepting initial_hp. Other actors are unaffected.
    pub fn register(&mut self, context: u64, target: EntityKey, seed: &SpawnSeed) -> Result<bool> {
        if seed.actor == 0 || target.actor != seed.actor { return Err("protected HP seed actor mismatch".into()); }
        if seed.initial_hp > i64::MAX as u64 { return Err("protected HP seed value out of range".into()); }
        let index = (context, target);
        if let Some(old)=self.bindings.get_mut(&index){if old.iv==seed.iv{old.conditional=false;return Ok(false);}}
        if !self.bindings.contains_key(&index) && self.bindings.len() >= MAX_KEYS {
            return Err("protected HP key capacity reached".into());
        }
        self.bindings.insert(index, Binding { iv: seed.iv, round_keys: expand_key(actor_key(seed.actor)), conditional:false });
        Ok(true)
    }

    pub fn decode(&self, context: u64, target: EntityKey, wire: i32, p: &[u8]) -> std::result::Result<u64, DecodeError> {
        if !(170205..=170210).contains(&wire) { return Err(DecodeError::Unsupported); }
        let actor = message_actor(wire, p).ok_or(DecodeError::Malformed)?;
        if actor != target.actor { return Err(DecodeError::ActorMismatch); }
        let binding = self.bindings.get(&(context, target)).ok_or(DecodeError::MissingIv)?;
        decode_with_keys(&binding.round_keys, binding.iv, p[12..28].try_into().unwrap())
    }
    /// A plain status declares IV bytes but does not verify their use with the
    /// protected scalar codec. Keep that provenance conditional, even on repeat.
    pub fn register_declared(&mut self,context:u64,target:EntityKey,seed:&SpawnSeed)->Result<bool>{
        let changed=self.register(context,target,seed)?;
        self.bindings.get_mut(&(context,target)).unwrap().conditional=true;Ok(changed)
    }

    pub fn forget(&mut self, target: EntityKey) { self.bindings.retain(|(_, k), _| *k != target); }
    pub fn forget_at(&mut self, context: u64, target: EntityKey) { self.bindings.remove(&(context,target)); }
    pub fn clear_context(&mut self, context: u64) { self.bindings.retain(|(c,_),_|*c!=context); }
    pub fn clear(&mut self) { self.bindings.clear(); }
    pub fn key_count(&self) -> usize { self.bindings.len() }
    pub fn mark_conditional(&mut self,context:u64){for ((c,_),binding) in &mut self.bindings{if *c==context{binding.conditional=true;}}}
    pub fn conditional(&self,context:u64,target:EntityKey)->bool{self.bindings.get(&(context,target)).is_some_and(|b|b.conditional)}
}

fn actor_key(actor: u32) -> [u8; 16] {
    let mut key = [0; 16];
    key[..8].copy_from_slice(&(actor as u64).to_le_bytes());
    key[8..].copy_from_slice(&(actor as u64).to_le_bytes());
    key
}

fn value(actor: u32, iv: [u8; 16], cipher: [u8; 16]) -> std::result::Result<u64, DecodeError> {
    decode_with_keys(&expand_key(actor_key(actor)), iv, cipher)
}

fn decode_with_keys(keys: &[u8; 176], iv: [u8; 16], cipher: [u8; 16]) -> std::result::Result<u64, DecodeError> {
    let plain = cbc_block(keys, iv, cipher);
    if plain[8..] != [8; 8] { return Err(DecodeError::InvalidPadding); }
    let hp = u64::from_le_bytes(plain[..8].try_into().unwrap());
    if hp > i64::MAX as u64 { return Err(DecodeError::OutOfRange); }
    Ok(hp)
}

// Fixed-size, portable AES-128 inverse primitive. No external crypto/runtime
// dependency, allocation, file access, or variable-sized ciphertext is involved.
const fn multiply(mut a: u8, mut b: u8) -> u8 {
    let mut result = 0;
    let mut i = 0;
    while i < 8 {
        if b & 1 != 0 { result ^= a; }
        let high = a & 0x80;
        a <<= 1;
        if high != 0 { a ^= 0x1b; }
        b >>= 1;
        i += 1;
    }
    result
}

const fn boxes() -> ([u8; 256], [u8; 256]) {
    let mut s = [0; 256];
    let mut inv = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut power = 254;
        let mut a = i as u8;
        let mut reciprocal = 1;
        while power != 0 {
            if power & 1 != 0 { reciprocal = multiply(reciprocal, a); }
            a = multiply(a, a);
            power >>= 1;
        }
        let x = reciprocal;
        let mapped = x ^ x.rotate_left(1) ^ x.rotate_left(2) ^ x.rotate_left(3) ^ x.rotate_left(4) ^ 0x63;
        s[i] = mapped;
        inv[mapped as usize] = i as u8;
        i += 1;
    }
    (s, inv)
}
const SBOX: [u8; 256] = boxes().0;
const INVERSE: [u8; 256] = boxes().1;

fn expand_key(key: [u8; 16]) -> [u8; 176] {
    let mut result = [0; 176];
    result[..16].copy_from_slice(&key);
    let mut rcon = 1;
    for offset in (16..176).step_by(4) {
        let mut word: [u8; 4] = result[offset - 4..offset].try_into().unwrap();
        if offset % 16 == 0 {
            word.rotate_left(1);
            for b in &mut word { *b = SBOX[*b as usize]; }
            word[0] ^= rcon;
            rcon = multiply(rcon, 2);
        }
        for i in 0..4 { result[offset + i] = result[offset + i - 16] ^ word[i]; }
    }
    result
}

fn decrypt_block(keys: &[u8; 176], cipher: [u8; 16]) -> [u8; 16] {
    let mut state = cipher;
    for i in 0..16 { state[i] ^= keys[160 + i]; }
    for round in (0..10).rev() {
        let before = state;
        for column in 0..4 {
            for row in 0..4 {
                state[4 * column + row] = INVERSE[before[4 * ((column + 4 - row) % 4) + row] as usize];
            }
        }
        for i in 0..16 { state[i] ^= keys[16 * round + i]; }
        if round != 0 {
            for column in state.chunks_exact_mut(4) {
                let a: [u8; 4] = column.try_into().unwrap();
                column[0] = multiply(a[0], 14) ^ multiply(a[1], 11) ^ multiply(a[2], 13) ^ multiply(a[3], 9);
                column[1] = multiply(a[0], 9) ^ multiply(a[1], 14) ^ multiply(a[2], 11) ^ multiply(a[3], 13);
                column[2] = multiply(a[0], 13) ^ multiply(a[1], 9) ^ multiply(a[2], 14) ^ multiply(a[3], 11);
                column[3] = multiply(a[0], 11) ^ multiply(a[1], 13) ^ multiply(a[2], 9) ^ multiply(a[3], 14);
            }
        }
    }
    state
}

fn cbc_block(keys: &[u8; 176], iv: [u8; 16], cipher: [u8; 16]) -> [u8; 16] {
    let mut plain = decrypt_block(keys, cipher);
    for i in 0..16 { plain[i] ^= iv[i]; }
    plain
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex<const N: usize>(s: &str) -> [u8; N] { crate::capture_file::unhex(s).unwrap().try_into().unwrap() }

    #[test]
    fn nist_aes128_inverse_and_cbc_vectors() {
        let key = expand_key(hex("000102030405060708090a0b0c0d0e0f"));
        assert_eq!(decrypt_block(&key, hex("69c4e0d86a7b0430d8cdb78070b4c55a")), hex("00112233445566778899aabbccddeeff"));
        let key = expand_key(hex("2b7e151628aed2a6abf7158809cf4f3c"));
        assert_eq!(cbc_block(&key, hex("000102030405060708090a0b0c0d0e0f"), hex("7649abac8119b246cee98e9b12e9197d")), hex("6bc1bee22e409f96e93d7e117393172a"));
    }

    fn target() -> EntityKey { EntityKey { process: 1, binding: 2, actor: 9, generation: 3 } }
    fn update(cipher: &str) -> Vec<u8> {
        let mut p = 9u64.to_le_bytes().to_vec();
        p.extend(16u32.to_le_bytes());p.extend(crate::capture_file::unhex(cipher).unwrap());p.push(0);p
    }
    fn spawn() -> Vec<u8> {
        let mut p = vec![0; 1000];
        p[..8].copy_from_slice(&9u64.to_le_bytes());
        p[572..580].copy_from_slice(&9u64.to_le_bytes());
        p[580] = 1;p[581..585].copy_from_slice(&16u32.to_le_bytes());
        p[585..601].copy_from_slice(&hex::<16>("c71938e787b6c816204a2c56c78a251d"));
        p[601..605].copy_from_slice(&16u32.to_le_bytes());
        p[605..621].copy_from_slice(&std::array::from_fn::<_,16,_>(|i|i as u8));p[625]=1;
        p[890..898].copy_from_slice(&u64::MAX.to_le_bytes());p[898..902].copy_from_slice(&3u32.to_le_bytes());p
    }

    #[test]
    fn strict_scalar_padding_and_range() {
        let iv = std::array::from_fn(|i|i as u8);
        assert_eq!(value(9, iv, hex("c71938e787b6c816204a2c56c78a251d")), Ok(111));
        assert_eq!(value(9, iv, hex("478642e2bcbd73ebb6feafa2dd8b6a48")), Ok(0));
        assert_eq!(value(9, iv, hex("e90ff40af6120c7051cdbe7e4d46dfb2")), Err(DecodeError::OutOfRange));
        assert_eq!(value(9, iv, hex("ce26dc2e00f7d307005dbdb02888da2d")), Err(DecodeError::InvalidPadding));
        let mut wrong = iv;wrong[15] ^= 1;
        assert_eq!(value(9, wrong, hex("c71938e787b6c816204a2c56c78a251d")), Err(DecodeError::InvalidPadding));
    }

    #[test]
    fn supported_spawn_envelope_and_bounded_fields() {
        let p = spawn();let seed = spawn_seed(20051, &p).unwrap().unwrap();
        assert_eq!((seed.actor, seed.initial_hp, seed.iv_offset), (9,111,605));
        let wrapped = [42u64.to_le_bytes().as_slice(),p.as_slice()].concat();
        assert_eq!(spawn_seed(20517,&wrapped).unwrap().unwrap().iv_offset,613);
        assert_eq!(spawn_seed(20054,&p),Ok(Some(seed.clone())));
        for length in 0..626 { assert!(!matches!(spawn_seed(20051,&p[..length]),Ok(Some(_)))); }
        assert_eq!(spawn_seed(20051,&p[..626]),Ok(Some(seed.clone())));
        for (offset,byte) in [(572,8),(581,8),(601,8)] {
            let mut bad=p.clone();bad[offset]=byte;
            assert!(spawn_seed(20051,&bad).is_err(),"offset {offset}");
        }
        let mut plain=p.clone();plain[580]=0;assert_eq!(spawn_seed(20051,&plain),Ok(None));
        let mut pet=p.clone();pet[898]=4;assert_eq!(spawn_seed(20051,&pet),Ok(Some(seed.clone())));
        for offset in [621,625,626,630]{let mut raw=p.clone();raw[offset]=255;assert_eq!(spawn_seed(20051,&raw),Ok(Some(seed.clone())));}
        let mut bad=p.clone();bad[585]^=1;assert_eq!(spawn_seed(20051,&bad),Err(DecodeError::InvalidPadding));
    }

    #[test]
    fn exact_actor_context_and_incarnation_scope() {
        let seed=spawn_seed(20051,&spawn()).unwrap().unwrap();let mut decoder=Decoder::default();
        let p=update("c71938e787b6c816204a2c56c78a251d");
        assert_eq!(decoder.decode(7,target(),170206,&p),Err(DecodeError::MissingIv));
        assert!(decoder.register(7,target(),&seed).unwrap());assert!(!decoder.register(7,target(),&seed).unwrap());
        for wire in 170206..=170210 { assert_eq!(decoder.decode(7,target(),wire,&p),Ok(111)); }
        assert_eq!(decoder.decode(8,target(),170206,&p),Err(DecodeError::MissingIv));
        for other in [EntityKey{process:2,..target()},EntityKey{binding:3,..target()},EntityKey{generation:4,..target()}] {
            assert_eq!(decoder.decode(7,other,170206,&p),Err(DecodeError::MissingIv));
        }
        assert_eq!(decoder.decode(7,EntityKey{actor:10,..target()},170206,&p),Err(DecodeError::ActorMismatch));
        assert_eq!(decoder.decode(7,target(),170205,&p),Ok(111));
        for length in 0..p.len() { assert_eq!(decoder.decode(7,target(),170206,&p[..length]),Err(DecodeError::Malformed)); }
        let mut raw=p.clone();raw[28]=255;assert_eq!(decoder.decode(7,target(),170206,&raw),Ok(111));
        let mut bad=p.clone();bad[8]=8;assert_eq!(decoder.decode(7,target(),170206,&bad),Err(DecodeError::Malformed));
        decoder.forget(target());assert_eq!(decoder.decode(7,target(),170206,&p),Err(DecodeError::MissingIv));
    }

    #[test]
    fn changed_iv_replaces_only_one_binding_and_clear_removes_all() {
        let mut seed=spawn_seed(20051,&spawn()).unwrap().unwrap();let mut decoder=Decoder::default();
        decoder.register(7,target(),&seed).unwrap();decoder.register(8,target(),&seed).unwrap();
        seed.iv=std::array::from_fn(|i|i as u8+16);seed.initial_hp=500;
        assert!(decoder.register(7,target(),&seed).unwrap());assert_eq!(decoder.key_count(),2);
        let newer=update("220f68c39be88115862c958caffe0ba6");let old=update("c71938e787b6c816204a2c56c78a251d");
        assert_eq!(decoder.decode(7,target(),170206,&newer),Ok(500));
        assert_eq!(decoder.decode(7,target(),170206,&old),Err(DecodeError::InvalidPadding));
        assert_eq!(decoder.decode(8,target(),170206,&old),Ok(111));
        decoder.clear_context(7);assert_eq!(decoder.key_count(),1);
        assert_eq!(decoder.decode(7,target(),170206,&newer),Err(DecodeError::MissingIv));
        assert_eq!(decoder.decode(8,target(),170206,&old),Ok(111));
        decoder.forget_at(7,target());assert_eq!(decoder.key_count(),1);
        decoder.clear();assert_eq!(decoder.key_count(),0);
        assert_eq!(decoder.decode(7,target(),170206,&newer),Err(DecodeError::MissingIv));
    }

    #[test]
    fn key_capacity_is_explicit_and_duplicate_refresh_is_allowed() {
        let mut decoder=Decoder::default();let mut seed=spawn_seed(20051,&spawn()).unwrap().unwrap();
        for actor in 1..=MAX_KEYS as u32 {
            seed.actor=actor;decoder.register(7,EntityKey{actor,..target()},&seed).unwrap();
        }
        seed.actor=MAX_KEYS as u32+1;
        assert!(decoder.register(7,EntityKey{actor:seed.actor,..target()},&seed).is_err());
        seed.actor=9;assert!(!decoder.register(7,target(),&seed).unwrap());
        assert_eq!(decoder.key_count(),MAX_KEYS);
    }
}
