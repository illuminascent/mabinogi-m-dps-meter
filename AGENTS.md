# Project instructions and handoff

Updated 2026-11-08, Asia/Tokyo.

## Current state

Source and the validated local meter are **0.2.2**; Windows file version is
0.2.2.0 and capture ABI remains 2. Release **v0.2.2** follows the GitHub prerelease
convention. The previous v0.2.1 baseline is commit
`619bf40058d2727c2036d617344c1ae7ecb5f20b`; that release remains unchanged.
The user explicitly authorized the commit, push and GitHub release for 0.2.2,
superseding the earlier publication block. Building or publishing does not
authorize live capture, terminating a running app, or future releases.

Read [native/README.md](native/README.md) for module ownership/build commands and
[docs/uniform-attribution-spec.md](docs/uniform-attribution-spec.md) before damage
changes. That contract supersedes earlier forced-roster, no-terminal-unassigned,
equal-share, eight-message and disk-backed proposals. Historical private notes
are evidence, not current implementation requirements.

## Accepted damage rules

1. Packet-level interpretation comes first. Encounter, owner and skill grouping
   are downstream. Every actor uses the same analysis; self/main-character
   selection cannot determine packet eligibility.
2. There is ONE allocation/fallback implementation. Strong packet support comes
   first, then eligible recent attackers of the same target. Mere appearance,
   presence, names or HP observations never establish attack activity.
3. Fallback activity uses the existing editable `Meter.idle_ns` split timeout.
   At timeout equality an actor is cold. A transport gap alone is not inactivity.
   No separate hot/warm multiplier or extra activity timeout is accepted.
4. The game has no friendly fire. Apply the shared eligibility predicate to all
   explicit, typed/model, fallback, reconciliation, training, retry and delivery
   paths. Reject same-actor and confirmed player-side pairs. An owned entity
   inherits Player side only through a nonconflicting confirmed Player owner.
   Generic NPC/pet profiles do not prove faction. Unknown affiliation remains
   uncertain; withdrawn ownership must revoke every retained cached side proof.
5. Fallback MAY FAIL when no eligible active attacker exists. Keep the detected
   amount explicitly unattributed with a cause. Do not manufacture activity,
   invent actor IDs, expand to bystanders or settle old loss from later presence.
6. Missing attack records do not prove HP loss false. The user confirmed the
   town rooster periodically attacks players without those records. Preserve
   true incoming HP observations even when this model cannot assign an attacker.
7. Preserve all observed fields before interpretation. Raw explicit amount and
   HP-effective impact are different quantities. Checked raw-minus-auxiliary is
   a scoped observed interpretation, not a universal named game mechanism.
   Forbidden source claims stay in diagnostics but cannot consume HP overlap or
   train. Estimates and uncertain/contested evidence never train the model.
8. Bound memory AND compute. No disk spill, historical packet index or growing
   assignment history. About 10 MB is already generous; bounded-history tail
   losses are tolerated and must be counted. Three seconds was not accepted.

## Architectural seams

- **Numeric:** canonical finite same-object HP comparison, including original
  wire target and incarnation. Initial cannot borrow a stale Health baseline.
  MAX is nonnumeric. Gaps/opaque messages/IV refresh do not erase finite endpoint
  losses; retain uncertainty. Duplicate parallel observations are nonadditive.
- **Health:** target-local packet association, orphan claims, raw/net overlap and
  clean-anchor admission. Registry changes revalidate facts before training.
- **Attribution:** sole resolver, bounded recent-attack spans, explicit failure
  causes, immutable leased credits and known-source delivery debt.
- **Replay/Session:** feed Registry-backed roles, recheck publication, commit
  then ACK. Arithmetic rejection retains the exact lease. Capacity alternatives
  still require active/eligible support. A committed receipt ACKs idempotently.
- **Meter:** atomically credit authoritative actor amounts, then maintain derived
  views. Ability refinement never republishes damage. Reset must drain both
  outbox and known-source debt or retain the previous measurement boundary.

Preserve per-origin conservation:

```text
detected obligations = published + queued + known-source owed + unattributed
detected HP loss = matched eligible explicit impact + inferred + unattributed HP
```

Distinguish no eligible activity, side conflict and delivery-capacity failure.
Conservation does not prove actor accuracy, correct packet-role semantics or
complete decoding. Do not claim that all packets/fields are understood because
a selected family or reference model agrees. The earlier ignored-field failure
is a recorded methodological integrity finding; challenge semantic references
with distributions, correlations, competing models and actual packet evidence.

## Validation and artifacts

The final 0.2.2 source passed **295 private-inclusive Rust tests**, **17 Windows
eligibility tests**, dedicated allocation pressure, all 16 retained replay inputs,
no-capture system check, demo (8,900 / six hits / 1,780 DPS), native UI stage 4 and actual
application replay. Windows version resources were independently checked.
Original capture hashes, frame counts and field census are unchanged. Town
replay retains 19,142 incoming HP loss and credits zero damage to bystanders.
Original14 captures retain 48,273 explicit hits/585389332raw damage; no eligible
source leaves 7,211,752 HP unattributed. Those overlapping inputs are regression
evidence, not independent teammate ground truth.

Conservative inference ceiling: **9,671,560 bytes**, below 10 MiB. Dedicated Windows
allocator-requested saturation peak: **4,567,950 bytes**. These are not whole-process
RSS. Measure ordinary service and mass expiry separately; neither is network
latency. Association 100 ms capture locality / 250 ms release-watermark retention are
engineering policies, separate from the idle-split activity timeout.

Validated local artifact: `dist/mabi-meter.exe`, SHA256
`c7cd8ffabf8dfe8e33c1d7568606872fc33072b865b1f751d50db9c135325dd8`.
Only that dist EXE was replaced; do not promise deleted alternate binaries.
Evidence/review: `.local/town-credit-20261108/validation.json`,
`binary-validation.json`, `implementation-report.txt`, `review.txt`,
`replay-results.json`, `town-replay.json`, `windows-pressure-result.txt`.
Earlier handoff history is archived locally at
`.local/town-credit-20261108/AGENTS-before-documentation-commit.md`.

## Repository work

Keep recordings, audits, identity caches, reference distributions, historical
research and binaries ignored. Track current source, synthetic tests, README
files, this handoff and the current attribution contract. Do not bulk force-add
ignored directories. Private protocol/identity evidence, when available, remains
in `docs/protocol-research.md` and `.local`; consult it before changing wire grammar.

Use the locked offline build/test scripts in the README. Private tests require
`-LocalFixtures`; public synthetic tests do not. Build/test/smoke must not start
live capture. A reviewer requested to be fresh must start without inherited
conversation history. Report scope and observed limits rather than a blanket
accuracy/completeness claim. Commit/push/release are separate actions; the current
authorization covers committing, pushing and publishing v0.2.2 only.
