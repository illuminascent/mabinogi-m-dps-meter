"""Bounded JP20054 appearance grammar, independently reconstructed as data.

Curated from our static research readers, not bundled helper code. Validated
against 62 derivation records, 17 held-out records and 224 later appearances.
No sample execution, capture or ignored-artifact imports. The post-UID u32 is
MMH's displayed server/realm; UID low16 is its character index.
"""
import math
import struct
import unicodedata

class Rejected(ValueError):
    pass

def rust_strip(s):
    # Rust char::is_whitespace (Unicode White_Space), Python also strips 1C..1F.
    return s.strip(' \t\n\r\v\f\u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000')

def valid_name(s):
    return bool(rust_strip(s)) and len(s.encode('utf-16-le')) // 2 <= 32 and all(
        c in ' -_' or c.isalpha() or unicodedata.category(c)[0] == 'N' for c in s)

class Cursor:
    def __init__(self, data):
        self.data = data
        self.p = 0
        self.stage = 'header'
        self.meta = {}

    def require(self, condition, reason):
        if not condition:
            raise Rejected(f'{self.stage}@{self.p}: {reason}')

    def take(self, n):
        self.require(0 <= n <= len(self.data) - self.p, f'need {n}, remaining {len(self.data)-self.p}')
        p = self.p
        self.p += n
        return self.data[p:self.p]

    def skip(self, n):
        self.take(n)

    def read(self, fmt):
        return struct.unpack('<' + fmt, self.take(struct.calcsize('<' + fmt)))[0]

    def u8(self): return self.read('B')
    def u16(self): return self.read('H')
    def u32(self): return self.read('I')
    def u64(self): return self.read('Q')
    def f32(self):
        v = self.read('f')
        self.require(math.isfinite(v), 'nonfinite f32')
        return v

    def string(self):
        n = self.u32()
        self.require(n <= 4096, 'string byte length >4096')
        b = self.take(n)
        s = b.decode('utf-16-le' if n % 2 == 0 else 'utf-8', errors='replace')
        return rust_strip(s.replace('\0', ''))

    def count(self, limit=4096):
        n = self.u32()
        self.require(n <= limit, f'count >{limit}')
        return n

    def array(self, size, limit=4096):
        self.skip(size * self.count(limit))

    def call(self, label, *args):
        old = self.stage
        self.stage = label
        p = self.p
        getattr(self, 'h' + label)(*args)
        self.stage = old

    def hBD2F0(self):
        self.skip(9)
        for _ in range(2):
            n = self.read('i')
            self.require(n >= 0, 'negative blob length')
            self.skip(n)
        self.skip(5)

    def hBD440(self): self.array(4)
    def hBD4D0(self): self.string(); self.skip(45)
    def hBFE20(self): self.array(4); self.array(8)
    def hBF9A0(self):
        for _ in range(self.count()): self.call('BFE20')
    def hBFA30(self):
        for _ in range(self.count(256)):
            self.string(); self.skip(4); self.call('BFE20')
    def hC02C0(self):
        for _ in range(self.count(256)):
            self.string()
            value = self.read('i')
            self.require(-4096 <= value <= 4096, 'signed value outside +-4096')
            self.call('BFE20')
    def hBFB40(self, extra):
        self.skip(21); self.f32(); self.string(); self.string(); self.skip(7 + extra)
    def hC03F0(self, extra):
        self.require(self.u32() <= 100, 'trait integer >100')
        self.skip(17)
        scale = self.f32()
        self.string(); self.string(); self.skip(7 + extra)
        self.require(0 < scale < 10.0, 'trait scale not in (0,10)')
    def hC0690(self, extra):
        self.require(self.u32() <= 100, 'trait integer >100')
        self.string(); self.skip(17)
        scale = self.f32()
        self.string(); self.string(); self.skip(7 + extra)
        self.require(0 < scale < 10.0, 'trait scale not in (0,10)')
    def job(self, n):
        self.require(self.u32() <= 200, 'job >200')
        self.require(self.u16() <= 2, 'type >2')
        self.f32(); self.skip(n)
    def hBD6A0(self, extra):
        self.job(16); self.array(4)
        for _ in range(3): self.call('BF9A0')
        self.call('C02C0'); self.call('C0690', extra)
    def hBD8B0(self, extra):
        self.job(20); self.array(4)
        for _ in range(3): self.call('BF9A0')
        self.call('C02C0'); self.call('C03F0', extra)
    def hBDBA0(self): self.skip(13); self.string(); self.skip(31)
    def hBDDA0(self): self.skip(4); self.array(4)
    def hBDE50(self, extra_tail, extra_byte):
        for _ in range(3): self.f32()
        self.skip(4)
        scale = self.f32()
        self.skip(1 + extra_byte)
        self.f32(); self.f32(); self.skip(16); self.array(8); self.skip(8)
        self.f32(); self.skip(3)
        if extra_tail:
            self.skip(8); self.f32(); self.skip(8); self.f32(); self.skip(7)
        self.require(scale >= 0, 'negative scale')
    def hBE220(self, extra): self.skip(41 + extra)
    def hBE2D0(self): self.skip(21)
    def hBE390(self):
        self.array(4)
        for _ in range(self.count()):
            self.skip(16)
            self.require(self.u32() <= 1_000_000, 'record value >1000000')
            self.skip(8)
    def hBE5B0(self): self.string(); self.skip(4)
    def hBE630(self): self.f32()
    def hBE6A0(self): self.skip(28)
    def hBE730(self): self.skip(25); self.array(8); self.skip(1); self.string()
    def hBE8B0(self): self.skip(2); self.string(); self.skip(1)
    def hBE940(self): self.skip(19)
    def hBE9E0(self):
        for _ in range(self.count()): self.string()
        self.array(4)
    def hBEB30(self):
        for _ in range(self.count()): self.string(); self.string()
    def hBEC40(self):
        offset = self.p
        uid = self.u64()
        self.require(uid != 0, 'zero UID')
        kind = self.u32()
        self.require(1 <= kind <= 128, 'kind not1..128')
        name = self.string()
        timestamp = self.u64()
        self.require(1_500_000_000_000 <= timestamp <= 2_200_000_000_000, 'timestamp outside bounds')
        tail = self.u32()
        self.require(valid_name(name), 'invalid display name')
        self.meta.update(uid=uid, uid_offset=offset, display_name=name,
                         server=uid & 65535, character_code_bytes=list(uid.to_bytes(8,'little')[2:6]),
                         character_code=uid.to_bytes(8,'little')[2:6].hex().upper(),
                         identity_kind=kind, timestamp=timestamp, identity_tail=tail)
    def hBEE60(self): self.skip(1); self.array(8,128)
    def hBEF40(self):
        self.skip(12)
        for _ in range(3):
            if self.u8(): self.skip(8)
        self.string(); self.skip(17)
    def hBF120(self): self.skip(8); self.array(4); self.skip(5)
    def hBF200(self):
        identity = self.read('q'); name = self.string(); self.skip(8)
        level = self.read('i'); another = self.read('q'); self.skip(5)
        if identity > 0 and rust_strip(name):
            self.require(1 <= level <= 100 and another >= 0 and valid_name(name), 'optional personal record invalid')
    def hBF3F0(self, extra):
        self.skip(6); self.f32(); self.skip(20); self.array(4)
        for _ in range(3): self.call('BF9A0')
        self.call('BFA30'); self.call('BFB40',extra)
    def hBF5D0(self): self.skip(17)
    def hBF670(self): self.skip(49)
    def hBF760(self): self.skip(8); self.string()
    def hBF7E0(self):
        self.skip(8); n=self.u32(); self.require(n<=1048576,'blob >1048576'); self.skip(n+6)

