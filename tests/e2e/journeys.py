"""kindric's end-to-end journeys, run inside the image by tests/e2e/inside.sh: the real app on a
screen that does not exist, driven through tauri-driver, with the fake claude (tests/e2e/bin/claude).

Each journey starts from nothing: no kindric data, one account, the sessions it asks for. Its
screenshot goes to $E2E_OUT (default ~/e2e-out), taken at its end.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import unittest
from typing import Any

from driver import CONTROL, ELEMENT_KEY, ENTER, SHIFT, Driver, wait_until

HOME = os.path.expanduser("~")
ACCOUNT = "work"
PROJECT = os.path.join(HOME, "project")
OUT = os.environ.get("E2E_OUT", os.path.join(HOME, "e2e-out"))
# What the fake claude knows of the account's sessions (tests/e2e/bin/claude).
FAKE_SESSIONS = os.path.join(HOME, ".local/share/kindric-accounts", ACCOUNT, "fake/sessions.json")
# Where the app writes its trace (KINDRIC_TRACE=1, set by inside.sh).
TRACE = os.path.join(HOME, ".local/share/kindric/trace.log")
# Everything kindric, kindric-accounts and the fake claude write in the container's home.
STATE = [".claude", ".config/kindric-accounts", ".local/share/kindric", ".local/share/kindric-accounts",
         ".local/share/io.github.louisstani.kindric"]
# The chips of the bar, as the page draws them: [state, title, open in a pane] for each.
CHIPS = """return [...document.querySelectorAll('#chips li.chip:not(.more)')].map(
  (chip) => [[...chip.classList].filter((name) => name !== 'chip' && name !== 'open').join(' '),
             chip.querySelector('.chip-title').textContent,
             chip.classList.contains('open')]);"""


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


def fake_sessions() -> list[dict[str, Any]]:
    try:
        with open(FAKE_SESSIONS) as source:
            sessions: list[dict[str, Any]] = json.load(source)
            return sessions
    except (OSError, ValueError):
        return []


def fake_detail(name: str) -> str | None:
    """What the fake claude says the session with this name is doing, None if it has no such session."""
    return next((str(session.get("detail", "")) for session in fake_sessions() if session.get("name") == name), None)


def fake_running(name: str) -> bool:
    """Whether the fake claude has a process for the session with this name."""
    return any(session.get("name") == name and session.get("pid") for session in fake_sessions())


def stop_every_session() -> None:
    """Stops every session the fake claude still runs: their processes outlive the app."""
    for session in fake_sessions():
        if session.get("pid"):
            subprocess.run(["kindric-accounts", "run", ACCOUNT, "stop", session["id"]], cwd=PROJECT, capture_output=True, timeout=30)


def forget_everything() -> None:
    for path in STATE:
        shutil.rmtree(os.path.join(HOME, path), ignore_errors=True)


def page_ready() -> bool:
    """The app marks the page ready once its first list is drawn (app.js, mark("page ready"))."""
    try:
        with open(TRACE) as trace:
            return "page ready" in trace.read()
    except OSError:
        return False


def chip(driver: Driver, title: str) -> tuple[str, bool] | None:
    """The state the bar shows for the session with this title and whether a pane shows it, None
    while it is not in the bar."""
    chips: list[list[Any]] = driver.script(CHIPS)
    return next(((str(state), bool(shown)) for state, title_shown, shown in chips if title_shown == title), None)


def chip_state(driver: Driver, title: str) -> str | None:
    found = chip(driver, title)
    return None if found is None else found[0]


def dialog_open(driver: Driver, dialog: str) -> bool:
    return bool(driver.script("return document.getElementById(arguments[0]).open", dialog))


def picker_rows(driver: Driver) -> list[tuple[str, str]]:
    """The sessions Ctrl+P lists: (name, detail) for each."""
    rows: list[list[str]] = driver.script("""return [...document.querySelectorAll('#picker-results li.session')].map(
      (row) => [row.querySelector('.name').textContent, row.querySelector('.detail').textContent]);""")
    return [(str(name), str(detail)) for name, detail in rows]


def open_from_bar(driver: Driver, title: str) -> None:
    """Clicks the session's chip, then waits for its pane."""
    found = driver.script("""return [...document.querySelectorAll('#chips li.chip:not(.more)')].find(
      (chip) => chip.querySelector('.chip-title').textContent === arguments[0]);""", title)
    if not isinstance(found, dict) or ELEMENT_KEY not in found:
        raise AssertionError(f"no chip titled {title} in the bar")
    driver.click(str(found[ELEMENT_KEY]))
    wait_until(f"{title} open in a pane", lambda: (chip(driver, title) or ("", False))[1])


