# Mabinogi Mobile DPS meter

Latest prerelease: [v0.2.5](https://github.com/illuminascent/mabinogi-m-dps-meter/releases/tag/v0.2.5).
Version **0.2.5 / Windows 0.2.5.0 / capture ABI 2**. Hits includes inferred attacks;
damage interpretation preserves v0.2.3 behavior. See [the Rust guide](native/README.md)
and [attribution contract](docs/uniform-attribution-spec.md).
The unchanged [standalone research tool is available from v0.2.3](https://github.com/illuminascent/mabinogi-m-dps-meter/releases/tag/v0.2.3).

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

Run `mabi-meter.exe` from a writable folder and approve the Windows UAC request
for its capture worker. Play normally while the meter learns character names.
Recordings and window settings are saved in a `recordings` folder beside the EXE.
Application errors and capture diagnostics are written there as
`meter-*.log.jsonl`; the meter has no status footer.

- Choose an encounter, then select or search for a target. **All targets** shows
  the combined view. New encounters become the active view automatically.
- Use **Split** to start a new measurement while keeping learned names and
  character selection. Capture continues.
- Edit the idle timeout to control when an encounter ends. The default is
  15 seconds without your character's combat.
- Right-click a source row to view its pets and skill breakdown.
- Close the window to stop capture. Each recording session lasts at most one hour.

The **Estimated** column shows damage inferred from HP changes. It contributes
to damage totals and DPS. **Hits** shows the best combined count of explicit
and inferred attacks; **Crit** uses explicit records.
Explicit and implicit damage are reconciled without treating an explicit record
as complete coverage of its actor.
Pets are grouped with their identified owner.

The meter remembers up to 32 encounters. Joining a fight may include earlier
combat retained in the current session. Capture and decoder diagnostics are
written to the recording logs; incomplete observations can limit coverage.

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

The optional legacy Tk research UI and the ctypes wrapper use Python. The meter
and native research UI are standalone Rust EXEs. The wrapper in `src/mabi_dps/native.py`
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

`mabi-research.exe` provides native Windows controls for collecting evidence
about missing teammate events and HP updates, including comparisons between
players. It is a **single standalone EXE**: no Python, Tk, capture DLL or extra
runtime files are required. Capture, game discovery, UAC and ownership filtering
use the existing Rust worker. Build it without replacing your release meter:

```powershell
./scripts/build-research.ps1 -UseDocker
./dist/mabi-research.exe
```

Omit `-UseDocker` with a configured Windows Rust toolchain. Copy just
`dist/mabi-research.exe` to a writable folder on each player's Windows PC and
double-click it. The native UI is the research build's default even if the EXE
is renamed. `research-ui --output DIRECTORY` selects another recording parent:

```powershell
./dist/mabi-research.exe research-ui --output D:/captures
```

The optional `python ./scripts/capture_epochs.py` controls still work with
Python 3.12+ and Tk; `--backend PATH` selects the research EXE. Its JSON stdin
backend remains available as `mabi-research.exe research [--output DIRECTORY]`.
Neither is needed by other players using the standalone window.

1. Launch the research UI and approve UAC. Capture begins immediately when the
   game has an owned connection. Launch before entering the party/instance so
   identity and target-appearance messages are available as warm-up evidence.
2. Enter an epoch label and notes (party size, who attacks, target, observed HP,
   known damage/DPS), then click **Start epoch** just before the experiment.
3. Click **End epoch** after it. Repeat for more experiments; capture and warm-up
   continue between epochs. Notes at start and end are both retained.
4. Use **Quit and save** to drain pending traffic and finalize the session.
   Closing an active epoch marks its end. Sessions have the usual one-hour cap.

For cross-player comparisons, run one copy on each PC, use matching epoch labels
and record which player attacked and which observed. Keep each player's entire
session folder separate and identify the player when collecting folders. Each
PC records its own clock and command receipt times; matching labels do not
synchronize clocks or make button clicks simultaneous. Align shared packet/event
evidence during analysis before comparing timing or coverage.

`mabi-research.exe --research-ui-smoke NEW_REPORT.json` checks native Start, End,
Unicode notes and close/save using a synthetic recorder, without UAC or capture.

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
