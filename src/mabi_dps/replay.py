"""Replay accumulator shared by the CLI and desktop UI, with an audit trail."""
from collections import Counter, deque
from dataclasses import asdict, dataclass
import json
import struct
from pathlib import Path

from .capture import CaptureIssue, read_capture
from .compression import brotli_decoder
from .identity import combat_names, name_coverage, parse_jp_npc_name, parse_jp_spawn_name, parse_player_name, valid_identity_anchor
from .lifecycle import EntityRegistry, LifecycleLimitError
from .protocol import parse_attack, parse_damage
from .stream import Segment, TcpStream

MAX_FLOWS = 4096
MAX_FRAMES = 100_000
MAX_PAYLOAD_BYTES = 64 * 1024 * 1024
INFORMATIONAL = {'retransmission', 'scene_marker', 'identity_boundary', 'identity_alignment', 'identity_cache'}


@dataclass(frozen=True)
class Hit:
    timestamp_ns: int
    scope: str
    actor: int
    target: int
    amount: int
    critical: bool
    power: bool
    fast: bool
    additional: bool
    is_dot: bool
    raw_flags: str
    actor_generation: int = 1
    target_generation: int = 1

    @property
    def actor_key(self):
        return self.scope, self.actor, self.actor_generation

    @property
    def target_key(self):
        return self.scope, self.target, self.target_generation


def resolve_target(target, keys):
    """Legacy scope/ID filters work only while there is one incarnation."""
    if target is None:
        return None
    if len(target) == 3:
        return target
    if len(target) != 2:
        raise ValueError('Target requires scope, raw ID and incarnation generation')
    matches = {key for key in keys if key[:2] == target}
    if len(matches) > 1:
        raise ValueError('Target ID has multiple incarnations; select its generation')
    return next(iter(matches), (*target, 1))


def select_hits(hits, target=None):
    data = list(hits)
    resolved = resolve_target(target, (hit.target_key for hit in data))
    return [hit for hit in data if resolved is None or hit.target_key == resolved]


def summarize(hits: list[Hit], target=None):
    selected = select_hits(hits, target)
    start = min((hit.timestamp_ns for hit in selected), default=None)
    end = max((hit.timestamp_ns for hit in selected), default=None)
    duration = (end - start) / 1_000_000_000 if selected else 0.0
    groups = {}
    for hit in selected:
        row = groups.setdefault(hit.actor_key, {
            'scope': hit.scope, 'actor': hit.actor, 'actor_generation': hit.actor_generation,
            'damage': 0, 'hits': 0,
            'critical_hits': 0, 'power_hits': 0, 'fast_hits': 0,
            'additional_hits': 0, 'dot_hits': 0,
        })
        row['damage'] += hit.amount
        row['hits'] += 1
        for flag, column in [('critical', 'critical_hits'), ('power', 'power_hits'),
                             ('fast', 'fast_hits'), ('additional', 'additional_hits'),
                             ('is_dot', 'dot_hits')]:
            row[column] += int(getattr(hit, flag))
    total = sum(hit.amount for hit in selected)
    rows = sorted(groups.values(), key=lambda row: (-row['damage'], row['scope'], row['actor'], row['actor_generation']))
    for row in rows:
        row['dps'] = row['damage'] / duration if duration > 0 else None
        row['share'] = row['damage'] / total if total else 0.0
        row['critical_rate'] = row['critical_hits'] / row['hits']
    return {'damage': total, 'hits': len(selected), 'duration_seconds': duration,
            'dps': total / duration if duration > 0 else None,
            'start_ns': start, 'end_ns': end, 'actors': rows}


def chart_points(hits: list[Hit], target=None):
    """Cumulative and trailing-five-second DPS at each delivery timestamp.

    Five-second windows are (t - 5s, t]; their denominator stays five even near
    the start. Delivery batches are combined before plotting a point.
    """
    selected = sorted(select_hits(hits, target), key=lambda hit: hit.timestamp_ns)
    batches = []
    for hit in selected:
        if batches and batches[-1][0] == hit.timestamp_ns:
            batches[-1][1] += hit.amount
        else:
            batches.append([hit.timestamp_ns, hit.amount])
    if not batches:
        return []
    points, left, rolling, total = [], 0, 0, 0
    start = batches[0][0]
    for index, (timestamp, amount) in enumerate(batches):
        total += amount
        rolling += amount
        while left <= index and batches[left][0] <= timestamp - 5_000_000_000:
            rolling -= batches[left][1]
            left += 1
        elapsed = (timestamp - start) / 1_000_000_000
        points.append((elapsed, total / elapsed if elapsed else None, rolling / 5))
    return points


