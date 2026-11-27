"""Whether the page's memory keeps growing, in the image, on a session lived faster
(E2E_SCRIPT=leak.py FAKE_SPEED=10 tests/e2e/run.sh; not E2E_NO_TRACE, the page is known ready by
the trace).

LEAK_BUSY sessions of the fake claude at work (default 2), two waiting, then the app left for
MEASURE_IDLE seconds (default 600) with:
- LEAK_VIEW=bar: the first tab, MEASURE_PANES of the working sessions open in panes (0, 1 or 2);
- LEAK_VIEW=control: the control center shown, no pane.
LEAK_PAUSED=N adds N sessions stopped before the app starts, as the paused rows of a long-used account.
LEAK_OUTPUT=1: the working sessions write in their panes as Claude Code does while it works (the
fake claude's chatter). FAKE_SPEED=N: each working session N times faster, in its steps (a tool
call in the transcript every 10/N s) and in its output.
LEAK_ACTIONS=1: what a user does, N times faster too: every 60/N s, in turn, the control center
and back, Ctrl+P opened and closed, the other pane focused.
LEAK_KEYS=1: down then up twice a second (a redraw of the control center each time), during the
first LEAK_KEYS_FOR seconds.
Printed: one `sample <seconds> <private KiB of WebKitWebProcess> <private KiB of kindric-desktop>
nodes <DOM nodes> <MiB of each WebKit allocator>` every 30 s, then `slope <name> <MiB per minute>`, the least-squares slope of each.
"""
from __future__ import annotations

import os
import sys
import time

from driver import ARROW_DOWN, CONTROL, ESCAPE, SHIFT, Driver, wait_until
from journeys import ACCOUNT, CHIPS, PROJECT, accounts, application, dialog_open, fake_detail, fake_sessions, forget_everything, open_from_bar, overview_hidden, page_ready, stop_every_session
from measure import app_processes, memory_kib

IDLE = float(os.environ.get("MEASURE_IDLE", "600"))
PANES = int(os.environ.get("MEASURE_PANES", "0"))
VIEW = os.environ.get("LEAK_VIEW", "bar")
PAUSED = int(os.environ.get("LEAK_PAUSED", "0"))
BUSY = int(os.environ.get("LEAK_BUSY", "2"))
KEYS = os.environ.get("LEAK_KEYS") == "1"
ACTIONS = os.environ.get("LEAK_ACTIONS") == "1"
SPEED = float(os.environ.get("FAKE_SPEED") or 1)
# The fake claude chatters in the panes of the sessions whose name starts with "chatty".
BUSY_PREFIX = "chatty" if os.environ.get("LEAK_OUTPUT") == "1" else "busy"
ARROW_UP = ""
KEYS_FOR = float(os.environ.get("LEAK_KEYS_FOR", "1e9"))
STEP = 30.0
# One user action a minute, lived SPEED times faster.
ACTION_EVERY = 60.0 / SPEED


def private_of(name: str) -> int:
    pids = []
    for pid in app_processes():
        try:
            with open(f"/proc/{pid}/comm") as comm:
                if comm.read().strip() == name:
                    pids.append(pid)
        except OSError:
            continue
    # Private, not Pss: Pss shares the libraries with the apps of other containers on the same
    # image, and jumps when one of them ends.
    return memory_kib(pids, "Private_Clean", "smaps_rollup") + memory_kib(pids, "Private_Dirty", "smaps_rollup")


PARTS = ("WKFastMalloc", "JSGigacage", "memfd:WebKitSharedMemory", "[heap]")


def parts_of(name: str) -> str:
    """The private KiB of the processes called `name` in WebKit's allocators, the others together."""
    sizes = dict.fromkeys(PARTS + ("other",), 0)
    for pid in app_processes():
        try:
            with open(f"/proc/{pid}/comm") as comm:
                if comm.read().strip() != name:
                    continue
            with open(f"/proc/{pid}/smaps") as smaps:
                part = "other"
                for line in smaps:
                    fields = line.split()
                    if "-" in fields[0] and ":" not in fields[0]:
                        mapping = " ".join(fields[5:])
                        part = next((p for p in PARTS if p in mapping), "other")
                    elif fields[0] in ("Private_Clean:", "Private_Dirty:"):
                        sizes[part] += int(fields[1])
        except OSError:
            continue
    return " ".join(f"{part}={kib // 1024}" for part, kib in sizes.items())


def slope(points: list[tuple[float, int]]) -> float:
    """MiB per minute, by least squares."""
    count = len(points)
    mean_t = sum(t for t, _ in points) / count
    mean_v = sum(v for _, v in points) / count
    spread = sum((t - mean_t) ** 2 for t, _ in points)
    return sum((t - mean_t) * (v - mean_v) for t, v in points) / spread * 60 / 1024


