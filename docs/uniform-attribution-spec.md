# Uniform packet attribution contract

2026-11-09. Published release 0.2.6 / Windows 0.2.6.0 / capture ABI 2.
The user-approved v0.2.4, v0.2.5 and v0.2.6 prereleases are published and their downloads
hash-verified. That publication authority is fulfilled; it does not authorize a
future release. See AGENTS.md for exact release commits, artifacts and evidence.
Damage interpretation preserves released v0.2.3 and remade v0.2.4 behavior. Explicit
and implicit are evidence categories, not main-versus-other actor categories.
An explicit record cannot exclude its actor's other damage or identify local self.
Main selection cannot determine packet eligibility. Raw amount and HP-effective
impact remain distinct. No release/build authority permits live capture or stopping
an app. Keep detailed domain findings local and limit claims to retained evidence.

## Plain Hits display without damage changes

The user requested one best Hits number, without estimated/incomplete markers.
Rows and encounter summary combine explicit records and post-commit inferred attack
counts. Existing audit `hits` fields remain explicit-credit counts; `display_hits`
drives the UI. Crit continues to use explicit hits rather than the displayed total.
Distinct current supported observations count individually, including
identical notification bodies. Explicit overlap counts once. Sparse evidence uses
one per distinct source/target HP obligation; receipt retries and delivery chunks
cannot add hits. Late explicit evidence can replace an inferred contribution.
Metadata follows existing target/owner/encounter membership and cannot choose
recipients, amounts, eligibility, activity, training or boundaries. Crit retains
its explicit-only denominator and observed flags. Counter history is finite;
count pressure cannot reject or delay damage. Pure zero-impact notifications
without inferred credit are not newly reconstructed by this change.

This is the chosen best estimate, not proof of universal physical multiplicity.
The offline audit remains evidence about competing notification interpretations;
packet identity and allocation count do not establish physical hit identity.
All retained inputs must preserve parsed quantities, source/target/skill/origin
attribution, HP reconciliation and unattributed amounts/causes. Equal grand totals
alone are insufficient. Validate full streams and existing audits, excluding only
new count metadata/resource fields and label formatting. Estimates never train.
The read-only audit and export tools cannot feed new weights or constraints back
into the damage resolver. Current evidence and bounded storage are in AGENTS.md.

## Scope and invariants

0.2.6 replaces only the meter's unpinned recording format.
Accepted research JSONL logs remain compatible. OS-timestamped encounter entries
share lossless raw context inside a session folder; labels, scenario hints,
compression and recorded session controls cannot change damage interpretation,
eligibility, inference storage, activity or encounter membership. Recording remains
outside the inference working set. Independently exporting raw evidence must not
require trusting the combat parser. See native/README.md for format and recovery.

Packet interpretation precedes owner, encounter and ability grouping. Every actor
uses the same rules. There is one allocation/fallback implementation. Detected
finite amounts retain an accounting disposition: published, queued, known-source
delivery debt, or explicitly unattributed. The user permits attribution failure
when no eligible active attacker exists. Mere presence is never activity.

The game has no friendly fire. Identical source/victim and confirmed player-side
pairs, including player-owned pets, cannot receive damage attribution. Apply the
shared predicate to explicit claims, typed HP candidates, retained activity,
orphan claims, training, retries and meter publication. Registry is authoritative;
bounded side-cache eviction cannot override its known identities. Generic NPC
type does not prove faction or hostility; unknown affiliation remains uncertain.
An owned-entity/pet profile also does not prove faction: its nonconflicting owner
must be confirmed Player before it inherits that side. Withdrawn/conflicting
ownership revokes the cached side, including actors outside the roster sample.
An explicit contradictory source claim stays in raw diagnostics, with a side
conflict disposition; it cannot consume HP overlap or train the model.

The town rooster periodically attacks players without attack notifications.
Those HP drops are real incoming damage. A cold model rejection is a coverage
loss, not proof the numeric observation was false. No town exclusion, new HP
filter, hot/warm multiplier or extra activity timeout follows from this example.

## Ownership seams

