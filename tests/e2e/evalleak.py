"""Whether scripts run in the page, each one different and large, leave memory in WebKit
(E2E_SCRIPT=evalleak.py tests/e2e/run.sh): what Tauri does for every app.emit, whose payload is
written into the script. EVAL_KIB (default 100) the size of each, EVAL_COUNT (default 600) how many.
Printed: the private MiB of WebKitWebProcess by allocator every 100 scripts, then 60 s after.
"""
from __future__ import annotations

import json
import os
import sys
import time

from driver import CONTROL, SHIFT, Driver, wait_until
from journeys import ACCOUNT, PROJECT, accounts, application, forget_everything, overview_hidden, page_ready
from leak import parts_of, private_of

SIZE = int(os.environ.get("EVAL_KIB", "100")) * 1024
COUNT = int(os.environ.get("EVAL_COUNT", "600"))


def sample(label: str) -> None:
    print(f"{label} {private_of('WebKitWebProces') // 1024} MiB {parts_of('WebKitWebProces')}", flush=True)


def main() -> int:
    forget_everything()
    os.makedirs(PROJECT, exist_ok=True)
    accounts("add", ACCOUNT)
    driver = Driver.start(application())
    try:
        wait_until("the page ready", page_ready)
        driver.press(CONTROL, SHIFT, "1")
        wait_until("the first tab", lambda: overview_hidden(driver))
        time.sleep(10)
        sample("before")
        for count in range(1, COUNT + 1):
            # Like an event: a JSON payload in the text of the script, never twice the same.
            payload = json.dumps({"n": count, "sessions": ["x" * 90 + str(i) for i in range(SIZE // 100)]})
            driver.script(f"window.__last = {payload}.n; return window.__last;")
            if count % 100 == 0:
                sample(f"after {count}")
        time.sleep(60)
        sample("60 s later")
    finally:
        driver.quit()
    return 0


if __name__ == "__main__":
    sys.exit(main())
