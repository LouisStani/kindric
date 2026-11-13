"""kindric's end-to-end journeys, run inside the image by tests/e2e/inside.sh: the real app on a
screen that does not exist, driven through tauri-driver, with the fake claude (tests/e2e/bin/claude).

Screenshots go to $E2E_OUT (default ~/e2e-out), one per journey, taken at its end.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import unittest
from typing import Any

from driver import Driver, wait_until

ACCOUNT = "work"
PROJECT = os.path.expanduser("~/project")
OUT = os.environ.get("E2E_OUT", os.path.expanduser("~/e2e-out"))
# The chips of the bar, as the page draws them: [state, title] for each.
CHIPS = """return [...document.querySelectorAll('#chips li.chip:not(.more)')].map(
  (chip) => [[...chip.classList].filter((name) => name !== 'chip' && name !== 'open').join(' '),
             chip.querySelector('.chip-title').textContent]);"""


def accounts(*words: str) -> str:
    """kindric-accounts with these words, from the test project; its output, or the test fails."""
    done = subprocess.run(["kindric-accounts", *words], cwd=PROJECT, capture_output=True, text=True, timeout=30)
    if done.returncode != 0:
        raise AssertionError(f"kindric-accounts {' '.join(words)} exited {done.returncode}: {done.stderr.strip()}")
    return done.stdout


def application() -> str:
    found = shutil.which("kindric-desktop")
    if found is None:
        raise AssertionError("kindric-desktop is not in PATH")
    return found


def chip_state(driver: Driver, title: str) -> str | None:
    """The state the bar shows for the session with this title, None while it is not there."""
    chips: list[list[Any]] = driver.script(CHIPS)
    return next((str(state) for state, shown in chips if shown == title), None)


class SessionInTheBar(unittest.TestCase):
    """A session started before the app is found at start, shown working, then finished: its turn
    ended and nobody has looked at it yet."""

    driver: Driver

    @classmethod
    def setUpClass(cls) -> None:
        os.makedirs(PROJECT, exist_ok=True)
        os.makedirs(OUT, exist_ok=True)
        accounts("add", ACCOUNT)
        accounts("run", ACCOUNT, "--bg", "-n", "in-the-bar", "first journey work:8")
        cls.driver = Driver.start(application())

    @classmethod
    def tearDownClass(cls) -> None:
        try:
            cls.driver.screenshot(os.path.join(OUT, "session-in-the-bar.png"))
        finally:
            cls.driver.quit()
            accounts("run", ACCOUNT, "stop", "in-the-bar")

    def test_working_then_finished(self) -> None:
        working = wait_until("the session in the bar", lambda: chip_state(self.driver, "in-the-bar"))
        self.assertEqual(working, "working")
        wait_until("the turn finished", lambda: chip_state(self.driver, "in-the-bar") == "finished")


if __name__ == "__main__":
    unittest.main(verbosity=2)
