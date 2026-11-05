"""Reuse observed names only within the same verified game process session."""
from dataclasses import asdict
import json
from pathlib import Path
import tempfile

from .identity import Identity

DEFAULT_PATH = Path(__file__).resolve().parents[2] / '.local/live-identity-cache.json'
MAX_BYTES = 16 * 1024 * 1024


def owner_key(engine):
    owners = engine.process_identity
    if not owners or any(not pid or not created for pid, created in owners):
        return None
    return [list(owner) for owner in owners]


def load_names(engine, path=DEFAULT_PATH):
    owners = owner_key(engine)
    path = Path(path)
    if owners is None or not path.exists():
        return 0
    if path.stat().st_size > MAX_BYTES:
        raise ValueError('Identity cache exceeds size limit')
    data = json.loads(path.read_text(encoding='utf-8'))
    if not isinstance(data, dict):
        raise ValueError('Invalid identity cache document')
    if data.get('format') != 'mabi-dps-name-cache-v1' or data.get('owners') != owners:
        return 0
    rows = data.get('identities', [])
    if not isinstance(rows, list) or len(rows) > engine.entities.limit:
        raise ValueError('Invalid identity cache entries')
    identities = [Identity(**row) for row in rows]
    binding = data.get('self_binding')
    if binding is not None and (not isinstance(binding, dict)
            or type(binding.get('actor')) is not int or not 0 < binding['actor'] < 2**32
            or not isinstance(binding.get('evidence'), str) or not binding['evidence']):
        raise ValueError('Invalid cached self assignment')
    if len({identity.actor for identity in identities}) != len(identities):
        raise ValueError('Duplicate identity cache actor')
    for identity in identities:
        if (type(identity.actor) is not int or not 0 < identity.actor < 2**32
                or not isinstance(identity.name, str) or not 0 < len(identity.name) <= 256
                or identity.kind not in ('player', 'npc_lookup')
                or (identity.kind == 'player' and (type(identity.uid) is not int or identity.uid <= 0))
                or (identity.kind == 'npc_lookup' and (not isinstance(identity.template_key, str)
                    or len(identity.template_key) != 8))):
            raise ValueError('Invalid cached identity')
    scope = engine.identity_scope('')
    loaded = 0
    for identity in identities:
        if (scope, identity.actor) in engine.entities.active:
            continue
        ref = engine.add_identity(scope, identity)
        engine.entities.states[ref].metadata_status = 'cached'
        loaded += 1
    if binding is not None and binding['evidence'].removeprefix('cached assignment: ')=='user_selection':
        ref = engine.entities.observe(scope, binding['actor'], engine.last_timestamp or 0)
        engine.set_self(ref.key, evidence='cached assignment: ' + binding['evidence'].removeprefix('cached assignment: '))
    if loaded:
        engine.report('identity_cache', f'Loaded {loaded} cached names for the same verified game process')
    return loaded


def save_names(engine, path=DEFAULT_PATH):
    owners = owner_key(engine)
    if owners is None:
        return
    scope = engine.identity_scope('')
    identities = [asdict(engine.actor_names[ref.key]) for ref in engine.entities.active.values()
                  if ref.scope == scope and ref.key in engine.actor_names]
    binding = None
    if engine.self_key is not None and engine.self_tracker.operator_key is not None:
        ref = engine.entities.active.get(engine.self_key[:2])
        if ref is not None and ref.scope == scope and ref.key == engine.self_key:
            binding = {'actor': ref.actor, 'evidence': engine.self_evidence}
    data = json.dumps({'format': 'mabi-dps-name-cache-v1', 'owners': owners,
                       'saved_capture_time_ns': engine.last_timestamp,
                       'identities': identities, 'self_binding': binding}, ensure_ascii=False).encode('utf-8')
    if len(data) > MAX_BYTES:
        raise ValueError('Identity cache exceeds size limit')
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix=path.name + '.', delete=False) as output:
            temporary = Path(output.name)
            output.write(data)
        temporary.replace(path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
