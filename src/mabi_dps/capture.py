"""Read explicit JSONL segments or classic PCAP using the standard library."""
from dataclasses import dataclass
import ipaddress
import json
from pathlib import Path
import struct

from .stream import Segment


@dataclass(frozen=True)
class CaptureIssue:
    code: str
    detail: str
    reset_streams: bool = False


def json_segments(path: Path):
    with path.open(encoding='utf-8') as source:
        for number, line in enumerate(source, 1):
            if not line.strip():
                continue
            try:
                row = json.loads(line)
                if row.get('type') == 'loss':
                    if not isinstance(row.get('code'), str) or not isinstance(row.get('detail'), str):
                        raise ValueError('Loss records require code and detail strings')
                    reset = row.get('reset_streams', True)
                    if type(reset) is not bool:
                        raise ValueError('reset_streams must be a boolean')
                    yield CaptureIssue(row['code'], row['detail'], reset)
                    continue
                for field in ('timestamp_ns', 'sport', 'dport', 'seq'):
                    if type(row[field]) is not int:
                        raise ValueError(f'{field} must be an integer')
                if row['timestamp_ns'] < 0 or not 0 <= row['seq'] < 2**32:
                    raise ValueError('invalid timestamp or sequence')
                if not all(0 < row[field] <= 65535 for field in ('sport', 'dport')):
                    raise ValueError('invalid port')
                for field in ('src', 'dst'):
                    if not isinstance(row[field], str) or not row[field]:
                        raise ValueError(f'{field} must be a nonempty string')
                flags = {flag: row.get(flag, flag == 'ack')
                         for flag in ('syn', 'fin', 'rst', 'ack')}
                if any(type(flag) is not bool for flag in flags.values()):
                    raise ValueError('TCP flags must be booleans')
                payload = bytes.fromhex(row.get('payload_hex', ''))
                if len(payload) > 8 * 1024 * 1024:
                    raise ValueError('segment exceeds 8 MiB')
                yield Segment(*(row[field] for field in
                                ('timestamp_ns', 'src', 'sport', 'dst', 'dport', 'seq')),
                              payload=payload, **flags)
            except (ValueError, TypeError, KeyError) as exc:
                raise ValueError(f'JSONL line {number}: {exc}') from exc


def tcp_segment(packet: bytes, linktype: int, timestamp_ns: int):
    packet = bytes(packet)
    if linktype == 1:  # Ethernet, including stacked VLAN tags
        if len(packet) < 14:
            raise ValueError('short Ethernet header')
        kind = struct.unpack_from('!H', packet, 12)[0]
        offset = 14
        while kind in (0x8100, 0x88A8):
            if len(packet) < offset + 4:
                raise ValueError('short VLAN header')
            kind = struct.unpack_from('!H', packet, offset + 2)[0]
            offset += 4
        if kind not in (0x0800, 0x86DD):
            return None
        packet = packet[offset:]
    elif linktype not in (101, 228, 229):
        raise ValueError(f'unsupported PCAP link type {linktype}')
    if not packet:
        raise ValueError('empty IP packet')
    version = packet[0] >> 4
    if version == 4:
        if len(packet) < 20:
            raise ValueError('short IPv4 header')
        header = (packet[0] & 15) * 4
        length = struct.unpack_from('!H', packet, 2)[0]
        if header < 20 or length < header or length > len(packet):
            raise ValueError('invalid/truncated IPv4 length')
        if struct.unpack_from('!H', packet, 6)[0] & 0x3FFF:
            raise ValueError('IPv4 fragmentation is unsupported')
        if packet[9] != 6:
            return None
        src, dst = str(ipaddress.ip_address(packet[12:16])), str(ipaddress.ip_address(packet[16:20]))
        tcp = packet[header:length]
    elif version == 6:
        if len(packet) < 40:
            raise ValueError('short IPv6 header')
        length = 40 + struct.unpack_from('!H', packet, 4)[0]
        if length > len(packet):
            raise ValueError('truncated IPv6 packet')
        if packet[6] in (0, 43, 44, 50, 51, 60):
            raise ValueError('IPv6 extension headers are unsupported')
        if packet[6] != 6:
            return None
        src, dst = str(ipaddress.ip_address(packet[8:24])), str(ipaddress.ip_address(packet[24:40]))
        tcp = packet[40:length]
    else:
        raise ValueError('unsupported IP version')
    if len(tcp) < 20:
        raise ValueError('short TCP header')
    header = (tcp[12] >> 4) * 4
    if not 20 <= header <= len(tcp):
        raise ValueError('invalid TCP header length')
    sport, dport, seq = struct.unpack_from('!HHI', tcp)
    flags = tcp[13]
    return Segment(timestamp_ns, src, sport, dst, dport, seq, tcp[header:],
                   bool(flags & 2), bool(flags & 1), bool(flags & 4), bool(flags & 16))


def pcap_segments(path: Path, port: int):
    """Classic PCAP only. An explicit current game port prevents accidental
    interpretation of unrelated traffic. No region-specific port is assumed.
    """
    if not 1 <= port <= 65535:
        raise ValueError('PCAP requires a game port in 1..65535')
    magic = {b'\xd4\xc3\xb2\xa1': ('<', 1000), b'\xa1\xb2\xc3\xd4': ('>', 1000),
             b'\x4d\x3c\xb2\xa1': ('<', 1), b'\xa1\xb2\x3c\x4d': ('>', 1)}
    with path.open('rb') as source:
        header = source.read(24)
        if len(header) != 24 or header[:4] not in magic:
            raise ValueError('Expected classic PCAP; PCAPNG is not supported yet')
        endian, scale = magic[header[:4]]
        major, minor, _, _, snaplen, network = struct.unpack(endian + 'HHiIII', header[4:])
        linktype = network & 0xFFFF
        if (major, minor) != (2, 4) or linktype not in (1, 101, 228, 229):
            raise ValueError('Unsupported PCAP version or link type')
        while True:
            record = source.read(16)
            if not record:
                break
            if len(record) != 16:
                raise ValueError('Truncated PCAP record header')
            seconds, fraction, captured, original = struct.unpack(endian + 'IIII', record)
            if captured > min(snaplen, 8 * 1024 * 1024) or captured > original:
                raise ValueError('Invalid PCAP captured length')
            if fraction >= 1_000_000_000 // scale:
                raise ValueError('Invalid PCAP subsecond timestamp')
            packet = source.read(captured)
            if len(packet) != captured:
                raise ValueError('Truncated PCAP packet data')
            if captured < original:
                yield CaptureIssue('capture_truncated', 'Packet shorter than original length')
                continue
            try:
                segment = tcp_segment(packet, linktype, seconds * 1_000_000_000 + fraction * scale)
            except ValueError as exc:
                yield CaptureIssue('packet_rejected', str(exc))
                continue
            if segment and port in (segment.sport, segment.dport):
                yield segment


def read_capture(path: Path, port=None):
    if path.suffix.lower() in ('.pcap', '.cap'):
        if port is None:
            raise ValueError('Supply the current game TCP port for PCAP replay')
        return pcap_segments(path, port)
    if path.suffix.lower() != '.jsonl':
        raise ValueError('Choose .jsonl TCP segments or a classic .pcap file')
    return json_segments(path)
