"""Transport-independent TCP replay. No sockets, process access, or capture DLLs."""
from dataclasses import dataclass
import struct
from typing import Callable

from .protocol import Frame, normalize_jp, read_frame

MAX_BUFFER = 8 * 1024 * 1024
MAX_PENDING = 256
GAP_NS = 3_000_000_000
MODULO = 1 << 32
# Only the simple anchors established by static analysis. Structured timestamp
# anchors are deliberately omitted until their validation is implemented.
EMPTY_ANCHORS = {20110, 20280}
BODY_ANCHORS = {20520, 20047, 120830, 120828, 120637, 120458, 120461}


@dataclass(frozen=True)
class Segment:
    timestamp_ns: int
    src: str
    sport: int
    dst: str
    dport: int
    seq: int
    payload: bytes = b''
    syn: bool = False
    fin: bool = False
    rst: bool = False
    ack: bool = True

    @property
    def key(self) -> tuple:
        return self.src, self.sport, self.dst, self.dport

    @property
    def connection(self) -> tuple:
        return tuple(sorted(((self.src, self.sport), (self.dst, self.dport))))


def distance(seq: int, expected: int) -> int:
    """Signed TCP serial arithmetic, valid within a half sequence space."""
    return ((seq - expected + (1 << 31)) % MODULO) - (1 << 31)


class Framer:
    def __init__(self, report: Callable[[str, str], None], *, aligned=False,
                 decompress=None, identity_anchor=None):
        self.report = report
        self.aligned = aligned
        self.decompress = decompress
        self.buffer = bytearray()
        self.identity_anchor = identity_anchor

    def reset(self, aligned=False):
        self.buffer.clear()
        self.aligned = aligned

    def feed(self, data: bytes) -> list[Frame]:
        if len(self.buffer) + len(data) > MAX_BUFFER:
            self.report('framing_overflow', 'Discarded application buffer over 8 MiB')
            self.reset()
            return []
        self.buffer.extend(data)
        frames = []
        offset = 0
        skipped = 0
        while len(self.buffer) - offset >= 8:
            probe = None
            control = self.buffer[offset:offset + 4] == b'MMCG'
            if control:
                length, = struct.unpack_from('<i', self.buffer, offset + 4)
                header, compression, opcode = 8, 0, None
                valid = 0 <= length <= 65_536
                anchor = valid
            else:
                if len(self.buffer) - offset < 9:
                    break
                wire, length, compression = struct.unpack_from('<iiB', self.buffer, offset)
                opcode = normalize_jp(wire)
                header = 9
                valid = (100 <= opcode <= 500_000 and 0 <= length <= 4_194_304
                         and compression in (0, 1))
                anchor = (compression == 0 and ((opcode in EMPTY_ANCHORS and length == 0)
                          or (opcode in BODY_ANCHORS and length > 0)))
                if (not self.aligned and valid and self.identity_anchor is not None
                        and wire in (20054, 20517)):
                    anchor = False
                    candidate_end = offset + header + length
                    if 0 < length <= 262144 and (not compression or self.decompress is not None):
                        if len(self.buffer) < candidate_end:
                            break
                        try:
                            candidate = read_frame(bytes(self.buffer[offset:candidate_end]), self.decompress)
                        except ValueError:
                            pass
                        else:
                            if candidate is not None and self.identity_anchor(candidate):
                                probe, anchor = candidate, True
            if not valid or (not self.aligned and not anchor):
                if self.aligned:
                    self.report('alignment_lost', 'Invalid application header; seeking an anchor')
                self.aligned = False
                offset += 1
                skipped += 1
                continue
            end = offset + header + length
            if len(self.buffer) < end:
                break
            self.aligned = True
            if probe is not None:
                frames.append(probe)
                self.report('identity_alignment', f'Validated appearance anchor for wire {probe.wire_opcode}')
            elif compression and self.decompress is None:
                self.report('compressed_skipped', 'Brotli frame skipped: bounded decoder unavailable')
                frames.append(Frame(wire, opcode, compression,
                                    bytes(self.buffer[offset + header:end]), end - offset, False))
            else:
                try:
                    frame = read_frame(bytes(self.buffer[offset:end]), self.decompress)
                except ValueError as exc:
                    self.report('frame_rejected', str(exc))
                    frames.append(Frame(wire if not control else None, opcode, compression,
                                        bytes(self.buffer[offset + header:end]), end - offset, False))
                else:
                    frames.append(frame)
            offset = end
        if offset:
            del self.buffer[:offset]
        if skipped:
            self.report('unaligned_bytes', f'Discarded {skipped} bytes while seeking an anchor')
        return frames