class Replay:
    def __init__(self, *, assume_aligned=False, decompress=None):
        self.assume_aligned = assume_aligned
        self.decompress = decompress
        self.streams = {}
        self.connections = {}
        self.frames = []
        self.hits = []
        self.issues = Counter()
        self.issue_details = []
        self.payload_bytes = 0
        self.segment_count = 0
        self.last_timestamp = None
        self.finished = False
        self.entities = EntityRegistry(self.report)
        self.actor_names = self.entities.names
        self.ambiguous_names = self.entities.ambiguous
        self.identity_records = Counter()
        self.identity_evidence = deque(maxlen=128)
        self.self_key = None
        self.self_evidence = None
        from .self_tracking import SelfTracker
        from .health_inference import HealthInference
        self.self_tracker = SelfTracker()
        self.health_inference = HealthInference(self.add_estimate)
        self.estimated_hits = deque(maxlen=30000)
        self.estimate_evidence = deque(maxlen=30000)
        self.frame_sequence = 0

    def name_coverage(self, target=None):
        return name_coverage(self.actor_names, (hit.actor_key for hit in select_hits(self.hits, target)),
                             (hit.target_key for hit in self.hits))

    def connection_scope(self, connection):
        scope = f"C{connection['id']}.{connection['generation']}.S{connection['scene']}"
        return scope + (f".L{connection['continuity']}" if connection['continuity'] else '')

    def refresh_connection(self, connection, timestamp_ns, reason):
        # TCP lifecycle is distinct from game entity lifecycle.
        self.entities.refresh_scope(self.identity_scope(self.connection_scope(connection)), timestamp_ns, reason)

    def stream_issue(self, connection, code, detail):
        self.report(code, detail)
        if code in {'tcp_gap_timeout', 'tcp_pending_overflow', 'framing_overflow',
                    'alignment_lost', 'frame_rejected', 'compressed_skipped'}:
            self.health_inference.discard(code)
            scope = self.identity_scope(self.connection_scope(connection))
            self.entities.refresh_scope(scope, self.last_timestamp or 0, code)
            connection['continuity'] += 1

    def report(self, code, detail):
        self.issues[code] += 1
        if len(self.issue_details) < 200:
            self.issue_details.append({'code': code, 'detail': detail})

    @property
    def partial(self):
        return any(code not in INFORMATIONAL for code in self.issues)

    def feed(self, item):
        if self.finished:
            raise ValueError('Replay already finished')
        if isinstance(item, CaptureIssue):
            if item.reset_streams or item.code in ('capture_handover',):
                self.health_inference.discard(item.code)
            self.report(item.code, item.detail)
            if item.reset_streams:
                self.streams.clear()
                for connection in self.connections.values():
                    self.refresh_connection(connection, self.last_timestamp or 0, item.code)
                    connection.update(generation=connection['generation'] + 1,
                                      syn=None, scene=0, continuity=0, closed=False, fin_directions=set())
            return
        if not isinstance(item, Segment):
            raise TypeError('Replay requires Segment or CaptureIssue records')
        if self.last_timestamp is not None and item.timestamp_ns < self.last_timestamp:
            raise ValueError('Capture timestamps moved backwards; replay requires capture-time order')
        self.last_timestamp = item.timestamp_ns
        self.tick(item.timestamp_ns)
        self.segment_count += 1
        connection = self.connections.get(item.connection)
        if connection is None:
            connection = {'id': len(self.connections) + 1, 'generation': 1,
                          'syn': None, 'scene': 0, 'continuity': 0,
                          'closed': False, 'fin_directions': set()}
            self.connections[item.connection] = connection
        if connection['closed'] and not item.rst:
            if not item.syn and not item.payload:
                return
            connection.update(generation=connection['generation'] + 1,
                              syn=None, scene=0, continuity=0, closed=False, fin_directions=set())
        # Bare SYN starts a connection generation, while SYN+ACK initializes
        # just its direction. Retransmitted SYNs must not double count damage.
        if item.syn and not item.ack:
            syn = (item.key, item.seq)
            if connection['syn'] != syn:
                if any(key[0] == item.connection for key in self.streams):
                    self.refresh_connection(connection, item.timestamp_ns, 'tcp_restart')
                    for key in list(self.streams):
                        if key[0] == item.connection:
                            self.streams.pop(key).finish()
                    connection['generation'] += 1
                    connection['scene'] = 0
                    connection['continuity'] = 0
                    connection['fin_directions'].clear()
                connection['syn'] = syn
        key = item.connection, item.key
        if key not in self.streams:
            if len(self.streams) >= MAX_FLOWS:
                raise ValueError('Replay exceeds 4096 directional streams; use a narrower capture')
            self.streams[key] = TcpStream(
                lambda code, detail, owner=connection: self.stream_issue(owner, code, detail),
                assume_aligned=self.assume_aligned, decompress=self.decompress,
                identity_anchor=valid_identity_anchor)
        for frame in self.streams[key].feed(item):
            self.frame_sequence += 1
            context = connection['id'], connection['generation']
            if frame.decoded and frame.normalized_opcode in (20307, 20484):
                # These include state updates, not a validated entity death.
                self.refresh_connection(connection, item.timestamp_ns, 'scene_state_marker')
                connection['scene'] += 1
                connection['continuity'] = 0
                self.report('scene_marker', 'Scene/state marker refreshed metadata; entities retained')
            scope = self.connection_scope(connection)
            entity_scope = self.identity_scope(scope)
            record = {'timestamp_ns': item.timestamp_ns, 'scope': scope,
                      'flow': list(item.key), 'wire_opcode': frame.wire_opcode,
                      'normalized_opcode': frame.normalized_opcode,
                      'compression': frame.compression, 'payload_hex': frame.payload.hex(),
                      'payload_encoding': 'decoded' if frame.decoded else 'wire',
                      'kind': ('undecoded' if not frame.decoded else
                               'control' if frame.wire_opcode is None else 'unknown')}
            if not frame.decoded and frame.wire_opcode in (20054, 20517):
                self.identity_records['undecoded'] += 1
            try:
                if frame.decoded and frame.normalized_opcode == 21045:
                    damage = parse_damage(frame.payload)
                    actor_ref = self.entities.observe(entity_scope, damage.actor, item.timestamp_ns, damage=True)
                    target_ref = self.entities.observe(entity_scope, damage.target, item.timestamp_ns, damage=True)
                    hit = Hit(item.timestamp_ns, entity_scope, **asdict(damage),
                              actor_generation=actor_ref.generation, target_generation=target_ref.generation)
                    self.health_inference.damage(context,target_ref.key)
                    self.add_hit(hit)
                    record.update(kind='damage', decoded=asdict(damage),
                                  entities={'actor': asdict(actor_ref), 'target': asdict(target_ref)})
                elif frame.decoded and frame.normalized_opcode == 20460:
                    attack = parse_attack(frame.payload)
                    references = {role: asdict(self.entities.observe(entity_scope, actor, item.timestamp_ns))
                                  for role, actor in (('actor', attack.actor), ('target', attack.target)) if actor}
                    if attack.actor and attack.target and attack.actor!=attack.target:
                        source_ref=self.entities.active[(entity_scope,attack.actor)]
                        target_ref=self.entities.active[(entity_scope,attack.target)]
                        self.health_inference.attack(context,source_ref.key,target_ref.key,self.frame_sequence)
                        self.combat_activity(item.timestamp_ns,source_ref.key,target_ref.key)
                    tables = combat_names()
                    record.update(kind='attack', decoded=asdict(attack), entities=references,
                                  lookup_name=tables['Skills'].get(str(attack.skill_key)),
                                  display_name=tables['SkillDisplayNames'].get(str(attack.skill_key)),
                                  name_source='inherited combat-names table')
                elif frame.decoded and frame.wire_opcode in (20054, 20517,20051):
                    role = 'player' if frame.wire_opcode == 20054 else 'npc'
                    self.identity_records[f'{role}_received'] += 1
                    structured = False
                    if frame.wire_opcode == 20054:
                        from .appearance import parse_jp_player
                        identity=parse_jp_player(frame.payload)
                        structured=identity is not None
                        identity=identity or parse_player_name(frame.payload)
                    elif frame.wire_opcode==20051:
                        identity=parse_jp_spawn_name(frame.payload,combat_names()['Monsters'])
                    else:
                        identity=parse_jp_npc_name(frame.payload,combat_names()['Monsters'])
                    record['kind'] = 'identity_unresolved'
                    if identity is not None:
                        self.identity_records[f'{role}_resolved'] += 1
                        ref = self.add_identity(entity_scope, identity, item.timestamp_ns)
                        if structured:
                            self.self_tracker.appearance(context,ref.key,identity,item.timestamp_ns,self.frame_sequence)
                            self.sync_self()
                        record.update(kind='identity', decoded=asdict(identity), entity=asdict(ref))
                    else:
                        self.identity_records[f'{role}_unresolved'] += 1
                        actor_offset = 8 if frame.wire_opcode == 20517 else 0
                        if len(frame.payload) >= actor_offset + 4:
                            raw_actor, = struct.unpack_from('<I', frame.payload, actor_offset)
                            if raw_actor:
                                ref = self.entities.unresolved_appearance(entity_scope, raw_actor, item.timestamp_ns,
                                                                          f'wire {frame.wire_opcode}')
                                record['entity'] = asdict(ref)
                elif frame.decoded and frame.wire_opcode == 20590:
                    from .self_tracking import personal_prefix
                    prefix=personal_prefix(frame.payload)
                    if prefix is not None:
                        self.self_tracker.personal_record(context,prefix,item.timestamp_ns,self.frame_sequence)
                        self.sync_self()
                        self.identity_records['personal_received']+=1
                        record.update(kind='personal_identity',decoded={'code':prefix[2:].hex().upper(),
                                      'character_index':int.from_bytes(prefix[:2],'little')})
                elif frame.decoded and frame.wire_opcode == 170205:
                    from .health_inference import parse_health
                    health=parse_health(frame.payload)
                    if health is not None:
                        actor,hp,opaque=health
                        ref=self.entities.observe(entity_scope,actor,item.timestamp_ns)
                        self.health_inference.health(context,ref.key,hp,item.timestamp_ns,self.frame_sequence)
                        record.update(kind='health',decoded={'actor':actor,'current_hp':hp,'opaque':opaque})
            except LifecycleLimitError:
                raise
            except ValueError as exc:
                record.update(kind='rejected', error=str(exc))
                self.report('payload_rejected', str(exc))
            self.record_frame(record)
        if item.fin:
            connection['fin_directions'].add(item.key)
        fully_closed = (len(connection['fin_directions']) == 2 and
                        all(stream.closed for key, stream in self.streams.items() if key[0] == item.connection))
        if item.rst or fully_closed:
            self.refresh_connection(connection, item.timestamp_ns, 'tcp_reset' if item.rst else 'tcp_closed')
            for stream_key in list(self.streams):
                if stream_key[0] == item.connection:
                    self.streams.pop(stream_key).finish()
            connection['closed'] = True

    def add_hit(self, hit):
        self.hits.append(hit)

    def add_estimate(self,event):
        source,target=event['source'],event['target']
        hit=Hit(event['timestamp_ns'],source[0],source[1],target[1],event['amount'],
                False,False,False,False,False,'',actor_generation=source[2],target_generation=target[2])
        self.estimated_hits.append(hit)
        self.estimate_evidence.append(dict(event))
        return hit

    def tick(self,timestamp_ns):
        pass

    def combat_activity(self,timestamp_ns,source,target):
        pass

    def sync_self(self):
        key=self.self_tracker.operator_key
        if key is not None:
            current=self.entities.active.get(key[:2])
            if current is None or current.key!=key:self.self_tracker.assign(None,'retired operator binding')
        self.self_key,self.self_evidence=self.self_tracker.binding

    def identity_scope(self, scope):
        # Offline files lack process ownership. Keep distinct connections
        # separate, but transport restarts/state updates retain their directory.
        return scope.split('.', 1)[0]

    def add_identity(self, scope, identity, timestamp_ns=None):
        observed_at = (self.last_timestamp or 0) if timestamp_ns is None else timestamp_ns
        return self.entities.appearance(scope, identity, observed_at)

    def set_self(self, key, *, evidence):
        if not evidence:
            raise ValueError('Self assignment requires its evidence/source')
        if key is not None:
            key = tuple(key)
            if key not in {ref.key for ref in self.entities.states}:
                raise ValueError('Self assignment must reference an observed entity')
        if key is not None:
            ref=self.entities.active.get(key[:2])
            if ref is None or ref.key!=key:raise ValueError('Selected entity is no longer current')
        self.self_tracker.assign(key,evidence)
        self.sync_self()

    def self_identity(self):
        return {'key': self.self_key, 'evidence': self.self_evidence,
                'placeholder': '<ME>', 'packet_name': False,'tracking':self.self_tracker.export()}

    def record_frame(self, record):
        size = len(record['payload_hex']) // 2
        if len(self.frames) >= MAX_FRAMES or self.payload_bytes + size > MAX_PAYLOAD_BYTES:
            raise ValueError('Replay audit limit exceeded (100,000 frames / 64 MiB payload); split capture')
        self.payload_bytes += size
        self.remember_identity_frame(record)
        self.frames.append(record)

    def remember_identity_frame(self, record):
        if record['wire_opcode'] in (20054, 20517, 20590,20051):
            evidence = dict(record)
            evidence['payload_hex'] = record['payload_hex'][:131072]
            evidence['payload_truncated'] = len(record['payload_hex']) > 131072
            self.identity_evidence.append(evidence)

    def finish(self):
        if not self.finished:
            self.health_inference.flush('clean recording end')
            for stream in self.streams.values():
                stream.finish()
            self.finished = True
        return self

    def export(self):
        return {'format': 'mabi-dps-replay-v3', 'synthetic_or_unverified': True,
                'entity_scope_policy': 'connection lineage; no process metadata in portable TCP input',
                'partial': self.partial, 'segments': self.segment_count,
                'assume_aligned': self.assume_aligned,
                'issues': dict(self.issues), 'issue_details': self.issue_details,
                'summary': summarize(self.hits), 'hits': [asdict(hit) for hit in self.hits],
                'actor_names': [{'scope': scope, 'generation': generation, **asdict(identity)}
                                for (scope, _, generation), identity in self.actor_names.items()],
                'lifecycle': self.entities.export(),
                'identity_records': dict(self.identity_records),
                'identity_evidence': list(self.identity_evidence),
                'self_identity': self.self_identity(),
                'estimated_hits': [asdict(hit) for hit in self.estimated_hits],
                'estimate_evidence':list(self.estimate_evidence),
                'health_inference':dict(self.health_inference.counters),
                'name_coverage': self.name_coverage(),
                'frames': self.frames}


