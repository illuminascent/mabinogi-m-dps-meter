"""Capture worker orchestration and bounded live statistics.

Python decodes ordinary copied bytes. Rust owns driver callbacks. The worker
discovers its own game-owned flows; controllers cannot supply arbitrary filters.
"""
from collections import Counter, deque
from dataclasses import asdict
import json
from pathlib import Path
import threading
import time

from .capture import CaptureIssue, tcp_segment
from .compression import brotli_decoder
from .identity import name_coverage
from .encounter import EncounterPolicy
from .replay import Replay, chart_points, resolve_target, select_hits
from .stream import Segment

FILETIME_EPOCH = 116_444_736_000_000_000


def flow_key(row):
    return row['local_address'], row['local_port'], row['remote_address'], row['remote_port']


def scope_signature(plan):
    return (tuple(sorted((p['pid'], p['created_filetime']) for p in plan['processes'])),
            tuple(sorted((r['pid'], r['family'], *flow_key(r)) for r in plan['candidate_flows'])))


class Totals:
    def __init__(self):
        self.first = self.last = None
        self.rows = {}

    def add(self, hit):
        self.first = hit.timestamp_ns if self.first is None else min(self.first, hit.timestamp_ns)
        self.last = hit.timestamp_ns if self.last is None else max(self.last, hit.timestamp_ns)
        key = hit.actor_key
        if key not in self.rows and len(self.rows) >= 16384:
            raise ValueError('Live actor limit reached; start a fresh session')
        row = self.rows.setdefault(key, {'scope': hit.scope, 'actor': hit.actor,
              'actor_generation': hit.actor_generation, 'damage': 0,
              'hits': 0, 'critical_hits': 0, 'power_hits': 0, 'fast_hits': 0,
              'additional_hits': 0, 'dot_hits': 0})
        row['damage'] += hit.amount
        row['hits'] += 1
        for flag, name in [('critical', 'critical_hits'), ('power', 'power_hits'),
                           ('fast', 'fast_hits'), ('additional', 'additional_hits'), ('is_dot', 'dot_hits')]:
            row[name] += int(getattr(hit, flag))

    def summary(self):
        duration = (self.last - self.first) / 1e9 if self.first is not None else 0
        damage = sum(row['damage'] for row in self.rows.values())
        rows = [dict(row, dps=row['damage'] / duration if duration else None,
                     share=row['damage'] / damage if damage else 0,
                     critical_rate=row['critical_hits'] / row['hits']) for row in self.rows.values()]
        rows.sort(key=lambda row: (-row['damage'], row['scope'], row['actor'], row['actor_generation']))
        return {'damage': damage, 'hits': sum(row['hits'] for row in rows),
                'dps': damage / duration if duration else None, 'duration_seconds': duration,
                'start_ns': self.first, 'end_ns': self.last, 'actors': rows}


