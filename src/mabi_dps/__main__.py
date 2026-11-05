import argparse
import json
from pathlib import Path

from .replay import demo_replay, load_replay, write_demo


def main():
    parser = argparse.ArgumentParser(description='Independent DPS prototype: replay, discovery, and native live capture')
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('gui', help='Open the native Tkinter desktop meter')
    discover = commands.add_parser('discover', help='Read Windows game-owned TCP endpoints; does not capture')
    discover.add_argument('--pid', type=int, help='Restrict to one MabinogiMobile PID')
    discover.add_argument('--output', type=Path, help='Also save the read-only capture plan as JSON')
    live = commands.add_parser('live', help='Capture the live game through a separate administrator worker')
    live.add_argument('--seconds', type=int, default=30, help='Bounded session length, default 30 seconds')
    live.add_argument('--record', type=Path, help='Create JSONL TCP fixture and companion metadata; never overwrite')
    live.add_argument('--audit', type=Path, help='Save bounded live audit and whole-session totals')
    worker = commands.add_parser('_capture-worker', help=argparse.SUPPRESS)
    worker.add_argument('--pipe', required=True)
    worker.add_argument('--key', required=True)
    worker.add_argument('--parent-pid', type=int, required=True)
    worker.add_argument('--seconds', type=int, required=True)
    demo = commands.add_parser('demo', help='Decode generated synthetic TCP traffic')
    demo.add_argument('--write', type=Path, help='Also save synthetic JSONL TCP segments')
    replay = commands.add_parser('replay', help='Read JSONL TCP segments or classic PCAP')
    replay.add_argument('path', type=Path)
    replay.add_argument('--port', type=int, help='Current game TCP port; required for PCAP')
    replay.add_argument('--assume-aligned', action='store_true',
                        help='Trust the first captured payload as an application frame boundary')
    replay.add_argument('--audit', type=Path, help='Write decoded frames, raw IDs/payloads, and diagnostics')
    args = parser.parse_args()
    if args.command == 'gui':
        from .app import main as gui_main
        gui_main()
        return 0
    try:
        if args.command == '_capture-worker':
            if not 1 <= args.seconds <= 3600:
                raise ValueError('Capture duration must be in 1..3600 seconds')
            from .ipc import worker_connection
            from .live import capture_worker
            connection = worker_connection(args.pipe, args.key, args.parent_pid)
            try:
                capture_worker(connection, args.seconds)
            finally:
                connection.close()
            return 0
        if args.command == 'live':
            if not 1 <= args.seconds <= 3600:
                raise ValueError('Capture duration must be in 1..3600 seconds')
            from .live import run_live
            import threading
            import signal
            import time
            stop = threading.Event()
            previous = signal.signal(signal.SIGINT, lambda *_: stop.set())
            last_print = [0]

            def display(snapshot, status):
                now = time.monotonic()
                if now - last_print[0] >= 2 or status.startswith('Stopped'):
                    print(status, flush=True)
                    last_print[0] = now

            try:
                result = run_live(stop, display, seconds=args.seconds, record=args.record, audit=args.audit)
            finally:
                signal.signal(signal.SIGINT, previous)
            print(json.dumps(result.export()['summary'], indent=2))
            return 2 if result.partial else 0
        if args.command == 'discover':
            from .windows import discover as discover_windows
            report = discover_windows(pid=args.pid)
            output = json.dumps(report, indent=2)
            if args.output:
                args.output.write_text(output, encoding='utf-8')
            print(output)
            return 0 if report['candidate_flows'] else 1
        if args.command == 'demo':
            result = demo_replay()
            if args.write:
                write_demo(args.write)
        else:
            result = load_replay(args.path, port=args.port, assume_aligned=args.assume_aligned)
            if args.audit:
                args.audit.write_text(json.dumps(result.export(), indent=2), encoding='utf-8')
        report = result.export()
        print(json.dumps({key: report[key] for key in
                          ('format', 'partial', 'segments', 'issues', 'summary')}, indent=2))
        return 2 if result.partial else 0
    except (OSError, ValueError, EOFError) as exc:
        parser.exit(1, f'Replay failed: {exc}\n')


if __name__ == '__main__':
    raise SystemExit(main())
