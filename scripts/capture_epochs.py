"""Optional Tk research controls; capture and ownership checks stay in Rust.

Run with pythonw for no console. Uses only the Python standard library.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import queue
import subprocess
import threading
import time
import tkinter as tk
from tkinter import messagebox, ttk


def default_backend() -> Path:
    here = Path(__file__).resolve().parent
    local = here / "mabi-research.exe"
    return local if local.is_file() else here.parent / "dist" / "mabi-research.exe"


class Backend:
    """No packet-rate Python work. Only commands and twice-per-second status."""

    def __init__(self, executable: Path, output: Path | None = None):
        self.executable = executable.resolve()
        self.output = output.resolve() if output else None
        self.events: queue.Queue[dict] = queue.Queue(maxsize=64)
        self.process: subprocess.Popen | None = None

    def launch(self):
        args = [str(self.executable), "research"]
        if self.output:
            args += ["--output", str(self.output)]
        try:
            self.process = subprocess.Popen(
                args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT, encoding="utf-8", errors="replace",
                bufsize=1, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            )
        except OSError as error:
            self.events.put({"type": "launch_error", "error": str(error)})
            return
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        assert self.process and self.process.stdout
        try:
            while line := self.process.stdout.readline(65537):
                if len(line) > 65536:
                    raise ValueError("Backend status exceeds 64 KiB")
                try:
                    event = json.loads(line)
                    if not isinstance(event, dict):
                        raise ValueError("Expected a status object")
                except (ValueError, json.JSONDecodeError):
                    event = {"type": "message", "text": line.strip()}
                self.events.put(event)
        except (OSError, ValueError) as error:
            self.events.put({"type": "message", "text": str(error)})
        finally:
            self.events.put({"type": "exited", "code": self.process.wait()})

    def send(self, action: str, **fields):
        if not self.process or self.process.poll() is not None or not self.process.stdin:
            raise OSError("Capture backend is not running")
        command = {"action": action, **fields}
        for name in ("label", "notes"):
            if len(command.get(name, "").encode("utf-8")) > 4096:
                raise ValueError(f"{name.capitalize()} must fit in 4096 UTF-8 bytes")
        self.process.stdin.write(json.dumps(command, ensure_ascii=False) + "\n")
        self.process.stdin.flush()

    def disconnect(self):
        # EOF requests graceful finalization. Never terminate the elevated worker.
        if self.process and self.process.stdin:
            self.process.stdin.close()


class CaptureWindow:
    def __init__(self, root: tk.Tk, backend: Backend):
        self.root, self.backend = root, backend
        self.directory: Path | None = None
        self.active: dict | None = None
        self.pending = False
        self.stopping = False
        self.exited = False
        self.closing = False
        self.failed = False
        self.received_finished = False
        root.title("Mabinogi — capture epochs")
        root.minsize(650, 520)
        root.protocol("WM_DELETE_WINDOW", self.close)
        frame = ttk.Frame(root, padding=16)
        frame.pack(fill="both", expand=True)
        frame.columnconfigure(0, weight=1)
        frame.rowconfigure(10, weight=1)
        self.state = tk.StringVar(value="Starting capture backend…")
        ttk.Label(frame, textvariable=self.state, wraplength=640).grid(row=0, sticky="w")
        ttk.Label(frame, text="Warm-up traffic is retained from launch. Start / End only mark an epoch.\n"
                  "All captured game TCP is saved in both directions, including unknown messages.",
                  wraplength=640).grid(row=1, sticky="w", pady=(8, 12))
        self.stats = tk.StringVar(value="0 TCP segments · 0.00 MiB payload · 0 capture reports")
        ttk.Label(frame, textvariable=self.stats).grid(row=2, sticky="w")
        self.timer = tk.StringVar(value="No active epoch")
        ttk.Label(frame, textvariable=self.timer).grid(row=3, sticky="w", pady=(4, 12))
        ttk.Label(frame, text="Epoch label (for example: 2 players, teammate attacks only)").grid(row=4, sticky="w")
        self.label = tk.StringVar()
        ttk.Entry(frame, textvariable=self.label).grid(row=5, sticky="ew", pady=(3, 8))
        ttk.Label(frame, text="Notes / ground truth (party, target, actions, HP, measured damage or DPS)").grid(row=6, sticky="w")
        self.notes = tk.Text(frame, height=4, width=78, wrap="word")
        self.notes.grid(row=7, sticky="ew", pady=(3, 10))
        controls = ttk.Frame(frame)
        controls.grid(row=8, sticky="ew")
        self.start = ttk.Button(controls, text="Start epoch", command=lambda: self.command("start"), state="disabled")
        self.start.pack(side="left")
        self.end = ttk.Button(controls, text="End epoch", command=lambda: self.command("end"), state="disabled")
        self.end.pack(side="left", padx=8)
        self.folder = ttk.Button(controls, text="Open recordings", command=self.open_folder, state="disabled")
        self.folder.pack(side="left")
        self.quit = ttk.Button(controls, text="Quit and save", command=self.close)
        self.quit.pack(side="right")
        self.location = tk.StringVar(value="Waiting for session folder…")
        ttk.Label(frame, textvariable=self.location, wraplength=640).grid(row=9, sticky="w", pady=10)
        self.log = tk.Text(frame, height=7, width=78, wrap="word", state="disabled")
        self.log.grid(row=10, sticky="nsew")
        self.root.after(50, self.poll)
        self.root.after_idle(self.backend.launch)

    def append(self, text: str):
        self.log.configure(state="normal")
        self.log.insert("end", time.strftime("%H:%M:%S ") + text + "\n")
        if int(self.log.index("end-1c").split(".")[0]) > 300:
            self.log.delete("1.0", "50.0")
        self.log.see("end")
        self.log.configure(state="disabled")

    def buttons(self):
        enabled = not (self.pending or self.stopping or self.exited or self.failed)
        self.start.configure(state="normal" if enabled and not self.active else "disabled")
        self.end.configure(state="normal" if enabled and self.active else "disabled")

    def command(self, action: str):
        try:
            self.backend.send(action, label=self.label.get(), notes=self.notes.get("1.0", "end-1c"))
            self.pending = True
            self.buttons()
        except (OSError, ValueError) as error:
            messagebox.showerror("Capture command", str(error), parent=self.root)

    def handle(self, event: dict):
        kind = event.get("type")
        if kind == "status":
            self.state.set(event["status"])
            self.active = event.get("active_epoch")
            self.directory = Path(event["directory"])
            self.location.set(str(self.directory))
            self.folder.configure(state="normal")
            self.stats.set(f'{event["segments"]:,} TCP segments · '
                           f'{event["payload_bytes"] / 1048576:.2f} MiB payload · '
                           f'{event["capture_reports"]:,} capture reports')
        elif kind == "ack":
            self.pending = False
            self.append({"start": "Epoch started.", "end": "Epoch ended; warm-up capture continues.",
                         "stop": "Saving and draining pending packets…"}.get(event["action"], event["action"]))
        elif kind == "rejected":
            self.pending = False
            self.append("Command rejected: " + event["error"])
        elif kind in ("launch_error", "finished"):
            self.failed = bool(event.get("error"))
            self.stopping = True
            if kind == "finished":
                self.received_finished = True
                self.handle(event["summary"])
                self.active = None
            else:
                self.exited = True
            text = str(event.get("error") or "Capture finished; session files saved.")
            self.state.set(text)
            self.append(text)
            self.quit.configure(text="Close")
        elif kind == "message":
            self.append(event["text"])
        elif kind == "exited":
            self.exited = True
            self.active = None
            if event["code"] or not self.received_finished:
                self.failed = True
                text = f'Backend exited ({event["code"]}). Session may be incomplete; see files and messages.'
                self.state.set(text)
                self.append(text)
            self.backend.disconnect()
        self.buttons()

    def poll(self):
        for _ in range(64):
            try:
                self.handle(self.backend.events.get_nowait())
            except queue.Empty:
                break
        if self.active:
            elapsed = max(0, (time.time_ns() - self.active["start_ns"]) // 1_000_000_000)
            self.timer.set(f'Epoch {self.active["id"]}: {self.active["label"]}  ·  {elapsed // 60:02}:{elapsed % 60:02}')
        else:
            self.timer.set("No active epoch — warm-up continues" if not self.stopping else "Capture stopped")
        if self.closing and self.exited:
            if self.failed:
                self.closing = False  # Leave errors and the recording location visible.
                self.quit.configure(state="normal", text="Close")
            else:
                self.root.destroy()
                return
        self.root.after(100, self.poll)

    def open_folder(self):
        if self.directory:
            os.startfile(str(self.directory))

    def close(self):
        if self.exited:
            self.root.destroy()
            return
        self.closing = True
        self.stopping = True
        self.state.set("Stopping capture and saving… (finish or dismiss any pending UAC prompt)")
        self.quit.configure(state="disabled")
        self.buttons()
        try:
            self.backend.send("stop")
        except OSError:
            self.backend.disconnect()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", type=Path, default=default_backend())
    parser.add_argument("--output", type=Path, help="Parent folder for new research sessions")
    args = parser.parse_args()
    root = tk.Tk()
    if not args.backend.is_file():
        root.withdraw()
        messagebox.showerror("Research backend missing", f"Missing {args.backend}\n"
                             "Build with ./scripts/build-research.ps1 first.", parent=root)
        root.destroy()
        return 1
    CaptureWindow(root, Backend(args.backend, args.output))
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
