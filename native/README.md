# Rust code guide

Release **0.2.5 / Windows 0.2.5.0 / capture ABI 2** preserves released v0.2.3
and remade v0.2.4 damage results across all 17 retained inputs. Hits uses bounded
post-commit count metadata. Actor-wide explicit/self exclusion is absent; UI and
logging improvements from 0.2.4 remain. Read [the handoff](../AGENTS.md) for current
validation evidence, cleanup policy and scoped publication authority.

Read the [attribution contract](../docs/uniform-attribution-spec.md) before
changing combat interpretation. The [root README](../README.md) covers use,
build prerequisites and capture research tooling.

## Data flow and ownership

```text
TCP input -> reassembly / wire observations -> Numeric -> Health -> Attribution
                                                                     |
                                                    immutable credit lease
                                                                     v
                                                       Meter commit -> ACK
```

| Module | Owns |
| --- | --- |
| `core/src/transport.rs`, `protocol.rs`, `hp_wire.rs` | Reassembly, framing and bounded wire grammar |
| `core/src/damage_wire.rs`, `hp_evidence.rs` | Field retention, acknowledgment counters and diagnostic previews |
| `core/src/numeric.rs` | Canonical same-object finite HP comparisons |
| `core/src/identity.rs`, `identity/ownership.rs` | Actor incarnations, identity evidence, ownership and shared damage-side eligibility |
| `core/src/health.rs` | Target-local packet association, explicit/HP reconciliation and clean-anchor admission |
| `core/src/combat.rs` | Existing action/state/skill evidence, partial clocks and diagnostic impact model |
| `core/src/attribution.rs` | Sole allocation/fallback, recent attack spans, unattributed causes and acknowledged delivery |
| `core/src/hit_count.rs` | Bounded post-commit display counts, explicit overlap and duplicate-delivery suppression; no damage decisions |
| `core/src/meter.rs` | Atomic actor amounts and downstream encounter, target, owner and skill views |
| `core/src/replay.rs`, `session.rs` | Production orchestration, retries, file replay and measurement boundaries |
| `app/src` | Win32 UI, capture ownership, authenticated worker IPC and raw recording |
| `app/src/diagnostics.rs` | Flushed JSONL application/error logs beside recordings; status stays outside the meter UI |
| `app/src/research_window.rs`, `research.rs` | Native research epoch controls and shared raw recorder; no combat interpretation |
| `capture/src` | Packet Monitor capture library and stable ABI |

All actors follow the same packet interpretation path. Explicit impact is matched
per event; a source with explicit records remains eligible for other inferred
damage. Names, main selection and owner grouping cannot choose packet eligibility. Legacy `Health` returned
allocations are compatibility previews; production delivery consumes only the
persistent Attribution outbox.

## Rules to preserve

- Apply the shared no-friendly-fire predicate before explicit attribution,
  inferred allocation, reconciliation, training, fallback, retry and publication.
  Registry proof overrides bounded role-cache loss. Generic NPC/owned-entity
  labels do not establish faction; ownership conflicts revoke derived side proof.
- Only actual attack or eligible finite explicit packets establish activity.
  Target-specific activity history uses `Meter.idle_ns`, the editable idle
  split setting. The remake retains eligible-active-attacker HP fallback; missing
  presentation records and ambiguous hit amounts are not extra rejection rules.
  Appearance/name/HP/presence cannot refresh it. There is no
  separate hot/warm multiplier or activity timeout.
- Preserve true finite HP changes across transport gaps with uncertainty.
  Duplicate parallel observations are nonadditive; a canonical Initial cannot
  borrow a stale association-cache baseline. MAX never enters arithmetic.
- Keep raw explicit amount separate from HP-effective impact. Conflicting source
  claims retain diagnostics but cannot subtract HP overlap or train the model.
  Estimated allocations are never training truth.
- Empty eligible support is a counted unattributed outcome. It cannot be settled
  later by a newly visible bystander. Distinguish missing activity, side conflicts
  and delivery-capacity failures.
- Lease an immutable credit and freeze its prepared skill. Commit Meter before
  ACK. An identical receipt retry is idempotent. Arithmetic rejection retains the
  lease; capacity reselection still passes the sole eligibility/fallback path.
  Reset drains both queued credits and known-source delivery debt before changing
  measurement membership. Ability refinement never republishes actor damage.

Accounting must conserve both explicit and inferred origins:

```text
detected obligations = published + queued + known-source owed + unattributed
detected HP loss = matched eligible explicit impact + inferred + unattributed HP
```

These equations establish accounting consistency, not correct physical actor
assignment or complete packet decoding.

## Bounded storage and work

Numeric retains 1,024 baselines. Health caps slots and pending budgets at 1,024,
facts at 4,096, per-target budgets/facts at 64/128, and orphan claims at 256.
Attribution caps side records at 512 and activity relationships, outbox rows and
known-source delivery rows at 1,024 each. Audit, model, clock, skill and temporary
storage have additional fixed caps. Recording and downstream meter aggregates
are independent; no disk index or spill extends the assignment working set.

