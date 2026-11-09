# Project instructions and handoff

Updated 2026-11-09, Asia/Tokyo. This is the current handoff; older local plans and
validation journals are historical. Read [native/README.md](native/README.md) and
[the attribution contract](docs/uniform-attribution-spec.md) before damage changes.

## Published releases and authority

| Prerelease | Runtime source commit | Meter SHA256 |
| --- | --- | --- |
| [v0.2.4](https://github.com/illuminascent/mabinogi-m-dps-meter/releases/tag/v0.2.4) | `a3719ed65ab2eb2b2cd6dda1efce4985e69da029` | `025c4c33c4e08eef459f9d6d94a906f519cc4919e6440e404d3a03449e4b3ba3` |
| [v0.2.5](https://github.com/illuminascent/mabinogi-m-dps-meter/releases/tag/v0.2.5) | `a7f62f1f3d9650ed740999d5a9717482c35ed577` | `6792071692d4842ed9f4e0c59e49f35cbf7dc8825f49a2e63e10080fd3227547` |
| [v0.2.6](https://github.com/illuminascent/mabinogi-m-dps-meter/releases/tag/v0.2.6) | `5149d4e3154367b927da2359743dae4e650d3520` | `30c6dbbf87ec1466a0a5940a18edd658c4d3246701ff3a6c11a8f082e07daf14` |

The frozen numerical baseline is released v0.2.3 at
`3e6124254fd8b636dd8717c440cfe79ca7633e3e`, meter SHA256
`e4735e2559c59465bb32dc244d5cc95a2143a2b9b87b407c787df36bd9271498`.
Its local comparison copies were cleaned; use its release asset when needed.

All three prereleases contain `mabi-meter.exe` and `SHA256SUMS.txt`. Anonymous downloads
were hash-verified; older releases and tags were preserved. Subsequent docs-only
main commits do not change the runtime source identified by these tags.

The user accepted, authorized and completed commit/push/publication of these two
versions and cleanup of redundant workspace binaries. That authority is fulfilled;
it does not authorize another release, live capture or termination of user apps.

The user separately cleared commit, push and GitHub prerelease publication of the
validated 0.2.6 recording revision. That publication is completed and its anonymous
downloads are hash-verified. This authority is fulfilled; it does not authorize
live capture, stopping user apps or further releases. Older releases and tags
were preserved. Subsequent documentation commits do not change the tagged runtime.

## Current local artifacts and cleanup

- `dist/mabi-meter.exe`: published **0.2.6 / Windows 0.2.6.0 / capture ABI 2**,
  matching the v0.2.6 meter hash above. The prior published 0.2.5 EXE was preserved
  at `.local/publish-0.2.6-20261109/previous-mabi-meter-0.2.5.exe` before replacement.
- `dist/mabi-meter-0.2.6.exe`: retained versioned copy of the published revision,
  **0.2.6 / Windows 0.2.6.0 / capture ABI 2**, SHA256
  `30c6dbbf87ec1466a0a5940a18edd658c4d3246701ff3a6c11a8f082e07daf14`.
  Byte-identical to the canonical meter. The research EXE remains unchanged.
- `dist/mabi-research.exe`: unchanged published **0.2.3** standalone research tool,
  SHA256 `e8db6381ceaed5f2ae8c94d44eb450525b632af82838e45711bec026673cc983`.
  It remains downloadable from v0.2.3 and is not duplicated in the newer releases.
- `dist/mabi_capture.dll`: retained optional legacy Python-wrapper DLL, **ABI 2**,
  SHA256 `f2d9c38966a0ec2cda16eb8db776bcfb7e9c1ae7b94a619dcd868a48810365d2`.
  Neither EXE needs this DLL; it was not republished with v0.2.4/v0.2.5.

Cleanup removed **27** superseded/portable/comparison EXE/DLL copies
(**89,910,138 bytes**), including the withdrawn drafts and historical local release
asset copies. The temporary release worktree was removed. Build/dependency caches,
recordings and the user's running desktop meter were preserved. The canonical
meter was the only meter EXE after that cleanup; the versioned 0.2.6 copy is also
retained now. Do not use deleted `*-remade`, `*-acceptance`
or `*-hits` paths from old reports as current artifact locations.

Historical hashes and evidence remain valid after binary cleanup. If a comparison
needs an old EXE, download that release's asset to a new ignored location and check
its recorded hash. Do not substitute a newly built current EXE for a frozen baseline.
Building research from the current tree uses the current package version; it does
not recreate the byte-identical distributed 0.2.3 research asset.

## Current behavior and invariants

**0.2.6 / Windows 0.2.6.0 / ABI 2** recording work: user
requested the revision bump and permits breaking changes to meter
recordings; accepted research logs stay compatible. Meter capture writes a shared
compressed `context.mrec` and OS-local-timestamped encounter entries within one
session folder. Include an observed internal scenario identifier where available;
use `unknown-instance` otherwise. User chose shared context, so keep the whole
session folder. Replay resolves an entry and selects its encounter; older views
evicted by the unchanged 32-encounter bound use their last retained prefix.
Raw export stays independent of the combat parser. No automatic giant audit
sidecar; regenerate with replay. No existing recording is migrated or removed.
Research writer, wire interpretation and allocation policy remain unchanged.
This validated revision is published at the exact source commit and artifact hash
above. Validation: 335 private-inclusive Rust tests, 22 focused native
Windows tests, 24 Python tests, actual meter/research UI smoke, demo, resource
version and no-capture system check. Ten currently available corpus inputs
(nine research sessions and world snapshot) preserve all TCP fields/bytes and
full application audits against the actual published 0.2.5 EXE, excluding no
audit fields. Raw conversion shrinks 376,071,100 to 35,916,346 bytes (90.45%);
new live journal overhead is additional and depends on the session. Seven old
top-level meter inputs disappeared during this work and were not recovered or
counted as tested. The research logs remain present and hash-verified unchanged.
Evidence is `.local/recording-20261109/`. See native/README.md for the binary
format and exact limitations. Publication/tag/download/canonical-artifact evidence
is `.local/publish-0.2.6-20261109/`. No live capture or app termination was performed.

Remade 0.2.4 preserves released v0.2.3 damage interpretation. Explicit and implicit
are evidence categories, not main-versus-other actor categories. One explicit record
neither identifies self nor excludes that actor's other damage from inference.
Main selection cannot determine packet eligibility or authoritative actor credits.

0.2.4 removes actor IDs and the entire diagnostic footer/reserved space. An unnamed
supported self binding displays `<YOU>`. Errors/diagnostics are flushed to
`recordings/meter-*.log.jsonl`; status tracking stays outside the meter UI.

0.2.5 shows one plain best **Hits** number in rows and encounter summary, without
estimated/incomplete markers. It combines explicit records and inferred attacks.
Distinct current supported observations count individually, including identical
notification bodies. Explicit overlap counts once; sparse evidence uses one per
distinct source/target HP obligation. Receipt retries/delivery chunks cannot add
hits. Late explicit counterparts can replace an inferred contribution. Crit keeps
explicit flags and its explicit-only denominator.

`hit_count.rs` runs after successful damage commit. Count metadata follows existing
target/split/archive/owner/late-join membership; it cannot change damage amounts,
recipients, eligibility, activity, model training, admission or ACK behavior.
Audit JSON `hits` retains the legacy explicit-record count; `display_hits` drives
the UI. Do not mistake a zero legacy count for a missing inferred UI count.
Explicit zero-impact records retain their existing count. Zero-impact notifications
without inferred damage credit are not newly reconstructed. This is a best estimate,
not a universal physical-hit grammar.

## Validation and evidence

0.2.5 passed **327 private-inclusive Rust tests**, **19 native Windows tests**,
actual UI smoke stage 4, resources/ABI, no-capture system check, demo, footer layout
and flushed error logs. The UI-only synthetic fixture displays 9 total Hits and a
row with 7 while retaining 6 and 4 explicit records; damage remains 8,900. Ordinary
`demo` has 8,900 damage / 6 explicit hits / 1,780 DPS. Remade 0.2.4 passed 315 Rust
and 68 focused Windows checks. These are recorded release validations, not a claim
that all checks were rerun during later documentation edits.

The published release's equivalence corpus was **17 retained inputs**: nine completed research sessions,
seven top-level live files and the world snapshot. It covers **8,030,723 packet
records, 41,297 explicit known prefixes, 240,365 HP observations and 215,724 HP
intervals**. Full event/allocation streams match remade 0.2.4. Direct fresh replays
of the actual released 0.2.3 EXE also match 0.2.5. Application comparison excludes
only new display/count-resource metadata and actor-label formatting; no damage,
actor/target/skill/origin assignment, old explicit-hit/flag or unattributed field
is ignored. Equal grand totals alone are insufficient.

The earlier paired run scores 634 positive reference records exactly after terminal
HP capping, including the two NPC cases. It has no mixed-source intervals and
cannot validate sharing weights or full decoding. The recordings overlap and are
not independent accuracy replications. One live file ended with a capture-pipe error;
equivalence covers its retained bytes, not the absent tail. The ten older historical
live captures were intentionally pruned: do not recover them or claim them tested.

All evidence below stays local and ignored:

| Evidence | Location |
| --- | --- |
| Release publication, tag/asset checks, exact cleanup inventory | `.local/publish-0.2.4-0.2.5-20261109/` (`publication.json`, `final-verification.json`, `cleanup.json`) |
| Current Hits implementation, tests, UI and damage equivalence | `.local/hits-ui-0.2.5-20261109/` (`report.txt`, `summary.json`, `candidate-results.json`) |
| Direct released 0.2.3 versus current 0.2.5 comparison | `.local/direct-0.2.3-0.2.5-20261109/verification.json` |
| Frozen remade 0.2.4 numerical baseline and input manifest | `.local/damage-equivalence-0.2.5-20261109/` (`manifest.json`, `baseline-results.json`) |
| Remade 0.2.4 and paired-reference validation | `.local/remake-0.2.4-20261109/` |
| Earlier read-only reconstruction study, not a pending implementation plan | `.local/hit-count-audit-20261109/` |
| 0.2.6 recording validation before publication: version, tests, raw roundtrip and full audit comparison | `.local/recording-20261109/` (`artifact.json`, `tests.json`, `corpus.json`, `smoke.json`, `report.txt`) |
| 0.2.6 source proof, publication, preserved releases and anonymous download hashes | `.local/publish-0.2.6-20261109/` (`source-proof.json`, `commit.json`, `publication.json`, `canonical.json`) |

An extreme inferred total in an older live input already occurs in released 0.2.3
and is preserved in 0.2.5. Its cause remains a separate open investigation; see
the local damage-equivalence report. Numerical equivalence does not prove physical
accuracy. Do not silently fix damage as part of hit-display or documentation work.

## Ownership, bounds and repository work

Packet interpretation precedes owner/encounter/skill grouping. Use one resolver and
the shared no-friendly-fire predicate on all explicit, modeled, fallback, training,
retry and publication paths. Only nonconflicting confirmed Player ownership can
confer Player side; withdrawn ownership revokes cached proof. Generic NPC/pet
profiles do not establish faction. Mere names, presence or HP observations cannot
establish attack activity. Use editable `Meter.idle_ns` only, cold at equality;
transport gaps alone do not establish inactivity. Missing notifications do not
disprove true HP loss. Preserve eligible-active fallback and uncertainty.

If no eligible active actor can claim a loss, retain it unattributed with a cause.
Capacity/delivery faults stay distinct. Preserve wire fields before interpretation,
raw amount versus HP-effective impact and per-origin/HP conservation. Estimates
and contested/uncertain evidence never train. Conservation does not prove actor accuracy.

Bound compute and inference memory, without disk spill/index or growing assignment
history. Both hit-count caches cap at 1,024 rows; their conservative ceiling is
204,880 bytes, included in the combined 9,876,536-byte inference ceiling (<10 MiB).
These bounds exclude recording, transport/identity diagnostics and downstream
meter aggregates; they are not process RSS. Association policies differ from
inactivity. Commit immutable damage then ACK; retry is idempotent. Ability refinement
never republishes damage. Reset drains queued/known-source debt or retains the old
measurement boundary.

Track current source, synthetic tests, READMEs, this handoff and the attribution
contract. Keep recordings, caches, binaries and protocol/identity evidence ignored.
The older local design notes under docs are historical; the attribution contract
supersedes their forced-roster, self exclusion, unique-notification, equal-share,
mandatory-no-unassigned, eight-message, three-second and disk-backed proposals.
Consult local `docs/protocol-research.md` before wire grammar changes.
Use locked offline scripts; `-LocalFixtures` enables private fixtures. Build/test/
smoke must not start capture. Do not stop user apps or prune recordings as cleanup.
Commit/push/publication are separate actions requiring scoped user authority.