def load_replay(path: Path, *, port=None, assume_aligned=False):
    replay = Replay(assume_aligned=assume_aligned, decompress=brotli_decoder())
    for item in read_capture(path, port):
        replay.feed(item)
    return replay.finish()


def demo_segments():
    """Newly generated synthetic bytes; no original binaries or traffic."""
    import struct
    start = 1_800_000_000_000_000_000
    seq = 1000
    fields = {'src': '192.0.2.1', 'sport': 23456, 'dst': '192.0.2.2', 'dport': 34567}
    yield Segment(start, seq=seq, syn=True, ack=False, **fields)
    seq += 1
    for index, (actor, amount) in enumerate(((101, 1200), (202, 900), (101, 1800),
                                            (202, 1500), (101, 2400), (202, 1100))):
        payload = bytearray(53)
        struct.pack_into('<I', payload, 0, actor)
        struct.pack_into('<I', payload, 8, 999)
        struct.pack_into('<Q', payload, 16, amount)
        payload[32] = index % 2
        frame = struct.pack('<iiB', 21196, len(payload), 0) + payload
        timestamp = start + (index + 1) * 1_000_000_000
        # Split frame, deliver its tail first, then retransmit it. This drives
        # the same path used by capture input rather than injecting totals.
        yield Segment(timestamp, seq=(seq + 19) % 2**32, payload=frame[19:], **fields)
        yield Segment(timestamp, seq=seq, payload=frame[:19], **fields)
        yield Segment(timestamp, seq=seq, payload=frame, **fields)
        seq += len(frame)


def demo_replay():
    replay = Replay()
    for segment in demo_segments():
        replay.feed(segment)
    return replay.finish()


def write_demo(path: Path):
    with path.open('w', encoding='utf-8', newline='\n') as output:
        for segment in demo_segments():
            row = asdict(segment)
            row['payload_hex'] = row.pop('payload').hex()
            output.write(json.dumps(row) + '\n')