def act(driver: Driver, count: int) -> None:
    """One user action, the next of the round."""
    action = count % 3
    if action == 0:
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        time.sleep(min(2.0, ACTION_EVERY / 3))
        driver.press(CONTROL, SHIFT, "1")
        wait_until("the first tab", lambda: overview_hidden(driver))
    elif action == 1:
        driver.press(CONTROL, "p")
        wait_until("the session finder", lambda: dialog_open(driver, "picker"))
        time.sleep(min(2.0, ACTION_EVERY / 3))
        driver.press(ESCAPE)
        wait_until("the session finder closed", lambda: not dialog_open(driver, "picker"))
    else:
        # The next pane, then the previous one next round (nothing with one pane).
        driver.press(CONTROL, "l" if count // 3 % 2 == 0 else "h")


def wait(driver: Driver, seconds: float, at: float, actions: list[int]) -> None:
    """The time between two samples, with what LEAK_KEYS or LEAK_ACTIONS asks."""
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        if KEYS and at < KEYS_FOR:
            driver.press(ARROW_DOWN)
            time.sleep(0.25)
            driver.press(ARROW_UP)
            time.sleep(0.25)
        elif ACTIONS:
            act(driver, actions[0])
            actions[0] += 1
            time.sleep(max(0.0, min(ACTION_EVERY, until - time.monotonic())))
        else:
            time.sleep(max(0.0, until - time.monotonic()))


def main() -> int:
    if VIEW not in ("bar", "control"):
        print(f"LEAK_VIEW is bar or control, not {VIEW}", file=sys.stderr)
        return 2
    forget_everything()
    os.makedirs(PROJECT, exist_ok=True)
    accounts("add", ACCOUNT)
    working = [f"{BUSY_PREFIX}-{n}" for n in range(BUSY)]
    for name in working:
        accounts("run", ACCOUNT, "--bg", "-n", name, "leak work:86400")
    accounts("run", ACCOUNT, "--bg", "-n", "first", "leak work:0")
    accounts("run", ACCOUNT, "--bg", "-n", "second", "leak work:0")
    paused = [f"paused-{n}" for n in range(PAUSED)]
    for name in paused:
        accounts("run", ACCOUNT, "--bg", "-n", name, "leak work:0")
    for name in paused:
        wait_until(f"{name}'s turn over", lambda: fake_detail(name) == "waiting for you", timeout=60)
        accounts("run", ACCOUNT, "stop", next(s["id"] for s in fake_sessions() if s.get("name") == name))
    driver = Driver.start(application())
    try:
        wait_until("the page ready", page_ready)
        # The app opens on the control center, where the bar shows no session.
        driver.press(CONTROL, SHIFT, "1")
        wait_until("the first tab", lambda: overview_hidden(driver))
        wait_until("the sessions in the bar", lambda: len(driver.script(CHIPS)) >= BUSY + 2, timeout=60)
        if VIEW == "bar" and PANES >= 1:
            open_from_bar(driver, working[0])
        if VIEW == "bar" and PANES >= 2:
            driver.press(CONTROL, SHIFT, "d")  # split: the second session goes in the new pane
            open_from_bar(driver, working[1] if BUSY >= 2 else "first")
        if VIEW == "control":
            driver.press(CONTROL, SHIFT, "0")
            wait_until("the control center", lambda: not overview_hidden(driver))
        print(f"view {VIEW} panes {PANES if VIEW == 'bar' else 0} busy {BUSY} paused {PAUSED} speed {SPEED:g}"
              f" output {BUSY_PREFIX == 'chatty'} actions {ACTIONS} idle {IDLE:.0f} s", flush=True)
        time.sleep(10)
        web: list[tuple[float, int]] = []
        app: list[tuple[float, int]] = []
        actions = [0]
        started = time.monotonic()
        while True:
            at = time.monotonic() - started
            web.append((at, private_of("WebKitWebProces")))
            app.append((at, private_of("kindric-desktop")))
            nodes = driver.script("return document.getElementsByTagName('*').length")
            print(f"sample {at:.0f} {web[-1][1]} {app[-1][1]} nodes {nodes} {parts_of('WebKitWebProces')}", flush=True)
            if at >= IDLE:
                break
            wait(driver, STEP, at, actions)
        print(f"actions {actions[0]}", flush=True)
        print(f"slope WebKitWebProcess {slope(web):.2f} MiB/min", flush=True)
        print(f"slope kindric-desktop {slope(app):.2f} MiB/min", flush=True)
    finally:
        driver.quit()
        stop_every_session()
    return 0


if __name__ == "__main__":
    sys.exit(main())
