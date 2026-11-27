"""Records the raw bytes a program writes to its terminal (160x50), each write as
<time f64><length u32><bytes>, for the fake claude to play back (FAKE_REPLAY).
tests/e2e/claude-output.rec: Claude Code 2.1.332 writing a story (2026-11-26).

Usage: record.py <out> <seconds> <folder> -- command...
"""
from __future__ import annotations

import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios
import time

ROWS, COLUMNS = 50, 160


def record(out: str, seconds: float, folder: str, command: list[str]) -> None:
    pid, terminal = pty.fork()
    if pid == 0:
        os.chdir(folder)
        os.execvp(command[0], command)
    fcntl.ioctl(terminal, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLUMNS, 0, 0))
    end = time.time() + seconds
    with open(out, "wb") as log:
        while time.time() < end:
            ready, _, _ = select.select([terminal], [], [], 0.2)
            if not ready:
                continue
            try:
                data = os.read(terminal, 65536)
            except OSError:
                break
            log.write(struct.pack("<dI", time.time(), len(data)) + data)
    os.kill(pid, signal.SIGTERM)
    time.sleep(1)
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def main() -> int:
    if len(sys.argv) < 6 or sys.argv[4] != "--":
        print(__doc__, file=sys.stderr)
        return 2
    record(sys.argv[1], float(sys.argv[2]), sys.argv[3], sys.argv[5:])
    return 0


if __name__ == "__main__":
    sys.exit(main())
