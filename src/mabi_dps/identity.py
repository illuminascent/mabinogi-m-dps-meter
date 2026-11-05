"""Conservative name heuristics recovered statically and checked on live records.

Names never affect damage. Character IDs remain the low uint32 wire value.
NPC names are inherited lookup text, not strings supplied by the JP server.
"""
from dataclasses import dataclass, replace
from functools import lru_cache
import json
from pathlib import Path
import struct
import unicodedata


@lru_cache(maxsize=1)
def combat_names():
    return json.loads((Path(__file__).resolve().parents[2] / 'assets/data/combat-names.json')
                      .read_text(encoding='utf-8'))


@dataclass(frozen=True)
class Identity:
    actor: int
    name: str
    kind: str
    source_offset: int
    uid: int | None = None
    job_key: int | None = None
    template_key: str | None = None
    embedded_timestamp_ms: int | None = None
    record_reference_id: int | None = None
    realm: int | None = None
    character_index: int | None = None


def entity_label(names, key, *, role='actor', self_key=None):
    """Presentation uses only an exact incarnation match, never a raw-ID alias."""
    _, actor, generation = key
    identity = names.get(key)
    suffix = f', incarnation {generation}' if generation > 1 else ''
    return (f'{identity.name} [{actor}{suffix}]' if identity is not None
            else f'<ME> [{actor}{suffix}]' if key == self_key
            else f'Name pending ({role} {actor}{suffix})')


def name_coverage(names, actor_keys, target_keys):
    result = {}
    for role, keys in (('actors', actor_keys), ('targets', target_keys)):
        keys = set(keys)
        missing = sorted(key for key in keys if key not in names)
        result[role] = {'resolved': len(keys) - len(missing), 'total': len(keys),
                        'missing': [{'scope': scope, 'actor': actor, 'generation': generation}
                                    for scope, actor, generation in missing]}
    return result


def valid_identity_anchor(frame):
    """Validate a complete decoded appearance before midstream alignment.

    Both observed name message types are Brotli-compressed. A wire opcode
    alone is insufficient. Keep startup probes small and require the recovered
    record structure; this does not change decoding limits once aligned.
    """
    if not frame.decoded or len(frame.payload) > 262144:
        return False
    if frame.wire_opcode == 20054:
        return parse_player_name(frame.payload) is not None
    if frame.wire_opcode == 20517 and len(frame.payload) >= 128:
        return parse_jp_npc_name(frame.payload, combat_names()['Monsters']) is not None
    return False


def parse_player_name(payload: bytes) -> Identity | None:
    """JP 20054 / internal 20047, B.E method 0x06000CC8.

    Scan the helper's UID/type/UTF-16/timestamp structure. Accept only one
    distinct valid name. This is structure matching, not a full entity schema.
    The embedded timestamp is metadata; it is never used as a hit timestamp.
    """
    if len(payload) < 40:
        return None
    actor, = struct.unpack_from('<I', payload)
    if not actor:
        return None
    candidates = {}
    for offset in range(8, len(payload) - 31):
        uid, kind, length = struct.unpack_from('<Qii', payload, offset)
        if not uid or not 1 <= kind <= 128 or not 2 <= length <= 64 or length % 2:
            continue
        if offset + 28 + length > len(payload):
            continue
        timestamp, = struct.unpack_from('<q', payload, offset + 16 + length)
        if not 1_500_000_000_000 <= timestamp <= 2_200_000_000_000:
            continue
        try:
            name = payload[offset + 16:offset + 16 + length].decode('utf-16-le', errors='strict')
        except UnicodeError:
            continue
        if not name.strip() or any(unicodedata.category(char) == 'Cc' for char in name):
            continue
        job = (struct.unpack_from('<I', payload, offset + length + 49)[0]
               if offset + length + 53 <= len(payload) else None)
        candidates[name] = Identity(actor, name, 'player', offset, uid=uid, job_key=job,
                                    embedded_timestamp_ms=timestamp)
    return next(iter(candidates.values())) if len(candidates) == 1 else None


def parse_npc_name(payload: bytes, monsters: dict[str, str]) -> Identity | None:
    """B.E method 0x06000CC9's entity-template lookup, with ambiguity rejection.

    Inherited internal opcode 20044's entity body. JP 20517 wraps this body;
    use parse_jp_npc_name for that wire record.
    The key is four raw bytes as uppercase hex, NOT the runtime actor ID.
    """
    if len(payload) < 20:
        return None
    actor, = struct.unpack_from('<I', payload)
    if not actor:
        return None
    candidates = {}
    for offset in range(len(payload) - 12, 7, -1):
        if payload[offset:offset + 4] != b'\x03\0\0\0' or payload[offset + 8:offset + 12] != b'\0' * 4:
            continue
        key = payload[offset + 4:offset + 8].hex().upper()
        if key in monsters:
            candidates.setdefault(key, Identity(actor, monsters[key], 'npc_lookup', offset,
                                                 template_key=key))
    return next(iter(candidates.values())) if len(candidates) == 1 else None


def parse_jp_npc_name(payload: bytes, monsters: dict[str, str]) -> Identity | None:
    """JP 20517: eight-byte reference prefix followed by the entity body.

    Entry captures match damage runtime IDs at +8, not +0. The outer field's
    meaning is unproven; preserve it without treating it as a lifecycle event.
    """
    if len(payload) < 28:
        return None
    identity = parse_npc_name(payload[8:], monsters)
    if identity is None:
        return None
    reference, = struct.unpack_from('<Q', payload)
    return replace(identity, source_offset=identity.source_offset + 8,
                   record_reference_id=reference)


def parse_jp_spawn_name(payload: bytes,monsters: dict[str,str]) -> Identity | None:
    """Observed JP20051 NPC branch; metadata only, never a lifecycle event.

    Seventeen dungeon records independently match the template predicate and
    fixed footer. The linked summon branch has a different discriminator and
    deliberately remains unresolved here.
    """
    if len(payload)<110:return None
    if struct.unpack_from('<Q',payload,len(payload)-110)[0] != 2**64-1:return None
    identity=parse_npc_name(payload,monsters)
    return identity if identity is not None and identity.source_offset==len(payload)-102 else None