class Journey(unittest.TestCase):
    """The app started from nothing, with the account and the sessions `started` names
    (name, task), stopped and screenshotted at the end."""

    started: list[tuple[str, str]] = []
    driver: Driver

    @classmethod
    def setUpClass(cls) -> None:
        forget_everything()
        os.makedirs(PROJECT, exist_ok=True)
        os.makedirs(OUT, exist_ok=True)
        accounts("add", ACCOUNT)
        for name, task in cls.started:
            accounts("run", ACCOUNT, "--bg", "-n", name, task)
        cls.driver = Driver.start(application())
        wait_until("the page ready", page_ready)

    @classmethod
    def tearDownClass(cls) -> None:
        try:
            cls.driver.screenshot(os.path.join(OUT, f"{cls.__name__}.png"))
        finally:
            cls.driver.quit()
            stop_every_session()


class SessionInTheBar(Journey):
    """A session started before the app is found at start, shown working, then finished: its turn
    ended and nobody has looked at it yet."""

    started = [("in-the-bar", "first journey work:8")]

    def test_working_then_finished(self) -> None:
        working = wait_until("the session in the bar", lambda: chip_state(self.driver, "in-the-bar"))
        self.assertEqual(working, "working")
        wait_until("the turn finished", lambda: chip_state(self.driver, "in-the-bar") == "finished")


class NewSessionFromCtrlN(Journey):
    """Ctrl+N, a folder and a name, Enter: the session starts, shows in the bar, opens in a pane,
    and what is typed in the pane reaches it."""

    def test_start_open_and_type(self) -> None:
        driver = self.driver
        driver.press(CONTROL, "n")
        wait_until("the new session form", lambda: dialog_open(driver, "new-session"))
        directory = driver.find("#new-form input[name=directory]")
        driver.clear(directory)
        driver.type_into(directory, PROJECT)
        driver.type_into(driver.find("#new-form input[name=name]"), "from-ctrl-n" + ENTER)

        wait_until("the session in the bar", lambda: chip_state(driver, "from-ctrl-n"))
        wait_until("the form closed", lambda: not dialog_open(driver, "new-session"))
        wait_until("the session open in a pane", lambda: (chip(driver, "from-ctrl-n") or ("", False))[1])
        self.assertIsNotNone(fake_detail("from-ctrl-n"), "the fake claude has no session named from-ctrl-n")

        terminal = wait_until("the pane's terminal", lambda: driver.find("#panes .xterm-helper-textarea"))
        driver.type_into(terminal, "hello work:4" + ENTER)
        wait_until("the typed turn reaching the session", lambda: fake_detail("from-ctrl-n") == "working on: hello work:4")


class HiddenThenFoundWithCtrlP(Journey):
    """Ctrl+Shift+K stops the session of the pane and hides it from the bar; Ctrl+P still finds
    it, and choosing it brings it back to the bar, started again."""

    started = [("to-hide", "hide me work:1")]

    def test_hide_find_and_bring_back(self) -> None:
        driver = self.driver
        wait_until("the session in the bar", lambda: chip_state(driver, "to-hide"))
        open_from_bar(driver, "to-hide")

        driver.press(CONTROL, SHIFT, "k")
        wait_until("the question before hiding", lambda: dialog_open(driver, "confirm-hide"))
        driver.click(driver.find("#hide-form button[type=submit]"))
        wait_until("the session gone from the bar", lambda: chip_state(driver, "to-hide") is None)
        wait_until("the session stopped", lambda: not fake_running("to-hide"))

        driver.press(CONTROL, "p")
        wait_until("the session finder", lambda: dialog_open(driver, "picker"))
        driver.type_into(driver.find("#picker-query"), "to-hide")
        found = wait_until("the hidden session in the finder", lambda: next((detail for name, detail in picker_rows(driver) if name == "to-hide"), None))
        self.assertIn("hidden", found)

        driver.type_into(driver.find("#picker-query"), ENTER)
        wait_until("the session back in the bar", lambda: chip_state(driver, "to-hide"))
        wait_until("the session open in a pane", lambda: (chip(driver, "to-hide") or ("", False))[1])
        wait_until("the session started again", lambda: fake_running("to-hide"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
