# Mabinogi Mobile DPS meter

Latest prerelease: [v0.2.2](https://github.com/illuminascent/mabinogi-m-dps-meter/releases/tag/v0.2.2).
Version **0.2.2** adds bounded packet-level credit assignment, shared
no-friendly-fire eligibility and idle-based fallback to the HP parsing and
evidence retention introduced in v0.2.1. See the [Rust code guide](native/README.md) and
[attribution contract](docs/uniform-attribution-spec.md).

A standalone Rust DPS meter for the Japanese Windows client. It passively reads
game-owned TCP traffic through Windows Packet Monitor and displays each
encounter's top 16 damage sources, with per-target views and available names.
It does not read game memory, run the reference helpers, or use party-sharing
servers.

The Rust application is a **single EXE**. Its icon, capture library, Brotli
decoder, and lookup data are embedded. No Python, WebView, Npcap, or separate
capture DLL is required. The tested platform is Windows 11 x64 build 26100;
other Windows builds and ARM64 have not been validated.

## Use

Run `mabi-meter.exe` from a writable folder. The UI stays unprivileged and asks
Windows UAC to start a capture worker using the same EXE. Use the game normally
while the meter learns character identities. It creates a `recordings` folder
beside the EXE for raw game TCP records, audits, metadata, and window settings.

- Choose an encounter, then select or search for a target. **All targets** stays
  accessible in the sidebar.
- New battle automatically selects the current encounter and All targets.
- **Split** starts a new measurement while retaining learned identities, names,
  and self attribution. It does not stop capture or reset packet warmup.
- Encounters end after 15 seconds without self combat by default. The timeout
  is editable. Close the window to stop capture; a session is capped at one hour.
  This setting also controls how long a recent attacker remains eligible for
  estimated damage. Presence and appearance messages do not refresh activity.
- Joining an active fight pulls in retained earlier combat connected to the
  joined actor or target: target to attackers, then attackers to other targets,
  recursively. The encounter starts at that graph's earliest retained damage.
  Links use full actor incarnations and the configured idle timeout; Split and
  verified run boundaries prevent backtracking across them. Damage already
  assigned to an archived encounter stays there. Backtracking is limited by the
  retained history (65,536 explicit records and 65,536 HP allocations); eviction
  of unassigned combat marks coverage partial.

The **Estimated** column identifies damage inferred from target HP changes and
attack records. These amounts are included in displayed DPS and remain separate
from explicit damage in audits. Primary functionality has been exercised in
live play; inferred numerical accuracy still needs controlled in-game testing.
Partial attachment, missing identity records, and capture loss remain visible.
Current history is bounded at 32 encounters. Packet scene announcements are not
treated as proof of a new instance.

Player tracking is independent of the damage rows in the selected encounter.
The status line reports how many player identities are remembered for the
session, including cached players from earlier rooms. Continuing actor bindings
survive room refreshes and transport gaps; supported personal-state and profile
records can resolve actor replacements without a new appearance announcement.
Ambiguous replacements remain separate. Disk caches restore names, not current
actors or self assignment.
If entry omits personal state, a fresh exact UID can continue self after the old
TCP connection verifiably closes. Replacements do not depend on display names.
Operator assignment follows proven participant aliases and becomes unresolved
when those aliases become ambiguous. Reused TCP tuples get separate connection
lifetimes; delayed old identity records cannot replace a newer incarnation.

After a TCP hole, the decoder retains complete later buffered messages and
reacquires framing after the bounded wait. The missing bytes still mark coverage
partial. Direct packet associations respect transport continuity; finite HP
endpoints for the same actor can still supply an uncertain net decrease across
the gap. This can recover observations older builds discarded with the suffix.

Observed pets are grouped with their evidenced owner while their original actor
references and separate contributions remain in the audit. Right-click a source
row to see its pet contribution and skill breakdown, with explicit and estimated
amounts separated. Unrecognized skill IDs retain their amounts; internal action
names are used when observed, otherwise the numeric ID is shown.

HP estimation reconciles eligible explicit impact before allocating the residual.
The observed raw amount and checked raw-minus-auxiliary HP impact are retained as
different quantities. All actors use the same packet-level analysis before owner,
encounter or skill grouping. A bounded shared actor/ability/component model
weights ambiguous allocations; estimated allocations never train that model.

All attribution paths reject identical source/victim and confirmed friendly
player-side pairs. Owned entities inherit that side only from a nonconflicting,
confirmed player owner; an NPC or pet profile alone does not prove affiliation.
Contradictory raw claims remain visible in diagnostics and cannot consume HP
overlap or train the model.

When stronger packet association is unavailable, the single fallback considers
eligible recent attackers of that target within the configured idle timeout.
It does not distribute damage among visible bystanders. No eligible attacker
leaves an explicitly unattributed amount. Missing attack records do not prove HP
loss false: some town NPC attacks cause real incoming damage without those
records. Allocations do not increase observed hit or critical counts.

Audit v5 records numeric observations, actor allocations, unattributed causes,
publication receipts, ownership, skills and partial clock bounds. Pending
known-source delivery survives consumer rejection; reset drains it before
changing the measurement. Assignment uses bounded RAM with no disk spill or
historical packet index. The conservative inference-storage ceiling is below
10 MiB; independent recordings and accumulated meter results are separate.

HP evidence is acknowledged before opcode, actor and key checks. Every framed
body receives a structural classification; HP-shaped candidates retain a bounded
preview and an interpretation outcome. Complete bodies remain in raw recordings,
and audit history evictions are counted. An HP-shaped string remains an
unconfirmed candidate.

The JP common actor-status prefix supplies plain or protected HP and IV metadata.
Updates share one length-prefixed scalar envelope across the six observed types.
Uninterpreted status fields and the update trailer are retained without value
gates. Actor generations, bounds, padding and numeric checks still apply before
HP accounting. MAX sentinels do not participate in arithmetic and preserve the
last finite comparison. Numeric decreases across continuity breaks remain
eligible with uncertainty. Keys retained
after gaps, and IVs declared only by plain status, carry conditional provenance.
CBC padding does not authenticate a value. Decoding HP does not independently
verify the source attribution of every teammate amount.

Recoverable metadata errors degrade detail without abandoning later damage in
the same extracted batch. A fatal decoder failure pauses semantic processing
while raw recording continues. The status and saved metadata report that pause;
already detected pending credit remains owned for delivery.

Timeline evidence is partial. The decoder retains embedded clock candidates,
unwraps counters within a compatible TCP direction, and separates apparent clock
resets into epochs. Clock-bearing messages before and after an observation form
provisional packet-order brackets. Incomplete brackets leave timing unknown;
counter resets and bracket disagreements label uncertainty and exclude model
training rather than discard HP budgets. Death-event timestamp and effect-instance
creation-time candidates remain supporting evidence. Local association uses
100 ms of original capture time and a 250 ms input-release watermark wait. These
engineering policies are separate from the idle activity timeout and are not
verified physical server-time bounds. Full server timing reconstruction and
independent teammate accuracy remain open.

## Build

Use a current Rust toolchain, Windows C/C++ build tools, and the Windows SDK
resource compiler. With MSVC, run from an x64 Developer PowerShell where
`cargo`, the linker, and `rc.exe` are available:

```powershell
./scripts/rust-task.ps1 -Task Fetch
./scripts/build-rust.ps1
./dist/mabi-meter.exe
```

The first step fetches the dependencies pinned by `native/Cargo.lock`. Builds
and tests then run offline with that lockfile. Build caches go in `.build`, and
the release EXE goes in `dist`. Neither directory is committed.

The GNU cross-build is available with `-UseDocker -Image IMAGE` when you have
a Rust image with the `x86_64-pc-windows-gnu` target, MinGW linker and
`x86_64-w64-mingw32-windres`. Docker is an optional build tool, not a runtime
dependency. Local validation used that cross-build; MSVC needs separate
environment validation.

Useful commands:

```powershell
./dist/mabi-meter.exe --check-system
./dist/mabi-meter.exe ui --demo
./dist/mabi-meter.exe replay capture.jsonl --audit replay-audit.json
./dist/mabi-meter.exe live --seconds 30
./scripts/test-rust.ps1
cargo test --manifest-path native/capture/Cargo.toml --offline --locked
```

`--check-system` probes APIs and game process ownership without starting capture.
Replay accepts TCP JSONL or classic PCAP; PCAP requires `--port NUMBER`.
No game port is hardcoded. `--assume-aligned` is only for captures with a known
initial application boundary. Replay exit code 2 denotes partial coverage.

## Python DLL wrappers and tests

The optional research UI below and the ctypes wrapper use Python. The meter
itself remains a standalone Rust EXE. The wrapper in `src/mabi_dps/native.py`
exposes the capture DLL's ABI structures, adapter identities,
capture open/poll/close calls, and loss counters. The caller supplies approved
TCP tuples. It does not provide a Python meter, GUI, replay decoder, automatic
game discovery, or an elevated worker.

The wrappers and tests use only the Python standard library. Build the optional
DLL and run its tests with Python 3.12+:

```powershell
./scripts/build-capture.ps1
python -m unittest discover -s tests -v
```

The DLL is written to `dist/mabi_capture.dll`. Tests cover ABI layouts, tuple
conversion, capture lifetime, and error handling. On Windows with a built DLL,
they also load the real ABI and check invalid open requests without starting
capture. Meter and protocol regressions live in the Rust test suite.

## Focused research captures

`scripts/capture_epochs.py` is a small, separate Tk UI for collecting evidence
about missing teammate events and HP updates. It requires Python 3.12+ with Tk
(included in the standard Windows installer). Capture, game discovery, UAC and
ownership filtering use the existing Rust worker; no capture DLL is needed.
Build the research backend without replacing your release meter:

```powershell
./scripts/build-research.ps1 -UseDocker
pythonw ./dist/capture-epochs.pyw
```

Omit `-UseDocker` with a configured Windows Rust toolchain. You can also run
`python ./scripts/capture_epochs.py`; `--backend PATH` selects the research EXE
and `--output DIRECTORY` selects a parent folder for new sessions. The `.pyw`
and `mabi-research.exe` in `dist` can be copied together to another Windows PC
with Python and Tk installed.

1. Launch the research UI and approve UAC. Capture begins immediately when the
   game has an owned connection. Launch before entering the party/instance so
   identity and target-appearance messages are available as warm-up evidence.
2. Enter an epoch label and notes (party size, who attacks, target, observed HP,
   known damage/DPS), then click **Start epoch** just before the experiment.
3. Click **End epoch** after it. Repeat for more experiments; capture and warm-up
   continue between epochs. Notes at start and end are both retained.
4. Use **Quit and save** to drain pending traffic and finalize the session.
   Closing an active epoch marks its end. Sessions have the usual one-hour cap.

Each session is a new folder under `dist/recordings/research` by default:

| File | Contents |
| --- | --- |
| `traffic.jsonl` | Raw approved game TCP payloads, sequence/flags, endpoints, original capture timestamps and ownership epochs; both directions and unknown messages included |
| `events.jsonl` | Append-only epoch boundaries, labels/notes, ownership/process changes, capture reports and completion status |
| `session.meta.json` | Epoch index and session completion/error summary; updated on boundaries and shutdown |

All warm-up, in-epoch, and between-epoch traffic stays in the same file. Keep
the **whole session folder** for analysis. For an epoch, replay its preceding
traffic to warm transport/identities, then measure only packet timestamps in
`start_ns <= timestamp_ns < end_ns`. Bounds are controller command receipt times
in Unix nanoseconds. Ownership verification delays delivery by about 1.25s:
file row positions at a click are **not** packet-time boundaries. After End,
allow the tail to arrive or use Quit, which drains it. Epoch changes never
reset transport or identity evidence. The research recorder retains identity
messages for later replay; it does not run a live combat decoder or name cache.

This captures game-owned TCP, including unrecognized application messages;
it is not a machine-wide capture, UDP recorder, or original link-layer PCAP.
Capture loss and rejected packets are reported, and no lossless guarantee is
implied. No damage, opcode, or actor-identity filter decides what gets saved.
The raw path stays independent of combat decoding and attribution. Existing
replay accepts `traffic.jsonl`; ownership and epoch context live in the journal
and metadata, so generic replay alone does not apply those research boundaries.
The files contain private game traffic and remain ignored by Git.

## Repository layout

| Path | Purpose |
| --- | --- |
| `native/app` | Frameless Win32 UI, process discovery, authenticated worker IPC, embedded icon |
| `native/core` | TCP reassembly, bounded decoding, identity, packet attribution, encounters, totals and replay |
| `native/core/data` | Embedded JP protocol lookup map |
| `native/capture` | Our Packet Monitor library, linked directly into the Rust EXE |
| `src/mabi_dps` | Python ctypes capture DLL wrapper |
| `scripts` | Windows build/test entry points and optional Python epoch capture UI |
| `tests` | Python DLL wrapper, ABI and research UI tests |
| `assets/data` | Embedded combat lookup labels; provenance is in `assets/provenance.json` |

Private capture-based Rust tests use ignored files in `.local/test-fixtures`.
Run them with `./scripts/test-rust.ps1 -LocalFixtures` (add `-UseDocker` for
Docker). They remain ignored by default. Public synthetic tests need no
reference distribution.
Reference binaries, extracted UI artwork, recordings, research notes, prototype
artifacts and compiled outputs are excluded from this repository.
The current attribution contract and repository agent instructions are tracked;
historical research and private replay evidence remain local.
