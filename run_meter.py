"""Repository-local launcher: python run_meter.py [gui|demo|replay ...]."""
from pathlib import Path
import os
import subprocess
import sys

ROOT = Path(__file__).resolve().parent
# The local venv holds optional Brotli support. The privileged capture-only
# worker intentionally uses the base interpreter to preserve its launch PID.
local_python = ROOT / '.venv' / 'Scripts' / 'python.exe'
if (os.name == 'nt' and local_python.is_file() and '_capture-worker' not in sys.argv
        and Path(sys.prefix).resolve() != local_python.parent.parent.resolve()):
    child = subprocess.Popen([str(local_python), str(Path(__file__).resolve()), *sys.argv[1:]])
    while True:
        try:
            raise SystemExit(child.wait())
        except KeyboardInterrupt:
            # Console Ctrl+C also reaches the child, whose live command stops
            # the worker gracefully. Keep its inherited output handles open.
            continue
sys.path.insert(0, str(ROOT / 'src'))
from mabi_dps.__main__ import main

if __name__ == '__main__':
    if len(sys.argv) == 1:
        sys.argv.append('gui')
    raise SystemExit(main())
