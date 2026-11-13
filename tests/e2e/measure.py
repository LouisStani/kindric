"""What kindric-desktop costs, measured the same way before and after a change, in the image
(E2E_SCRIPT=measure.py E2E_NO_TRACE=1 tests/e2e/inside.sh, or tests/e2e/run.sh --measure).

Three sessions of the fake claude (one working, two waiting), two of them open in panes, then
the app left alone. Printed, one per line as `name value unit`:
- start: from launching the app to the three sessions drawn in the bar;
- cpu: processor time of the app's processes (kindric-desktop, WebKit's) while left alone, in % of
  one core; with no GPU here, drawing (WebGL included) is done by the processor and counts;
- rss, pss: their memory at the end (pss shares the libraries between them);
- claude calls: how many times the app ran claude while left alone (the real one costs about
  0.55 s of processor per `agents --json`);
- binaries: the size of the three programs.
"""
from __future__ import annotations

import os
import shutil
import sys
import time

from driver import CONTROL, SHIFT, Driver, wait_until
from journeys import ACCOUNT, CHIPS, HOME, accounts, application, forget_everything, open_from_bar, stop_every_session, PROJECT

IDLE = float(os.environ.get("MEASURE_IDLE", "60"))
# Sessions opened in panes before the app is left alone: 0, 1 or 2.
PANES = int(os.environ.get("MEASURE_PANES", "2"))
# Written by the fake claude (FAKE_CLAUDE_CALLS, set by inside.sh).
CALLS = os.environ.get("FAKE_CLAUDE_CALLS", os.path.join(HOME, "claude-calls.log"))
# /proc/<pid>/comm keeps 15 characters.
NAMES = ("kindric-desktop", "WebKitWebProces", "WebKitNetworkPr", "WebKitGPUProces")
TICKS = os.sysconf("SC_CLK_TCK")


def app_processes() -> list[int]:
    found = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/comm") as comm:
                if comm.read().strip() in NAMES:
                    found.append(int(entry))
        except OSError:
            continue
    return found


def cpu_seconds(pids: list[int]) -> float:
    total = 0
    for pid in pids:
        try:
            with open(f"/proc/{pid}/stat") as stat:
                fields = stat.read().rsplit(")", 1)[1].split()
            total += int(fields[11]) + int(fields[12])
        except OSError:
            continue
    return total / TICKS


def memory_kib(pids: list[int], field: str, source: str) -> int:
    total = 0
    for pid in pids:
        try:
            with open(f"/proc/{pid}/{source}") as lines:
                for line in lines:
                    if line.startswith(field + ":"):
                        total += int(line.split()[1])
        except OSError:
            continue
    return total


def calls_since(start: float) -> int:
    try:
        with open(CALLS) as log:
            return sum(1 for line in log if float(line.split(" ", 1)[0]) >= start)
    except OSError:
        return 0


def report(name: str, value: float, unit: str) -> None:
    print(f"{name} {value:.1f} {unit}", flush=True)


def main() -> int:
    forget_everything()
    os.makedirs(PROJECT, exist_ok=True)
    accounts("add", ACCOUNT)
    accounts("run", ACCOUNT, "--bg", "-n", "busy", "measure work:3600")
    accounts("run", ACCOUNT, "--bg", "-n", "first", "measure work:0")
    accounts("run", ACCOUNT, "--bg", "-n", "second", "measure work:0")
    started = time.monotonic()
    driver = Driver.start(application())
    try:
        wait_until("three sessions in the bar", lambda: len(driver.script(CHIPS)) == 3, timeout=60)
        report("start", (time.monotonic() - started) * 1000, "ms")
        if PANES >= 1:
            open_from_bar(driver, "first")
        if PANES >= 2:
            driver.press(CONTROL, SHIFT, "d")  # split: the second session goes in the new pane
            open_from_bar(driver, "second")
        time.sleep(5)
        pids = app_processes()
        before, since = cpu_seconds(pids), time.time()
        time.sleep(IDLE)
        report("cpu", (cpu_seconds(pids) - before) / IDLE * 100, "%")
        report("claude calls", calls_since(since), f"in {IDLE:.0f} s")
        report("processes", len(pids), "")
        report("rss", memory_kib(pids, "VmRSS", "status") / 1024, "MiB")
        report("pss", memory_kib(pids, "Pss", "smaps_rollup") / 1024, "MiB")
        for pid in pids:
            with open(f"/proc/{pid}/comm") as comm:
                name = comm.read().strip()
            report(f"pss {name}", memory_kib([pid], "Pss", "smaps_rollup") / 1024, "MiB")
            report(f"cpu-total {name}", cpu_seconds([pid]), "s")
    finally:
        driver.quit()
        stop_every_session()
    for program in ("kindric-desktop", "kindric-accounts", "kindric"):
        found = shutil.which(program)
        if found:
            report(f"binary {program}", os.path.getsize(found) / 1024 / 1024, "MiB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
