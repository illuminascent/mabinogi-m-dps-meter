# Rust code guide

Current source version: **0.2.2**. The published baseline is v0.2.1; publishing
0.2.2 remains blocked. The application and embedded capture code share version
0.2.2, Windows file version is 0.2.2.0, and capture ABI remains 2.

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
| `core/src/health.rs` | Packet association, explicit/HP reconciliation and clean-anchor admission |
| `core/src/combat.rs` | Existing action/state/skill evidence, partial clocks and constrained damage model |
| `core/src/attribution.rs` | Sole allocator/fallback, recent attack history, unattributed causes and acknowledged delivery |
| `core/src/meter.rs` | Atomic actor amounts and downstream encounter, target, owner and skill views |
| `core/src/replay.rs`, `session.rs` | Production orchestration, retries, file replay and measurement boundaries |
| `app/src` | Win32 UI, capture ownership, authenticated worker IPC and raw recording |
| `capture/src` | Packet Monitor capture library and stable ABI |

All actors follow the same interpretation path. Self selection, display names
and owner grouping cannot choose packet eligibility. Legacy `Health` returned
allocations are compatibility previews; production delivery consumes only the
persistent Attribution outbox.

## Rules to preserve

- Apply the shared no-friendly-fire predicate before explicit attribution,
  inferred allocation, reconciliation, training, fallback, retry and publication.
  Registry proof overrides bounded role-cache loss. Generic NPC/owned-entity
  labels do not establish faction; ownership conflicts revoke derived side proof.
- Only actual attack or eligible finite explicit packets establish activity.
  Fallback uses target-specific history and `Meter.idle_ns`, the editable idle
  split setting. Appearance/name/HP/presence cannot refresh it. There is no
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

The current conservative inference ceiling is 9,671,560 bytes. The dedicated
Windows pressure test measured 4,567,950 allocator-requested bytes, including
temporary allocations; neither number is whole-process RSS. Run
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
./scripts/test-rust.ps1
./scripts/test-rust.ps1 -LocalFixtures
New-Item -ItemType Directory -Force .local | Out-Null
./dist/mabi-meter.exe --version
./dist/mabi-meter.exe --check-system
./dist/mabi-meter.exe demo
./dist/mabi-meter.exe --ui-smoke .local/ui-smoke.json
./dist/mabi-meter.exe replay capture.jsonl --audit .local/replay-audit.json
```

Build/test commands use the locked offline dependencies after the explicit
Fetch step documented in the root README. Add `-UseDocker` for the GNU cross
build. `-LocalFixtures` requires private files under `.local/test-fixtures`;
ordinary synthetic tests do not. Audit and UI smoke output paths must be new.
System check, demo and UI smoke do not start live capture. Replay exit code 2
means partial coverage, not necessarily a decoder failure.

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
| `equal_share_census` | Historical equal-share reason-label census; not current fallback coverage |

Review the example's usage before running it. Offline collectors are not fed
back into inference and are not live storage dependencies.

The 0.2.2 implementation passed 295 private-inclusive Rust tests, 17 Windows
eligibility tests, allocation-pressure checks and all 16 retained replay inputs
(14 original captures, world snapshot and completed town capture). Original
capture hashes, frame counts and field census were preserved. Town incoming HP
loss remains counted with zero bystander credit. These are regression results,
not independent teammate-accuracy validation. Raw recordings and detailed local
review/build evidence remain ignored.
