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

The **Estimated** column identifies damage inferred from target HP changes and
attack records. These amounts are included in displayed DPS and remain separate
from explicit damage in audits. Primary functionality has been exercised in
live play; inferred numerical accuracy still needs controlled in-game testing.
Partial attachment, missing identity records, and capture loss remain visible.
Current history is bounded at 32 encounters. Packet scene announcements are not
treated as proof of a new instance.

## Build

Use a current Rust toolchain, Windows C/C++ build tools, and the Windows SDK
resource compiler. With MSVC, run from an x64 Developer PowerShell where
`cargo`, the linker, and `rc.exe` are available:

```powershell
.scripts
ust-task.ps1 -Task Fetch
.scriptsuild-rust.ps1
.distmabi-meter.exe
```

The first step fetches the dependencies pinned by `native/Cargo.lock`. Builds
and tests then run offline with that lockfile. Build caches go in `.build`, and
the release EXE goes in `dist`. Neither directory is committed.

The existing GNU cross-build is available with `-UseDocker -Image IMAGE` when
you have a Rust image with the `x86_64-pc-windows-gnu` target, MinGW linker and
`x86_64-w64-mingw32-windres`. Docker is an optional build tool, not a runtime
dependency. Local validation used that cross-build; MSVC needs separate
environment validation.

Useful commands:

```powershell
.distmabi-meter.exe --check-system
.distmabi-meter.exe ui --demo
.distmabi-meter.exe replay capture.jsonl --audit replay-audit.json
.distmabi-meter.exe live --seconds 30
.scripts	est-rust.ps1
cargo test --manifest-path native/capture/Cargo.toml --offline --locked
```

`--check-system` probes APIs and game process ownership without starting capture.
Replay accepts TCP JSONL or classic PCAP; PCAP requires `--port NUMBER`.
No game port is hardcoded. `--assume-aligned` is only for captures with a known
initial application boundary. Replay exit code 2 denotes partial coverage.

## Python support tools

The independent Python replay, capture, and comparison implementation is kept
under `src/mabi_dps`. The Rust application does not import or launch it.
Use Python 3.12+ with Tkinter for the optional comparison GUI:

```powershell
python -m venv .venv
..venvScriptspython.exe -m pip install -r requirements-live.txt
..venvScriptspython.exe run_meter.py demo
..venvScriptspython.exe run_meter.py replay capture.jsonl --audit replay-audit.json
..venvScriptspython.exe -m unittest discover -s tests -v
```

Only the optional Python live path needs our capture DLL:

```powershell
.scriptsuild-capture.ps1
..venvScriptspython.exe run_meter.py gui
```

That DLL is built into `dist/mabi_capture.dll`. Python recordings go in the
repository's ignored `recordings` directory and its name cache goes in `.local`.
The Python GUI remains a comparison tool and has different controls from the
Rust release.

## Repository layout

| Path | Purpose |
| --- | --- |
| `native/app` | Frameless Win32 UI, process discovery, authenticated worker IPC, embedded icon |
| `native/core` | TCP reassembly, bounded decoding, identity, encounters, totals, replay, synthetic tests |
| `native/capture` | Our Packet Monitor library, linked directly into the Rust EXE |
| `src/mabi_dps` | Python replay, recording and comparison tools; shared JP opcode map |
| `scripts` | Windows build and test entry points |
| `tests` | Python regressions |
| `assets/data` | Embedded combat lookup labels; provenance is in `assets/provenance.json` |

Private capture-based tests are optional and skip or remain ignored when their
local fixtures are unavailable. Public synthetic tests need no reference
distribution. Reference binaries, extracted UI artwork, recordings, research
notes, prototype artifacts and compiled outputs are excluded from this repository.