class TcpStream:
    """One direction, first-arrival overlap policy, bounded gap buffering.

    The caller supplies capture-order timestamps. Frames emitted while a gap
    closes all receive the closing segment's time, rather than fabricated hit
    times. Midstream attachment requires an anchor unless explicitly trusted.
    """
    def __init__(self, report, *, assume_aligned=False, decompress=None, identity_anchor=None):
        self.report = report
        self.framer = Framer(report, aligned=assume_aligned, decompress=decompress,
                             identity_anchor=identity_anchor)
        self.expected = None
        self.syn_seq = None
        self.fin_seq = None
        self.pending = []
        self.pending_bytes = 0
        self.gap_since = None
        self.closed = False

    def reset(self, aligned=False):
        self.expected = None
        self.fin_seq = None
        self.pending.clear()
        self.pending_bytes = 0
        self.gap_since = None
        self.framer.reset(aligned)

    def feed(self, segment: Segment) -> list[Frame]:
        if segment.rst:
            if self.pending or self.framer.buffer or self.fin_seq is not None:
                self.report('reset_with_buffer', 'TCP reset discarded unfinished data')
            self.reset()
            self.closed = True
            return []
        if self.closed:
            if not segment.syn:
                return []
            self.closed = False
        if segment.syn and self.syn_seq != segment.seq:
            if self.expected is not None:
                self.report('tcp_restart', 'TCP SYN started a new stream')
            self.reset(aligned=True)
            self.syn_seq = segment.seq
            self.expected = (segment.seq + 1) % MODULO
        seq = (segment.seq + int(segment.syn)) % MODULO
        if self.expected is None:
            self.expected = seq
        if self.gap_since is not None and segment.timestamp_ns - self.gap_since > GAP_NS:
            self.report('tcp_gap_timeout', 'Gap exceeded three seconds; discarded pending data')
            self.reset()
            self.expected = seq
        if segment.payload:
            delta = distance(seq, self.expected)
            payload = segment.payload
            if delta < 0:
                payload = payload[-delta:]
                seq = self.expected
                self.report('retransmission', 'Ignored duplicate TCP bytes')
            if payload:
                # Preserve earlier buffered bytes on overlaps, even when the
                # gap-closing segment overlaps a future segment.
                pieces = [(seq, payload)]
                for old_seq, old_data in self.pending:
                    old_start = distance(old_seq, self.expected)
                    old_end = old_start + len(old_data)
                    remaining = []
                    for part_seq, part in pieces:
                        start = distance(part_seq, self.expected)
                        end = start + len(part)
                        if end <= old_start or start >= old_end:
                            remaining.append((part_seq, part))
                        else:
                            if start < old_start:
                                remaining.append((part_seq, part[:old_start - start]))
                            if end > old_end:
                                remaining.append(((self.expected + old_end) % MODULO,
                                                  part[old_end - start:]))
                    pieces = remaining
                self.pending.extend(pieces)
                self.pending_bytes += sum(len(part) for _, part in pieces)
        if segment.fin:
            self.fin_seq = (seq + len(segment.payload)) % MODULO
        if len(self.pending) > MAX_PENDING or self.pending_bytes > MAX_BUFFER:
            self.report('tcp_pending_overflow', 'Pending TCP data exceeded its limit')
            self.reset()
            return []
        frames = []
        self.pending.sort(key=lambda item: distance(item[0], self.expected))
        while self.pending and distance(self.pending[0][0], self.expected) <= 0:
            part_seq, part = self.pending.pop(0)
            self.pending_bytes -= len(part)
            trim = -distance(part_seq, self.expected)
            part = part[trim:]
            if part:
                self.expected = (self.expected + len(part)) % MODULO
                frames.extend(self.framer.feed(part))
        waiting_for_fin = self.fin_seq is not None and distance(self.fin_seq, self.expected) > 0
        if self.pending or waiting_for_fin:
            if self.gap_since is None:
                self.gap_since = segment.timestamp_ns
        else:
            self.gap_since = None
        # An out-of-order FIN is remembered until the missing bytes arrive.
        if self.fin_seq == self.expected and not self.pending:
            self.expected = (self.expected + 1) % MODULO
            self.fin_seq = None
            self.closed = True
        return frames

    def finish(self):
        if self.pending or self.fin_seq is not None:
            self.report('tcp_gap_at_eof', f'{len(self.pending)} segments / pending FIN beyond a TCP gap')
        if self.framer.buffer:
            self.report('frame_tail_at_eof', f'{len(self.framer.buffer)} unfinished application bytes')