def valid_jp_name(name):
    # Three retained player appearances and one embedded personal record contain
    # U+30FB, a punctuation character rejected by native Score2's predicate.
    return valid_name(name.replace('\u30fb','A'))

def observed_profile_gates(result):
    """Narrow evidence fingerprint, distinct from uninterpreted field meaning.

    Unseen variants should remain rejected until supported by new evidence;
    the firstu64 of the final24byte region varies acrosscaptures and is opaque.
    """
    if not result.get('ok') or not result.get('full'):return False
    if (result['common_start'],result['prefix_string'],result['flag'],
        result['optional_four_probe'])!=(21,'',1,False):return False
    if not 1<=result['identity_kind']<=128:return False
    fields={f['label']:bytes.fromhex(f['bytes']) for f in result['opaque_fields']}
    if fields.get('after-first-job')!=bytes(4):return False
    if fields.get('after-second-job')!=bytes.fromhex('000000000000c027'):return False
    if fields.get('before-identity')!=bytes(1):return False
    tail=fields.get('before-first-final-blob',b'')
    return len(tail)==24 and tail[8:]==bytes.fromhex('0000803fffffffffffffffff00000000')

class JP(Cursor):
    def opaque(self,label,n):
        start=self.p
        raw=self.take(n)
        self.meta.setdefault('opaque_fields',[]).append({'label':label,'offset':start,'bytes':raw.hex()})

    def hBEC40(self):
        offset=self.p
        uid=self.u64();kind=self.u32();name=self.string();timestamp=self.u64();tail=self.u32()
        self.require(uid!=0,'zero UID')
        self.require(1<=kind<=128,'kind not1..128')
        self.require(1_500_000_000_000<=timestamp<=2_200_000_000_000,'timestamp outside bounds')
        self.require(valid_jp_name(name),'invalid JP display name')
        self.meta.update(uid=uid,uid_offset=offset,display_name=name,server=uid&65535,
                         character_code=uid.to_bytes(8,'little')[2:6].hex().upper(),
                         identity_kind=kind,timestamp=timestamp,identity_tail=tail)

    def hBF200(self):
        offset=self.p
        identity=self.read('q');name=self.string();unknown=self.read('q')
        level=self.read('i');another=self.read('q');self.skip(5)
        self.meta['optional_personal']={'offset':offset,'identity':identity,'name':name,
                                       'unknown':unknown,'level':level,'another':another}
        if identity>0 and rust_strip(name):
            self.require(1<=level<=100 and another>=0 and valid_jp_name(name),'invalid optional personal record')