1. Wire adapters/surveys retain original fields before enrichment. MAX sentinels
   have a nonnumeric disposition; finite amounts do not inherit magnitude or
   self/teammate preference gates from a legacy adapter.
2. Numeric owns canonical same-object HP comparison. Its key retains original
   wire target, process, binding and incarnation. Gaps, opaque messages and IV
   refresh do not erase finite endpoints; uncertainty is retained. Initial
   explicitly establishes a baseline. Duplicate parallel observations are
   nonadditive; unique lower endpoints remain eligible. Proven replacement
   retires the matching comparison. Bounded baseline eviction limits future
   comparisons, never erases an already detected amount.
3. Health owns target-local association and raw/net reconciliation. It preserves
   bounded orphan claims. Checked raw-minus-auxiliary remains a scoped impact
   candidate; unsupported arithmetic uses labeled uncertain overlap. Forbidden
   facts are removed before reconciliation/training, including when identity
   proof arrives later while the budget remains pending. Only clean independent
   anchors train the existing constrained actor/ability/component/target model.
   Estimates, conflicts and uncertain gap/batch evidence are not training truth.
   A zero endpoint with one isolated concordant eligible explicit/presentation
   pair may prove terminal excess. Subtract only `impact - observed HP loss`
   when positive, leaving the existing auxiliary interpretation intact. Missing,
   conditional, sentinel-contested, truncated, batched or role-disagreeing
   evidence cannot supply that cap. Never clip a surviving endpoint.
4. Attribution owns sole allocation/fallback, actual attack history, bounded
   known-source delivery debt, immutable leases and acknowledgments. Strong
   current packet support is used first; otherwise eligible recent attackers
   of the same target are considered. No eligible support records an unattributed
   outcome. Later appearances or unrelated future attacks cannot settle it.
5. Replay feeds exact Registry-backed pair roles, revalidates stored facts when
   identity changes, and is the only production credit consumer. It freezes
   prepared skill, commits Meter, then ACKs. Capacity alternatives must intersect
   eligible active support and admitted meter sources. Empty alternatives are
   counted delivery-capacity failures. Arithmetic rejection retains the lease.
   Already committed receipts ACK idempotently before any attempted reselection.
6. Meter rejects known forbidden pairs before mutation and atomically credits
   actor amounts. Derived detail pressure cannot partially reject actor credit.
   Explicit credit publishes with Unknown ability when needed; bounded later ability
   refinement changes metadata without republishing damage. Boundaries drain
   both outbox and known-source debt, or retain the old measurement boundary.

Conservation is detected explicit + inferred residual = published + queued +
known-source owed + unattributed. Both origins and unattributed reasons are
checked separately. Canonical HP independently reconciles to matched eligible
explicit impact + inferred + unattributed. Raw explicit amount is not asserted
to equal physical HP impact. Conservation alone does not prove actor accuracy.
Observed finite raw explicit amount = detected explicit credit obligations +
proven clipped overkill. Count clipping separately from unattributed damage;
retained wire observations and raw HP claims keep their original amounts.
Only unleased credits may be clipped. A leased or coarsened packet keeps its
amount; attempted clipping that lost packet identity is counted as unavailable.

## Activity and bounds

Activity is established by actual attack or eligible finite explicit records.
Appearance, name, HP and presence observations never refresh it. Warm history
uses only Meter.idle_ns, the existing idle-split setting and shared default.
At timeout equality the actor is cold. Eligibility is evaluated at the HP
observation's capture time, not decoder/UI wall time. A transport gap alone is
not inactivity. Entity process/binding/incarnation and target scope remain.

Compact first/last continuous activity spans preserve earlier eligible attacks
when later same-pair packets arrive before a budget resolves. Spans merge only
across gaps shorter than the configured idle timeout. Changing the setting
cannot reinterpret a span merged under a different timeout as new evidence.
Current typed packet association remains separate from this recent-activity
fallback; there are no hot/warm share multipliers.

