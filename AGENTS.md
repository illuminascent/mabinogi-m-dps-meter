# Project instructions and handoff

Updated 2026-11-09, Asia/Tokyo.

## Release state

The user accepted remade **0.2.4 / Windows 0.2.4.0 / capture ABI 2** and authorized
commit, push and GitHub prerelease publication. This tag contains the remade source,
with released v0.2.3 damage behavior. Actor-wide explicit/self exclusion is removed.
UI labels omit actor IDs; unnamed supported self uses <YOU>; the status footer and
reserved space are removed. Flushed errors/diagnostics go to recordings/meter-*.log.jsonl.

Validated meter SHA256:
025c4c33c4e08eef459f9d6d94a906f519cc4919e6440e404d3a03449e4b3ba3.
315 private-inclusive Rust tests and 68 focused Windows tests passed. Retained
research/world replays conserve; 634 positive paired records agree after terminal
HP capping with no missing reference cases. System check, demo, UI geometry and
error logs passed. Inference ceiling 9,671,656 bytes; requested-allocation pressure
peak 4,568,709 bytes, not process RSS. Evidence is ignored under
.local/remake-0.2.4-20261109/ and related numerical-comparison folders.
Research capture EXE is unchanged at released 0.2.3 and available from that release.

## Required rules

Read native/README.md and docs/uniform-attribution-spec.md before damage changes.
Packet observations precede encounter, owner and skill grouping. Use one resolver
and the shared no-friendly-fire predicate on every path. Main selection cannot
choose packet eligibility. Names/presence/HP observations are not attack activity.
Use the editable Meter.idle_ns timeout, cold at equality. Missing attack notifications
do not disprove true HP reduction. Preserve eligible-active fallback and uncertainty.
No eligible active attacker may leave loss unattributed with a cause; capacity
faults are distinct. Preserve observed raw fields and per-origin/HP conservation.
Uncertain or contested estimates never train. The paired run has no mixed-source
intervals and cannot validate sharing weights or complete packet semantics.

Bound memory and compute below 10 MiB; no disk spill/index or growing assignment
history. Association policies are distinct from inactivity. Commit then ACK
immutable credits; retry is idempotent. Ability refinement never republishes damage.
Reset drains queued credits and known-source debt before membership moves.
Keep recordings, identity/protocol research and binaries ignored. Track source,
synthetic tests and current docs. Consult local docs/protocol-research.md before
wire changes. Use locked offline scripts; -LocalFixtures enables private fixtures.
No live capture or termination of user apps is authorized. The ten historical live
captures were intentionally pruned; they are not pending recovery or current tests.
The user separately authorized publishing 0.2.5, which follows this source revision.
