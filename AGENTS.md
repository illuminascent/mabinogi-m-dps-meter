# Project instructions and handoff

Updated 2026-11-09, Asia/Tokyo.

## Release state and authorization

The user accepted and explicitly authorized commit, push and GitHub prerelease
publication of **v0.2.4** and **v0.2.5**, plus cleanup of redundant local binaries.
This supersedes the earlier acceptance-only/publication-blocked instructions.
No future release, live capture or termination of user apps is authorized.

Current source/meter is **0.2.5 / Windows 0.2.5.0 / capture ABI 2**. Canonical local
meter: dist/mabi-meter.exe. Approved meter SHA256:
6792071692d4842ed9f4e0c59e49f35cbf7dc8825f49a2e63e10080fd3227547.
The remade v0.2.4 release meter SHA256 is:
025c4c33c4e08eef459f9d6d94a906f519cc4919e6440e404d3a03449e4b3ba3.
The existing research EXE and optional capture DLL are unchanged; research remains
available from v0.2.3. Their independent code/build path remains supported.

Published prereleases: [v0.2.4](https://github.com/illuminascent/mabinogi-m-dps-meter/releases/tag/v0.2.4)
at a3719ed65ab2eb2b2cd6dda1efce4985e69da029 and
[v0.2.5](https://github.com/illuminascent/mabinogi-m-dps-meter/releases/tag/v0.2.5)
at a7f62f1f3d9650ed740999d5a9717482c35ed577. Both meter/checksum assets were
anonymously downloaded and verified; previous releases/tags remain unchanged.
Publication authority is fulfilled; no later release is authorized.

Cleanup removed 27 superseded/portable/release-comparison EXE/DLL copies
(89,910,138 bytes). dist now contains one current meter, the unchanged research
EXE and optional capture DLL. Build/dependency caches, recordings and the user's
running desktop meter were preserved. Historical local artifact paths in old
validation reports may no longer exist; retain hashes and redownload from the
corresponding GitHub release when required. Cleanup inventory and remote metadata
are ignored under .local/publish-0.2.4-0.2.5-20261109/.

## Current behavior

The remade 0.2.4 restores released v0.2.3 damage interpretation. Explicit and
implicit are evidence categories, not main-versus-other actor categories. One
explicit record neither identifies self nor excludes that actor's other damage
from inference. UI labels omit IDs; unknown-name supported self uses <YOU>.
No status footer or reserved footer space remains. Flushed errors/diagnostics go
to recordings/meter-*.log.jsonl; status tracking is outside the meter UI.

0.2.5 changes Hits to one plain best count in rows and encounter summary. It
combines explicit records and inferred attacks without estimated/incomplete markers.
Distinct current supported observations count individually; explicit overlap counts
once. Sparse evidence uses one per distinct source/target HP obligation. Chunks and
receipt retries cannot add hits. Late explicit counterparts can replace inferred
contributions. Crit retains explicit flags and its explicit-only denominator.

hit_count.rs runs only after damage commit. Metadata follows existing target,
split/archive, owner grouping and late-join membership. It cannot influence damage
amounts/recipients, eligibility, activity, model training, ACKs or admission. Both
count caches cap at 1,024 rows. Count ceiling 204,880 bytes; combined conservative
inference ceiling 9,876,536 bytes (<10MiB), not RSS. Zero-impact notifications without
any inferred damage credit are not newly reconstructed; explicit records keep their
existing zero-impact count. These are best estimates, not a universal hit grammar.

## Validation and evidence

327 private-inclusive Rust tests and 19 native Windows tests passed for 0.2.5.
Actual native UI stage4 checks displayed 9 versus 6 explicit hits, and row 7 versus 4,
with damage unchanged. Version/resources, ABI, no-capture system check, demo,
footer geometry and flushed error logs passed. Remade 0.2.4 passed 315 Rust/68 Windows
checks; both release artifacts retain their validated hashes.

All 17 retained inputs preserve damage amounts/attributions: 8,030,723 packet records,
41,297 explicit known prefixes, 240,365 HP observations, 215,724 HP intervals. Full
stream/allocation fingerprints match remade 0.2.4. Direct released 0.2.3 application
replays also match current 0.2.5 after excluding only display/count-resource metadata
and actor-label formatting. No damage-bearing field is ignored. The earlier paired
run scores 634 positive reference records exactly after terminal HP capping, but
has no mixed-source intervals and cannot validate sharing weights/full decoding.
One old live input already has an extreme inferred total in 0.2.3; it remains
unchanged and needs separate analysis. Equivalence is not physical accuracy.

Evidence stays local/ignored: .local/hits-ui-0.2.5-20261109/, direct-0.2.3-0.2.5-20261109/,
damage-equivalence-0.2.5-20261109/, remake-0.2.4-20261109/ and
publish-0.2.4-0.2.5-20261109/. Release metadata/checksums and cleanup inventory are
in the publication folder. Redundant local EXE/DLL copies may be removed after
remote hash verification; redownload a historical release asset when needed.
The pipe-disconnected live file is compared over retained bytes, not its absent
tail. Ten historical live captures were intentionally pruned; do not claim them
tested or pending recovery. Do not recover/prune user recordings during cleanup.

## Repository and damage rules

Read native/README.md and docs/uniform-attribution-spec.md before damage changes.
Packet interpretation comes before owner, encounter and skill grouping. Use one
resolver/fallback and the shared no-friendly-fire predicate everywhere. Main
selection cannot choose packet eligibility. Owned entities inherit Player side
only through a nonconflicting confirmed Player owner; withdrawn ownership revokes
cached proof. Generic NPC/pet names do not prove faction. Mere names/presence/HP
observations cannot establish attack activity. Use editable Meter.idle_ns only,
cold at equality; a transport gap alone is not inactivity. Missing notifications
do not disprove true HP loss; retain eligible-active fallback and uncertainty.
No eligible actor may leave loss unattributed with a cause; delivery/capacity
faults stay distinct. Preserve all wire fields before interpretation, raw amount
versus HP-effective impact and per-origin/HP conservation. Uncertain/contested
estimates never train. Never claim conservation establishes correct actors.

Bound compute and memory; no disk spill/index or growing assignment history.
Association policies differ from inactivity. Commit immutable damage credit then
ACK; retries are idempotent. Ability refinement never republishes damage. Reset
drains queued credits/known-source debt or retains its old measurement boundary.
Keep recordings, caches, protocol/identity research and binaries ignored. Track
current source, synthetic tests, READMEs and this contract/handoff. Consult local
docs/protocol-research.md before wire grammar changes. Use locked offline scripts;
-LocalFixtures enables private fixtures. Build/test/smoke must not start live capture.
Do not terminate user apps. Commit, push and publication require scoped user authority.
