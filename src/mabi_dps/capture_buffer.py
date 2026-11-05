"""Privileged-only bounded prelude buffer; unrelated payloads never export."""
from collections import OrderedDict,deque


def connection_key(segment):
    return tuple(sorted(((segment.src,segment.sport),(segment.dst,segment.dport))))


class OwnershipBuffer:
    def __init__(self,*,holdback=1.25,max_bytes=64*1024*1024,max_packets=32768):
        self.holdback=holdback
        self.max_bytes=max_bytes
        self.max_packets=max_packets
        self.pending=deque()
        self.bytes=0
        self.owners=None
        self.approved={}
        self.generations=OrderedDict()
        self.syns=OrderedDict()
        self.lost=0
        self.unowned_dropped=0

    def clear(self):
        self.pending.clear();self.bytes=0;self.approved.clear();self.generations.clear();self.syns.clear()

    def approve(self,plan,now):
        owners=tuple(sorted((p['pid'],p['created_filetime']) for p in plan['processes']))
        if self.owners is not None and self.owners!=owners:self.clear()
        self.owners=owners
        for row in plan['candidate_flows']:
            key=tuple(sorted(((row['local_address'],row['local_port']),
                              (row['remote_address'],row['remote_port']))))
            self.approved[key]=(self.generations.get(key,0),now+self.holdback+1)
        for key,(_,expiry) in list(self.approved.items()):
            if now>expiry:self.approved.pop(key,None)

    def push(self,segment,metadata,now):
        key=connection_key(segment)
        if segment.syn and not segment.ack:
            syn=(segment.key,segment.seq)
            if self.syns.get(key)!=syn:
                self.syns[key]=syn
                self.generations[key]=self.generations.get(key,0)+1
                self.approved.pop(key,None)
                if len(self.generations)>65536:
                    old,_=self.generations.popitem(last=False)
                    self.syns.pop(old,None);self.approved.pop(old,None)
        generation=self.generations.get(key,0)
        cost=len(segment.payload)+256
        if cost>self.max_bytes:
            self.lost+=1;return
        while self.pending and (len(self.pending)>=self.max_packets or self.bytes+cost>self.max_bytes):
            self.bytes-=self.pending.popleft()[-1];self.lost+=1
        self.pending.append((now,key,generation,segment,metadata,cost));self.bytes+=cost

    def drain(self,now,*,force=False):
        while self.pending and (force or now-self.pending[0][0]>=self.holdback):
            arrived,key,generation,segment,metadata,cost=self.pending.popleft();self.bytes-=cost
            approval=self.approved.get(key)
            if approval is not None and approval[0]==generation and now<=approval[1]:
                yield segment,metadata
            else:self.unowned_dropped+=1
