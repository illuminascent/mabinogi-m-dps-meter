"""Offline reference for layouts recovered from MMH-0.28.0-jp.4.

Reads supplied bytes only: no process access, capture drivers, network, or sample
assembly loading. This is a partial reconstruction, not a live DPS meter.
See AGENTS.md for findings and the location of local prototype evidence.
"""
from dataclasses import asdict, dataclass
import json
import math
from pathlib import Path
import struct
from typing import Callable

JP_MAP = {int(k): v for k, v in json.loads(
    Path(__file__).with_name('jp_protocol_map.json').read_text(encoding='utf-8')
).items()}


def normalize_jp(opcode: int) -> int:
    """Exact normalization used by this JP build, including unknown-ID fallback."""
    return JP_MAP.get(opcode, opcode + 200_000)


@dataclass(frozen=True)
class Frame:
    wire_opcode: int | None
    normalized_opcode: int | None
    compression: int
    payload: bytes
    consumed: int
    decoded: bool = True


def read_frame(data: bytes, decompress: Callable[[bytes], bytes] | None = None) -> Frame | None:
    """Read one COMPLETE frame at a KNOWN boundary in a REASSEMBLED TCP stream.

    None means incomplete input. A bad header raises ValueError. Does not scan
    arbitrary TCP segments for apparent damage frames. For compression=1,
    supply a trusted Brotli decoder with a streaming 32 MiB output limit.
    Control record MMCG is consumed and returned with opcode=None.
    """
    if len(data) < 8:
        return None
    if data[:4] == b'MMCG':
        length, = struct.unpack_from('<i', data, 4)
        if not 0 <= length <= 65_536:
            raise ValueError('invalid MMCG length')
        end = 8 + length
        return None if len(data) < end else Frame(None, None, 0, data[8:end], end)
    if len(data) < 9:
        return None
    wire_opcode, length, compression = struct.unpack_from('<iiB', data)
    opcode = normalize_jp(wire_opcode)
    if not 100 <= opcode <= 500_000:
        raise ValueError('opcode outside recovered framing range')
    if not 0 <= length <= 4_194_304 or compression not in (0, 1):
        raise ValueError('invalid length or compression flag')
    end = 9 + length
    if len(data) < end:
        return None
    payload = data[9:end]
    if compression:
        if decompress is None:
            raise ValueError('Brotli decoder required for this frame')
        payload = decompress(payload)
        if len(payload) > 33_554_432:
            raise ValueError('decompressed payload too large')
    return Frame(wire_opcode, opcode, compression, payload, end)


@dataclass(frozen=True)
class Damage:
    actor: int
    target: int
    amount: int
    critical: bool
    power: bool
    fast: bool
    additional: bool
    is_dot: bool
    raw_flags: str


def parse_damage(payload: bytes) -> Damage:
    """JP 21196 -> internal 21045. Offsets relative to decompressed payload.

    Source: method 0x06000CCF. IDs occupy wider wire slots, but MMH reads their
    LOW 32 BITS. Do not silently replace these reads with uint64 without checking
    real captures and updating all identity/attack parsers consistently.
    """
    if len(payload) < 53:
        raise ValueError('damage payload shorter than 53 bytes')
    actor, = struct.unpack_from('<I', payload, 0)
    target, = struct.unpack_from('<I', payload, 8)
    amount, = struct.unpack_from('<Q', payload, 16)
    if not actor or not target or actor == target or amount > 1_000_000_000_000:
        raise ValueError('damage event rejected by recovered sanity checks')
    return Damage(actor, target, amount,
                  bool(payload[32] & 1), bool(payload[33] & 2),
                  bool(payload[33] & 4), bool(payload[35] & 8),
                  bool(payload[36] & 16), payload[32:39].hex().upper())


@dataclass(frozen=True)
class Attack:
    actor: int
    target: int
    skill_key: int
    key_source: str
    skill_meta: str


def parse_attack(payload: bytes) -> Attack:
    """JP 20526 -> internal 20460. No damage is added for this message.

    Source: batch filter 0x06000CF5 and skill-key helper 0x06000E4F.
    Matching attacks to damage is a separate heuristic; see the report.
    """
    if len(payload) < 35:
        raise ValueError('attack payload shorter than 35 bytes')
    actor, = struct.unpack_from('<I', payload, 0)
    target, = struct.unpack_from('<I', payload, 8)
    key1, key2 = struct.unpack_from('<II', payload, 16)
    return Attack(actor, target, key1 or key2,
                  'Key1' if key1 else ('Key2' if key2 else 'none'),
                  payload[24:32].hex().upper())


def damage_per_second(amount: int, elapsed_seconds: float) -> float:
    """Recovered arithmetic, 0x06000A12; caller selects the combat time range."""
    return amount / elapsed_seconds if elapsed_seconds > 0 and math.isfinite(elapsed_seconds) else 0.0


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('kind', choices=['damage', 'attack', 'frame'])
    parser.add_argument('file', type=Path, help='raw payload, or already aligned TCP application bytes')
    args = parser.parse_args()
    data = args.file.read_bytes()
    if args.kind == 'frame':
        frame = read_frame(data)
        result = None if frame is None else {**asdict(frame), 'payload': frame.payload.hex()}
    else:
        result = asdict(parse_damage(data) if args.kind == 'damage' else parse_attack(data))
    print(json.dumps(result, indent=2))
