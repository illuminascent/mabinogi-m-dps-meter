# Project instructions and handoff

Updated 2026-11-09, Asia/Tokyo.

## Current state

Local source is now **0.2.4**, Windows file version **0.2.4.0**, ABI 2.
The user requires acceptance before any 0.2.4 release; the 0.2.3 publication
authorization does not authorize this revision. The released 0.2.3 evidence
below remains historical. Current work removes actor IDs and routine identity/
coverage footer diagnostics, uses `<YOU>` for unnamed self/eligible unowned
explicit-source placeholders while self is unresolved, and excludes every
direct explicit source (including pets) plus identified self from HP inference.
Explicit matching/training and raw fields remain retained. See the 0.2.4
addendum in the attribution contract.

Validated local acceptance EXE: `dist/mabi-meter.exe`, SHA256
`4a0c6e610d3b4109952eb3bea0c6015ee53efe4898e37a732f8765e9d85df29f`.
Evidence: `.local/revision-0.2.4/validation.json` and `report.txt`; 315 private-
inclusive Rust tests, eight focused Windows tests, nine retained research
replays plus the world snapshot, no-capture system check, demo and native UI
stage 4 passed. Frame counts, raw/credited explicit totals and HP loss match
the prior evidence. Conservative inference ceiling is 9,747,528 bytes; Windows
allocator-requested saturation peak is 4,605,598 bytes (neither is RSS).
Only the meter EXE was replaced. Research EXE remains at its released 0.2.3
hash. No push, release or live capture was performed.

The 0.2.4 acceptance follow-up removes the entire meter status footer and its
layout reservation; chart/sidebar use the normal 16-pixel bottom padding.
Meter UI updates now carry only snapshots. Errors, decoder pauses, rejected
commands, cache failures and capture reports go to flushed
`recordings/meter-*.log.jsonl` beside the EXE. Status messages are backend log
events. Diagnostic I/O failures fall back to stderr without aborting recording.
Current follow-up evidence: `.local/revision-0.2.4/footer-validation.json`,
`footer-tests.log` and `footer-build.log`. All 307 public Rust tests passed,
including Unicode/multiline append-log coverage; final EXE passed layout-aware
UI smoke, isolated application-error log creation without capture, system check,
demo and unchanged town/paired replay totals. Earlier private-inclusive and
full-corpus checks above predate this UI/logging follow-up. Release still waits
for user acceptance. The user authorized committing this work locally on main
on 2026-11-09; that authorization does not include push or publication.

### Released 0.2.3 handoff (historical)

The released source and validated meter revision were **0.2.3**; Windows file version is
0.2.3.0 and capture ABI remains 2. Release **v0.2.3** follows the GitHub prerelease
convention. The previous v0.2.1 baseline is commit
`619bf40058d2727c2036d617344c1ae7ecb5f20b`; that release remains unchanged.
The user explicitly authorized committing on main, pushing and publishing the
0.2.3 GitHub prerelease on 2026-11-08, superseding the earlier 0.2.2-only
publication authorization. Building or publishing does not
authorize live capture, terminating a running app, or future releases.

The current source/EXE includes an explicit-overkill follow-up
to released v0.2.2; the user requested the next patch revision, keeping major/minor
at 0.2, so its version is 0.2.3 / 0.2.3.0. Earlier releases remain unchanged.
Current artifact and validation evidence are recorded below.

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

The released 0.2.2 baseline passed **295 private-inclusive Rust tests**, **17 Windows
eligibility tests**, dedicated allocation pressure, all 16 retained replay inputs,
no-capture system check, demo (8,900 / six hits / 1,780 DPS), native UI stage 4 and actual
application replay. Windows version resources were independently checked.
Original capture hashes, frame counts and field census are unchanged. Town
replay retains 19,142 incoming HP loss and credits zero damage to bystanders.
Original14 captures retain 48,273 explicit hits/585389332raw damage; no eligible
source leaves 7,211,752 HP unattributed. Those overlapping inputs are regression
evidence, not independent teammate ground truth.

Current conservative inference ceiling: **9,671,656 bytes**, below 10 MiB. Dedicated Windows
allocator-requested saturation peak: **4,568,709 bytes**. These are not whole-process
RSS. Measure ordinary service and mass expiry separately; neither is network
latency. Association 100 ms capture locality / 250 ms release-watermark retention are
engineering policies, separate from the idle-split activity timeout.

Validated local artifact: `dist/mabi-meter.exe`, SHA256
`e4735e2559c59465bb32dc244d5cc95a2143a2b9b87b407c787df36bd9271498`.
Only that dist EXE was replaced; do not promise deleted alternate binaries.
Current evidence is `.local/explicit-overkill-20261108/`; the released baseline
EXE hash was `c7cd8ffabf8dfe8e33c1d7568606872fc33072b865b1f751d50db9c135325dd8`.
Released baseline evidence/review: `.local/town-credit-20261108/validation.json`,
`binary-validation.json`, `implementation-report.txt`, `review.txt`,
`replay-results.json`, `town-replay.json`, `windows-pressure-result.txt`.
Earlier handoff history is archived locally at
`.local/town-credit-20261108/AGENTS-before-documentation-commit.md`.

Local research tooling now builds a standalone native epoch UI with
`scripts/build-research.ps1` (`research-ui` Cargo feature). Copy only
`dist/mabi-research.exe`; Python/Tk and a separate capture DLL are unnecessary.
Native and legacy stdin controls share the raw recording session. Public Rust
tests passed; the final EXE passed a synthetic native UI save cycle, Unicode
notes, no-capture system check and Windows resource/import verification from
an isolated folder with Python removed from its PATH. Live capture and a second
PC were not tested. Evidence: `.local/research-standalone-20261108/validation-1794107867820384500.json`.
The 0.2.3 research release asset passed a fresh portable no-Python save cycle,
system check and version-resource verification. Evidence is in
`.local/release-0.2.3/validation.json`.
Research SHA256: `e8db6381ceaed5f2ae8c94d44eb450525b632af82838e45711bec026673cc983`.
The validated meter EXE above retains its original hash. Cross-player sessions
need matching labels and later event alignment; PC clocks are not synchronized
by the tool. Its source and standalone EXE are included in the 0.2.3 release.

Local 0.2.3 validation/evidence and domain notes are kept separately in
`.local/explicit-overkill-20261108/report.txt` and `validation.json`; paired
reference evidence is in `.local/cross-reference-20261108/`. Keep detailed
domain findings and capture-analysis notes in local documents, not READMEs.

The user confirmed on 2026-11-08 that the ten older `live-*.jsonl` inputs from
the historical original14 corpus were intentionally pruned the previous night.
Their absence is not pending recovery or validation. Use retained completed
`dist/recordings/research` sessions (currently nine), plus retained private
fixtures/world snapshot where appropriate. Historical reports do not establish
that pruned raw files remain available or were replayed after pruning.

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
authorization covers committing on main, pushing and publishing v0.2.3 only.
