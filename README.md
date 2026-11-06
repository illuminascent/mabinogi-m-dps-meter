# Mabinogi Mobile DPS meter

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

Observed pets are grouped with their evidenced owner while their original actor
references and separate contributions remain in the audit. Right-click a source
row to see its pet contribution and skill breakdown, with explicit and estimated
amounts separated. Unrecognized skill IDs retain their amounts; internal action
names are used when observed, otherwise the numeric ID is shown.

HP estimation accounts for explicit contributions before allocating a residual.
After stronger source matching, unresolved non-self loss is equally shared among
distinct immediate attackers, with deterministic integer rounding. Activity is
limited to eight messages in that TCP direction and 32 ms; a separate 200 ms wait
allows delayed evidence without extending activity. Loss without eligible
candidates remains unassigned. Allocations do not increase observed hit or
critical counts. Audit v2 records the decisions, unassigned amounts, ownership,
skill evidence and partial clock bounds; clock bounds are not exact HP timestamps.

Timeline evidence is partial. The decoder retains embedded clock candidates,
unwraps counters within a compatible TCP direction, and separates apparent clock
resets into epochs. Clock-bearing messages before and after an observation form
provisional packet-order brackets. Complete brackets can reject incompatible
associations; incomplete brackets leave timing unknown. Death-event timestamp
and effect-instance creation-time candidates remain supporting evidence. HP
matching still primarily uses the capture-time and message-order window above.
The clock constraints have not yet demonstrated improved attribution, and a
verified reconstruction of server combat timing remains future work.

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
| `native/core` | TCP reassembly, bounded decoding, identity, encounters, totals, replay, tests |
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
