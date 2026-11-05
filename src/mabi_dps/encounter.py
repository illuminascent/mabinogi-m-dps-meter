"""Small local encounter policy; capture and identity learning stay active."""
from collections import deque
from dataclasses import dataclass,field


@dataclass
class Encounter:
    sequence: int
    started_ns: int
    state: str='active'
    ended_ns: int | None=None
    last_self_ns: int | None=None
    last_event_ns: int | None=None
    explicit: dict=field(default_factory=dict)
    inferred: dict=field(default_factory=dict)


class EncounterPolicy:
    def __init__(self,totals_factory,*,idle_seconds=15):
        self.totals_factory=totals_factory
        self.idle_ns=int(idle_seconds*1_000_000_000)
        self.current=None
        self.history=deque(maxlen=16)
        self.sequence=0
        self.status='warming up'

    def activity(self,timestamp_ns,*,self_involved=False):
        e=self.current
        if e is not None and e.state=='active':
            e.last_event_ns=max(e.last_event_ns or timestamp_ns,timestamp_ns)
            if self_involved:e.last_self_ns=max(e.last_self_ns or timestamp_ns,timestamp_ns)

    def add(self,hit,*,inferred=False,self_key=None):
        involved=self_key is not None and self_key in (hit.actor_key,hit.target_key)
        e=self.current
        if e is None or e.state!='active':
            if self_key is not None and not involved:return
            if e is not None:self.history.append(e)
            self.sequence+=1
            self.current=e=Encounter(self.sequence,hit.timestamp_ns)
            self.status='in battle'
        # A delayed HP estimate belongs only to the encounter where its HP fell.
        if hit.timestamp_ns<e.started_ns:return
        e.started_ns=min(e.started_ns,hit.timestamp_ns)
        totals=e.inferred if inferred else e.explicit
        for key in (None,hit.target_key):totals.setdefault(key,self.totals_factory()).add(hit)
        self.activity(hit.timestamp_ns,self_involved=involved)

    def idle_due(self,now_ns,self_key=None):
        e=self.current
        if e is None or e.state!='active':return False
        last=e.last_self_ns if self_key is not None else e.last_event_ns
        # Self can be confirmed after encounter start; allow a fresh observation.
        if last is None:last=e.last_event_ns
        return last is not None and now_ns-last>=self.idle_ns

    def end(self):
        if self.current is not None and self.current.state=='active':
            e=self.current;e.state='ended';e.ended_ns=e.last_event_ns or e.last_self_ns
            self.status='ended; waiting for battle'

    def reset(self):
        if self.current is not None:
            self.end()
            self.history.append(self.current)
        self.current=None
        self.status='ready; waiting for battle'

    def export(self):
        e=self.current
        return {'status':self.status,'idle_seconds':self.idle_ns/1e9,
                'sequence':e.sequence if e else self.sequence,'started_ns':e.started_ns if e else None,
                'ended_ns':e.ended_ns if e else None,'last_self_ns':e.last_self_ns if e else None}
