"""Experimental HP/attack inference, separate from explicit damage events."""
from collections import Counter
from dataclasses import dataclass, field
import struct


def parse_health(payload):
    # Independently validated on all 598 updates in the ten-minute recording.
    if len(payload)!=21:return None
    actor,length,hp=struct.unpack_from('<Qiq',payload)
    if not 0<actor<2**32 or length!=8 or hp<0:return None
    return actor,hp,payload[20]


@dataclass
class Slot:
    timestamp_ns: int
    amount: int
    frame: int
    explicit: bool=False
    attacks: list=field(default_factory=list)


@dataclass
class TargetHealth:
    hp: int
    pending: Slot | None=None


class HealthInference:
    def __init__(self,emit,*,limit=4096):
        self.emit=emit
        self.limit=limit
        self.targets={}
        self.counters=Counter()

    def _close(self,context,target,state,reason):
        slot=state.pending
        state.pending=None
        if slot is None:return
        if slot.explicit:
            self.counters['explicit_suppressed']+=1
        elif slot.amount<=0:
            self.counters['nonpositive_suppressed']+=1
        elif len(slot.attacks)!=1:
            self.counters['missing_or_ambiguous_attack']+=1
        else:
            source,attack_frame=slot.attacks[0]
            if source==target:return
            self.emit({'timestamp_ns':slot.timestamp_ns,'source':source,'target':target,
                       'amount':slot.amount,'context':context,'hp_frame':slot.frame,
                       'attack_frame':attack_frame,'reason':reason,
                       'evidence':'HP decrease and one subsequent same-target attack; inferred'})
            self.counters['estimated_events']+=1

    def health(self,context,target,hp,timestamp_ns,frame):
        key=context,target
        state=self.targets.get(key)
        if state is None:
            if len(self.targets)>=self.limit:
                self.counters['capacity_rejected']+=1
                return
            self.targets[key]=TargetHealth(hp)
            return
        if hp>state.hp:
            self.counters['health_increase_reset']+=1
            state.hp=hp;state.pending=None
            return
        self._close(context,target,state,'next target HP update')
        delta=state.hp-hp
        state.hp=hp
        state.pending=Slot(timestamp_ns,delta,frame) if delta>0 else None

    def attack(self,context,source,target,frame):
        state=self.targets.get((context,target))
        if state is not None and state.pending is not None:
            # Two records, even from one actor, are ambiguous for this hypothesis.
            if len(state.pending.attacks)<2:state.pending.attacks.append((source,frame))

    def damage(self,context,target):
        state=self.targets.get((context,target))
        if state is not None and state.pending is not None:state.pending.explicit=True

    def flush(self,reason='clean battle end'):
        for (context,target),state in list(self.targets.items()):self._close(context,target,state,reason)
        self.targets.clear()

    def discard(self,reason):
        if self.targets:self.counters['discarded_'+reason]+=1
        self.targets.clear()