Inference storage is capped below 10 MiB. The dedicated pressure test measures
allocator-requested bytes including temporary allocations; this is not process
RSS. Current measurements are recorded in the handoff. Run
`credit_bounds` and `causal_memory` when changing state layout or capacities.

For per-target pending budgets P and facts F, one association resolution costs
O(PF + F² + F log F); resolving all budgets can cost O(P²F + PF²).
Deadline expiry is indexed. Activity lookup and fallback sort bounded support.
Registry-revision validation and publication have explicit bounded work sets.
Measure mass expiry separately from ordinary per-input service and UI latency.

## Build and verification

From the repository root:

```powershell
./scripts/build-rust.ps1
# If the standard EXE is running, build a separate acceptance candidate:
./scripts/build-rust.ps1 -OutputName mabi-meter-next.exe
./scripts/test-rust.ps1
./scripts/test-rust.ps1 -LocalFixtures
New-Item -ItemType Directory -Force .local | Out-Null
./dist/mabi-meter.exe --version
./dist/mabi-meter.exe --check-system
./dist/mabi-meter.exe demo
./dist/mabi-meter.exe --ui-smoke .local/ui-smoke.json
./dist/mabi-meter.exe replay capture.jsonl --audit .local/replay-audit.json
./scripts/build-research.ps1 -UseDocker
./dist/mabi-research.exe --research-ui-smoke .local/research-ui-smoke.json
```

Build/test commands use the locked offline dependencies after the explicit
Fetch step documented in the root README. Add `-UseDocker` for the GNU cross
build. `-LocalFixtures` requires private files under `.local/test-fixtures`;
ordinary synthetic tests do not. Audit and UI smoke output paths must be new.
System check, demo and UI smoke do not start live capture. Replay exit code 2
means partial coverage, not necessarily a decoder failure.

The research build enables `research-ui` to open native epoch controls by
default. Only `dist/mabi-research.exe` is replaced; the validated meter stays
in place. This EXE can be copied alone to another Windows PC with no Python,
Tk or separate capture DLL. The native and legacy stdin controls share one raw
recording session implementation. The research UI smoke uses synthetic commands
and the actual recorder, checks Unicode labels/notes and close/save, and never
starts the elevated worker. See the root README for cross-player collection.

The optional capture DLL has its own excluded workspace/lockfile:

```powershell
cargo test --manifest-path native/capture/Cargo.toml --locked --offline
```

Useful core examples, run with `cargo run --manifest-path native/Cargo.toml
-p mabi_meter_core --locked --offline --example NAME -- ARGUMENTS`:

Direct Cargo commands require dependencies cached under their active
`CARGO_HOME`. To reuse the cache populated by the build scripts, set
`CARGO_HOME` to the absolute `.build/cargo` path and `CARGO_TARGET_DIR` to
`.build/rust` before running them.

| Example | Purpose |
| --- | --- |
| `evaluate_attribution CAPTURE` | Production replay audit |
| `evaluate_causal CAPTURE` | Streaming interval/obligation checks, bounded-storage and service measurements |
| `credit_bounds` | Conservative inference storage contract |
| `audit_damage_fields` | Complete known-prefix field diagnostics |
| `export_cross_reference CAPTURE NEW_EVENTS.jsonl\|- NEW_AUDIT.json` | Read-only packet/HP/interval export; `-` streams JSONL to stdout, summary to stderr |
| `equal_share_census` | Historical equal-share reason-label census; not current fallback coverage |

Review the example's usage before running it. Offline collectors are not fed
back into inference and are not live storage dependencies.

`scripts/audit-hit-counts.py` compares presentation-based hit-count hypotheses
against these exports without feeding any result into damage allocation. It
requires an independent baseline application audit and checks damage assignments,
skills, origins, HP accounting and unattributed causes. For a small offline run:

```powershell
./dist/mabi-meter-0.2.4-remade.exe replay capture.jsonl --audit .local/baseline.json
# Run export_cross_reference with capture.jsonl .local/events.jsonl .local/observed.json
python scripts/audit-hit-counts.py --events .local/events.jsonl --audit .local/observed.json --baseline-audit .local/baseline.json --output .local/hit-counts.json
python -m unittest discover -s scripts/tests -v
```

Use new output paths. Large runs can pipe the exporter's stdout directly into
the audit's `--events -` input; event files are optional evidence outputs, never
lookup indexes. The observer retains at most 4,096 compact combat packets and
12 examples. Final audit comparison loads the bounded diagnostic reports
separately. Packet counts and identical-payload collapse remain competing
hypotheses; unknown physical multiplicity stays unknown. This is offline research
tooling; it adds no Python dependency to the meter EXE. The production Hits UI
uses a separate bounded post-commit counter. Each evidence cache retains at most
1,024 rows; its conservative ceiling is 204,880 bytes, included in the current
9,876,536-byte inference ceiling. Display counts never change damage or crit flags.