def walk_jp(data,full=True):
    c=JP(data)
    try:
        c.stage='JP-header'
        header=c.u64();c.skip(9);prefix=c.string();h=c.p
        c.stage='JP-common-fixed'
        c.skip(402)
        stats=[c.f32() for _ in range(4)]
        flag_offset=c.p;flag=c.u8();probe=False
        if flag and len(data)-c.p>=52:
            values=struct.unpack_from('<I6Q',data,c.p)
            probe=values[0]<=64 and all(v<=100_000_000 for v in values[1:])
            if probe:c.skip(4)
        stat_values=[c.u64() for _ in range(6)]
        c.skip(24)
        c.stage='JP-three-A8'
        for _ in range(3):c.array(8)
        c.skip(36);level=c.read('i');c.skip(8)
        for label in ('BD2F0','BD440','BD4D0'):c.call(label)
        c.call('BD6A0',1)
        c.opaque('after-first-job',4)
        c.call('BD8B0',1)
        c.opaque('after-second-job',8)
        c.call('BDBA0');c.skip(5);c.call('BDDA0');c.call('BDE50',1,1);c.call('BE220',1)
        c.skip(30)
        for label in ('BE2D0','BE390','BE5B0','BE630'):c.call(label)
        c.skip(16);c.call('BE6A0');c.skip(1);c.call('BE730');c.skip(13)
        for label in ('BE8B0','BE940','BE9E0','BEB30','BD440'):c.call(label)
        c.opaque('before-identity',1)
        c.call('BEC40');c.call('BEE60');c.skip(4);c.call('BEF40');c.skip(13)
        c.call('BF120');c.call('BF200');c.skip(9)
        c.call('BF3F0',1);c.skip(5);c.call('BF3F0',1);c.skip(22)
        c.call('BF5D0');c.call('BF670');c.skip(18);c.call('BF760')
        c.opaque('before-first-final-blob',24)
        c.call('BF7E0');c.call('BF7E0')
        c.stage='JP-final-validation'
        c.require(level>0,'nonpositive level');c.require(header&0xffffffff!=0,'zero actor')
        if full:c.require(c.p==len(data),'unconsumed trailing bytes')
        c.meta.update(actor_id=header&0xffffffff,header_reference=header,level=level,
                      common_start=h,prefix_string=prefix,stat_floats=stats,
                      flag_offset=flag_offset,flag=flag,optional_four_probe=probe,
                      stat_values=stat_values,consumed=c.p,total=len(data),full=c.p==len(data))
        return {'ok':True,**c.meta}
    except Rejected as error:
        return {'ok':False,'consumed':c.p,'error':str(error)}


def parse_jp_player(payload):
    """Full grammar and fixed-field fingerprint; no scanning fallback."""
    if not 1762 <= len(payload) <= 33_554_432:
        return None
    result=walk_jp(payload)
    if not observed_profile_gates(result):
        return None
    from .identity import Identity
    return Identity(result['actor_id'],result['display_name'],'player',result['uid_offset'],
                    uid=result['uid'],embedded_timestamp_ms=result['timestamp'],
                    record_reference_id=result['header_reference'],
                    realm=result['identity_kind'],character_index=result['uid']&65535)