class LiveSnapshot:
    def __init__(self, engine):
        self.hits = list(engine.hits)
        self.frames = list(engine.frames)
        self.issues = Counter(engine.issues)
        self.issue_details = list(engine.issue_details)
        self.segment_count = engine.segment_count
        self.frame_count = engine.frame_count
        self.history_truncated = (engine.totals[None].summary()['hits'] > len(self.hits)
            or engine.inferred_totals[None].summary()['hits']>len(engine.estimated_hits))
        self.partial = engine.partial
        self.summaries = {key: value.summary() for key, value in engine.totals.items()}
        self.actor_names = dict(engine.actor_names)
        self.lifecycle = engine.entities.export()
        self.identity_records = dict(engine.identity_records)
        self.identity_evidence = list(engine.identity_evidence)
        self.self_key = engine.self_key
        self.self_evidence = engine.self_evidence
        self.self_tracking = engine.self_tracker.export()
        self.estimated_hits = list(engine.estimated_hits)
        self.estimate_evidence = list(engine.estimate_evidence)
        self.inferred_summaries = {key:value.summary() for key,value in engine.inferred_totals.items()}
        self.health_counters = dict(engine.health_inference.counters)
        self.encounter = engine.encounters.export()
        current=engine.encounters.current
        self.encounter_explicit={key:value.summary() for key,value in current.explicit.items()} if current else {}
        self.encounter_inferred={key:value.summary() for key,value in current.inferred.items()} if current else {}
        self.last_command = engine.last_command
        self.command_error = engine.command_error
        self.record_path = engine.record_path
        self.capture_summary=dict(engine.capture_summary)
        context=engine.self_tracker.latest_context
        if context is None and engine.self_tracker.appearances:context=next(reversed(engine.self_tracker.appearances))
        self.observed_players=[{'key':key,**asdict(data[0]),'seen_ns':data[1]}
            for key,data in engine.self_tracker.appearances.get(context,{}).items()]

    def name_coverage(self, target=None):
        actors = {(row['scope'], row['actor'], row['actor_generation'])
                  for row in self.summary(target)['actors']}
        return name_coverage(self.actor_names, actors,
                             (key for key in self.summaries if key is not None))

    def summary(self, target=None):
        target = resolve_target(target, (key for key in self.summaries if key is not None))
        return self.summaries.get(target, Totals().summary())

    def view_summary(self,target=None,*,encounter=True,estimates=True):
        explicit=self.encounter_explicit if encounter else self.summaries
        inferred=self.encounter_inferred if encounter else self.inferred_summaries
        keys=set(explicit)|set(inferred)
        target=resolve_target(target,(key for key in keys if key is not None))
        a=explicit.get(target,Totals().summary())
        b=inferred.get(target,Totals().summary()) if estimates else Totals().summary()
        times=[x for s in (a,b) for x in (s['start_ns'],s['end_ns']) if x is not None]
        start,end=(min(times),max(times)) if times else (None,None)
        duration=(end-start)/1e9 if times else 0
        rows={}
        for summary,inferred_row in ((a,False),(b,True)):
            for original in summary['actors']:
                key=original['scope'],original['actor'],original['actor_generation']
                row=rows.setdefault(key,dict(original,damage=0,hits=0,explicit_damage=0,estimated_damage=0,
                    explicit_hits=0,estimated_hits=0,critical_rate=None))
                row['damage']+=original['damage'];row['hits']+=original['hits']
                row['estimated_damage' if inferred_row else 'explicit_damage']+=original['damage']
                row['estimated_hits' if inferred_row else 'explicit_hits']+=original['hits']
                if not inferred_row:row['critical_rate']=original['critical_rate']
        damage=a['damage']+b['damage']
        for row in rows.values():
            row['dps']=row['damage']/duration if duration else None
            row['share']=row['damage']/damage if damage else 0
        ranked=sorted(rows.values(),key=lambda r:(-r['damage'],r['scope'],r['actor'],r['actor_generation']))
        return {'damage':damage,'hits':a['hits']+b['hits'],'dps':damage/duration if duration else None,
                'duration_seconds':duration,'start_ns':start,'end_ns':end,'actors':ranked,
                'explicit_damage':a['damage'],'estimated_damage':b['damage'],
                'explicit_hits':a['hits'],'estimated_hits':b['hits']}

    def view_chart_points(self,target=None,*,encounter=True,estimates=True):
        hits=self.hits+(self.estimated_hits if estimates else [])
        if encounter:
            start=self.encounter['started_ns']
            if start is None:return []
            end=self.encounter['ended_ns']
            hits=[h for h in hits if h.timestamp_ns>=start and (end is None or h.timestamp_ns<=end)]
        selected=sorted(select_hits(hits,target),key=lambda h:h.timestamp_ns)
        points=chart_points(selected)
        if not points:return []
        summary=self.view_summary(target,encounter=encounter,estimates=estimates)
        origin=selected[0].timestamp_ns
        offset=(origin-summary['start_ns'])/1e9
        amounts=Counter()
        for hit in selected:amounts[hit.timestamp_ns]+=hit.amount
        total=summary['damage']-sum(hit.amount for hit in selected)
        result=[]
        for elapsed,_,rolling in points:
            timestamp=origin+round(elapsed*1e9);total+=amounts[timestamp]
            full_elapsed=offset+elapsed
            if self.history_truncated and timestamp<origin+5_000_000_000:rolling=None
            result.append((full_elapsed,total/full_elapsed if full_elapsed else None,rolling))
        step=max(1,(len(result)+598)//599)
        prepared=result[::step]
        if prepared[-1]!=result[-1]:prepared.append(result[-1])
        return prepared

    def chart_points(self, target=None):
        target = resolve_target(target, (key for key in self.summaries if key is not None))
        selected = select_hits(self.hits, target)
        points = chart_points(selected)
        if not points:
            return []
        summary = self.summary(target)
        origin = selected[0].timestamp_ns
        offset = (origin - summary['start_ns']) / 1e9
        baseline = summary['damage'] - sum(hit.amount for hit in selected)
        total = baseline
        amounts = Counter()
        for hit in selected:
            amounts[hit.timestamp_ns] += hit.amount
        result = []
        for elapsed, _, rolling in points:
            timestamp = origin + round(elapsed * 1e9)
            total += amounts[timestamp]
            full_elapsed = elapsed + offset
            if self.history_truncated and timestamp < self.hits[0].timestamp_ns + 5_000_000_000:
                rolling = None
            result.append((full_elapsed, total / full_elapsed if full_elapsed else None, rolling))
        return result

    def export(self):
        return {'format': 'mabi-dps-live-audit-v3', 'partial': self.partial,
                'entity_scope_policy': 'game process session; transport scope is recorded separately on frames',
                'history_is_bounded': True, 'segments': self.segment_count,
                'frame_count': self.frame_count, 'issues': dict(self.issues),
                'issue_details': self.issue_details, 'summary': self.summary(),
                'hits': [asdict(hit) for hit in self.hits],
                'actor_names': [{'scope': scope, 'generation': generation, **asdict(identity)}
                                for (scope, _, generation), identity in self.actor_names.items()],
                'lifecycle': self.lifecycle,
                'identity_records': self.identity_records,
                'identity_evidence': self.identity_evidence,
                'self_identity': {'key': self.self_key, 'evidence': self.self_evidence,
                                  'placeholder': '<ME>', 'packet_name': False,'tracking':self.self_tracking},
                'estimated_hits':[asdict(hit) for hit in self.estimated_hits],
                'estimate_evidence':self.estimate_evidence,'health_inference':self.health_counters,
                'encounter':self.encounter,'encounter_summary':self.view_summary(),
                'record_path':self.record_path,'last_command':self.last_command,'command_error':self.command_error,
                'capture_summary':self.capture_summary,
                'observed_players':self.observed_players,
                'name_coverage': self.name_coverage(),
                'frames': self.frames}


class LiveReplay(Replay):
    def __init__(self, *, decompress=None, hit_limit=30000, frame_limit=1000,assume_aligned=False):
        super().__init__(decompress=decompress,assume_aligned=assume_aligned)
        self.hits = deque(maxlen=hit_limit)
        self.frames = deque(maxlen=frame_limit)
        self.totals = {None: Totals()}
        self.frame_count = 0
        self.epoch = 1
        self.process_identity = None
        self.inferred_totals={None:Totals()}
        self.encounters=EncounterPolicy(Totals)
        self.last_command=0
        self.command_error=None
        self.record_path=None
        self.capture_summary={}

    def add_hit(self, hit):
        key = hit.target_key
        if key not in self.totals and len(self.totals) >= 4096:
            raise ValueError('Live target limit reached; start a fresh session')
        self.totals.setdefault(key, Totals()).add(hit)
        self.totals[None].add(hit)
        self.hits.append(hit)
        self.encounters.add(hit,self_key=self.self_key)

    def add_estimate(self,event):
        hit=super().add_estimate(event)
        for key in (None,hit.target_key):self.inferred_totals.setdefault(key,Totals()).add(hit)
        self.encounters.add(hit,inferred=True,self_key=self.self_key)
        return hit

    def combat_activity(self,timestamp_ns,source,target):
        self.encounters.activity(timestamp_ns,self_involved=self.self_key in (source,target) if self.self_key else False)

    def tick(self,timestamp_ns):
        if self.encounters.idle_due(timestamp_ns,self.self_key):
            self.health_inference.flush('15-second self inactivity')
            self.encounters.end()

    def reset_measurement(self):
        # Reset partitions combat only. All packet-learned identities, names,
        # incarnations, self evidence and operator assignments remain intact.
        self.health_inference.discard('manual_reset')
        self.encounters.reset()

    def apply_command(self,command):
        sequence=command.get('id',self.last_command+1)
        result={'id':sequence,'applied':False,'reason':None}
        try:
            if type(sequence) is not int or sequence<=self.last_command:raise ValueError('Stale command sequence')
            kind=command.get('type')
            if kind=='set_self':self.set_self(command.get('key'),evidence='user_selection')
            elif kind=='reset':self.reset_measurement()
            elif kind=='idle_seconds':
                seconds=float(command.get('seconds'))
                if not 1<=seconds<=300:raise ValueError('Idle timeout must be in 1..300 seconds')
                self.encounters.idle_ns=round(seconds*1e9)
            else:raise ValueError('Unknown local UI command')
            result['applied']=True
        except (ValueError,TypeError) as exc:result['reason']=str(exc)
        if type(sequence) is int:self.last_command=max(self.last_command,sequence)
        self.command_error=result['reason']
        result['snapshot']=self.snapshot()
        return result

    def identity_scope(self, scope):
        # All selected sockets belong to the attached game process set. Packet
        # delivery, socket churn and state refreshes do not create characters.
        return f'E{self.epoch}'

    def record_frame(self, record):
        record['scope'] = f"E{self.epoch}/{record['scope']}"
        self.remember_identity_frame(record)
        size = len(record['payload_hex']) // 2
        while self.frames and (len(self.frames) == self.frames.maxlen or self.payload_bytes + size > 32 * 1024 * 1024):
            self.payload_bytes -= len(self.frames.popleft()['payload_hex']) // 2
        self.frames.append(record)
        self.payload_bytes += size
        self.frame_count += 1

    def new_transport(self, reason):
        self.report('live_attachment', reason)
        self.entities.retire_all(self.last_timestamp or 0, 'game_process_identity_changed')
        self.streams.clear()
        self.connections.clear()
        self.epoch += 1
        self.self_key = self.self_evidence = None
        from .self_tracking import SelfTracker
        self.self_tracker=SelfTracker()
        self.health_inference.discard('process_replaced')
        self.encounters.end()

    def attach_plan(self, plan):
        identity = scope_signature(plan)[0]
        changed = identity != self.process_identity
        if self.process_identity is not None and changed:
            self.new_transport('Game instance changed; new actor/transport epoch')
        elif self.process_identity is None:
            self.report('live_attachment', 'Attached midstream; counts cover observed aligned data only')
        self.process_identity = identity
        allowed = {tuple(sorted(((row['local_address'], row['local_port']),
                                 (row['remote_address'], row['remote_port']))))
                   for row in plan['candidate_flows']}
        # Ending tuples can still have approved packets in the prelude buffer.
        # Captured FIN/RST closes streams; an ownership poll is not transport EOF.
        return changed

    def snapshot(self):
        return LiveSnapshot(self)


def capture_worker(connection, seconds):
    """Privileged NIC capture with delayed, exact game-ownership export."""
    from .ipc import receive_json, send_json
    from .native import NativeCapture, adapter_identities
    from .windows import discover
    from .capture_buffer import OwnershipBuffer
    import hashlib
    capture,signature,native_signature=None,None,None
    plan=None
    adapters={}
    pending=OwnershipBuffer()
    deadline=time.monotonic()+seconds
    next_discovery=next_adapters=0
    observed=0
    dedup=deque(maxlen=4096)
    dedup_set=set()
    started=False

    def send_ready(now,force=False):
        nonlocal observed
        for segment,metadata in pending.drain(now,force=force):
            row=asdict(segment);row['payload_hex']=row.pop('payload').hex()
            send_json(connection,{'type':'segment','segment':row,'metadata':metadata})
            observed+=1

    try:
        while time.monotonic()<deadline:
            if connection.poll():
                if receive_json(connection).get('type')!='stop':raise ValueError('Unsupported capture control message')
                break
            now=time.monotonic()
            if now>=next_discovery:
                plan=discover()
                if not plan['capabilities']['administrator_token']:raise OSError('Capture worker requires an administrator token')
                if plan['processes'] and not plan['identity_verified']:raise OSError('Cannot verify game process creation identity')
                pending.approve(plan,now)
                owners,tuples=scope_signature(plan)
                rows=plan['candidate_flows']
                if rows and (not adapters or now>=next_adapters or any(r['local_address'] not in adapters for r in rows)):
                    adapters=adapter_identities();next_adapters=now+2
                nic_rows={}
                for row in rows:
                    identities=adapters.get(row['local_address'],set())
                    if len(identities)!=1:raise OSError('No unique capture NIC for a game-owned address')
                    nic_rows.setdefault(next(iter(identities)),row)
                # Brief socket-table gaps do not tear down a running NIC capture.
                selected_nics=tuple(sorted(nic_rows)) if nic_rows else (native_signature[1] if native_signature and owners else ())
                wanted_native=owners,selected_nics
                if wanted_native!=native_signature:
                    if capture:
                        send_ready(now,force=True)
                        capture.close();capture=None
                        pending.clear();pending.approve(plan,now)
                        send_json(connection,{'type':'loss','code':'capture_handover',
                            'detail':'Game process or NIC changed; native capture restarted','reset_streams':False})
                    if nic_rows and owners:
                        capture=NativeCapture(list(nic_rows.values()),adapters=adapters,broad_tcp=True)
                        started=True
                    native_signature=wanted_native
                    dedup.clear();dedup_set.clear()
                new_signature=owners,tuples
                if new_signature!=signature:
                    signature=new_signature
                    send_json(connection,{'type':'attachment','plan':plan,'active':capture is not None,
                        'ownership_refresh_seconds':0.5,'ownership_holdback_seconds':pending.holdback})
                next_discovery=now+0.5
            if capture is None:
                time.sleep(0.02)
                continue
            loss=capture.loss()
            oversized=capture.oversized() if hasattr(capture,'oversized') else 0
            contended,full=capture.queue_reports() if hasattr(capture,'queue_reports') else (0,0)
            if loss:
                send_json(connection,{'type':'loss',
                    'code':'capture_oversized_packet' if oversized==loss else 'native_capture_loss',
                    'detail':f'{loss} global capture reports; {oversized} oversized, {contended} queue contentions, {full} queue full; game continuity checked by TCP sequence',
                    'reset_streams':False})
            got_packet=False
            for _ in range(256):
                packet=capture.poll()
                if packet is None:break
                got_packet=True
                missed=packet.missed_read+packet.missed_write
                if missed:send_json(connection,{'type':'loss','code':'packet_stream_loss',
                    'detail':f'{missed} packets missed in broad native stream; game continuity checked by TCP sequence','reset_streams':False})
                raw=bytes(packet.data[:packet.length])
                key=(packet.component,packet.group,packet.packet_count,packet.timestamp,packet.direction,
                     hashlib.blake2b(raw,digest_size=16).digest())
                if key in dedup_set:continue
                if len(dedup)==dedup.maxlen:dedup_set.discard(dedup.popleft())
                dedup.append(key);dedup_set.add(key)
                link={1:1,3:101}.get(packet.kind)
                if link is None:continue
                timestamp=(packet.timestamp-FILETIME_EPOCH)*100
                if abs(timestamp-time.time_ns())>600*1_000_000_000:
                    raise OSError('Unrecognized Packet Monitor timestamp layout')
                try:segment=tcp_segment(raw,link,timestamp)
                except ValueError as exc:
                    send_json(connection,{'type':'loss','code':'packet_rejected','detail':str(exc)});continue
                if segment is None:continue
                metadata={'native_timestamp':packet.timestamp,'group':packet.group,'component':packet.component,
                          'packet_count':packet.packet_count,'appearance':packet.appearance,'direction':packet.direction}
                pending.push(segment,metadata,time.monotonic())
            send_ready(time.monotonic())
            if pending.lost:
                send_json(connection,{'type':'loss','code':'ownership_buffer_loss',
                    'detail':f'{pending.lost} buffered packets dropped; ownership prelude may be incomplete','reset_streams':False})
                pending.lost=0
            if not got_packet:time.sleep(0.005)
        send_ready(time.monotonic(),force=True)
        send_json(connection,{'type':'stopped','packets':observed,'capture_started':started,
                             'unowned_packets_discarded':pending.unowned_dropped})
    except (OSError,ValueError) as exc:
        try:send_json(connection,{'type':'error','detail':str(exc)})
        except OSError:pass
    finally:
        pending.clear()
        if capture:capture.close()


def apply_live_commands(engine,commands,on_command=None,persist=None):
    """Actual queue-to-core acknowledgement path, also used by GUI tests."""
    import queue
    replies=[]
    if commands is None:return replies
    for _ in range(32):
        try:command=commands.get_nowait()
        except queue.Empty:break
        reply=engine.apply_command(command)
        reply['persistence_error']=None
        if reply['applied'] and persist is not None:
            try:persist(engine)
            except (OSError,ValueError) as exc:reply['persistence_error']=str(exc)
        replies.append(reply)
        if on_command is not None:on_command(reply)
    return replies


def run_live(stop, on_snapshot, *, seconds=3600, record=None, audit=None, commands=None,on_command=None):
    """Unprivileged controller, also usable by the CLI without Tkinter."""
    from .ipc import PrivatePipe, launch_worker, receive_json, send_json
    from .name_cache import load_names, save_names
    import queue
    engine = LiveReplay(decompress=brotli_decoder())
    engine.record_path=str(record) if record else None
    pipe = PrivatePipe()
    connection = None
    output = None
    metadata = None
    last_snapshot = 0
    last_cache_save = 0
    status = 'Waiting for administrator capture worker'
    clean_end=False
    try:
        if record:
            output = Path(record).open('x', encoding='utf-8', buffering=1)
            metadata = Path(str(record) + '.metadata.jsonl').open('x', encoding='utf-8', buffering=1)
        worker_pid = launch_worker(pipe, seconds)
        connection = pipe.accept(worker_pid, stop)
        deadline = time.monotonic() + seconds + 10
        stop_sent = False
        while time.monotonic() < deadline:
            now = time.monotonic()
            apply_live_commands(engine,commands,on_command,save_names)
            if stop.is_set() and not stop_sent:
                send_json(connection, {'type': 'stop'})
                stop_sent = True
                deadline = min(deadline, now + 5)
            if connection.poll(0.05):
                message = receive_json(connection)
                kind = message.get('type')
                if kind == 'segment':
                    row = message['segment']
                    values = dict(row)
                    values['payload'] = bytes.fromhex(values.pop('payload_hex'))
                    if engine.last_timestamp is not None and values['timestamp_ns'] < engine.last_timestamp:
                        values['timestamp_ns'] = engine.last_timestamp
                        engine.report('timestamp_adjusted', 'Clamped backward capture time; original retained in recording')
                    if output:
                        output.write(json.dumps(dict(row, timestamp_ns=values['timestamp_ns'])) + '\n')
                        metadata.write(json.dumps(message['metadata']) + '\n')
                    engine.feed(Segment(**values))
                elif kind == 'attachment':
                    changed = engine.attach_plan(message['plan'])
                    if changed:
                        try:
                            load_names(engine)
                        except (OSError, ValueError, TypeError) as exc:
                            engine.report('identity_cache_error', str(exc))
                    if output:
                        output.write(json.dumps({'type': 'loss', 'code': 'live_attachment',
                                                 'detail': 'Capture ownership/endpoint update',
                                                 'reset_streams': changed}) + '\n')
                        metadata.write(json.dumps(message) + '\n')
                    status = 'Live: waiting for alignment' if message['active'] else 'Waiting for MabinogiMobile'
                elif kind == 'loss':
                    engine.feed(CaptureIssue(message['code'], message['detail'], message.get('reset_streams', True)))
                    if output:
                        output.write(json.dumps(message) + '\n')
                elif kind == 'error':
                    raise OSError(message['detail'])
                elif kind == 'stopped':
                    engine.capture_summary=dict(message)
                    if metadata:metadata.write(json.dumps(message)+'\n')
                    status = f"Stopped: {message['packets']} approved packets"
                    clean_end=True
                    break
                else:
                    raise ValueError('Unknown capture worker message')
            if now - last_snapshot >= 0.2:
                engine.tick(time.time_ns()-1_250_000_000)
                if engine.frame_count:
                    status = f'Live: {engine.frame_count} frames / {engine.totals[None].summary()["hits"]} hits'
                on_snapshot(engine.snapshot(), status)
                last_snapshot = now
            if now - last_cache_save >= 5:
                try:
                    save_names(engine)
                except (OSError, ValueError) as exc:
                    engine.report('identity_cache_error', str(exc))
                last_cache_save = now
    except BaseException as exc:
        status = f'Capture failed: {exc}'
        engine.report('capture_error', str(exc))
        raise
    finally:
        if connection:
            try:
                send_json(connection, {'type': 'stop'})
            except OSError:
                pass
            connection.close()
        pipe.close()
        if output:
            output.close()
        if metadata:
            metadata.close()
        if not clean_end:engine.health_inference.discard('unclean_capture_end')
        engine.finish()
        engine.encounters.end()
        try:
            save_names(engine)
        except (OSError, ValueError) as exc:
            engine.report('identity_cache_error', str(exc))
        snapshot = engine.snapshot()
        if audit:
            Path(audit).write_text(json.dumps(snapshot.export(), indent=2), encoding='utf-8')
        on_snapshot(snapshot, status)
    return snapshot