No disk spill/index or all-history packet retrieval is used. Association policy
remains 250 ms input-release capture-watermark retention and 100 ms local capture
association. These are empirical engineering policies, not activity thresholds
or server causality axioms. Three seconds was not accepted. Upstream ownership
holdback and UI publication are separate latency factors.

With an existing target-local HP baseline, hold explicit publication until its
HP reconciliation finalizes. The bounded fallback deadline is existing capture
locality plus existing availability retention (100 + 250 ms by default), covering
an explicit packet that precedes its HP update. Keep an associated fact until
that pending budget resolves. No-baseline explicit credits need no hold. EOF and
measurement boundaries resolve HP first, then release unmatched raw credits.
Pressure coarsening releases its hold so continuous new input cannot starve
delivery; packet-specific clipping may then fail with its own diagnostic count.

Numeric retains 1,024 baselines. Health retains 1,024 slots, 1,024 pending budgets
(64 per target), 4,096 facts(128 per target), 256 orphan claims and bounded audit/
preview/scratch. Attribution retains 512 side records, 1,024 activity relationships,
1,024 outbox rows and 1,024 known-source delivery rows, 16 frame references per credit.
Existing bounded Combat/Timeline/model and 512 pending ability refinements remain.
The post-commit hit counter adds fixed limits of 1,024 seen-observation rows and
1,024 fallback keys. Its conservative ceiling is 204,880 bytes, included in the
combined 9,876,536-byte inference ceiling. Neither is a whole-process RSS bound;
recording, transport/identity diagnostics and downstream aggregates are separate.
Pressure may coarsen target/time/ability/flags/hit count, retaining source and
origin in delivery debt. A final storage-capacity failure has a separate counted
amount; it is never mislabeled cold and never assigns an arbitrary bystander.

Storage is O(S+G+B+A+bounded dependencies). Target association costs
O(PF+F²+F log F); resolving all target budgets can cost O(P²F+PF²).
Explicit finalization adds O(E(C+D)) for E consumed explicit claims and bounded
queued/owed rows C/D; at most 4,096 row visits per claim are counted separately.
Fallback scans/sorts bounded support. Activity lookup is bounded by 1,024 pairs.
Registry-revision revalidation scans bounded facts/relationships before training.
Publication processes at most 1,024 leases per invocation; boundary draining is
bounded and cannot switch membership while delivery remains pending. Report
actual service measurements and conservative allocation bounds below 10 MiB.

For cross-player validation, align shared event payloads/order before comparing
amounts. Keep finite raw explicit amounts, checked HP-impact candidates and
terminal HP clipping separate. Report unmatched and unsupported cohorts; an
identical single-source HP cell cannot validate mixed-source allocation weights.
Peer capture labels remain offline validation evidence and cannot turn runtime
estimates into training truth or justify adding unobserved overkill to HP loss.

## Validation

Test all direct/modeled/fallback/retry/pressure paths, player/pet combinations,
raw conflict without HP overlap/training, later identity and cache eviction,
idle boundary/config changes, appearance-only cold state, gaps, later-attack
overwrite, immutable receipts and full-outbox reset. Inventory and replay the available stable
research/live inputs, retained world snapshot and relevant private fixtures, keeping
hashes, frame counts and raw-field census. The 0.2.5 release corpus was nine completed
research sessions, seven live files and the world snapshot (17 inputs). One live
file ended with a pipe error; equivalence covers the retained prefix, not an absent
tail. Do not silently drop incomplete inputs or treat an active growing file as a
completed recording. Future files are outside the old release manifest. The user confirmed on 2026-11-08 that the ten
older `live-*.jsonl` inputs in the historical original14 corpus were deliberately
pruned; their absence is not a pending recovery task. Use the current retained corpus for new validation. Preserve true numeric HP denominator and
report unattributed causes.
Build Windows app, system check without capture, demo, UI smoke and app replay;
keep the canonical meter at dist/mabi-meter.exe. If it is running, use a separate
build filename without stopping the app. Validation alone does not authorize
commit, push, publication, live capture or app termination. Unknown affiliations and bounded-history accuracy remain
limitations; no assertion of universal packet completeness or teammate accuracy.
