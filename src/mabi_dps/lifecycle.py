"""Entity incarnations. Wire adapters must provide evidence, never guess death.

Appearance is not spawn, loss is not despawn, and a numeric ID is not identity.
Historical references remain valid after retirement and slot reuse.
"""
from collections import deque
from dataclasses import asdict, dataclass

from .identity import Identity


class LifecycleLimitError(ValueError):
    pass


@dataclass(frozen=True, order=True)
class EntityRef:
    scope: str
    actor: int
    generation: int

    @property
    def key(self):
        return self.scope, self.actor, self.generation


@dataclass
class EntityState:
    ref: EntityRef
    first_observed_ns: int
    last_seen_ns: int
    birth_reason: str
    fingerprint: tuple | None = None
    has_damage: bool = False
    retired_ns: int | None = None
    retirement_reason: str | None = None
    metadata_status: str = 'unknown'


class EntityRegistry:
    def __init__(self, report, *, limit=32768):
        self.report = report
        self.limit = limit
        self.active = {}
        self.states = {}
        self.serials = {}
        self.names = {}
        self.ambiguous = set()
        self.closed_scopes = set()
        self.events = deque(maxlen=4096)

    def _event(self, state, timestamp_ns, event, evidence):
        self.events.append({'timestamp_ns': timestamp_ns, 'event': event,
                            'entity': asdict(state.ref), 'evidence': evidence})

    def _new(self, scope, actor, timestamp_ns, reason):
        if scope in self.closed_scopes:
            raise ValueError('Cannot create an entity in a retired scope')
        if not 0 < actor < 2**32:
            raise ValueError('Entity ID must be a nonzero uint32')
        if len(self.states) >= self.limit:
            raise LifecycleLimitError('Entity incarnation limit reached; start a fresh session')
        key = scope, actor
        generation = self.serials.get(key, 0) + 1
        ref = EntityRef(scope, actor, generation)
        state = EntityState(ref, timestamp_ns, timestamp_ns, reason)
        self.serials[key] = generation
        self.states[ref] = state
        self.active[key] = ref
        self._event(state, timestamp_ns, 'observed', reason)
        return state

    def observe(self, scope, actor, timestamp_ns, *, damage=False):
        ref = self.active.get((scope, actor))
        state = (self.states[ref] if ref is not None else
                 self._new(scope, actor, timestamp_ns, 'damage_observed' if damage else 'entity_observed'))
        if timestamp_ns < state.last_seen_ns:
            raise ValueError('Entity event precedes its last observation')
        state.last_seen_ns = timestamp_ns
        state.has_damage |= damage
        return state.ref

    def _retire(self, state, timestamp_ns, reason):
        if timestamp_ns < state.last_seen_ns:
            raise ValueError('Retirement precedes entity observation')
        state.retired_ns = timestamp_ns
        state.retirement_reason = reason
        self.active.pop((state.ref.scope, state.ref.actor), None)
        self._event(state, timestamp_ns, 'retired', reason)

    def despawn(self, scope, actor, timestamp_ns, *, evidence):
        """Call only for a confirmed lifecycle event, not an appearance update.

        No JP despawn opcode is bound yet. This entry point is exercised by
        explicit fixtures until a wire adapter has been independently validated.
        """
        if not evidence:
            raise ValueError('Despawn requires evidence')
        ref = self.active.get((scope, actor))
        if ref is None:
            last_generation = self.serials.get((scope, actor))
            if last_generation is not None:
                return EntityRef(scope, actor, last_generation)
            state = self._new(scope, actor, timestamp_ns, 'despawn_without_prior_observation')
        else:
            state = self.states[ref]
        self._retire(state, timestamp_ns, 'despawn: ' + evidence)
        return state.ref

    def spawn(self, scope, actor, timestamp_ns, *, evidence, identity=None):
        if not evidence:
            raise ValueError('Spawn requires evidence')
        if identity is not None and identity.actor != actor:
            raise ValueError('Spawn identity has a different actor ID')
        old = self.active.get((scope, actor))
        if old is not None:
            self._retire(self.states[old], timestamp_ns, 'replaced_by_confirmed_spawn')
        state = self._new(scope, actor, timestamp_ns, 'spawn: ' + evidence)
        if identity is not None:
            self._bind(state, identity)
        self._event(state, timestamp_ns, 'spawn', evidence)
        return state.ref

    @staticmethod
    def _fingerprint(identity):
        return identity.kind, identity.uid, identity.template_key

    def _bind(self, state, identity):
        state.fingerprint = self._fingerprint(identity)
        state.metadata_status = 'observed'
        self.names[state.ref.key] = identity

    def appearance(self, scope, identity: Identity, timestamp_ns):
        active = self.active.get((scope, identity.actor))
        state = self.states[active] if active is not None else None
        fingerprint = self._fingerprint(identity)
        if state is not None and timestamp_ns < state.last_seen_ns:
            raise ValueError('Appearance precedes entity observation')
        reason = None
        if state is not None:
            if state.fingerprint is not None and state.fingerprint != fingerprint:
                reason = ('player_uid_changed' if state.fingerprint[0] == 'player'
                          and identity.kind == 'player' else 'identity_type_or_template_changed')
        if reason is not None:
            self._retire(state, timestamp_ns, reason)
            state = None
            self.report('identity_boundary', reason)
        if state is None:
            state = self._new(scope, identity.actor, timestamp_ns, reason or 'identity_observed')
        state.last_seen_ns = timestamp_ns
        key = state.ref.key
        if key in self.ambiguous:
            return state.ref
        previous = self.names.get(key)
        if previous is not None and previous.name != identity.name:
            # Same known UID/template, contradictory name: a rename or parser
            # ambiguity is possible. Do not invent a spawn or rewrite history.
            del self.names[key]
            self.ambiguous.add(key)
            self._event(state, timestamp_ns, 'name_conflict', 'same identity fingerprint, differing name')
            self.report('identity_conflict', f'Conflicting name for {state.ref}; using raw ID')
        else:
            self._bind(state, identity)
        return state.ref

    def refresh_scope(self, scope, timestamp_ns, reason):
        """Transport/state refresh invalidates freshness, not entity identity.

        Retain names as cached metadata until another appearance confirms or
        replaces them. Neither missing bytes nor a state marker proves death.
        """
        for ref in self.active.values():
            if ref.scope == scope and self.states[ref].fingerprint is not None:
                self.states[ref].metadata_status = 'cached'
        self.events.append({'timestamp_ns': timestamp_ns, 'event': 'metadata_refresh',
                            'scope': scope, 'evidence': reason})

    def retire_scope(self, scope, timestamp_ns, reason):
        if scope not in self.closed_scopes and len(self.closed_scopes) >= self.limit:
            raise LifecycleLimitError('Entity scope limit reached; start a fresh session')
        for ref in list(self.active.values()):
            if ref.scope == scope:
                self._retire(self.states[ref], timestamp_ns, reason)
        self.closed_scopes.add(scope)

    def unresolved_appearance(self, scope, actor, timestamp_ns, evidence):
        """A parser failure supplies no replacement identity or death evidence."""
        ref = self.observe(scope, actor, timestamp_ns)
        state = self.states[ref]
        if state.fingerprint is not None:
            state.metadata_status = 'cached'
        self._event(state, timestamp_ns, 'unresolved_appearance', evidence)
        return ref

    def retire_all(self, timestamp_ns, reason):
        for scope in {ref.scope for ref in self.active.values()}:
            self.retire_scope(scope, timestamp_ns, reason)

    def manual_reset(self,timestamp_ns,*,retain=()):
        """Explicit operator cleanup, not a claimed wire despawn or map change."""
        retain=set(retain)
        for ref in list(self.active.values()):
            if ref.key not in retain:self._retire(self.states[ref],timestamp_ns,'operator_scene_reset')

    def export(self):
        return {'incarnations': [asdict(state) for state in self.states.values()],
                'events': list(self.events), 'event_history_limit': self.events.maxlen,
                'wire_despawn_supported': False,
                'scene_identity': 'state markers refresh metadata; no decoded scene identity',
                'metadata_policy': 'late names enrich existing references; refresh retains cached names; '
                                   'confirmed replacement/despawn/process restart separates identities'}
