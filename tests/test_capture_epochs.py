"""Research controls only: no capture or elevation in these tests."""
import io
import json
from pathlib import Path
import queue
import sys
import tkinter as tk
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from capture_epochs import Backend, CaptureWindow


class FakeProcess:
    def __init__(self):
        self.stdin = io.StringIO()

    def poll(self):
        return None


class FakeBackend:
    def __init__(self):
        self.events = queue.Queue()
        self.sent = []

    def launch(self):
        pass

    def send(self, action, **fields):
        self.sent.append((action, fields))

    def disconnect(self):
        pass


def status(active=None):
    return {"type": "status", "status": "Capture running", "directory": "D:/recordings/test",
            "segments": 123, "payload_bytes": 2048, "capture_reports": 1,
            "active_epoch": active, "completed_epochs": 0}


class ResearchControlsTests(unittest.TestCase):
    def test_unicode_commands_and_disconnect_request_eof(self):
        backend = Backend(Path("unused.exe"))
        backend.process = FakeProcess()
        backend.send("start", label="二人", notes="teammate only")
        self.assertEqual(json.loads(backend.process.stdin.getvalue()),
                         {"action": "start", "label": "二人", "notes": "teammate only"})
        with self.assertRaises(ValueError):
            backend.send("end", notes="あ" * 1366)
        backend.disconnect()
        self.assertTrue(backend.process.stdin.closed)

    def test_ui_acknowledgements_rejection_and_graceful_close(self):
        root = tk.Tk()
        root.withdraw()
        try:
            backend = FakeBackend()
            app = CaptureWindow(root, backend)
            app.handle(status())
            self.assertEqual(str(app.start["state"]), "normal")
            app.label.set("8 person raid")
            app.notes.insert("1.0", "self idle; teammate attacks")
            app.command("start")
            self.assertEqual(backend.sent[0][0], "start")
            self.assertEqual(str(app.start["state"]), "disabled")
            app.handle({"type": "ack", "action": "start"})
            app.handle(status({"id": 1, "label": "8 person raid", "start_ns": 1}))
            self.assertEqual(str(app.end["state"]), "normal")
            self.assertIn("123 TCP segments", app.stats.get())
            app.command("end")
            app.handle({"type": "rejected", "error": "test rejection"})
            self.assertFalse(app.pending)
            self.assertEqual(str(app.end["state"]), "normal")
            app.close()
            self.assertEqual(backend.sent[-1][0], "stop")
            self.assertTrue(app.closing)
            self.assertEqual(str(app.start["state"]), "disabled")
            app.handle({"type": "finished", "completed": True, "error": None, "summary": status()})
            app.handle({"type": "exited", "code": 0})
            self.assertFalse(app.failed)
            self.assertIsNone(app.active)
        finally:
            root.destroy()

    def test_backend_failure_keeps_evidence_location_visible(self):
        root = tk.Tk()
        root.withdraw()
        try:
            app = CaptureWindow(root, FakeBackend())
            app.handle(status())
            app.handle({"type": "finished", "completed": False, "error": "disk full", "summary": status()})
            app.handle({"type": "exited", "code": 1})
            app.closing = True
            app.poll()
            self.assertFalse(app.closing)
            self.assertTrue(app.failed)
            self.assertTrue(root.winfo_exists())
            self.assertEqual(app.location.get(), str(Path("D:/recordings/test")))
            self.assertEqual(str(app.start["state"]), "disabled")
        finally:
            root.destroy()


if __name__ == "__main__":
    unittest.main()
