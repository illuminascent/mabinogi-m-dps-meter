"""Connection-scoped personal/appearance evidence; cached names never vote."""
from collections import OrderedDict, deque
import struct


def personal_prefix(payload):
    # Actual JP20590: five full snapshots and their paired appearances verified.
    # Retain six bytes as code/index; do not invent the missing high UID bits.
    if not 64 <= len(payload) <= 33_554_432:
        return None
    if struct.unpack_from('<H',payload)[0] > 64 or not struct.unpack_from('<I',payload,2)[0]:
        return None
    return payload[:6]


class SelfTracker:
    def __init__(self):
        self.personal = OrderedDict()
        self.appearances = OrderedDict()
        self.latest_context = None
        self.packet_key = None
        self.packet_evidence = None
        self.operator_key = None
        self.operator_evidence = None
        self.latest_personal_order = None
        self.current_prefix = None
        self.known_uid = None
        self.events = deque(maxlen=128)

    def personal_record(self,context,prefix,timestamp_ns,frame):
        order=timestamp_ns,frame
        if self.latest_personal_order is not None and order<self.latest_personal_order:return
        if self.current_prefix is not None and self.current_prefix!=prefix:
            self.known_uid=None
            self.operator_key=self.operator_evidence=None
        self.current_prefix=prefix
        self.latest_personal_order=order
        self.personal[context]=(prefix,timestamp_ns,frame)
        self.personal.move_to_end(context)
        if len(self.personal)>4096:self.personal.popitem(last=False)
        self.latest_context=context
        self._reconcile(context)

    def appearance(self,context,key,identity,timestamp_ns,frame):
        records=self.appearances.setdefault(context,OrderedDict())
        for old in list(records):
            if old[:2]==key[:2] and old!=key:records.pop(old)
        records[key]=(identity,timestamp_ns,frame)
        records.move_to_end(key)
        if len(records)>128:records.popitem(last=False)
        self.appearances.move_to_end(context)
        if len(self.appearances)>4096:self.appearances.popitem(last=False)
        self._reconcile(context)

    def _reconcile(self,context):
        if context!=self.latest_context or context not in self.personal:return
        prefix,personal_time,personal_frame=self.personal[context]
        matches=[(key,data) for key,data in self.appearances.get(context,{}).items()
                 if data[0].uid is not None and
                 (data[0].uid==self.known_uid if self.known_uid is not None else
                  data[0].uid.to_bytes(8,'little')[:6]==prefix)]
        old=self.packet_key
        self.packet_key=matches[0][0] if len(matches)==1 else None
        if self.packet_key is None:
            self.packet_evidence='ambiguous_packet_identity' if matches else 'awaiting_matching_appearance'
        else:
            identity,when,appearance_frame=matches[0][1]
            self.known_uid=identity.uid
            self.packet_evidence='JP20590 + structured JP20054'
        if old!=self.packet_key:
            self.events.append({'old':old,'new':self.packet_key,'context':list(context),
                                'personal_frame':personal_frame,'evidence':self.packet_evidence})

    def assign(self,key,evidence):
        self.operator_key=key
        self.operator_evidence=evidence if key is not None else None

    @property
    def binding(self):
        if self.operator_key is not None:return self.operator_key,self.operator_evidence
        return self.packet_key,self.packet_evidence if self.packet_key is not None else None

    def reset_observations(self):
        # Manual scene cleanup preserves a verified self, not stale other actors.
        key=self.packet_key
        retained=self.appearances.get(self.latest_context,{}).get(key)
        self.appearances.clear()
        if retained is not None:self.appearances[self.latest_context]=OrderedDict([(key,retained)])
        if key is not None:
            self.events.append({'old':key,'new':key,'evidence':'manual reset; verified self retained'})

    def export(self):
        return {'packet_key':self.packet_key,'packet_evidence':self.packet_evidence,
                'operator_key':self.operator_key,'latest_context':self.latest_context,
                'uid':self.known_uid,'character_index':int.from_bytes(self.current_prefix[:2],'little') if self.current_prefix else None,
                'events':list(self.events)}
