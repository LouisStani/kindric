"""kindric's end-to-end journeys, run inside the image by tests/e2e/inside.sh: the real app on a
screen that does not exist, driven through tauri-driver, with the fake claude (tests/e2e/bin/claude).

Each journey starts from nothing: no kindric data, one account, the sessions it asks for. Its
screenshot goes to $E2E_OUT (default ~/e2e-out), taken at its end.
"""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import time
import unittest
from typing import Any

from driver import ALT, ARROW_DOWN, ARROW_LEFT, ARROW_RIGHT, CONTROL, DELETE, ELEMENT_KEY, ENTER, ESCAPE, F2, F3, F4, SHIFT, TAB, Driver, wait_until

HOME = os.path.expanduser("~")
ACCOUNT = "work"
PROJECT = os.path.join(HOME, "project")
OUT = os.environ.get("E2E_OUT", os.path.join(HOME, "e2e-out"))
# What the fake claude knows of the account's sessions (tests/e2e/bin/claude).
FAKE_SESSIONS = os.path.join(HOME, ".local/share/kindric-accounts", ACCOUNT, "fake/sessions.json")
# Where the app writes its trace (KINDRIC_TRACE=1, set by inside.sh).
TRACE = os.path.join(HOME, ".local/share/kindric/trace.log")
# The theme kindric writes for Claude Code to follow (claude_theme.rs).
CLAUDE_THEME = os.path.join(HOME, ".claude/themes/kindric.json")
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


def overview_hidden(driver: Driver) -> bool:
    return bool(driver.script("return document.getElementById('overview').hidden"))


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
        cls.prepare()
        for name, task in cls.started:
            accounts("run", ACCOUNT, "--bg", "-n", name, task)
        cls.before_app()
        cls.driver = Driver.start(application())
        wait_until("the page ready", page_ready)
        cls.first_tab()

    @classmethod
    def prepare(cls) -> None:
        """What a journey sets up once the account exists, before the app starts."""

    @classmethod
    def before_app(cls) -> None:
        """What a journey does once its sessions are started, before the app starts."""

    @classmethod
    def first_tab(cls) -> None:
        """kindric opens on the control center (tab 0), where the bar shows no session: a journey
        starts on the first tab, as the user does to work in a pane."""
        cls.driver.press(CONTROL, SHIFT, "1")
        wait_until("the first tab shown", lambda: overview_hidden(cls.driver))

    @classmethod
    def restart(cls) -> None:
        """Closes the app and starts it again, as the user would."""
        cls.driver.quit()
        os.remove(TRACE)
        cls.driver = Driver.start(application())
        wait_until("the page ready again", page_ready)
        cls.first_tab()

    @classmethod
    def tearDownClass(cls) -> None:
        try:
            cls.driver.screenshot(os.path.join(OUT, f"{cls.__name__}.png"))
        finally:
            cls.driver.quit()
            stop_every_session()
            # What the app did, to read when the journey failed.
            if os.path.exists(TRACE):
                shutil.copyfile(TRACE, os.path.join(OUT, f"{cls.__name__}.trace.log"))


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


class RenamedWithF2(Journey):
    """F2 gives the session of the pane a short name: the bar shows it, and so does the next start."""

    started = [("long-name", "rename me work:1")]

    def test_rename_and_keep_the_name(self) -> None:
        driver = self.driver
        wait_until("the session in the bar", lambda: chip_state(driver, "long-name"))
        open_from_bar(driver, "long-name")
        driver.press(F2)
        wait_until("the name form", lambda: dialog_open(driver, "rename"))
        label = driver.find("#rename-form input[name=label]")
        driver.clear(label)
        driver.type_into(label, "router" + ENTER)
        wait_until("the new name in the bar", lambda: chip_state(driver, "router"))
        self.assertIsNone(chip_state(driver, "long-name"))

        type(self).restart()
        wait_until("the new name after a restart", lambda: chip_state(self.driver, "router"))


class NoticeUnderTheBar(Journey):
    """A notice shows under the bar, at its right, without taking room from the sessions, then goes."""

    started = [("in-the-bar", "first task work:0")]

    def box(self, css: str) -> list[float]:
        found: list[float] = self.driver.script("const box = document.querySelector(arguments[0]).getBoundingClientRect(); return [box.left, box.top, box.width, box.right];", css)
        return found

    def test_notice_shows_then_goes(self) -> None:
        driver = self.driver
        wait_until("the session in the bar", lambda: chip_state(driver, "in-the-bar"))
        before = self.box("#chips")
        driver.press(CONTROL, SHIFT, "e")
        wait_until("the notice", lambda: "theme:" in driver.text(driver.find("#status")) or None)
        left, top, width, right = self.box("#status")
        self.assertGreaterEqual(top, 50, "under the bar")
        self.assertGreater(width, 40)
        self.assertLessEqual(right, driver.script("return window.innerWidth"))
        self.assertEqual(self.box("#chips"), before, "the sessions keep their room")
        self.assertEqual(driver.text(driver.find("#brand")), "kindric")
        driver.screenshot(os.path.join(OUT, "NoticeUnderTheBar-shown.png"))
        wait_until("the notice gone", lambda: driver.script("return document.getElementById('status').hidden"))


class ThemeWithCtrlShiftE(Journey):
    """Ctrl+Shift+E moves to the next theme (ember, then paper): the page and Claude Code's theme
    follow, and the next start keeps it."""

    def background(self) -> str:
        return str(self.driver.script("return getComputedStyle(document.documentElement).getPropertyValue('--background').trim()"))

    def claude_draws_light(self) -> bool:
        try:
            with open(CLAUDE_THEME) as theme:
                return str(json.load(theme).get("base", "")).startswith("light")
        except (OSError, ValueError):
            return False

    def test_next_theme_kept(self) -> None:
        driver = self.driver
        self.assertEqual(self.background(), "#1a0e06")
        driver.press(CONTROL, SHIFT, "e")
        wait_until("the paper theme", lambda: self.background() == "#f7efe4")
        self.assertEqual(driver.script("return document.documentElement.style.colorScheme"), "light")
        wait_until("Claude Code told to draw light", self.claude_draws_light)

        type(self).restart()
        wait_until("the paper theme after a restart", lambda: self.background() == "#f7efe4")


def tabs(driver: Driver) -> list[tuple[str, bool]]:
    """The tabs of the window: (name, active) for each."""
    found: list[list[Any]] = driver.script("""return [...document.querySelectorAll('#tabs button.tab')].map(
      (tab) => [tab.querySelector('.tab-name').textContent, tab.classList.contains('active')]);""")
    return [(str(name), bool(active)) for name, active in found]


def chip_titles(driver: Driver) -> list[str]:
    chips: list[list[Any]] = driver.script(CHIPS)
    return [str(title) for _, title, _ in chips]


class ProjectTab(Journey):
    """Ctrl+Shift+T adds a project tab; Ctrl+N there starts a session of the account at Enter, in
    the project's folder; Ctrl+Shift+X removes the tab and the session keeps running."""

    def test_add_start_in_it_and_remove(self) -> None:
        driver = self.driver
        driver.press(CONTROL, SHIFT, "t")
        wait_until("the project form", lambda: dialog_open(driver, "new-project"))
        directory = driver.find("#project-form input[name=directory]")
        driver.clear(directory)
        driver.type_into(directory, PROJECT)
        driver.click(driver.find("#project-form button[type=submit]"))
        wait_until("the project tab, active", lambda: ("project", True) in tabs(driver))

        driver.press(CONTROL, "n")
        wait_until("the accounts to start with", lambda: dialog_open(driver, "quick-launch"))
        driver.press(ENTER)
        wait_until("a session of the fake claude", lambda: len(fake_sessions()) == 1)
        session = fake_sessions()[0]
        self.assertEqual(session["cwd"], PROJECT)
        wait_until("the session in the project's bar", lambda: len(chip_titles(driver)) == 1)

        driver.press(CONTROL, SHIFT, "x")
        wait_until("the project tab gone", lambda: all(name != "project" for name, _ in tabs(driver)))
        self.assertTrue(fake_sessions()[0].get("pid"), "the session stopped with its tab")


# The account's folder, as Claude Code gives it to the MCP servers it starts.
ACCOUNT_DIR = os.path.join(HOME, ".local/share/kindric-accounts", ACCOUNT)


def mcp_call(session_id: str, tool: str, arguments: dict[str, Any]) -> str:
    """Calls a tool of `kindric mcp` as the session `session_id` would, through Claude Code; its text."""
    text, refused = mcp_answer(session_id, tool, arguments)
    if refused:
        raise AssertionError(f"{tool} refused: {text}")
    return text


def mcp_answer(session_id: str, tool: str, arguments: dict[str, Any], account_dir: str = ACCOUNT_DIR) -> tuple[str, bool]:
    """Calls a tool of `kindric mcp` as a session of the account in `account_dir` would: its text,
    and whether the tool refused."""
    requests = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize",
         "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "1"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": tool, "arguments": arguments}},
    ]
    environment = dict(os.environ, CLAUDE_CODE_SESSION_ID=session_id, CLAUDE_CONFIG_DIR=account_dir)
    done = subprocess.run(["kindric", "mcp"], input="".join(json.dumps(request) + "\n" for request in requests),
                          capture_output=True, text=True, timeout=30, env=environment, cwd=PROJECT)
    for line in done.stdout.splitlines():
        answer = json.loads(line)
        if answer.get("id") == 2:
            result = answer.get("result", {})
            text = "".join(part.get("text", "") for part in result.get("content", []))
            refused = bool(result.get("isError") or "error" in answer)
            return (text or str(answer.get("error", ""))), refused
    raise AssertionError(f"kindric mcp gave no answer to {tool}: {done.stderr.strip()}")


def session_id_of(name: str) -> str:
    return next(str(session["sessionId"]) for session in fake_sessions() if session.get("name") == name)


def chip_badge(driver: Driver, title: str) -> str | None:
    """The unread count the session's chip shows, None without one."""
    found = driver.script("""const chip = [...document.querySelectorAll('#chips li.chip:not(.more)')].find(
      (chip) => chip.querySelector('.chip-title').textContent === arguments[0]);
      const badge = chip && chip.querySelector('.badge');
      return badge ? badge.textContent : null;""", title)
    return None if found is None else str(found)


class MessageBetweenSessions(Journey):
    """A session leaves a message to another through `kindric mcp`: the bar counts it on the
    other's chip, Ctrl+Shift+M shows it, and the count goes once the session read it."""

    started = [("sender", "write work:0"), ("receiver", "read work:0")]

    def test_send_show_and_read(self) -> None:
        driver = self.driver
        wait_until("both sessions in the bar", lambda: chip_state(driver, "sender") and chip_state(driver, "receiver"))
        said = mcp_call(session_id_of("sender"), "send_message", {"to": "receiver", "text": "the build is green"})
        self.assertIn("Message left for receiver", said)
        wait_until("the unread count on the receiver", lambda: chip_badge(driver, "receiver") == "1")
        self.assertIsNone(chip_badge(driver, "sender"))

        open_from_bar(driver, "receiver")
        driver.press(CONTROL, SHIFT, "m")
        wait_until("the messages window", lambda: dialog_open(driver, "messages"))
        wait_until("the message in it", lambda: "the build is green" in driver.text(driver.find("#messages-list")))

        read = mcp_call(session_id_of("receiver"), "read_messages", {})
        self.assertIn("the build is green", read)
        wait_until("the count gone once read", lambda: chip_badge(driver, "receiver") is None)


def git(*words: str) -> None:
    subprocess.run(["git", *words], cwd=PROJECT, check=True, capture_output=True, timeout=30)


def fresh_repository() -> None:
    """The test project as a git working copy with one commit and nothing changed."""
    shutil.rmtree(PROJECT, ignore_errors=True)
    os.makedirs(PROJECT)
    git("init", "-q")
    git("config", "user.email", "tester@example.invalid")
    git("config", "user.name", "tester")
    with open(os.path.join(PROJECT, "README"), "w") as readme:
        readme.write("test project\n")
    git("add", "README")
    git("commit", "-q", "-m", "start")


def texts(driver: Driver, css: str) -> list[str]:
    found: list[str] = driver.script("return [...document.querySelectorAll(arguments[0])].map((node) => node.textContent);", css)
    return [str(text) for text in found]


def click_text(driver: Driver, css: str, text: str) -> None:
    """Clicks the first element `css` matches whose text is `text`."""
    found = driver.script("return [...document.querySelectorAll(arguments[0])].find((node) => node.textContent.trim() === arguments[1]);", css, text)
    if not isinstance(found, dict) or ELEMENT_KEY not in found:
        raise AssertionError(f"nothing like {css} says {text!r}")
    driver.click(str(found[ELEMENT_KEY]))


def add_project(driver: Driver) -> None:
    """Ctrl+Shift+T on the test project: its tab, active."""
    driver.press(CONTROL, SHIFT, "t")
    wait_until("the project form", lambda: dialog_open(driver, "new-project"))
    directory = driver.find("#project-form input[name=directory]")
    driver.clear(directory)
    driver.type_into(directory, PROJECT)
    driver.click(driver.find("#project-form button[type=submit]"))
    wait_until("the project tab, active", lambda: ("project", True) in tabs(driver))


def carry_messages(driver: Driver) -> None:
    """The "add" of the welcome list: the messages server for the account and for ~/.claude, the
    last thing left to do, after which the welcome list no longer covers the page."""
    wait_until("the add of the welcome list", lambda: "add" in texts(driver, "#welcome button"))
    click_text(driver, "#welcome button", "add")
    wait_until("the welcome list gone", lambda: bool(driver.script("return document.getElementById('welcome').hidden")))


class WhatASessionDoes(Journey):
    """The row of a session at work says what it does (decision 499), on the line under it
    (decision 500): the prompt that started its turn, then the last sentence Claude wrote in it."""

    started = [("builder", "ship it work:60 say:I_compile_the_release_build_before_packing."), ("drafter", "draft the notes work:60")]

    def doing(self, name: str) -> str | None:
        found = self.driver.script("""const row = [...document.querySelectorAll('article.control-row')].find(
          (row) => row.querySelector('.row-name').textContent === arguments[0]);
          const under = row && row.querySelector('.row-task');
          return under ? under.textContent : null;""", name)
        return None if found is None else str(found)

    def test_prompt_then_sentence(self) -> None:
        driver = self.driver
        carry_messages(driver)
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        wait_until("the sentence on the row", lambda: self.doing("builder") == "▸ I compile the release build before packing." or None, timeout=30)
        self.assertEqual(self.doing("drafter"), "▸ draft the notes work:60", "nothing said yet: the prompt")
        driver.screenshot(os.path.join(OUT, "WhatASessionDoes.png"))


class ControlCenter(Journey):
    """Tab 0 lists the project and the account on the side and a row per session. The session
    picked shows its last steps and takes a prompt typed in its field; the project's head counts
    the files its sessions changed."""

    started = [("alpha", "first task work:0"), ("beta", "second task touch:made.txt work:0")]

    @classmethod
    def setUpClass(cls) -> None:
        fresh_repository()
        super().setUpClass()

    def row_detail(self, name: str) -> list[str]:
        return texts(self.driver, "article.control-row.selected .detail-steps li") if self.picked() == name else []

    def picked(self) -> str | None:
        found = texts(self.driver, "article.control-row.selected .row-name")
        return found[0] if found else None

    def test_rows_detail_prompt_and_counts(self) -> None:
        driver = self.driver
        carry_messages(driver)
        add_project(driver)
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        wait_until("a row per session", lambda: sorted(texts(driver, "article.control-row .row-name")) == ["alpha", "beta"])
        self.assertIn("project", texts(driver, ".control-project-name"))
        self.assertIn("work", texts(driver, ".account-head strong"))

        click_text(driver, ".control-project .control-project-name", "project")
        wait_until("the project's head", lambda: texts(driver, ".control-head h1") == ["project"])
        # Started outside kindric, the sessions have no photo of the copy at their start: a file
        # git does not track counts for every session of the copy, alpha too (changes.rs).
        wait_until("the file beta wrote, counted", lambda: "2 files changed" in texts(driver, ".control-figure"))
        self.assertIn("1 file", texts(driver, "article.control-row .row-badge.files"))

        click_text(driver, "article.control-row .row-name", "alpha")
        # A list comes back from wait_until even empty: None says not yet.
        steps = wait_until("alpha's last steps", lambda: self.row_detail("alpha") if any("first task" in step for step in self.row_detail("alpha")) else None)
        self.assertTrue(any("done: first task" in step for step in steps), steps)

        field = wait_until("alpha's prompt field", lambda: driver.find(f"[data-key='prompt/{session_id_of('alpha')}']"))
        driver.type_into(field, "go on work:3" + ENTER)
        wait_until("the prompt reaching alpha", lambda: fake_detail("alpha") == "working on: go on work:3")
        self.assertEqual(fake_detail("beta"), "waiting for you")


class NarrowControlCenter(Journey):
    """In a narrow window, a row of the control center keeps what its session does readable, on a
    line of its own, and nothing comes out of its card; in a wide one, the row is a single line."""

    started = [("long-running", "a task that works for a long time work:300"), ("finished-one", "a short task work:0")]

    # For each row: the width of what the session does, whether it sits under the name, and the
    # parts that come out of the card on the right.
    MEASURE = """return [...document.querySelectorAll('article.control-row')].map((row) => {
      const right = row.getBoundingClientRect().right;
      const doing = row.querySelector('.row-doing').getBoundingClientRect();
      return {
        doing: doing.width,
        under: doing.top >= row.querySelector('.row-name').getBoundingClientRect().bottom,
        out: [...row.querySelectorAll('*')].filter((node) => node.getBoundingClientRect().right > right + 1).map((node) => node.className || node.tagName),
      };
    });"""

    def rows_at(self, width: int) -> list[dict[str, Any]]:
        self.driver.resize(width, 860)
        wait_until(f"the window at {width} px", lambda: self.driver.script("return window.innerWidth") == width)
        rows: list[dict[str, Any]] = self.driver.script(self.MEASURE)
        return rows

    def test_rows_fit_the_window(self) -> None:
        driver = self.driver
        carry_messages(driver)
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        wait_until("a row per session", lambda: sorted(texts(driver, "article.control-row .row-name")) == ["finished-one", "long-running"])

        for row in self.rows_at(900):
            self.assertEqual(row["out"], [], "nothing comes out of the card")
            self.assertTrue(row["under"], "what it does goes under the name")
            self.assertGreaterEqual(row["doing"], 150, "what it does stays readable")
        driver.screenshot(os.path.join(OUT, f"{type(self).__name__}-900.png"))

        for row in self.rows_at(1400):
            self.assertEqual(row["out"], [], "nothing comes out of the card")
            self.assertFalse(row["under"], "a wide row is a single line")


def hook_input(session_id: str, tool: str, tool_input: dict[str, Any]) -> str:
    """What Claude Code gives its PermissionRequest hook, in the shape of the trial of 2026-11-16
    (permission.rs keeps it in its tests)."""
    return json.dumps({"session_id": session_id, "transcript_path": "/t.jsonl", "cwd": PROJECT, "permission_mode": "default",
                       "hook_event_name": "PermissionRequest", "tool_name": tool, "tool_input": tool_input, "permission_suggestions": []})


def asked_by_hook(session_id: str, tool: str, tool_input: dict[str, Any]) -> subprocess.Popen[str]:
    """`kindric permission-hook` started as Claude Code starts it for a session of the account: it
    puts the question in the control center and waits for the answer."""
    hook = subprocess.Popen(["kindric", "permission-hook"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            text=True, cwd=PROJECT, env={**os.environ, "CLAUDE_CONFIG_DIR": ACCOUNT_DIR})
    assert hook.stdin is not None
    hook.stdin.write(hook_input(session_id, tool, tool_input))
    hook.stdin.close()
    return hook


def prompts_of(session_id: str) -> list[str]:
    """What the user said to the session, turn by turn, as its transcript keeps it."""
    prompts: list[str] = []
    projects = os.path.join(ACCOUNT_DIR, "projects")
    for folder in os.listdir(projects) if os.path.isdir(projects) else []:
        path = os.path.join(projects, folder, f"{session_id}.jsonl")
        if os.path.exists(path):
            with open(path) as source:
                entries = [json.loads(line) for line in source if line.strip()]
            # A tool's result is a user entry too, its content a list.
            prompts += [entry["message"]["content"] for entry in entries
                        if entry.get("type") == "user" and isinstance(entry["message"]["content"], str)]
    return prompts


def decision_of(hook: subprocess.Popen[str]) -> dict[str, Any]:
    """The decision the hook prints for Claude Code once the user answered."""
    assert hook.stdout is not None and hook.stderr is not None
    with hook.stdout, hook.stderr:
        try:
            hook.wait(timeout=15)
        except subprocess.TimeoutExpired:
            hook.kill()
            hook.wait()
            raise AssertionError("the hook still waits: the answer did not reach it") from None
        out, err = hook.stdout.read(), hook.stderr.read()
    if not out.strip():
        raise AssertionError(f"the hook printed nothing (exit {hook.returncode}): {err.strip()}")
    printed: dict[str, Any] = json.loads(out)
    decided: dict[str, Any] = printed["hookSpecificOutput"]["decision"]
    return decided


class AnswerFromTheControlCenter(Journey):
    """What a session asks is answered from its row, without opening it: a permission (Allow,
    then Deny), a question of AskUserQuestion (one click on an option), and a `kindric ask`, whose
    answer is typed into the session with the question recalled (336)."""

    started = [("asks", "a task that asks work:0")]

    def requests(self) -> list[str]:
        return texts(self.driver, "article.control-row .request")

    def answered(self) -> None:
        wait_until("the request gone from the row", lambda: self.requests() == [] or None)

    def test_permission_question_and_ask(self) -> None:
        driver = self.driver
        carry_messages(driver)
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        wait_until("the session's row", lambda: texts(driver, "article.control-row .row-name") == ["asks"])
        session = session_id_of("asks")

        hook = asked_by_hook(session, "Bash", {"command": "touch e2e-made.txt", "description": "Create a file"})
        wait_until("the command alone in the row", lambda: any("touch e2e-made.txt" in text for text in texts(driver, "article.control-row .request-command")))
        click_text(driver, "article.control-row .request button", "Allow")
        self.assertEqual(decision_of(hook)["behavior"], "allow")
        self.answered()

        hook = asked_by_hook(session, "Bash", {"command": "rm -r build", "description": "Remove the build"})
        wait_until("the second command in the row", lambda: any("rm -r build" in text for text in texts(driver, "article.control-row .request-command")))
        click_text(driver, "article.control-row .request button", "Deny")
        self.assertEqual(decision_of(hook)["behavior"], "deny")
        self.answered()

        question = {"question": "Red or blue?", "header": "Color", "multiSelect": False,
                    "options": [{"label": "Red", "description": "Answer red"}, {"label": "Blue", "description": "Answer blue"}]}
        hook = asked_by_hook(session, "AskUserQuestion", {"questions": [question]})
        wait_until("the question in the row", lambda: "Red or blue?" in texts(driver, "article.control-row .request-question"))
        click_text(driver, "article.control-row .request button", "2 Blue")
        decided = decision_of(hook)
        self.assertEqual((decided["behavior"], decided["updatedInput"]["answers"]), ("allow", {"Red or blue?": "Blue"}))
        self.answered()

        asked = subprocess.run(["kindric", "ask", "Keep the old routes?", "They have no callers left. I keep writing the tests meanwhile."],
                               cwd=PROJECT, capture_output=True, text=True, timeout=10, env={**os.environ, "CLAUDE_CODE_SESSION_ID": session})
        self.assertEqual(asked.returncode, 0, asked.stderr)
        field = wait_until("the ask's field in the row", lambda: driver.find_all("article.control-row .request.is-ask .request-field") or None)
        driver.type_into(field[0], "keep them" + ENTER)
        typed = 'Answer to your question "Keep the old routes?": keep them'
        try:
            wait_until("the answer typed into the session, with its question", lambda: prompts_of(session)[-1:] == [typed], timeout=30)
        except AssertionError as error:
            raise AssertionError(f"{error}; the session's last prompts: {prompts_of(session)[-2:]!r}") from error
        self.answered()


class HookGoneLeavesTheOverview(Journey):
    """A permission whose hook still waits stays in the control center, however long it waits; once
    Claude Code ends that hook without a word (killed with its session), the request leaves the
    page instead of taking an answer nobody reads (356, case of 2026-11-18)."""

    started = [("asks", "a task that asks work:0")]

    def requests(self) -> list[str]:
        return texts(self.driver, "article.control-row .request")

    def test_waiting_hook_stays_killed_hook_goes(self) -> None:
        driver = self.driver
        carry_messages(driver)
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        wait_until("the session's row", lambda: texts(driver, "article.control-row .row-name") == ["asks"])
        hook = asked_by_hook(session_id_of("asks"), "Bash", {"command": "touch e2e-made.txt", "description": "Create a file"})
        wait_until("the command in the row", lambda: any("touch e2e-made.txt" in text for text in texts(driver, "article.control-row .request-command")))
        # Longer than the 10 s after which a request nobody touches is taken for gone.
        time.sleep(13)
        self.assertEqual(len(self.requests()), 1, "the hook still waits: the request stays")
        self.assertEqual(len(overview_requests()), 1)

        hook.kill()
        hook.wait()
        wait_until("the request gone from the row", lambda: self.requests() == [] or None, timeout=20)
        self.assertEqual(overview_requests(), [], "its file goes too")
        wait_until("nothing needs you", lambda: texts(driver, "h2.control-group.is-need") == [] or None, timeout=10)


class AskLeavesWhenHidden(Journey):
    """A `kindric ask` has no hook that could stop waiting: once the user hides its session
    (Ctrl+Shift+K), nobody answers it any more, and it leaves the control center (356)."""

    started = [("asks", "a task that asks work:0")]

    def test_hiding_the_session_drops_its_question(self) -> None:
        driver = self.driver
        session = session_id_of("asks")
        asked = subprocess.run(["kindric", "ask", "Keep the old routes?", "Asked by the end-to-end journey."],
                               cwd=PROJECT, capture_output=True, text=True, timeout=10, env={**os.environ, "CLAUDE_CODE_SESSION_ID": session})
        self.assertEqual(asked.returncode, 0, asked.stderr)
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        wait_until("the question in the row", lambda: driver.find_all("article.control-row .request.is-ask") or None)
        self.assertEqual(len(overview_requests()), 1)

        driver.press(CONTROL, SHIFT, "1")
        wait_until("the first tab shown", lambda: overview_hidden(driver))
        open_from_bar(driver, "asks")
        driver.press(CONTROL, SHIFT, "k")
        wait_until("the question before hiding", lambda: dialog_open(driver, "confirm-hide"))
        driver.click(driver.find("#hide-form button[type=submit]"))
        wait_until("the session stopped", lambda: not fake_running("asks"))
        wait_until("its question's file gone", lambda: overview_requests() == [] or None, timeout=10)

        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center again", lambda: not overview_hidden(driver))
        wait_until("no question left in the page", lambda: driver.find_all("article.control-row .request") == [] or None, timeout=10)


class SetupLeftWithSessions(Journey):
    """The messages server is missing but sessions exist: the welcome list no longer covers the
    control center, one line on it says what is left and opens F1, whose "add" makes it go (452)."""

    started = [("already-here", "a task work:0")]

    def test_the_control_center_stays_with_a_line(self) -> None:
        driver = self.driver
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        line = wait_until("the line of what is left", lambda: texts(driver, ".control-setup") or None)
        self.assertEqual(line, ["1 thing left to set up · F1 shows it"])
        self.assertTrue(driver.script("return document.getElementById('welcome').hidden"))
        wait_until("the session's row", lambda: texts(driver, "article.control-row .row-name") == ["already-here"])

        click_text(driver, ".control-setup", line[0])
        wait_until("the help with the list", lambda: dialog_open(driver, "help"))
        wait_until("its add", lambda: "add" in texts(driver, "#help-checks button"))
        click_text(driver, "#help-checks button", "add")
        wait_until("the line gone once added", lambda: texts(driver, ".control-setup") == [] or None)

# The mark of each tab of the bar, as the page draws it: [name, state class, sign in its dot].
TAB_MARKS = """return [...document.querySelectorAll('#tabs button.tab')].map((tab) => {
  const dot = tab.querySelector('.dot');
  const state = ['asking', 'finished', 'waiting'].find((name) => tab.classList.contains(name)) ?? '';
  return [tab.querySelector('.tab-name').textContent, state, dot ? dot.textContent : ''];
});"""


def tab_mark(driver: Driver, name: str) -> tuple[str, str] | None:
    """The state and the sign the tab `name` shows, None without that tab."""
    marks: list[list[str]] = driver.script(TAB_MARKS)
    return next(((str(state), str(sign)) for tab, state, sign in marks if tab == name), None)


def dot_sign(driver: Driver, title: str) -> str | None:
    """The sign in the dot of the session's chip: "?", "✓" or ""; None while it is not in the bar."""
    found = driver.script("""const chip = [...document.querySelectorAll('#chips li.chip:not(.more)')].find(
      (chip) => chip.querySelector('.chip-title').textContent === arguments[0]);
    return chip ? chip.querySelector('.dot').textContent : null;""", title)
    return None if found is None else str(found)


class QuestionFirstInTheBar(Journey):
    """A session that waits for the user's permission shows "?" before a finished turn, in the bar
    and on its tab, and is not marked finished meanwhile (355). Answered, it works, then shows its
    finished turn. A `kindric ask` it leaves puts the "?" back at once, though no account changed."""

    started = [("finished-one", "a short task work:6"), ("asks-one", "a task that needs a permission work:9 ask:permission_prompt")]

    def titles(self) -> list[str]:
        chips: list[list[Any]] = self.driver.script(CHIPS)
        return [str(title) for _, title, _ in chips]

    def test_question_first_then_answered_then_asked_again(self) -> None:
        driver = self.driver
        wait_until("the finished turn", lambda: chip_state(driver, "finished-one") == "finished", timeout=30)
        wait_until("the question in the bar", lambda: chip_state(driver, "asks-one") == "asking", timeout=30)
        self.assertEqual(self.titles(), ["asks-one", "finished-one"])
        self.assertEqual(dot_sign(driver, "asks-one"), "?")
        self.assertEqual(dot_sign(driver, "finished-one"), "✓")
        self.assertEqual(tab_mark(driver, "other"), ("asking", "?"))

        session = next(session for session in fake_sessions() if session.get("name") == "asks-one")
        answered = subprocess.run(["kindric-accounts", "run", ACCOUNT, "attach", session["id"]], input="yes, go on work:4\n",
                                  cwd=PROJECT, capture_output=True, text=True, timeout=30)
        self.assertEqual(answered.returncode, 0, answered.stderr)
        wait_until("the answered session at work", lambda: chip_state(driver, "asks-one") == "working")
        self.assertEqual(tab_mark(driver, "other"), ("finished", "✓"))
        wait_until("its turn finished", lambda: chip_state(driver, "asks-one") == "finished", timeout=30)

        asked = subprocess.run(["kindric", "ask", "Keep the old routes?", "Asked by the end-to-end journey."],
                               cwd=PROJECT, capture_output=True, text=True, timeout=10,
                               env={**os.environ, "CLAUDE_CODE_SESSION_ID": session["sessionId"]})
        self.assertEqual(asked.returncode, 0, asked.stderr)
        wait_until("the ask's question in the bar", lambda: chip_state(driver, "asks-one") == "asking", timeout=10)
        self.assertEqual(self.titles()[0], "asks-one")


OVERVIEW = os.path.join(HOME, ".local/share/kindric/overview")


def overview_requests() -> list[str]:
    """The requests waiting in kindric's overview, as files."""
    return [name for _, _, files in os.walk(OVERVIEW) for name in files if name.endswith(".json")]


class AskAnsweredInTheSession(Journey):
    """A `kindric ask` answered by typing into the session itself, not through kindric (363): the
    request leaves the overview and the "?" leaves the bar."""

    started = [("asks-in-session", "a short task work:1")]

    def test_typed_answer_clears_the_question(self) -> None:
        driver = self.driver
        wait_until("the finished turn", lambda: chip_state(driver, "asks-in-session") == "finished", timeout=30)
        session = next(session for session in fake_sessions() if session.get("name") == "asks-in-session")
        asked = subprocess.run(["kindric", "ask", "Keep the old routes?", "Asked by the end-to-end journey."],
                               cwd=PROJECT, capture_output=True, text=True, timeout=10,
                               env={**os.environ, "CLAUDE_CODE_SESSION_ID": session["sessionId"]})
        self.assertEqual(asked.returncode, 0, asked.stderr)
        wait_until("the question in the bar", lambda: dot_sign(driver, "asks-in-session") == "?", timeout=10)
        self.assertEqual(len(overview_requests()), 1)

        # A message of the same second as the question does not count as its answer.
        time.sleep(1.1)
        typed = subprocess.run(["kindric-accounts", "run", ACCOUNT, "attach", session["id"]], input="keep them work:1\n",
                               cwd=PROJECT, capture_output=True, text=True, timeout=30)
        self.assertEqual(typed.returncode, 0, typed.stderr)
        wait_until("the question gone from the overview", lambda: not overview_requests(), timeout=10)
        wait_until("the turn finished, no question left", lambda: dot_sign(driver, "asks-in-session") == "✓", timeout=30)


def turn_end(session: dict[str, Any], state: str, detail: str, text: str) -> None:
    """The end of a turn as Claude Code writes it in the job's timeline.jsonl, which the fake
    claude does not write."""
    path = os.path.join(ACCOUNT_DIR, "jobs", session["id"], "timeline.jsonl")
    with open(path, "a") as timeline:
        timeline.write(json.dumps({"at": "2026-11-18T10:03:50.523Z", "state": state, "detail": detail, "text": text}) + "\n")


def question_check(session: dict[str, Any]) -> None:
    """`kindric question-check` run as Claude Code runs a Stop hook of the session: the fake claude
    runs no hook."""
    event = json.dumps({"session_id": session["sessionId"], "hook_event_name": "Stop", "cwd": PROJECT, "stop_hook_active": False})
    done = subprocess.run(["kindric", "question-check"], input=event, cwd=PROJECT, capture_output=True, text=True, timeout=10,
                          env={**os.environ, "CLAUDE_JOB_DIR": os.path.join(ACCOUNT_DIR, "jobs", session["id"])})
    if done.returncode != 0 or done.stdout or done.stderr:
        raise AssertionError(f"question-check exited {done.returncode}: {done.stdout!r} {done.stderr!r}")


class QuestionOfTheReply(Journey):
    """A question the session wrote in its reply, not with `kindric ask`, reaches the control center
    at the end of its turn (366); answered there, it is typed into the session. A turn that asks
    nothing puts nothing there."""

    started = [("replies", "a task that ends with a question work:0")]

    def test_question_of_the_reply_answered_from_the_page(self) -> None:
        driver = self.driver
        carry_messages(driver)
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        wait_until("the session's row", lambda: texts(driver, "article.control-row .row-name") == ["replies"])
        session = next(each for each in fake_sessions() if each.get("name") == "replies")

        turn_end(session, "done", "menu finished", "The menu is ready.")
        question_check(session)
        self.assertEqual(overview_requests(), [], "a turn that asks nothing")

        turn_end(session, "blocked", "Which colour for the buttons?", "The menu is ready.\n\nWhich colour for the buttons?")
        question_check(session)
        wait_until("the question in the row", lambda: texts(driver, "article.control-row .request.is-ask .request-question") == ["Which colour for the buttons?"])
        self.assertEqual(texts(driver, "article.control-row .request.is-ask .request-context"), ["The menu is ready."])
        self.assertEqual(texts(driver, "article.control-row .row-badge.keeps"), [], "its turn is over: it does not work meanwhile")
        question_check(session)
        self.assertEqual(len(overview_requests()), 1, "asked once")
        driver.screenshot(os.path.join(OUT, "QuestionOfTheReply-asked.png"))

        field = wait_until("the answer field", lambda: driver.find_all("article.control-row .request.is-ask .request-field") or None)
        driver.type_into(field[0], "orange" + ENTER)
        typed = 'Answer to your question "Which colour for the buttons?": orange'
        wait_until("the answer typed into the session", lambda: prompts_of(session["sessionId"])[-1:] == [typed], timeout=30)
        wait_until("the request gone from the row", lambda: texts(driver, "article.control-row .request") == [] or None)


ISOLATED = "solo"


def sessions_of(account: str) -> list[dict[str, Any]]:
    """What the fake claude knows of the sessions of `account`."""
    try:
        with open(os.path.join(HOME, ".local/share/kindric-accounts", account, "fake/sessions.json")) as source:
            sessions: list[dict[str, Any]] = json.load(source)
            return sessions
    except (OSError, ValueError):
        return []


def settings_of(session: dict[str, Any]) -> dict[str, Any]:
    """The settings file the session was started with (--settings)."""
    args = [str(arg) for arg in session.get("args", [])]
    if "--settings" not in args:
        raise AssertionError(f"started without --settings: {args}")
    with open(args[args.index("--settings") + 1]) as source:
        settings: dict[str, Any] = json.load(source)
        return settings


def hook_commands(settings: dict[str, Any]) -> dict[str, list[str]]:
    """Each hook event of the settings with the commands it runs."""
    return {event: [hook["command"] for matcher in matchers for hook in matcher.get("hooks", [])]
            for event, matchers in settings.get("hooks", {}).items()}


def away_hook(event: str, session_id: str, command: str | None = None, **fields: Any) -> str:
    """What `kindric away-hook` answers Claude Code for this event of the session; `fields` are
    added to what Claude Code gives the hook."""
    asked: dict[str, Any] = {"hook_event_name": event, "session_id": session_id, **fields}
    if command is not None:
        asked.update(tool_name="Bash", tool_input={"command": command})
    done = subprocess.run(["kindric", "away-hook"], input=json.dumps(asked), capture_output=True, text=True, timeout=30)
    return done.stdout


class AwayOnAnIsolatedAccount(Journey):
    """A session of an isolated account, started from Ctrl+N with the account picked in the list,
    gets the hook of the absence and none of the messages. Ctrl+Shift+Y then says the user is away:
    the session is told so in the user's name, and the hook keeps it working and refuses a push."""

    @classmethod
    def prepare(cls) -> None:
        accounts("add", ISOLATED)
        accounts("isolate", ISOLATED)

    def test_isolated_session_away(self) -> None:
        driver = self.driver
        driver.press(CONTROL, "n")
        wait_until("the new session form", lambda: dialog_open(driver, "new-session"))
        wait_until("both accounts in the form", lambda: len(texts(driver, "#new-account .launch-list li")) == 2)
        driver.press(ARROW_DOWN)
        wait_until("the isolated account picked", lambda: texts(driver, "#new-account .launch-list li.selected")[0].startswith(ISOLATED))
        directory = driver.find("#new-form input[name=directory]")
        driver.clear(directory)
        driver.type_into(directory, PROJECT)
        driver.type_into(driver.find("#new-form input[name=name]"), "alone" + ENTER)
        wait_until("the session open in a pane", lambda: (chip(driver, "alone") or ("", False))[1])

        session = wait_until("the session of the isolated account", lambda: next((s for s in sessions_of(ISOLATED) if s.get("name") == "alone"), None))
        hooks = hook_commands(settings_of(session))
        for event in ("Stop", "PreToolUse", "PermissionRequest"):
            self.assertTrue(any("away-hook" in command for command in hooks.get(event, [])), f"no away-hook on {event}: {hooks}")
        every = [command for commands in hooks.values() for command in commands]
        self.assertFalse([command for command in every if "wait-message" in command or "unread" in command], hooks)

        driver.press(CONTROL, SHIFT, "y")
        wait_until("the away form", lambda: dialog_open(driver, "away"))
        driver.click(driver.find("#away-form button[type=submit]"))
        wait_until("the form closed", lambda: not dialog_open(driver, "away"))
        wait_until("away until, in the notice", lambda: "away until" in driver.text(driver.find("#status")))
        wait_until("the session told in the user's name", lambda: next((str(s.get("detail", "")) for s in sessions_of(ISOLATED) if s.get("name") == "alone"), "").startswith("working on: I am away for the next 60 minutes"))

        session_id = str(session["sessionId"])
        self.assertIn('"decision":"block"', away_hook("Stop", session_id))
        self.assertIn('"permissionDecision":"deny"', away_hook("PreToolUse", session_id, "git push"))
        self.assertEqual(away_hook("PreToolUse", session_id, "cargo test"), "")

        # Nothing left without the user: what it needs goes to the overview, and the session may
        # stop until that is answered; its commands are still checked (433).
        blocked = subprocess.run(["kindric", "away", "blocked", session_id, "the key of the bench"],
                                 cwd=PROJECT, capture_output=True, text=True, timeout=10)
        self.assertEqual(blocked.returncode, 0, blocked.stderr)
        self.assertIn("You may end your turn now", blocked.stdout)
        wait_until("the need in the bar", lambda: dot_sign(driver, "alone") == "?", timeout=10)
        self.assertEqual(away_hook("Stop", session_id), "")
        self.assertIn('"permissionDecision":"deny"', away_hook("PreToolUse", session_id, "git push"))
        requests = [os.path.join(folder, name) for folder, _, files in os.walk(OVERVIEW) for name in files if name.endswith(".json")]
        self.assertEqual(len(requests), 1, requests)
        with open(requests[0]) as request:
            self.assertEqual(json.load(request)["question"], "the key of the bench")
        # Answered, the request goes as the page settles it: the session works on the answer.
        os.remove(requests[0])
        self.assertIn('"decision":"block"', away_hook("Stop", session_id))

        # Relaunched by the hook, the session ran no tool: it may stop, its last words in the
        # overview, instead of being relaunched in a loop (435).
        transcript = os.path.join(OUT, "away-idle.jsonl")
        with open(transcript, "w") as lines:
            lines.write(json.dumps({"type": "user", "isMeta": True, "message": {"role": "user",
                        "content": "Stop hook feedback:\nThe user is away for 50 more minutes and asked you to keep working."}}) + "\n")
            lines.write(json.dumps({"type": "assistant", "message": {"role": "assistant",
                        "content": [{"type": "text", "text": "Still waiting on the key."}]}}) + "\n")
        self.assertEqual(away_hook("Stop", session_id, stop_hook_active=True, transcript_path=transcript, cwd=PROJECT), "")
        wait_until("the idle turn in the overview", lambda: len(overview_requests()) == 1, timeout=10)
        self.assertEqual(away_hook("Stop", session_id, stop_hook_active=True, transcript_path=transcript, cwd=PROJECT), "", "still waiting")
        self.assertEqual(len(overview_requests()), 1, "asked once")

    @classmethod
    def tearDownClass(cls) -> None:
        super().tearDownClass()
        for session in sessions_of(ISOLATED):
            if session.get("pid"):
                subprocess.run(["kindric-accounts", "run", ISOLATED, "stop", session["id"]], cwd=PROJECT, capture_output=True, timeout=30)


# What the fake xdg-open was handed, one line per call (tests/e2e/bin/xdg-open).
OPENED = os.path.join(HOME, "xdg-open.log")


def opened() -> list[str]:
    try:
        with open(OPENED) as log:
            return log.read().splitlines()
    except OSError:
        return []


def artifact_heads(driver: Driver) -> list[str]:
    """The groups of the artifacts window, as "name · count"."""
    return texts(driver, "#artifacts li.artifact-group")


def artifact_row(driver: Driver, name: str) -> str:
    """The row of the file `name` in the artifacts window."""
    found = driver.script("""return [...document.querySelectorAll('#artifacts li.artifact')].find(
      (row) => row.querySelector('.artifact-name')?.textContent === arguments[0]);""", name)
    if not isinstance(found, dict) or ELEMENT_KEY not in found:
        raise AssertionError(f"no file {name} in the artifacts window")
    return str(found[ELEMENT_KEY])


def artifact_head(driver: Driver, group: str) -> str:
    found = driver.script("return document.querySelector(`#artifacts li.artifact-group[data-group='${arguments[0]}']`);", group)
    if not isinstance(found, dict) or ELEMENT_KEY not in found:
        raise AssertionError(f"no group {group} in the artifacts window")
    return str(found[ELEMENT_KEY])


class ArtifactsWindow(Journey):
    """Two files a session kept for the user, in the window of Ctrl+Shift+A: a new group, a file
    dragged onto it, the group renamed, a file opened with the system's app, the group removed and
    its file back in unsorted."""

    started = [("maker", "make touch:notes.txt touch:plan.md work:0")]

    @classmethod
    def prepare(cls) -> None:
        if os.path.exists(OPENED):
            os.remove(OPENED)

    def test_group_drag_rename_open_remove(self) -> None:
        driver = self.driver
        add_project(driver)
        wait_until("the files of the session", lambda: all(os.path.exists(os.path.join(PROJECT, name)) for name in ("notes.txt", "plan.md")))
        for name in ("notes.txt", "plan.md"):
            mcp_call(session_id_of("maker"), "keep_artifact", {"path": name, "for": "user"})

        driver.press(CONTROL, SHIFT, "a")
        wait_until("the artifacts window", lambda: dialog_open(driver, "artifacts"))
        wait_until("both files in unsorted", lambda: artifact_heads(driver) == ["unsorted · 2"])

        driver.press("n")
        field = wait_until("the name of the new group", lambda: driver.find("#artifacts form.artifact-naming input"))
        driver.type_into(field, "drafts" + ENTER)
        wait_until("the new group", lambda: "drafts · 0" in artifact_heads(driver))

        driver.drag(artifact_row(driver, "notes.txt"), artifact_head(driver, "drafts"))
        wait_until("the file dragged into the group", lambda: "drafts · 1" in artifact_heads(driver))
        self.assertIn("unsorted · 1", artifact_heads(driver))

        driver.click(artifact_head(driver, "drafts"))
        driver.press("r")
        field = wait_until("the name of the group to rename", lambda: driver.find("#artifacts form.artifact-naming input"))
        driver.clear(field)
        driver.type_into(field, "final" + ENTER)
        wait_until("the group renamed", lambda: "final · 1" in artifact_heads(driver))

        driver.click(artifact_row(driver, "plan.md"))
        found = wait_until("plan.md handed to the system's app", lambda: [line for line in opened() if line.endswith("/plan.md")] or None)
        self.assertTrue(os.path.isfile(found[0]), found)

        driver.click(artifact_head(driver, "final"))
        driver.press(DELETE)
        wait_until("the question before removing", lambda: "remove the group final" in driver.text(driver.find("#artifacts .artifacts-body")))
        driver.press(ENTER)
        wait_until("the group gone, its file back in unsorted", lambda: artifact_heads(driver) == ["unsorted · 2"])


# When the fake claude says the login of the account runs out, and its log of `auth login`.
LOGIN_EXPIRES = os.path.join(ACCOUNT_DIR, "fake/login-expires")
LOGINS = os.path.join(ACCOUNT_DIR, "fake/logins.log")


def logins() -> int:
    try:
        with open(LOGINS) as log:
            return len(log.read().splitlines())
    except OSError:
        return 0


def account_actions(driver: Driver, account: str) -> list[str]:
    """The buttons of the account's row in the settings."""
    found: list[str] = driver.script("""const row = [...document.querySelectorAll('#settings li.account')].find(
      (item) => item.querySelector('.account-name')?.textContent === arguments[0]);
      return row ? [...row.querySelectorAll('.account-actions button')].map((button) => button.textContent) : [];""", account)
    return [str(text) for text in found]


def write_task(folder: str, number: int, subject: str, status: str, at: float, about: str = "") -> None:
    """A task as Claude Code writes it for TaskCreate and TaskUpdate, last changed at `at`."""
    path = os.path.join(folder, f"{number}.json")
    with open(path, "w") as task:
        json.dump({"id": str(number), "subject": subject, "description": about, "activeForm": f"{subject}…",
                   "status": status, "blocks": [], "blockedBy": []}, task)
    os.utime(path, (at, at))


class TaskListOnThePage(Journey):
    """A session's task list (Claude Code's TaskCreate and TaskUpdate) shows on its row: the task in
    progress, how many are done and the time left; its detail lists them all. kindric-accounts gives
    the session the hook that has a long turn make that list (368)."""

    # At rest by the time its tasks are written: after a restart, only the full list reads them.
    started = [("builds", "a task work:1")]

    def test_task_in_progress_count_and_time_left(self) -> None:
        driver = self.driver
        carry_messages(driver)
        session = next(each for each in fake_sessions() if each.get("name") == "builds")
        self.assertTrue(any(command.endswith(" task-check") for command in hook_commands(settings_of(session)).get("PreToolUse", [])),
                        "the session was started with the task-check hook")
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        wait_until("the session's row", lambda: texts(driver, "article.control-row .row-name") == ["builds"])
        wait_until("the session at rest", lambda: fake_detail("builds") == "waiting for you", timeout=30)

        folder = os.path.join(ACCOUNT_DIR, "tasks", session["sessionId"])
        os.makedirs(folder)
        start = time.time() - 600
        open(os.path.join(folder, ".lock"), "w").close()
        os.utime(os.path.join(folder, ".lock"), (start, start))
        write_task(folder, 1, "Write the reader", "completed", start + 120)
        # The task in progress changed in the turn: a list none of whose tasks did is left out (499).
        write_task(folder, 2, "Write the statistics", "in_progress", time.time(), "hours, codes, top paths")
        write_task(folder, 3, "Write the README", "pending", start)
        # Claude Code writes the job's state at each tool, which wakes kindric's list.
        os.utime(os.path.join(ACCOUNT_DIR, "jobs", session["id"], "state.json"))
        wait_until("the task under the row", lambda: texts(driver, "article.control-row .row-task") == ["▸ Write the statistics… · hours, codes, top paths"] or None, timeout=40)
        self.assertEqual(texts(driver, "article.control-row .row-badge.tasks"), ["1/3 · ~4 min left"])

        # At start, kindric lists every session at once: the list is read there too.
        type(self).restart()
        driver = self.driver
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center again", lambda: not overview_hidden(driver))
        wait_until("the task under the row after a restart", lambda: texts(driver, "article.control-row .row-task") == ["▸ Write the statistics… · hours, codes, top paths"] or None, timeout=20)

        click_text(driver, "article.control-row .row-name", "builds")
        wait_until("the list in the detail", lambda: texts(driver, ".detail-tasks .task") or None)
        self.assertEqual(texts(driver, ".detail-tasks .detail-title"), ["tasks · 1/3 · ~4 min left"])
        self.assertEqual(texts(driver, ".detail-tasks .task"), ["✓ Write the reader", "▸ Write the statistics", "○ Write the README"])
        self.assertEqual(texts(driver, ".detail-tasks .task-about"), ["hours, codes, top paths"])
        driver.screenshot(os.path.join(OUT, "TaskListOnThePage-detail.png"))

        for number, subject in [(2, "Write the statistics"), (3, "Write the README")]:
            write_task(folder, number, subject, "completed", time.time())
        os.utime(os.path.join(ACCOUNT_DIR, "jobs", session["id"], "state.json"))
        wait_until("the list gone once all is done", lambda: not texts(driver, ".detail-tasks .task") or None, timeout=40)
        self.assertEqual(texts(driver, "article.control-row .row-badge.tasks"), [], "no badge once all is done")
        self.assertEqual(texts(driver, "article.control-row .row-task"), [])

class RenewFromSettings(Journey):
    """A login that runs out in two days: the settings offer "renew" on the account's row; it runs
    Claude Code's `auth login` in the helper window, and once done the row no longer offers it.
    The browser part of a real login is not imitated: the fake signs in at once."""

    @classmethod
    def prepare(cls) -> None:
        soon = time.gmtime(time.time() + 2 * 86400)
        os.makedirs(os.path.dirname(LOGIN_EXPIRES), exist_ok=True)
        with open(LOGIN_EXPIRES, "w") as out:
            out.write(time.strftime("%Y-%m-%dT%H:%M:%S.000Z", soon))

    def test_renew_then_no_longer_offered(self) -> None:
        driver = self.driver
        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "accounts")
        wait_until("renew on the account's row", lambda: "renew" in account_actions(driver, ACCOUNT))
        self.assertEqual(logins(), 0)

        click_text(driver, "#settings li.account .account-actions button", "renew")
        wait_until("the login window", lambda: dialog_open(driver, "helper"))
        wait_until("Claude Code's login run", lambda: logins() == 1)
        # The window closes by itself once Claude Code's login ends; the fake one ends at once.
        if dialog_open(driver, "helper"):
            driver.click(driver.find("#helper-done"))
        wait_until("the login window closed", lambda: not dialog_open(driver, "helper"))
        wait_until("renew no longer offered", lambda: "renew" not in account_actions(driver, ACCOUNT) and account_actions(driver, ACCOUNT))


# A memory of one project for the atlas: where kindric looks without KINDRIC_MEMORY_DIR.
MEMORY = os.path.join(HOME, ".kindric/memory")


def note(path: str, text: str) -> None:
    full = os.path.join(MEMORY, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w") as out:
        out.write(text)


class AtlasZoomWithCtrlWheel(Journey):
    """The memory atlas, opened from the settings: a wheel event with Ctrl, what WebKit makes of a
    pinch of the touchpad, zooms into the project under the pointer. Neither a real pinch nor a
    wheel with Ctrl held can be sent through WebKit's WebDriver."""

    @classmethod
    def prepare(cls) -> None:
        shutil.rmtree(MEMORY, ignore_errors=True)
        note("projets/demo/INDEX.md", "- status/demo.md : where demo is\n- decisions/2026-11-01-001.md : first choice\n")
        note("projets/demo/status/demo.md", "---\ndescription: where demo is\nupdated: 2026-11-02\n---\n\n- all good\n")
        note("projets/demo/decisions/2026-11-01-001.md", "---\ndescription: first choice\nupdated: 2026-11-01\n---\n\nwe chose\n")

    def caption(self) -> str:
        return self.driver.text(self.driver.find("#atlas .atlas-caption"))

    def test_ctrl_wheel_goes_in(self) -> None:
        driver = self.driver
        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "memory")
        driver.click(driver.find("#settings .memory-atlas-open"))
        wait_until("the atlas", lambda: dialog_open(driver, "atlas"))
        wait_until("the whole map", lambda: self.caption().startswith("type to search"))
        # The map takes the wheel once the memory is read: the cartouche then counts it.
        wait_until("the memory read", lambda: "2 files" in driver.text(driver.find("#atlas .atlas-cartouche-count")))

        # WebKit's driver drops the held Ctrl from a wheel action (the page sees ctrlKey false), so
        # the wheel event WebKit makes of a pinch is handed to the canvas from the page itself.
        driver.script("""const canvas = arguments[0];
          const box = canvas.getBoundingClientRect();
          canvas.dispatchEvent(new WheelEvent('wheel', { deltaY: -100, ctrlKey: true, bubbles: true, cancelable: true,
            clientX: box.left + box.width / 2, clientY: box.top + box.height / 2 }));""", {ELEMENT_KEY: driver.find("#atlas canvas.atlas-map")})
        # A project of a single system opens on that system (setFocus): its planets to click.
        try:
            wait_until("zoomed into the project, on its system", lambda: self.caption().startswith("click a planet"))
        except AssertionError as error:
            raise AssertionError(f"{error}; the caption says: {self.caption()!r}") from error

    @classmethod
    def tearDownClass(cls) -> None:
        super().tearDownClass()
        shutil.rmtree(MEMORY, ignore_errors=True)


def age_transcript(name: str, days: int) -> None:
    """Makes the session `name` last active `days` ago: kindric reads that from its transcript."""
    session_id = session_id_of(name)
    found = [path for folder in os.listdir(os.path.join(ACCOUNT_DIR, "projects"))
             if os.path.exists(path := os.path.join(ACCOUNT_DIR, "projects", folder, f"{session_id}.jsonl"))]
    if not found:
        raise AssertionError(f"no transcript for {name}")
    then = time.time() - days * 86400
    for path in found:
        os.utime(path, (then, then))


class PurgeFromSettings(Journey):
    """The settings purge the sessions idle for more than 7 days, after a question that counts
    them: one goes, one Claude Code refuses (commits not pushed) stays with its reason, and the
    recent one stays."""

    started = [("old-done", "old work:1"), ("unpushed-old", "kept work:1"), ("recent", "new work:1")]

    @classmethod
    def before_app(cls) -> None:
        wait_until("the turns over", lambda: all(fake_detail(name) not in (None, "") and not fake_detail(name).startswith("working")
                                                 for name, _ in cls.started))
        age_transcript("old-done", 40)
        age_transcript("unpushed-old", 10)

    def purge_text(self, css: str) -> str:
        return self.driver.text(self.driver.find(f"#settings .purge {css}"))

    def test_purge_older_than_seven_days(self) -> None:
        driver = self.driver
        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "sessions")
        driver.click(driver.find("#settings .purge button[data-days='7']"))
        question = wait_until("the question", lambda: self.purge_text(".purge-question"))
        self.assertTrue(question.startswith("2 sessions of work, idle for more than 7 days"), question)
        driver.screenshot(os.path.join(OUT, "PurgeFromSettings-question.png"))

        driver.click(driver.find("#settings .purge .purge-go"))
        result = wait_until("the result", lambda: (text := self.purge_text(".purge-message")) != "purging…" and text)
        self.assertEqual(result, "1 session removed; 1 refused by Claude Code, kept:")
        self.assertIn("unpushed-old (work): ", self.purge_text(".purge-refused"))
        self.assertIn("commits not pushed", self.purge_text(".purge-refused"))
        names = sorted(session.get("name", "") for session in fake_sessions())
        self.assertEqual(names, ["recent", "unpushed-old"], "the old one removed, the others kept")
        wait_until("the removed one gone from the bar", lambda: chip_state(driver, "old-done") is None, timeout=10)
        self.assertIsNotNone(chip_state(driver, "unpushed-old"), "the refused one stays in the bar")
        self.assertTrue(fake_running("unpushed-old"), "the refused one keeps running")


MAILBOX = os.path.join(HOME, ".local/share/kindric/mailbox")


def leave_message(to: str, text: str, age_seconds: int) -> None:
    """A message to the session `to`, sent `age_seconds` ago, written as kindric writes them."""
    sent_at = int(time.time()) - age_seconds
    inbox = os.path.join(MAILBOX, session_id_of(to))
    os.makedirs(inbox, mode=0o700, exist_ok=True)
    message = {"from_session": "e2e-sender", "from_title": "sender", "from_account": ACCOUNT,
               "sent_at": sent_at, "text": text, "wake": False}
    with open(os.path.join(inbox, f"{sent_at:020}-1-{0:020}.json"), "w", encoding="utf-8") as file:
        json.dump(message, file)


def pause(names: list[str]) -> None:
    """Lets the turns of these sessions end, then stops them: they stay, paused."""
    wait_until("the turns over", lambda: all(fake_detail(name) not in (None, "") and not fake_detail(name).startswith("working")
                                             for name in names))
    for name in names:
        session = next(session for session in fake_sessions() if session.get("name") == name)
        accounts("run", ACCOUNT, "stop", session["id"])
    wait_until("the sessions stopped", lambda: not any(fake_running(name) for name in names))


def open_picker(driver: Driver) -> None:
    driver.press(CONTROL, "p")
    wait_until("the session finder", lambda: dialog_open(driver, "picker"))


def picker_badges(driver: Driver) -> dict[str, str | None]:
    """The unread count each row of Ctrl+P shows, None without one."""
    found: list[list[Any]] = driver.script("""return [...document.querySelectorAll('#picker-results li.session')].map(
      (row) => [row.querySelector('.name').textContent, row.querySelector('.badge')?.textContent ?? null]);""")
    return {str(name): None if badge is None else str(badge) for name, badge in found}


class QuietOldMessages(Journey):
    """A paused session counts only the messages of the last day (decision 375): an old message
    no longer gives it a badge, a recent one does, and Ctrl+Shift+M still shows the old one."""

    started = [("quiet-old", "old talk work:0"), ("quiet-new", "new talk work:0")]

    @classmethod
    def before_app(cls) -> None:
        pause(["quiet-old", "quiet-new"])
        leave_message("quiet-old", "sent two days ago", 2 * 86400)
        leave_message("quiet-new", "sent a minute ago", 60)

    def test_old_message_no_badge(self) -> None:
        driver = self.driver
        open_picker(driver)
        badges = wait_until("both sessions in the finder", lambda: (found := picker_badges(driver)).keys() >= {"quiet-old", "quiet-new"} and found)
        self.assertEqual(badges["quiet-new"], "1 msg", "a message of the last minute counts")
        self.assertIsNone(badges["quiet-old"], "a message of two days ago no longer counts")
        self.assertEqual(len(os.listdir(os.path.join(MAILBOX, session_id_of("quiet-old")))), 1, "the old message is kept")


class PausedLeavesTheControlCenter(Journey):
    """The control center shows the sessions of the bar (decision 497): a paused session is not on
    it, however recent its pause, and Ctrl+P still finds it."""

    started = [("napping", "nap work:0"), ("dormant", "old work:0"), ("awake", "awake work:0")]

    @classmethod
    def before_app(cls) -> None:
        pause(["napping", "dormant"])
        age_transcript("dormant", 5)

    def test_paused_not_shown(self) -> None:
        driver = self.driver
        carry_messages(driver)
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        wait_until("awake alone", lambda: texts(driver, "article.control-row .row-name") == ["awake"] or None)
        self.assertEqual(texts(driver, "h2.control-group.is-paused"), [], "no group of pauses")
        driver.screenshot(os.path.join(OUT, "PausedLeavesTheControlCenter.png"))
        driver.press(CONTROL, "p")
        wait_until("the session finder", lambda: dialog_open(driver, "picker"))
        wait_until("the paused ones in the finder", lambda: {"napping", "dormant"} <= {name for name, _ in picker_rows(driver)} or None)


def picker_entries(driver: Driver) -> list[tuple[str, str]]:
    """Ctrl+P from top to bottom: ("head", day) for a head, (name, when) for a session."""
    found: list[list[str]] = driver.script("""return [...document.querySelectorAll('#picker-results li')].map((item) =>
      item.classList.contains('picker-day') ? ['head', item.textContent]
        : [item.querySelector('.name').textContent, item.querySelector('.when')?.textContent ?? '']);""")
    return [(str(first), str(second)) for first, second in found]


def history_rows(driver: Driver) -> list[tuple[str, str, list[str], str]]:
    """What Ctrl+Shift+F lists: (name, excerpt, its marked words, count) for each session."""
    rows: list[list[Any]] = driver.script("""return [...document.querySelectorAll('#history .history-results li.session')].map((row) => [
      row.querySelector('.name').textContent, row.querySelector('.history-excerpt').textContent,
      [...row.querySelectorAll('.history-excerpt mark')].map((mark) => mark.textContent), row.querySelector('.history-count').textContent]);""")
    return [(str(name), str(excerpt), [str(word) for word in marked], str(count)) for name, excerpt, marked, count in rows]


class SearchHistoryWithCtrlShiftF(Journey):
    """Ctrl+Shift+F finds the sessions whose conversation holds every word typed, with an excerpt
    and the words marked; Enter opens the one chosen in a pane. From the overview, every session
    is searched."""

    started = [("logo-maker", "say:drew_the_shop_logo work:0"), ("parser", "say:fixed_the_parser work:0")]

    def search(self, words: str) -> list[tuple[str, str, list[str], str]]:
        driver = self.driver
        driver.press(CONTROL, SHIFT, "f")
        wait_until("the history search", lambda: dialog_open(driver, "history"))
        driver.type_into(driver.find("#history .history-query"), words)
        return wait_until(f"sessions found for {words}", lambda: history_rows(driver) or None)

    def test_find_then_open(self) -> None:
        driver = self.driver
        wait_until("the turns over", lambda: all((fake_detail(name) or "working").startswith("working") is False for name, _ in self.started))
        rows = self.search("LOGO shop")
        driver.screenshot(os.path.join(OUT, "SearchHistoryWithCtrlShiftF-found.png"))
        self.assertEqual([name for name, *_ in rows], ["logo-maker"], rows)
        _, excerpt, marked, count = rows[0]
        self.assertEqual(excerpt, "done: say:drew_the_shop_logo work:0", "the last message: the fake claude ends its turn so")
        self.assertEqual([word.lower() for word in marked], ["shop", "logo"], rows)
        self.assertEqual(count, "2 messages", "the prompt and how the turn ended: with work:0, the fake claude says nothing between")

        driver.type_into(driver.find("#history .history-query"), ENTER)
        wait_until("the search closed", lambda: not dialog_open(driver, "history"))
        wait_until("the session open in a pane", lambda: (chip(driver, "logo-maker") or ("", False))[1])

        driver.press(CONTROL, SHIFT, "0")
        wait_until("the overview", lambda: not overview_hidden(driver))
        rows = self.search("parser")
        self.assertEqual([name for name, *_ in rows], ["parser"], rows)
        driver.press(ESCAPE)
        wait_until("the search closed", lambda: not dialog_open(driver, "history"))


class PickerByDay(Journey):
    """Ctrl+P lists the sessions by day of last activity (decision 376): today first, then
    yesterday, then older days, each under its head with its time; the hidden ones last, under
    their own head."""

    started = [("fresh", "today work:0"), ("day-before", "yesterday work:0"), ("week-ago", "old work:0"), ("put-away", "hidden work:0")]

    @classmethod
    def before_app(cls) -> None:
        pause([name for name, _ in cls.started])
        age_transcript("day-before", 1)
        age_transcript("week-ago", 6)
        prefs = {session_id_of("put-away"): {"hidden": True}}
        data = os.path.join(HOME, ".local/share/kindric")
        os.makedirs(data, mode=0o700, exist_ok=True)
        with open(os.path.join(data, "sessions.json"), "w", encoding="utf-8") as file:
            json.dump(prefs, file)

    def test_days_times_and_hidden_last(self) -> None:
        driver = self.driver
        open_picker(driver)
        entries = wait_until("every session in the finder", lambda: (found := picker_entries(driver)) and len([e for e in found if e[0] != "head"]) == 4 and found)
        driver.screenshot(os.path.join(OUT, "PickerByDay-finder.png"))
        names = [first for first, _ in entries if first != "head"]
        heads = [second for first, second in entries if first == "head"]
        self.assertEqual(names, ["fresh", "day-before", "week-ago", "put-away"], entries)
        self.assertEqual(heads[0], "today", entries)
        self.assertEqual(heads[-1], "hidden", entries)
        self.assertEqual(entries[0], ("head", "today"), "a head before the first session")
        self.assertEqual(len(heads), 4, f"today, yesterday, six days ago, hidden: {entries}")
        self.assertEqual(heads[1], "yesterday", entries)
        self.assertNotIn(heads[2], {"today", "yesterday", "hidden"}, "six days ago is named by its date")
        for first, second in entries:
            if first not in ("head", "put-away"):
                self.assertRegex(second, r"^\d{1,2}:\d{2}", f"{first} shows its time")
        hidden_when = next(second for first, second in entries if first == "put-away")
        # The system's language says how: "today 13:21", or "today 01:21 PM" as in the image.
        self.assertRegex(hidden_when, r"^today \d{1,2}:\d{2}( [AP]M)?$", "a hidden one shows its day and time")


def stop_sessions_of(account: str) -> None:
    for session in sessions_of(account):
        if session.get("pid"):
            subprocess.run(["kindric-accounts", "run", account, "stop", session["id"]], cwd=PROJECT, capture_output=True, timeout=30)


def mcp_start(account_dir: str) -> subprocess.CompletedProcess[str]:
    """`kindric mcp` started as Claude Code starts it for a session of the account in `account_dir`."""
    hello = {"jsonrpc": "2.0", "id": 1, "method": "initialize",
             "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "1"}}}
    return subprocess.run(["kindric", "mcp"], input=json.dumps(hello) + "\n", capture_output=True, text=True, timeout=30, cwd=PROJECT,
                          env=dict(os.environ, CLAUDE_CODE_SESSION_ID="00000000-0000-0000-0000-000000000000", CLAUDE_CONFIG_DIR=account_dir))


class IsolatedOutOfMessages(Journey):
    """A session of an isolated account shows in the bar, but stays out of the messages: the
    other sessions do not see it in list_sessions, cannot leave it a message by name or by id, and
    the messages server refuses to start in its account."""

    started = [("open", "a task work:0")]

    @classmethod
    def prepare(cls) -> None:
        accounts("add", ISOLATED)
        accounts("isolate", ISOLATED)

    @classmethod
    def before_app(cls) -> None:
        accounts("run", ISOLATED, "--bg", "-n", "alone", "a private task work:0")

    def test_isolated_session_unreachable(self) -> None:
        driver = self.driver
        wait_until("both sessions in the bar", lambda: chip_state(driver, "open") and chip_state(driver, "alone"))
        alone = next(session for session in sessions_of(ISOLATED) if session.get("name") == "alone")
        me = session_id_of("open")

        listed, refused = mcp_answer(me, "list_sessions", {})
        self.assertFalse(refused, listed)
        self.assertIn("open (you)", listed)
        self.assertNotIn("alone", listed)
        self.assertNotIn(alone["sessionId"], listed)

        for to in ("alone", alone["sessionId"], alone["id"]):
            said, refused = mcp_answer(me, "send_message", {"to": to, "text": "secret"})
            self.assertTrue(refused, f"a message reached the isolated session through {to!r}: {said}")
        self.assertFalse(os.path.exists(os.path.join(MAILBOX, alone["sessionId"])), "a mailbox for the isolated session")

        started = mcp_start(os.path.join(HOME, ".local/share/kindric-accounts", ISOLATED))
        self.assertNotEqual(started.returncode, 0, started.stdout)
        self.assertIn("isolated", started.stderr)
        self.assertNotIn('"id":1', started.stdout.replace(" ", ""))

    @classmethod
    def tearDownClass(cls) -> None:
        super().tearDownClass()
        stop_sessions_of(ISOLATED)


WAITS = os.path.join(HOME, ".local/share/kindric/wait")


class WokenByAMessage(Journey):
    """A session at rest stays at rest for a message of information, and wakes for one that asks
    it to act now: its `kindric wait-message` hook ends, and the session starts a turn told of the
    message, shown working in the bar."""

    started = [("sender", "write work:0"), ("sleeper", "rest work:0")]

    def test_rest_then_woken(self) -> None:
        driver = self.driver
        wait_until("the sleeper's turn over", lambda: chip_state(driver, "sleeper") in ("finished", "waiting"), timeout=30)
        sleeper = session_id_of("sleeper")
        wait_until("the sleeper waiting for a message", lambda: os.path.exists(os.path.join(WAITS, sleeper)))
        turns = len(prompts_of(sleeper))

        mcp_call(session_id_of("sender"), "send_message", {"to": "sleeper", "text": "the build is green"})
        time.sleep(5)
        self.assertEqual(len(prompts_of(sleeper)), turns, "woken by a message of information")
        self.assertNotEqual(chip_state(driver, "sleeper"), "working")

        said = mcp_call(session_id_of("sender"), "send_message", {"to": "sleeper", "text": "run the tests now", "wake": True})
        self.assertIn("It acts on it now", said)
        woken = wait_until("the sleeper's new turn", lambda: prompts_of(sleeper)[turns:] or None, timeout=15)
        self.assertIn("unread message", woken[0])
        self.assertIn("sender", woken[0])
        wait_until("the sleeper working in the bar", lambda: chip_state(driver, "sleeper") == "working", timeout=10)


WORKTREE = os.path.join(HOME, "wt")


def start_in_project(driver: Driver) -> str:
    """Ctrl+N in the visible project tab, Enter on the account: the title of the new session, once
    its first turn is over and its pane shows it."""
    driver.press(CONTROL, "n")
    wait_until("the accounts to start with", lambda: dialog_open(driver, "quick-launch"))
    driver.press(ENTER)
    wait_until("a session of the fake claude", lambda: len(fake_sessions()) == 1)
    title = wait_until("the session in the bar", lambda: (chip_titles(driver) or [None])[0])
    wait_until("the session open in a pane", lambda: (chip(driver, title) or ("", False))[1])
    wait_until("its first turn over", lambda: fake_sessions()[0].get("detail") == "waiting for you", timeout=30)
    return title


# The terminal of the active pane of the visible tab: hidden tabs keep theirs in the page.
ACTIVE_TERMINAL = "#panes .tab-page:not([hidden]) .pane.active .xterm-helper-textarea"


def type_in_pane(driver: Driver, text: str) -> None:
    terminal = wait_until("the pane's terminal", lambda: driver.find(ACTIVE_TERMINAL))
    driver.type_into(terminal, text + ENTER)


class ChangedFilesInARepository(Journey):
    """Ctrl+Shift+G on a session started from Ctrl+N in a git project: a block for the folder it
    started in, compared with the state kindric wrote down at its start, with its own commit and
    its new file but not the commit of another session; a block for the worktree it went into,
    since the branch was created; Enter shows a diff, Escape goes back, then closes."""

    @classmethod
    def setUpClass(cls) -> None:
        fresh_repository()
        shutil.rmtree(WORKTREE, ignore_errors=True)
        super().setUpClass()

    def heads(self) -> list[str]:
        return texts(self.driver, "#changes li.artifact-group")

    def test_blocks_diff_and_back(self) -> None:
        driver = self.driver
        add_project(driver)
        start_in_project(driver)

        with open(os.path.join(PROJECT, "other.txt"), "w") as other:
            other.write("written by another session\n")
        git("add", "other.txt")
        git("commit", "-q", "-m", "another session")
        git("worktree", "add", "-q", "-b", "feature", WORKTREE)
        type_in_pane(driver, "touch:notes.txt commit:add_notes touch:draft.txt cd:../wt touch:wt.txt work:1")
        wait_until("the turn over", lambda: os.path.exists(os.path.join(WORKTREE, "wt.txt")) and fake_sessions()[0].get("detail") == "waiting for you", timeout=30)

        driver.press(CONTROL, SHIFT, "g")
        wait_until("the changes window", lambda: dialog_open(driver, "changes"))
        heads = wait_until("a block per copy", lambda: self.heads() if len(self.heads()) == 2 else None)
        self.assertTrue(heads[0].startswith(PROJECT + " · ") and heads[0].endswith(" · since the start"), heads)
        self.assertEqual(heads[1], f"{WORKTREE} · feature · since the branch was created")
        self.assertEqual(texts(driver, "#changes li.change .artifact-name"), ["draft.txt", "notes.txt", "wt.txt"])

        driver.press(ENTER)
        wait_until("the diff of draft.txt", lambda: "+written by" in driver.text(driver.find("#changes .changes-body")))
        self.assertEqual(texts(driver, "#changes .changes-title"), [f"draft.txt · {PROJECT}"])
        driver.press(ESCAPE)
        wait_until("the list again", lambda: len(texts(driver, "#changes li.change")) == 3)
        self.assertTrue(dialog_open(driver, "changes"))
        driver.press(ESCAPE)
        wait_until("the window closed", lambda: not dialog_open(driver, "changes"))

    @classmethod
    def tearDownClass(cls) -> None:
        super().tearDownClass()
        shutil.rmtree(WORKTREE, ignore_errors=True)


def git_in(folder: str, *words: str) -> str:
    done = subprocess.run(["git", *words], cwd=folder, check=True, capture_output=True, text=True, timeout=30)
    return done.stdout


def write(path: str, text: str) -> None:
    with open(path, "w") as out:
        out.write(text)


def branch_exists(name: str) -> bool:
    return subprocess.run(["git", "rev-parse", "--verify", "-q", f"refs/heads/{name}"], cwd=PROJECT, capture_output=True).returncode == 0


class WorktreesWindow(Journey):
    """Ctrl+Shift+B lists the worktrees of the project with their state. The main checkout, one
    with uncommitted changes and one a session works in are refused, by the window and by the app
    behind it; one not merged goes and its branch stays; one merged goes with its branch."""

    TREES = os.path.join(PROJECT, ".claude/worktrees")

    @classmethod
    def setUpClass(cls) -> None:
        fresh_repository()
        for name in ("done", "ahead", "dirty", "busy"):
            git("worktree", "add", "-q", "-b", name, os.path.join(cls.TREES, name))
        for name in ("done", "ahead"):
            write(os.path.join(cls.TREES, name, f"{name}.txt"), f"{name}\n")
            git_in(os.path.join(cls.TREES, name), "add", f"{name}.txt")
            git_in(os.path.join(cls.TREES, name), "commit", "-q", "-m", name)
        git("merge", "-q", "--ff-only", "done")
        write(os.path.join(cls.TREES, "dirty", "README"), "changed\n")
        super().setUpClass()

    @classmethod
    def before_app(cls) -> None:
        subprocess.run(["kindric-accounts", "run", ACCOUNT, "--bg", "-n", "in-busy", "work in the worktree work:0"],
                       cwd=os.path.join(cls.TREES, "busy"), check=True, capture_output=True, timeout=30)

    def path(self, name: str) -> str:
        return PROJECT if name == "main" else os.path.join(self.TREES, name)

    def rows(self) -> dict[str, str]:
        """Each worktree listed, by path, with the state shown."""
        found: list[list[str]] = self.driver.script("""return [...document.querySelectorAll('#worktrees li.worktree')].map(
          (row) => [row.title, row.querySelector('.worktree-state').textContent]);""")
        return {str(path): str(state) for path, state in found}

    def note(self) -> str:
        return texts(self.driver, "#worktrees .worktrees-note")[0]

    def choose(self, name: str) -> None:
        """Up to the first row, then down to the worktree `name`."""
        paths = list(self.rows())
        for _ in paths:
            self.driver.press("")
        for _ in range(paths.index(self.path(name))):
            self.driver.press(ARROW_DOWN)
        wait_until(f"{name} chosen", lambda: texts(self.driver, "#worktrees li.worktree.selected") and
                   self.driver.script("return document.querySelector('#worktrees li.worktree.selected').title") == self.path(name))

    def refused(self, name: str, why: str) -> None:
        self.choose(name)
        self.driver.press(DELETE)
        wait_until(f"{name} refused", lambda: why in self.note() or None)
        self.assertTrue(os.path.isdir(self.path(name)), f"{name} removed")

    def test_states_refusals_and_removals(self) -> None:
        driver = self.driver
        add_project(driver)
        wait_until("the session in the worktree in the bar", lambda: chip_state(driver, "in-busy"))
        driver.press(CONTROL, SHIFT, "b")
        wait_until("the worktrees window", lambda: dialog_open(driver, "worktrees"))
        rows = wait_until("every worktree", lambda: self.rows() if len(self.rows()) == 5 else None)
        self.assertEqual(rows, {
            PROJECT: "main checkout",
            self.path("done"): "merged",
            self.path("ahead"): "not merged · 1 commit ahead",
            self.path("dirty"): "uncommitted changes",
            self.path("busy"): "a session works here: in-busy",
        })

        self.refused("main", "the main checkout is never removed")
        self.refused("dirty", "it has uncommitted changes")
        self.refused("busy", "the session in-busy works in it")
        # The app refuses too, whatever the page asks.
        refusal = driver.script("""return window.__TAURI__.core.invoke('remove_worktree', { project: arguments[0], path: arguments[1] })
          .then(() => 'removed', (error) => `${error}`);""", PROJECT, self.path("busy"))
        self.assertEqual(refusal, "the session in-busy works in it")
        self.assertTrue(os.path.isdir(self.path("busy")))

        self.choose("ahead")
        driver.press(DELETE)
        wait_until("the question for ahead", lambda: "its branch ahead stays" in self.note() or None)
        driver.press(ENTER)
        wait_until("ahead removed, its branch kept", lambda: self.note().startswith("removed ahead; branch ahead kept") or None)
        self.assertFalse(os.path.exists(self.path("ahead")))
        self.assertTrue(branch_exists("ahead"))

        wait_until("the list without ahead", lambda: self.path("ahead") not in self.rows() or None)
        self.choose("done")
        driver.press(DELETE)
        wait_until("the question for done", lambda: "and its branch done" in self.note() or None)
        driver.press(ENTER)
        wait_until("done removed with its branch", lambda: self.note() == "removed done and its branch done" or None)
        self.assertFalse(os.path.exists(self.path("done")))
        self.assertFalse(branch_exists("done"))
        wait_until("three left", lambda: sorted(self.rows()) == sorted([PROJECT, self.path("dirty"), self.path("busy")]) or None)


class ImagesOfTheLastTurn(Journey):
    """A turn seen ending that cites two images it made shows them in a band of the pane, drawn;
    the next turn, which cites none, empties the band."""

    started = [("painter", "draw image:shot.png image:plot.png work:4")]

    BAND = "#panes .tab-page:not([hidden]) .pane.active .pane-images"

    def drawn(self) -> list[list[Any]]:
        """The images of the band: [alt, width the browser decoded] for each; [] when it is hidden."""
        found: list[list[Any]] = self.driver.script("""const band = document.querySelector(arguments[0]);
          if (!band || band.hidden) return [];
          return [...band.querySelectorAll('img')].map((image) => [image.alt, image.naturalWidth]);""", self.BAND)
        return found

    def test_band_then_empty(self) -> None:
        driver = self.driver
        # kindric looks for images when it sees a turn end, not in a turn ended before it started.
        wait_until("the turn seen working", lambda: chip_state(driver, "painter") == "working")
        wait_until("the turn seen over", lambda: chip_state(driver, "painter") == "finished", timeout=30)
        open_from_bar(driver, "painter")
        drawn = wait_until("both images drawn", lambda: self.drawn() if len(self.drawn()) == 2 and all(width for _, width in self.drawn()) else None)
        self.assertEqual(drawn, [["shot.png", 8], ["plot.png", 8]])

        type_in_pane(driver, "now write text work:3")
        wait_until("the next turn seen working", lambda: chip_state(driver, "painter") == "working")
        wait_until("the next turn seen over", lambda: chip_state(driver, "painter") in ("waiting", "finished"), timeout=30)
        time.sleep(1)
        self.assertEqual(self.drawn(), [], "the band of the turn before")


CLAUDE_HOME = os.path.join(HOME, ".claude")
ARROW_UP = ""


def environments(folder: str) -> list[tuple[str, list[str]]]:
    """What the fake claude noted of each call in an account's folder: its first words, and which of
    CLAUDE_CONFIG_DIR and the billing variables it had."""
    try:
        with open(os.path.join(folder, "fake/env.log")) as log:
            lines = [line.rstrip("\n").split("\t") for line in log if line.strip()]
    except OSError:
        return []
    # `--version` and `--help`, for the setup checklist, touch no account: they keep the environment.
    return [(words, present.split()) for words, present in lines if words not in ("--version", "--help")]


class DefaultAccount(Journey):
    """Claude Code's own configuration, ~/.claude, logged in: the account `default`, first in the
    Ctrl+N form. Its session starts, shows and stops through `claude` without CLAUDE_CONFIG_DIR, and
    no call of claude, on any account, gets the billing variable the app has."""

    @classmethod
    def prepare(cls) -> None:
        os.makedirs(os.path.join(CLAUDE_HOME, "fake"), exist_ok=True)
        write(os.path.join(CLAUDE_HOME, "fake/logged-in"), "")

    def test_default_without_config_dir_nor_billing(self) -> None:
        driver = self.driver
        self.assertIn("ANTHROPIC_API_KEY", os.environ, "the journey needs the billing variable of inside.sh")
        driver.press(CONTROL, "n")
        wait_until("the new session form", lambda: dialog_open(driver, "new-session"))
        accounts_listed = wait_until("both accounts in the form", lambda: texts(driver, "#new-account .launch-list li") if len(texts(driver, "#new-account .launch-list li")) == 2 else None)
        self.assertTrue(accounts_listed[0].startswith("default"), accounts_listed)
        for _ in accounts_listed:
            driver.press(ARROW_UP)
        wait_until("default picked", lambda: texts(driver, "#new-account .launch-list li.selected")[0].startswith("default"))
        directory = driver.find("#new-form input[name=directory]")
        driver.clear(directory)
        driver.type_into(directory, PROJECT)
        driver.type_into(driver.find("#new-form input[name=name]"), "mine" + ENTER)
        wait_until("the session open in a pane", lambda: (chip(driver, "mine") or ("", False))[1])
        session = wait_until("the session in ~/.claude", lambda: next((s for s in sessions_of_folder(CLAUDE_HOME) if s.get("name") == "mine"), None))
        self.assertEqual(session["cwd"], PROJECT)

        driver.press(CONTROL, SHIFT, "k")
        wait_until("the question before hiding", lambda: dialog_open(driver, "confirm-hide"))
        driver.click(driver.find("#hide-form button[type=submit]"))
        wait_until("the session stopped", lambda: not next(s for s in sessions_of_folder(CLAUDE_HOME) if s.get("name") == "mine").get("pid"))

        calls = environments(CLAUDE_HOME)
        for words in ("auth status", "agents --json", "--bg", "attach", "stop"):
            self.assertTrue(any(words in called for called, _ in calls), f"no {words} on default: {calls}")
        self.assertEqual([(called, present) for called, present in calls if present], [], "default given CLAUDE_CONFIG_DIR or billing")
        work = environments(ACCOUNT_DIR)
        self.assertTrue(work, "no call on the account work")
        self.assertEqual([(called, present) for called, present in work if present != ["CLAUDE_CONFIG_DIR"]], [], "work given billing")


def sessions_of_folder(folder: str) -> list[dict[str, Any]]:
    try:
        with open(os.path.join(folder, "fake/sessions.json")) as source:
            sessions: list[dict[str, Any]] = json.load(source)
            return sessions
    except (OSError, ValueError):
        return []


class ProgressLine(Journey):
    """What a session working for long last said it does, from its job file: in the tip of its
    chip, the head of its pane and Ctrl+P. Once it is stopped, from the history of the job file."""

    started = [("long-one", "a long task work:25")]
    STEP = "step 1 of: a long task work:25"

    def tip(self) -> str:
        return str(self.driver.script("""const chip = [...document.querySelectorAll('#chips li.chip:not(.more)')].find(
          (chip) => chip.querySelector('.chip-title').textContent === arguments[0]);
          chip.dispatchEvent(new MouseEvent('mouseenter'));
          const progress = document.querySelector('#tip .progress');
          return progress ? progress.textContent : '';""", "long-one"))

    def picker_progress(self) -> str | None:
        found = self.driver.script("""const row = [...document.querySelectorAll('#picker-results li.session')].find(
          (row) => row.querySelector('.name').textContent === arguments[0]);
          const progress = row && row.querySelector('.progress');
          return progress ? progress.textContent : null;""", "long-one")
        return None if found is None else str(found)

    def find(self) -> None:
        driver = self.driver
        driver.press(CONTROL, "p")
        wait_until("the session finder", lambda: dialog_open(driver, "picker"))
        driver.type_into(driver.find("#picker-query"), "long-one")

    def test_tip_head_finder_then_history(self) -> None:
        driver = self.driver
        wait_until("the session at work", lambda: chip_state(driver, "long-one") == "working")
        wait_until("its first step in the tip", lambda: self.tip() == self.STEP or None, timeout=20)
        open_from_bar(driver, "long-one")
        wait_until("its step in the pane's head", lambda: texts(driver, "#panes .tab-page:not([hidden]) .pane.active .head-progress") == [f"· {self.STEP}"] or None)
        self.find()
        wait_until("its step in the finder", lambda: self.picker_progress() == self.STEP or None)
        driver.press(ESCAPE)
        wait_until("the finder closed", lambda: not dialog_open(driver, "picker"))

        driver.press(CONTROL, SHIFT, "k")
        wait_until("the question before hiding", lambda: dialog_open(driver, "confirm-hide"))
        driver.click(driver.find("#hide-form button[type=submit]"))
        wait_until("the session stopped", lambda: not fake_running("long-one"))
        self.find()
        wait_until("its last step, from the history, in the finder", lambda: self.picker_progress() == self.STEP or None)


VISIBLE_PANES = "#panes .tab-page:not([hidden]) .pane"


class BackWhereLeft(Journey):
    """Closed on a project tab with two panes, the app starts again on that tab, its active pane's
    session open in a single pane (splits are not kept). The first visit of the other tab opens
    the session it showed."""

    started = [("first", "a task work:0"), ("second", "another task work:0")]

    @classmethod
    def before_app(cls) -> None:
        subprocess.run(["kindric-accounts", "run", ACCOUNT, "--bg", "-n", "elsewhere", "out of the project work:0"],
                       cwd=HOME, check=True, capture_output=True, timeout=30)

    def shown(self) -> list[str]:
        """The titles of the chips shown in a pane."""
        chips: list[list[Any]] = self.driver.script(CHIPS)
        return [str(title) for _, title, open_ in chips if open_]

    def test_tab_and_active_session_again(self) -> None:
        driver = self.driver
        wait_until("the session of the other tab", lambda: chip_state(driver, "elsewhere"))
        open_from_bar(driver, "elsewhere")
        add_project(driver)
        wait_until("both sessions in the project's bar", lambda: chip_state(driver, "first") and chip_state(driver, "second"))
        open_from_bar(driver, "first")
        driver.press(CONTROL, SHIFT, "d")
        wait_until("two panes", lambda: len(driver.find_all(VISIBLE_PANES)) == 2)
        open_from_bar(driver, "second")
        self.assertEqual(sorted(self.shown()), ["first", "second"])

        driver.quit()
        os.remove(TRACE)
        type(self).driver = driver = Driver.start(application())
        wait_until("the page ready again", page_ready)
        wait_until("the project tab, active", lambda: ("project", True) in tabs(driver) or None)
        self.assertTrue(overview_hidden(driver), "started on the control center")
        wait_until("the active pane's session open", lambda: self.shown() == ["second"] or None)
        self.assertEqual(len(driver.find_all(VISIBLE_PANES)), 1, "the split kept")

        driver.press(CONTROL, SHIFT, "1")
        wait_until("the other tab", lambda: ("other", True) in tabs(driver) or None)
        wait_until("its session open again", lambda: self.shown() == ["elsewhere"] or None)


class SplitStepAndClose(Journey):
    """Ctrl+Shift+D splits the active pane side by side when it is wider than tall, one above the
    other otherwise; Ctrl+H and Ctrl+L go from pane to pane; a separator dragged keeps its place;
    Ctrl+Shift+W closes a pane and its session keeps running."""

    started = [("left", "a task work:0"), ("top", "a task work:0"), ("bottom", "a task work:0")]

    def active_title(self) -> str:
        return texts(self.driver, f"{VISIBLE_PANES}.active .head-title")[0]

    def splits(self) -> list[str]:
        """The splits of the visible tab, outer first: "row" side by side, "column" one above the other."""
        found: list[str] = self.driver.script("""return [...document.querySelectorAll('#panes .tab-page:not([hidden]) .split')]
          .map((split) => split.classList.contains('row') ? 'row' : 'column');""")
        return [str(name) for name in found]

    def left_share(self) -> float:
        return float(self.driver.script("""const split = document.querySelector('#panes .tab-page:not([hidden]) .split.row');
          return split.firstElementChild.getBoundingClientRect().width / split.getBoundingClientRect().width;"""))

    def test_split_step_drag_close(self) -> None:
        driver = self.driver
        wait_until("the sessions in the bar", lambda: all(chip_state(driver, name) for name in ("left", "top", "bottom")))
        open_from_bar(driver, "left")
        driver.press(CONTROL, SHIFT, "d")
        wait_until("two panes side by side", lambda: self.splits() == ["row"] or None)
        open_from_bar(driver, "top")
        driver.press(CONTROL, SHIFT, "d")
        wait_until("the right pane cut in two, one above the other", lambda: self.splits() == ["row", "column"] or None)
        open_from_bar(driver, "bottom")
        self.assertEqual(len(driver.find_all(VISIBLE_PANES)), 3)

        driver.press(CONTROL, "h")
        wait_until("the previous pane", lambda: self.active_title() == "top" or None)
        driver.press(CONTROL, "l")
        wait_until("the next pane", lambda: self.active_title() == "bottom" or None)

        self.assertAlmostEqual(self.left_share(), 0.5, delta=0.02)
        divider = driver.find("#panes .tab-page:not([hidden]) .split.row > .divider")
        driver.drag(divider, driver.find(f"{VISIBLE_PANES} .screen"))
        share = wait_until("the separator moved", lambda: self.left_share() if self.left_share() < 0.4 else None)
        driver.press(CONTROL, "l")
        time.sleep(0.5)
        self.assertAlmostEqual(self.left_share(), share, delta=0.02, msg="the separator went back")

        for _ in range(3):
            if self.active_title() == "bottom":
                break
            driver.press(CONTROL, "l")
            time.sleep(0.2)
        self.assertEqual(self.active_title(), "bottom")
        driver.press(CONTROL, SHIFT, "w")
        wait_until("two panes left", lambda: len(driver.find_all(VISIBLE_PANES)) == 2 or None)
        self.assertEqual(self.splits(), ["row"])
        wait_until("bottom no longer shown", lambda: (chip(driver, "bottom") or ("", True))[1] is False or None)
        self.assertTrue(fake_running("bottom"), "the session stopped with its pane")


class DoneInThePane(Journey):
    """A session shown in a pane that is not the active one finishes its turn: its pane gets the
    green border and "✓ done", until it is looked at. One whose turn ends on a question does not."""

    started = [("worker", "a task work:8"), ("asker", "a task that asks work:8 ask:permission_prompt"), ("watched", "a task work:0")]

    def marks(self) -> dict[str, tuple[bool, bool]]:
        """For each pane of the visible tab, by its session: its border marked, its "done" shown."""
        found: list[list[Any]] = self.driver.script("""return [...document.querySelectorAll(arguments[0])].map((pane) =>
          [pane.querySelector('.head-title').textContent, pane.classList.contains('finished'), !pane.querySelector('.head-done').hidden]);""", VISIBLE_PANES)
        return {str(title): (bool(border), bool(done)) for title, border, done in found}

    def active_title(self) -> str:
        return texts(self.driver, f"{VISIBLE_PANES}.active .head-title")[0]

    def test_done_until_looked_at(self) -> None:
        driver = self.driver
        wait_until("the sessions in the bar", lambda: all(chip_state(driver, name) for name in ("worker", "asker", "watched")))
        open_from_bar(driver, "worker")
        driver.press(CONTROL, SHIFT, "d")
        wait_until("two panes", lambda: len(driver.find_all(VISIBLE_PANES)) == 2)
        open_from_bar(driver, "asker")
        driver.press(CONTROL, SHIFT, "d")
        wait_until("three panes", lambda: len(driver.find_all(VISIBLE_PANES)) == 3)
        open_from_bar(driver, "watched")
        self.assertEqual(self.marks(), {"worker": (False, False), "asker": (False, False), "watched": (False, False)})

        wait_until("worker's turn over", lambda: chip_state(driver, "worker") == "finished", timeout=30)
        wait_until("asker asking", lambda: chip_state(driver, "asker") == "asking", timeout=30)
        wait_until("worker's pane marked done", lambda: self.marks()["worker"] == (True, True) or None)
        self.assertEqual(self.marks()["asker"], (False, False), "a question marked done")

        for _ in range(3):
            if self.active_title() == "worker":
                break
            driver.press(CONTROL, "h")
            time.sleep(0.2)
        self.assertEqual(self.active_title(), "worker")
        wait_until("the mark gone once looked at", lambda: self.marks()["worker"] == (False, False) or None)
        self.assertNotEqual(chip_state(driver, "worker"), "finished")


SETTINGS = os.path.join(HOME, ".local/share/kindric/settings.json")
EMBER, PAPER = "#1a0e06", "#f7efe4"


def background(driver: Driver) -> str:
    return str(driver.script("return getComputedStyle(document.documentElement).getPropertyValue('--background').trim()"))


class ShortcutChanged(Journey):
    """The settings give "next theme" new keys: keys already taken are refused with the name of
    their action; the new keys work, the old ones no longer do, F1 shows them, and they are kept
    after a restart. "reset" gives the old keys back."""

    def shortcut_row(self, help_text: str) -> str:
        found = self.driver.script("""return [...document.querySelectorAll('#settings .shortcut')].find(
          (row) => row.querySelector('.shortcut-help').textContent === arguments[0]);""", help_text)
        if not isinstance(found, dict) or ELEMENT_KEY not in found:
            raise AssertionError(f"no shortcut {help_text!r} in the settings")
        return str(found[ELEMENT_KEY])

    def keys_shown(self) -> str:
        """The keys the settings show for "next theme"."""
        return str(self.driver.script("return arguments[0].querySelector('kbd').textContent;", {ELEMENT_KEY: self.shortcut_row("next theme")}))

    def press_in_row(self, label: str) -> None:
        found = self.driver.script("""return [...arguments[0].querySelectorAll('.shortcut-actions button')].find((button) => button.textContent === arguments[1]);""",
                                   {ELEMENT_KEY: self.shortcut_row("next theme")}, label)
        if not isinstance(found, dict) or ELEMENT_KEY not in found:
            raise AssertionError(f"no {label} on the row of next theme")
        self.driver.click(str(found[ELEMENT_KEY]))

    def open_shortcuts(self) -> None:
        driver = self.driver
        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "shortcuts")
        wait_until("the shortcuts", lambda: self.keys_shown())

    def close_settings(self) -> None:
        driver = self.driver
        for _ in range(2):
            if dialog_open(driver, "settings"):
                driver.press(ESCAPE)
                time.sleep(0.2)
        wait_until("the settings closed", lambda: not dialog_open(driver, "settings"))

    def test_change_refuse_keep_reset(self) -> None:
        driver = self.driver
        self.open_shortcuts()
        self.assertEqual(self.keys_shown(), "Ctrl+Shift+E")
        self.press_in_row("change")
        wait_until("waiting for the keys", lambda: self.keys_shown() == "press keys…" or None)
        driver.press(CONTROL, SHIFT, "k")
        try:
            wait_until("taken keys refused", lambda: texts(driver, "#settings .shortcuts-message")[0].startswith('Ctrl+Shift+K is already "stop and hide') or None)
        except AssertionError as error:
            focus = driver.script("const a = document.activeElement; return a ? `${a.tagName}.${a.className}` : 'none';")
            raise AssertionError(f"{error}; message {texts(driver, '#settings .shortcuts-message')!r}, keys {self.keys_shown()!r}, focus {focus}") from error
        driver.press(CONTROL, ALT, "t")
        wait_until("the new keys", lambda: self.keys_shown() == "Ctrl+Alt+T" or None)
        with open(SETTINGS) as source:
            self.assertEqual(json.load(source).get("keys", {}).get("next-theme"), "ctrl+alt+t")
        self.close_settings()

        driver.press(CONTROL, SHIFT, "e")
        time.sleep(0.5)
        self.assertEqual(background(driver), EMBER, "the old keys still change the theme")
        driver.press(CONTROL, ALT, "t")
        wait_until("the next theme from the new keys", lambda: background(driver) == PAPER or None)
        driver.press("")
        wait_until("the help", lambda: dialog_open(driver, "help"))
        helped = driver.script("""return [...document.querySelectorAll('#help kbd')].map((keys) => [keys.textContent, keys.nextElementSibling?.textContent]);""")
        self.assertIn(["Ctrl+Alt+T", "next theme"], helped)
        driver.press(ESCAPE)
        wait_until("the help closed", lambda: not dialog_open(driver, "help"))

        type(self).restart()
        driver = self.driver
        driver.press(CONTROL, ALT, "t")
        wait_until("the new keys after a restart", lambda: background(driver) not in (EMBER, PAPER) or None)

        self.open_shortcuts()
        self.press_in_row("reset")
        wait_until("the old keys back", lambda: self.keys_shown() == "Ctrl+Shift+E" or None)
        self.close_settings()
        driver.press(CONTROL, SHIFT, "e")
        wait_until("the old keys working", lambda: background(driver) == EMBER or None)


PROJECTS_FILE = os.path.join(HOME, ".local/share/kindric/projects.json")
THEMES = os.path.join(CLAUDE_HOME, "themes")


def project_themes() -> list[str | None]:
    """The theme of each project of projects.json."""
    try:
        with open(PROJECTS_FILE) as source:
            saved = json.load(source)
    except (OSError, ValueError):
        return []
    projects = saved.get("projects", saved) if isinstance(saved, dict) else saved
    return [project.get("theme") for project in projects]


class ProjectTheme(Journey):
    """Ctrl+Alt+E in a project tab goes through the themes for that project: its sessions' panes
    show in it while the window keeps kindric's, Claude Code gets a theme of the project, a session
    started there uses it, and after the last theme the project is back to kindric's."""

    def pane_background(self) -> str:
        return str(self.driver.script("return document.querySelector(arguments[0]).style.getPropertyValue('--background')", f"{VISIBLE_PANES}.active"))

    def test_project_theme_then_back(self) -> None:
        driver = self.driver
        add_project(driver)
        start_in_project(driver)
        driver.press(CONTROL, ALT, "e")
        wait_until("ember for the project", lambda: project_themes() == ["ember"] or None)
        driver.press(CONTROL, ALT, "e")
        wait_until("paper in the project's pane", lambda: self.pane_background() == PAPER or None)
        self.assertEqual(background(driver), EMBER, "the window left kindric's theme")
        self.assertEqual(project_themes(), ["paper"])
        written = wait_until("Claude Code's theme of the project", lambda: [name for name in os.listdir(THEMES) if name.startswith("kindric-")] if os.path.isdir(THEMES) and any(name.startswith("kindric-") for name in os.listdir(THEMES)) else None)
        self.assertEqual(len(written), 1, written)
        theme = written[0].removesuffix(".json")
        with open(os.path.join(THEMES, written[0])) as source:
            self.assertTrue(str(json.load(source).get("base", "")).startswith("light"), "paper drawn dark")

        driver.press(CONTROL, "n")
        wait_until("the accounts to start with", lambda: dialog_open(driver, "quick-launch"))
        driver.press(ENTER)
        wait_until("a second session", lambda: len(fake_sessions()) == 2)
        with open(settings_path(fake_sessions()[1])) as source:
            self.assertIn(theme, source.read(), "the session started without the project's theme")

        driver.press(CONTROL, ALT, "e")
        wait_until("graphite for the project", lambda: project_themes() == ["graphite"] or None)
        driver.press(CONTROL, ALT, "e")
        wait_until("back to kindric's theme", lambda: project_themes() == [None] or None)
        wait_until("the pane in kindric's theme", lambda: self.pane_background() == "" or None)


def settings_path(session: dict[str, Any]) -> str:
    args = [str(arg) for arg in session.get("args", [])]
    if "--settings" not in args:
        raise AssertionError(f"started without --settings: {args}")
    return args[args.index("--settings") + 1]


def trusted_by_account(directory: str) -> bool:
    try:
        with open(os.path.join(ACCOUNT_DIR, ".claude.json")) as source:
            return bool(json.load(source).get("projects", {}).get(directory, {}).get("hasTrustDialogAccepted"))
    except (OSError, ValueError):
        return False


class TrustAFolder(Journey):
    """Ctrl+N in a folder the account never trusted: Claude Code refuses, the form offers "trust it";
    Claude Code asks in the helper window, Down and Enter answer yes, and "done" starts the session."""

    @classmethod
    def prepare(cls) -> None:
        os.makedirs(os.path.join(ACCOUNT_DIR, "fake"), exist_ok=True)
        write(os.path.join(ACCOUNT_DIR, "fake/require-trust"), "")

    def test_refused_trusted_started(self) -> None:
        driver = self.driver
        driver.press(CONTROL, "n")
        wait_until("the new session form", lambda: dialog_open(driver, "new-session"))
        directory = driver.find("#new-form input[name=directory]")
        driver.clear(directory)
        driver.type_into(directory, PROJECT)
        driver.type_into(driver.find("#new-form input[name=name]"), "trusted" + ENTER)
        wait_until("the refusal, with trust it", lambda: "trust it" in texts(driver, "#new-message button") or None)
        self.assertIn("work has not trusted this folder yet.", texts(driver, "#new-message")[0])
        self.assertEqual(fake_sessions(), [])

        click_text(driver, "#new-message button", "trust it")
        wait_until("the helper window", lambda: dialog_open(driver, "helper"))
        # The terminal draws with WebGL: its text is not in the page. The pty keeps what is typed
        # until the fake claude reads its answer.
        driver.type_into(driver.find("#helper-screen .xterm-helper-textarea"), ARROW_DOWN + ENTER)
        wait_until("the folder trusted", lambda: trusted_by_account(PROJECT))
        driver.click(driver.find("#helper-done"))
        wait_until("the helper closed", lambda: not dialog_open(driver, "helper"))
        wait_until("the session started", lambda: chip_state(driver, "trusted"))
        self.assertEqual(fake_sessions()[0]["cwd"], PROJECT)


class ProviderAccountAddedWithItsKey(Journey):
    """The settings add an account whose model comes from OpenRouter: the address, model and key
    fields show for that provider, kindric-accounts keeps the key in the account's folder, readable
    by the user only, and the row shows the provider and model with "change key" instead of
    "log in". No login window opens."""

    ROUTER_DIR = os.path.join(HOME, ".local/share/kindric-accounts/router")

    def test_add_with_key(self) -> None:
        driver = self.driver
        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "accounts")
        wait_until("the account work", lambda: texts(driver, "#settings li.account .account-name") == ["work"] or None)
        hidden = driver.script("return document.querySelector('#settings .provider-key').hidden;")
        self.assertTrue(hidden, "the key field shows for a Claude login")

        driver.script("""const kind = document.querySelector('#settings .accounts-add select[name=kind]');
          kind.value = 'openrouter';
          kind.dispatchEvent(new Event('change'));""")
        shown = driver.script("""return ['.provider-url', '.provider-model', '.provider-key']
          .map((selector) => !document.querySelector('#settings ' + selector).hidden);""")
        self.assertEqual(shown, [True, True, True])
        driver.type_into(driver.find("#settings .accounts-add input[name=name]"), "router")
        driver.type_into(driver.find("#settings .accounts-add input[name=model]"), "qwen/qwen3-coder")
        driver.type_into(driver.find("#settings .accounts-add input[name=key]"), "sk-or-e2e" + ENTER)

        wait_until("router listed", lambda: "router" in texts(driver, "#settings li.account .account-name") or None)
        token = os.path.join(self.ROUTER_DIR, "kindric-token")
        with open(token, encoding="utf-8") as file:
            self.assertEqual(file.read().strip(), "sk-or-e2e")
        self.assertEqual(os.stat(token).st_mode & 0o777, 0o600)
        self.assertFalse(dialog_open(driver, "helper"), "a login window opened for a provider account")
        row = driver.script("""const row = [...document.querySelectorAll('#settings li.account')].find((item) => item.querySelector('.account-name').textContent === 'router');
          return [row.querySelector('.account-details').textContent, [...row.querySelectorAll('.account-actions button')].map((button) => button.textContent)];""")
        self.assertEqual(row, ["openrouter qwen/qwen3-coder", ["change key", "remove"]])
        key_field = driver.script("return document.querySelector('#settings .accounts-add input[name=key]').value;")
        self.assertEqual(key_field, "", "the key stays in the form")


class AccountAddedThenRemoved(Journey):
    """The settings add an account: kindric-accounts creates it and Claude Code's login runs in the
    helper window. "remove" asks first, then takes it off the list and leaves its folder on disk."""

    EXTRA_DIR = os.path.join(HOME, ".local/share/kindric-accounts/extra")

    def account_names(self) -> list[str]:
        return texts(self.driver, "#settings li.account .account-name")

    def test_add_log_in_remove(self) -> None:
        driver = self.driver
        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "accounts")
        wait_until("the account work", lambda: self.account_names() == ["work"] or None)

        driver.type_into(driver.find("#settings .accounts-add input[name=name]"), "extra" + ENTER)
        wait_until("Claude Code's login of extra", lambda: os.path.exists(os.path.join(self.EXTRA_DIR, "fake/logins.log")))
        if dialog_open(driver, "helper"):
            driver.click(driver.find("#helper-done"))
        wait_until("the login window closed", lambda: not dialog_open(driver, "helper"))
        wait_until("extra listed", lambda: "extra" in self.account_names() or None)
        self.assertIn("extra", accounts("list"))

        found = driver.script("""const row = [...document.querySelectorAll('#settings li.account')].find((item) => item.querySelector('.account-name').textContent === 'extra');
          return [...row.querySelectorAll('.account-actions button')].find((button) => button.textContent === 'remove');""")
        driver.click(str(found[ELEMENT_KEY]))
        question = wait_until("the question", lambda: texts(driver, "#settings .account-confirm") or None)
        self.assertEqual(question, [f"take extra off the list? its folder stays: {self.EXTRA_DIR}."])
        self.assertIn("extra", accounts("list"), "removed before the answer")
        click_text(driver, "#settings li.account .account-actions button.danger", "remove")
        wait_until("extra off the list", lambda: self.account_names() == ["work"] or None)
        self.assertNotIn("extra", accounts("list"))
        self.assertTrue(os.path.isdir(self.EXTRA_DIR), "the account's folder removed")


class CodexAccountAddedFromSettings(Journey):
    """The settings add a Codex account: no address, model or key field; kindric-accounts makes it
    a Codex account, Codex's login runs in the helper window, and the row says "codex"."""

    CODEX_DIR = os.path.join(HOME, ".local/share/kindric-accounts/cx")

    def test_add_and_log_in(self) -> None:
        driver = self.driver
        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "accounts")
        wait_until("the account work", lambda: texts(driver, "#settings li.account .account-name") == ["work"] or None)
        driver.script("""const kind = document.querySelector('#settings .accounts-add select[name=kind]');
          kind.value = 'codex';
          kind.dispatchEvent(new Event('change'));""")
        hidden = driver.script("""return ['.provider-url', '.provider-model', '.provider-key']
          .map((selector) => document.querySelector('#settings ' + selector).hidden);""")
        self.assertEqual(hidden, [True, True, True], "a Codex account asks for its name only")
        driver.type_into(driver.find("#settings .accounts-add input[name=name]"), "cx" + ENTER)

        wait_until("Codex's login of cx", lambda: os.path.exists(os.path.join(self.CODEX_DIR, "fake-logins.log")))
        with open(os.path.join(self.CODEX_DIR, "kindric-agent.json"), encoding="utf-8") as file:
            self.assertEqual(json.load(file), {"agent": "codex"})
        if dialog_open(driver, "helper"):
            driver.click(driver.find("#helper-done"))
        wait_until("the login window closed", lambda: not dialog_open(driver, "helper"))
        row = wait_until("cx logged in", lambda: driver.script("""const row = [...document.querySelectorAll('#settings li.account')].find((item) => item.querySelector('.account-name').textContent === 'cx');
          if (!row || !row.querySelector('.account-state').classList.contains('logged-in')) return null;
          return [row.querySelector('.account-details').textContent, [...row.querySelectorAll('.account-actions button')].map((button) => button.textContent)];"""))
        self.assertEqual(row, ["codex", ["remove"]])


class OpenCodeAccountAddedFromSettings(Journey):
    """The settings add an OpenCode account: no address, model or key field; kindric-accounts makes
    it an OpenCode account with a port of its own, and OpenCode's login runs in the helper window."""

    OPENCODE_DIR = os.path.join(HOME, ".local/share/kindric-accounts/oc")

    def test_add_and_log_in(self) -> None:
        driver = self.driver
        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "accounts")
        wait_until("the account work", lambda: texts(driver, "#settings li.account .account-name") == ["work"] or None)
        driver.script("""const kind = document.querySelector('#settings .accounts-add select[name=kind]');
          kind.value = 'opencode';
          kind.dispatchEvent(new Event('change'));""")
        hidden = driver.script("""return ['.provider-url', '.provider-model', '.provider-key']
          .map((selector) => document.querySelector('#settings ' + selector).hidden);""")
        self.assertEqual(hidden, [True, True, True], "an OpenCode account asks for its name only")
        driver.type_into(driver.find("#settings .accounts-add input[name=name]"), "oc" + ENTER)

        wait_until("OpenCode's login of oc", lambda: os.path.exists(os.path.join(self.OPENCODE_DIR, "data", "fake-logins.log")))
        with open(os.path.join(self.OPENCODE_DIR, "kindric-agent.json"), encoding="utf-8") as file:
            self.assertEqual(json.load(file), {"agent": "opencode"})
        with open(os.path.join(self.OPENCODE_DIR, "config", "opencode", "service.json"), encoding="utf-8") as file:
            self.assertEqual(json.load(file), {"port": 49375}, "the first port after OpenCode's own")
        if dialog_open(driver, "helper"):
            driver.click(driver.find("#helper-done"))
        wait_until("the login window closed", lambda: not dialog_open(driver, "helper"))
        wait_until("oc in the list", lambda: "oc" in texts(driver, "#settings li.account .account-name") or None)


CODEX_ACCOUNT_DIR = os.path.join(HOME, ".local/share/kindric-accounts/cx")


def codex_threads() -> list[dict[str, Any]]:
    """What the fake codex knows of the sessions of the Codex account cx (tests/e2e/bin/codex)."""
    try:
        with open(os.path.join(CODEX_ACCOUNT_DIR, "fake-threads.json"), encoding="utf-8") as source:
            threads: list[dict[str, Any]] = json.load(source)
            return threads
    except (OSError, ValueError):
        return []


def codex_thread(name: str) -> dict[str, Any] | None:
    return next((thread for thread in codex_threads() if thread.get("name") == name), None)


def codex_resumed() -> list[str]:
    """The lines "<id> <folder>" of each `codex resume` the fake ran."""
    try:
        with open(os.path.join(CODEX_ACCOUNT_DIR, "fake-resumed.log"), encoding="utf-8") as log:
            return log.read().splitlines()
    except OSError:
        return []


class CodexSessionsInKindric(Journey):
    """A Codex account's sessions, read from its daemon: in the bar with their states (a permission
    waiting is a question); one opens in a pane, Codex's TUI started in its folder; Ctrl+N on the
    Codex account starts a named session in the daemon; Ctrl+Shift+K stops the turn of a working
    one and hides it; the account's card has the limit its daemon gives; a message typed in the
    control center goes to Codex for the session; the command a session asks to run shows in the control center, and "a" allows it through the
    daemon; the purge archives the one idle for 10 days."""

    THREADS = [
        {"id": "11111111-1111-4111-8111-111111111111", "name": "codex-idle", "status": "idle", "turns": [{"id": "a1", "status": "completed"}]},
        {"id": "22222222-2222-4222-8222-222222222222", "name": "codex-asks", "status": "approval", "turns": [{"id": "b1", "status": "inProgress"}],
         "approval": {"itemId": "b-item", "command": "rm -rf build", "reason": "clean the build"}},
        {"id": "33333333-3333-4333-8333-333333333333", "name": "codex-busy", "status": "working",
         "turns": [{"id": "c1", "status": "completed"}, {"id": "c2", "status": "inProgress"}]},
        {"id": "44444444-4444-4444-8444-444444444444", "name": "codex-old", "status": "notLoaded", "turns": []},
    ]

    @classmethod
    def prepare(cls) -> None:
        accounts("add", "cx", "--agent", "codex")
        with open(os.path.join(CODEX_ACCOUNT_DIR, "fake-logged-in"), "w", encoding="utf-8") as file:
            file.write("chatgpt\n")
        threads = [dict(thread, cwd=PROJECT, archived=False) for thread in cls.THREADS]
        with open(os.path.join(CODEX_ACCOUNT_DIR, "fake-threads.json"), "w", encoding="utf-8") as file:
            json.dump(threads, file)
        # The old one was last active 10 days ago: its rollout says so.
        day = os.path.join(CODEX_ACCOUNT_DIR, "sessions/2026/11/01")
        os.makedirs(day, exist_ok=True)
        old = os.path.join(day, f"rollout-2026-11-01T10-00-00-{cls.THREADS[3]['id']}.jsonl")
        with open(old, "w", encoding="utf-8") as file:
            file.write("{}\n")
        then = time.time() - 10 * 86400
        os.utime(old, (then, then))
        accounts("run", "cx", "app-server", "daemon", "start")

    @classmethod
    def tearDownClass(cls) -> None:
        try:
            super().tearDownClass()
        finally:
            subprocess.run(["kindric-accounts", "run", "cx", "app-server", "daemon", "stop"], cwd=PROJECT, capture_output=True, timeout=30)

    def test_list_open_start_hide_purge(self) -> None:
        driver = self.driver
        wait_until("the Codex sessions in the bar", lambda: chip_state(driver, "codex-idle") and chip_state(driver, "codex-asks"))
        self.assertEqual(chip_state(driver, "codex-asks"), "asking")
        self.assertEqual(chip_state(driver, "codex-idle"), "waiting")
        self.assertEqual(chip_state(driver, "codex-busy"), "working")

        open_from_bar(driver, "codex-idle")
        wait_until("Codex's TUI started in the session's folder",
                   lambda: f"{self.THREADS[0]['id']} {PROJECT}" in codex_resumed() or None)

        driver.press(CONTROL, "n")
        wait_until("the new session form", lambda: dialog_open(driver, "new-session"))
        found = driver.script("""return [...document.querySelectorAll('#new-account .launch-list li')].find(
          (item) => item.textContent.trim().startsWith('cx'));""")
        self.assertTrue(isinstance(found, dict) and ELEMENT_KEY in found, "no cx in the new session form")
        driver.click(str(found[ELEMENT_KEY]))
        directory = driver.find("#new-form input[name=directory]")
        driver.clear(directory)
        driver.type_into(directory, PROJECT)
        driver.type_into(driver.find("#new-form input[name=name]"), "codex-new" + ENTER)
        started = wait_until("the session started in the daemon", lambda: codex_thread("codex-new"))
        self.assertEqual(started["cwd"], PROJECT)
        wait_until("the new session in the bar", lambda: chip_state(driver, "codex-new"))
        wait_until("the new session open in a pane", lambda: f"{started['id']} {PROJECT}" in codex_resumed() or None)
        # Before starting it, kindric gave the account its hooks, the ones that work in Codex.
        with open(os.path.join(CODEX_ACCOUNT_DIR, "hooks.json"), encoding="utf-8") as file:
            hooks = json.load(file)["hooks"]
        commands = sorted({hook["command"].split()[-1] for groups in hooks.values() for group in groups for hook in group["hooks"]})
        self.assertEqual(commands, ["away-hook", "overview-start", "unread"])
        self.assertTrue(os.path.exists(os.path.join(CODEX_ACCOUNT_DIR, ".kindric-hooks-trusted")), "Codex was not asked to trust them")

        open_from_bar(driver, "codex-busy")
        driver.press(CONTROL, SHIFT, "k")
        wait_until("the question before hiding", lambda: dialog_open(driver, "confirm-hide"))
        driver.click(driver.find("#hide-form button[type=submit]"))
        wait_until("the working session hidden", lambda: chip_state(driver, "codex-busy") is None)
        busy = codex_thread("codex-busy") or {}
        self.assertEqual([turn["status"] for turn in busy.get("turns", [])], ["completed", "interrupted"])

        driver.press(CONTROL, SHIFT, "0")
        card = """const card = [...document.querySelectorAll('.account-card')].find((card) => card.querySelector('strong').textContent === 'cx');
          return card ? [...card.querySelectorAll('.account-line > span:first-child, .account-percent, .account-note')].map((part) => part.textContent) : [];"""
        wait_until("the limit of cx on its card", lambda: driver.script(card)[:2] == ["30d", "12%"] or None)
        self.assertTrue(driver.script(card)[2].startswith("30d resets on "), driver.script(card))
        idle = self.THREADS[0]["id"]
        wait_until("codex-idle in the control center", lambda: driver.find(f"[data-key='row/{idle}']"))
        picked = "return document.querySelector('.control-row.selected')?.dataset.session ?? null"
        for _ in range(8):
            if driver.script(picked) == idle:
                break
            driver.press(ARROW_DOWN)
        self.assertEqual(driver.script(picked), idle, "codex-idle never picked with the arrow keys")
        driver.press(ENTER)
        field = wait_until("its field", lambda: driver.find(f"[data-key='prompt/{idle}']"))
        driver.type_into(field, "hello codex" + ENTER)
        wait_until("the message given to Codex for the session", lambda: (codex_thread("codex-idle") or {}).get("messages") == ["hello codex"] or None)

        driver.press(ESCAPE)
        asking = self.THREADS[1]["id"]
        command = wait_until("the command codex-asks asks to run", lambda: driver.text(driver.find(f"[data-key='row/{asking}'] .request-command")))
        self.assertEqual(command, "$ rm -rf build")
        driver.press("n")
        wait_until("codex-asks picked, the session that asks", lambda: driver.script(picked) == asking or None)
        driver.press("a")
        wait_until("the approval given to Codex", lambda: (codex_thread("codex-asks") or {}).get("decision") == "accept" or None)
        wait_until("the question gone from the row of codex-asks", lambda: not driver.find_all(f"[data-key='row/{asking}'] .request-command") or None)

        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "sessions")
        driver.click(driver.find("#settings .purge button[data-days='7']"))
        question = wait_until("the question", lambda: driver.text(driver.find("#settings .purge .purge-question")))
        self.assertTrue(question.startswith("1 session of cx, idle for more than 7 days"), question)
        driver.click(driver.find("#settings .purge .purge-go"))
        result = wait_until("the result", lambda: (text := driver.text(driver.find("#settings .purge .purge-message"))) != "purging…" and text)
        self.assertEqual(result, "1 session removed.")
        self.assertTrue((codex_thread("codex-old") or {}).get("archived"), "the old session not archived")
        self.assertFalse((codex_thread("codex-idle") or {}).get("archived"), "a session without a known age archived")


class ControlCenterKeysAndPromptAll(Journey):
    """In the control center: a paused session is not shown, and Ctrl+P opens it in a pane, started again;
    "prompt all" asks to confirm, then types the same message into each session of the project;
    Escape goes back to all projects, n picks the session that asks, a allows its permission,
    Enter goes into the picked row's field, o opens the picked session in a pane."""

    started = [("alpha", "first task work:0"), ("beta", "second task work:0"), ("dozing", "third task work:0")]

    @classmethod
    def before_app(cls) -> None:
        wait_until("dozing's turn over", lambda: fake_detail("dozing") == "waiting for you", timeout=30)
        accounts("run", ACCOUNT, "stop", next(s["id"] for s in fake_sessions() if s.get("name") == "dozing"))

    def heading(self) -> list[str]:
        return texts(self.driver, ".control-head h1")

    def picked(self) -> list[str]:
        return texts(self.driver, "article.control-row.selected .row-name")

    def to_control_center(self) -> None:
        self.driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(self.driver))

    def test_resume_prompt_all_and_keys(self) -> None:
        driver = self.driver
        carry_messages(driver)
        add_project(driver)
        self.to_control_center()
        click_text(driver, ".control-project .control-project-name", "project")
        wait_until("the project's rows", lambda: sorted(texts(driver, "article.control-row .row-name")) == ["alpha", "beta"] or None)

        # Paused, it is not on the page (decision 497): Ctrl+P opens it.
        driver.press(CONTROL, "p")
        wait_until("the session finder", lambda: dialog_open(driver, "picker"))
        driver.type_into(driver.find("#picker-query"), "dozing" + ENTER)
        wait_until("dozing open in a pane", lambda: overview_hidden(driver) and (chip(driver, "dozing") or ("", False))[1] or None)
        wait_until("dozing started again", lambda: fake_running("dozing"))

        self.to_control_center()
        wait_until("the project shown", lambda: self.heading() == ["project"] or None)
        wait_until("prompt all for the three", lambda: "prompt all 3" in texts(driver, ".control-actions button") or None)
        click_text(driver, ".control-actions button", "prompt all 3")
        field = wait_until("the field for all", lambda: driver.find("[data-key='prompt/all']"))
        driver.type_into(field, "together work:0" + ENTER)
        wait_until("the question before sending", lambda: "send to 3 sessions?" in texts(driver, ".prompt-all button") or None)
        self.assertEqual([name for name in ("alpha", "beta", "dozing") if "together work:0" in prompts_of(session_id_of(name))], [], "sent before the answer")
        click_text(driver, ".prompt-all button", "send to 3 sessions?")
        wait_until("the message typed into each", lambda: all("together work:0" in prompts_of(session_id_of(name)) for name in ("alpha", "beta", "dozing")) or None, timeout=30)

        # The button pressed is gone with the redraw: the keys still go to the control center.
        driver.press(ESCAPE)
        wait_until("all projects", lambda: self.heading() == ["all projects"] or None)
        hook = asked_by_hook(session_id_of("beta"), "Bash", {"command": "make", "description": "Build"})
        wait_until("beta's permission in its row", lambda: any("make" in text for text in texts(driver, "article.control-row .request-command")) or None)
        driver.press("n")
        wait_until("beta picked", lambda: self.picked() == ["beta"] or None)
        driver.press("a")
        self.assertEqual(decision_of(hook)["behavior"], "allow")

        driver.press(ENTER)
        focused = wait_until("beta's field focused", lambda: driver.script("return document.activeElement?.dataset?.key ?? null"))
        self.assertEqual(focused, f"prompt/{session_id_of('beta')}")
        # Escape leaves the field for the row: o then opens its session.
        driver.press(ESCAPE)
        wait_until("out of the field", lambda: driver.script("return document.activeElement?.dataset?.key ?? null") != f"prompt/{session_id_of('beta')}" or None)
        driver.press("o")
        wait_until("beta open in a pane", lambda: overview_hidden(driver) and (chip(driver, "beta") or ("", False))[1] or None)


class OpenRightAfterAPrompt(Journey):
    """A session opened right after a prompt was sent to it from the control center opens in a
    pane once kindric has typed the prompt, not refused as if another window showed it."""

    started = [("alpha", "first task work:0")]

    def test_open_while_typing(self) -> None:
        driver = self.driver
        carry_messages(driver)
        add_project(driver)
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        alpha = session_id_of("alpha")
        driver.press(ARROW_DOWN)
        wait_until("alpha picked", lambda: texts(driver, "article.control-row.selected .row-name") == ["alpha"] or None)
        driver.press(ENTER)
        field = wait_until("its field", lambda: driver.find(f"[data-key='prompt/{alpha}']"))
        driver.type_into(field, "quick work:0" + ENTER)
        driver.press(ESCAPE)
        driver.press("o")
        wait_until("the notice", lambda: "sending the message first" in driver.text(driver.find("#status")) or None, timeout=2)
        wait_until("alpha open in a pane", lambda: overview_hidden(driver) and (chip(driver, "alpha") or ("", False))[1] or None)
        self.assertIn("quick work:0", prompts_of(alpha))


def status_line(session_id: str, context: float, five_hour: float, seven_day: float) -> None:
    """kindric-accounts' status line run by Claude Code for a session of the account, with the
    figures Claude Code gives it."""
    given = {"session_id": session_id, "context_window": {"used_percentage": context},
             "rate_limits": {"five_hour": {"used_percentage": five_hour, "resets_at": time.time() + 3600},
                             "seven_day": {"used_percentage": seven_day, "resets_at": time.time() + 86400}}}
    done = subprocess.run(["kindric-accounts", "statusline"], input=json.dumps(given), capture_output=True, text=True, timeout=30,
                          cwd=PROJECT, env=dict(os.environ, CLAUDE_CONFIG_DIR=ACCOUNT_DIR))
    if done.returncode != 0:
        raise AssertionError(f"the status line exited {done.returncode}: {done.stderr.strip()}")


class UsageWindow(Journey):
    """F3 opens the usage window: the week of the account, its tokens by project, a worktree counted
    for its checkout, then 30 days with the arrow; the settings open it too. What an account with a
    key of Anthropic cost, from its status line, has a column of its own."""

    started = [("alpha", "first task work:0")]

    @classmethod
    def before_app(cls) -> None:
        folder = os.path.join(ACCOUNT_DIR, "projects", "-usage")
        os.makedirs(folder, exist_ok=True)

        def reply(days: float, cwd: str, name: str, output: int) -> str:
            at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() - days * 86400))
            usage = {"input_tokens": 0, "output_tokens": output, "cache_read_input_tokens": 900000, "cache_creation_input_tokens": 1000}
            return json.dumps({"type": "assistant", "timestamp": at, "cwd": cwd, "message": {"id": name, "model": "claude-opus-5-5", "content": [], "usage": usage}})

        with open(os.path.join(folder, "aaaaaaaa-0000-4000-8000-000000000001.jsonl"), "w", encoding="utf-8") as file:
            file.write(reply(2, "/work/shop", "m1", 4000) + "\n")
        with open(os.path.join(folder, "bbbbbbbb-0000-4000-8000-000000000002.jsonl"), "w", encoding="utf-8") as file:
            file.write(reply(2, "/work/shop/.claude/worktrees/cart", "m2", 4000) + "\n")
        with open(os.path.join(folder, "cccccccc-0000-4000-8000-000000000003.jsonl"), "w", encoding="utf-8") as file:
            file.write(reply(20, "/work/old", "m3", 49000) + "\n")

        accounts("add", "key", "--provider", "api")
        key_dir = os.path.join(os.path.dirname(ACCOUNT_DIR), "key")
        paid = "dddddddd-0000-4000-8000-000000000004"
        os.makedirs(os.path.join(key_dir, "projects", "-paid"))
        with open(os.path.join(key_dir, "projects", "-paid", f"{paid}.jsonl"), "w", encoding="utf-8") as file:
            file.write(reply(1, "/work/paid", "m4", 1000) + "\n")
        for total in (0.1, 0.42):
            given = json.dumps({"session_id": paid, "cost": {"total_cost_usd": total}})
            subprocess.run(["kindric-accounts", "statusline"], input=given, capture_output=True, text=True, timeout=30, check=True,
                           cwd=PROJECT, env=dict(os.environ, CLAUDE_CONFIG_DIR=key_dir))

    def figures(self, name: str) -> list[str]:
        found = self.driver.script("""const row = [...document.querySelectorAll('#usage .usage-section:nth-of-type(2) .usage-row')].find((row) => row.querySelector('.usage-name')?.textContent === arguments[0]);
          return row ? [...row.querySelectorAll('.usage-figure')].map((cell) => cell.textContent) : [];""", name)
        return [str(text) for text in found]

    def test_open_and_change_period(self) -> None:
        driver = self.driver
        # A week that starts again in a day: six days of it gone, past the first one.
        status_line(session_id_of("alpha"), 10, 20, 60)
        driver.press(F3)
        wait_until("the usage window", lambda: dialog_open(driver, "usage"))
        wait_until("the week of the account", lambda: texts(driver, "#usage .usage-week .usage-figure") == ["60%"] or None)
        self.assertTrue(texts(driver, "#usage .usage-week-text")[0].startswith("at this pace, 70% when it starts again "))
        wait_until("shop, its worktree included", lambda: self.figures("shop") == ["10k", "1.8M", ""] or None)
        self.assertEqual(self.figures("paid"), ["2k", "900k", "$0.42"])
        self.assertEqual(self.figures("old"), [], "20 days ago, not in the last 7")
        driver.press(ARROW_RIGHT)
        wait_until("30 days", lambda: self.figures("old") == ["50k", "900k", ""] or None)
        driver.press(ESCAPE)
        wait_until("closed", lambda: not dialog_open(driver, "usage"))

        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "accounts")
        driver.click(driver.find("#settings .usage-open"))
        wait_until("the usage window, the settings closed", lambda: dialog_open(driver, "usage") and not dialog_open(driver, "settings"))


class StatsWindow(Journey):
    """F4 opens the stats window: the tokens of the period, a graph per account, the activity map and
    the records; the arrow changes the period; the settings open it too."""

    started = [("alpha", "first task work:0")]

    @classmethod
    def before_app(cls) -> None:
        folder = os.path.join(ACCOUNT_DIR, "projects", "-stats")
        os.makedirs(folder, exist_ok=True)
        for days, output in ((1, 4000), (20, 60000)):
            at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() - days * 86400))
            usage = {"input_tokens": 0, "output_tokens": output, "cache_read_input_tokens": 6000, "cache_creation_input_tokens": 0}
            line = {"type": "assistant", "timestamp": at, "cwd": "/work/shop", "message": {"id": f"m{days}", "model": "claude-opus-5-5", "content": [], "usage": usage}}
            with open(os.path.join(folder, f"aaaaaaaa-0000-4000-8000-0000000000{days:02}.jsonl"), "w", encoding="utf-8") as file:
                file.write(json.dumps(line) + "\n")

    def hero(self) -> list[str]:
        return texts(self.driver, "#stats .stats-hero .stats-value")

    def test_open_and_change_period(self) -> None:
        driver = self.driver
        driver.press(F4)
        wait_until("the stats window", lambda: dialog_open(driver, "stats"))
        wait_until("the tokens of 30 days", lambda: self.hero() == ["76k"] or None)
        self.assertEqual(texts(driver, "#stats .stats-account .usage-name"), ["work"])
        self.assertEqual(len(driver.find_all("#stats .stats-account .usage-column")), 30)
        self.assertEqual(len(driver.find_all("#stats .stats-cell")), 30 * 24)
        self.assertIn("shop", texts(driver, "#stats .stats-value"), "the favorite project")
        driver.press(ARROW_LEFT)
        wait_until("the tokens of 7 days", lambda: self.hero() == ["10k"] or None)
        self.assertEqual(len(driver.find_all("#stats .stats-account .usage-column")), 7)
        driver.press(ESCAPE)
        wait_until("closed", lambda: not dialog_open(driver, "stats"))

        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "accounts")
        driver.click(driver.find("#settings .stats-open"))
        wait_until("the stats window, the settings closed", lambda: dialog_open(driver, "stats") and not dialog_open(driver, "settings"))


class RecapImage(Journey):
    """`r` in the stats window draws the recap of the last 30 days, `s` writes it in the home (there is
    no pictures folder here) as a PNG, a second time beside the first one."""

    started = [("alpha", "first task work:0")]

    @classmethod
    def before_app(cls) -> None:
        folder = os.path.join(ACCOUNT_DIR, "projects", "-recap")
        os.makedirs(folder, exist_ok=True)
        now = time.time()
        with open(os.path.join(folder, "bbbbbbbb-0000-4000-8000-000000000001.jsonl"), "w", encoding="utf-8") as file:
            for hour in range(1, 29 * 24):
                if hour % 24 > 14 or hour % 7 == 0:
                    continue
                at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(now - hour * 3600))
                usage = {"input_tokens": 0, "output_tokens": 2000 + hour * 37 % 9000, "cache_read_input_tokens": 400000, "cache_creation_input_tokens": 0}
                line = {"type": "assistant", "timestamp": at, "cwd": "/work/shop", "message": {"id": f"m{hour}", "model": "claude-opus-5-5", "content": [], "usage": usage}}
                file.write(json.dumps(line) + "\n")

    def note(self) -> str:
        return "".join(texts(self.driver, "#stats .recap-note"))

    def saved(self, name: str) -> tuple[int, int]:
        with open(os.path.join(HOME, name), "rb") as file:
            head = file.read(24)
        self.assertEqual(head[:8], b"\x89PNG\r\n\x1a\n")
        return int.from_bytes(head[16:20], "big"), int.from_bytes(head[20:24], "big")

    def test_draw_and_save(self) -> None:
        driver = self.driver
        driver.press(F4)
        wait_until("the stats window", lambda: dialog_open(driver, "stats") and texts(driver, "#stats .stats-hero .stats-value") or None)
        driver.press("r")
        wait_until("the note", lambda: "no name of an account" in self.note() or None)
        self.assertEqual(texts(driver, "#stats .usage-periods button"), [])
        pixel = """const canvas = document.querySelector('#stats canvas');
            const [red, green, blue] = canvas.getContext('2d').getImageData(arguments[0], arguments[1], 1, 1).data;
            return red + green + blue;"""
        wait_until("the band at the foot, bright", lambda: driver.script(pixel, 80, 1300) > 300 or None)
        self.assertLess(driver.script(pixel, 4, 700), 120, "the night around")
        label = driver.script("return document.querySelector('#stats canvas').getAttribute('aria-label')")
        self.assertIn("tokens in 30 days", label)
        self.assertNotIn("shop", label)
        driver.screenshot(os.path.join(OUT, "RecapImage-shown.png"))

        driver.press("s")
        wait_until("the first image", lambda: self.note().endswith("kindric-recap.png") or None)
        self.assertEqual(self.saved("kindric-recap.png"), (1080, 1350))
        driver.press("s")
        wait_until("the second beside it", lambda: self.note().endswith("kindric-recap-2.png") or None)
        shutil.copyfile(os.path.join(HOME, "kindric-recap.png"), os.path.join(OUT, "RecapImage-saved.png"))

        driver.press("r")
        wait_until("the stats again", lambda: texts(driver, "#stats .stats-hero .stats-value") or None)
        click_text(driver, "#stats .stats-actions button", "recap image")
        wait_until("the image again", lambda: driver.find_all("#stats canvas") or None)
        driver.press(ESCAPE)
        wait_until("closed", lambda: not dialog_open(driver, "stats"))


class ControlCenterFigures(Journey):
    """What the status line wrote down shows in the control center: the account's 5 h and 7 day
    limits, framed past 80 %, with when the 5 h window starts again, and how full each session's
    context is. A row counts the files its session kept for the user, the new ones too; a click
    opens the artifacts window, after which none is new."""

    started = [("alpha", "make touch:a.txt touch:b.txt work:0"), ("beta", "rest work:0")]

    def cell(self, name: str, css: str) -> str:
        found = self.driver.script("""const row = [...document.querySelectorAll('article.control-row')].find((row) => row.querySelector('.row-name').textContent === arguments[0]);
          const cell = row && row.querySelector(arguments[1]);
          return cell ? cell.textContent : null;""", name, css)
        return "" if found is None else str(found)

    def test_limits_context_and_kept(self) -> None:
        driver = self.driver
        carry_messages(driver)
        add_project(driver)
        wait_until("alpha's files", lambda: all(os.path.exists(os.path.join(PROJECT, name)) for name in ("a.txt", "b.txt")))
        alpha = session_id_of("alpha")
        status_line(alpha, 42.4, 85, 30)
        for name in ("a.txt", "b.txt"):
            mcp_call(alpha, "keep_artifact", {"path": name, "for": "user"})

        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        click_text(driver, ".control-project .control-project-name", "project")
        wait_until("the account's limits", lambda: texts(driver, ".account-card .account-percent") == ["85%", "30%"] or None)
        self.assertTrue(driver.script("return document.querySelector('.account-card').classList.contains('hot')"), "past 80 % and not framed")
        self.assertTrue(any(note.startswith("5h resets at ") for note in texts(driver, ".account-card .account-note")))
        self.assertEqual(texts(driver, ".account-card .account-provider"), ["claude"], "the card names where the model comes from (417)")
        wait_until("alpha's context", lambda: self.cell("alpha", ".row-ctx") == "ctx42%" or None)
        self.assertEqual(self.cell("beta", ".row-ctx"), "ctx—")

        wait_until("alpha's kept files, new", lambda: self.cell("alpha", ".row-kept") == "2 kept · 2 new" or None)
        self.assertEqual(self.cell("beta", ".row-kept"), "nothing kept")
        found = driver.script("""return [...document.querySelectorAll('article.control-row')].find((row) => row.querySelector('.row-name').textContent === 'alpha').querySelector('.row-kept');""")
        driver.click(str(found[ELEMENT_KEY]))
        wait_until("the artifacts window", lambda: dialog_open(driver, "artifacts"))
        driver.press(ESCAPE)
        wait_until("the artifacts window closed", lambda: not dialog_open(driver, "artifacts"))
        wait_until("none new once seen", lambda: self.cell("alpha", ".row-kept") == "2 kept" or None)


SECOND_PROJECT = os.path.join(HOME, "second")
THIRD_PROJECT = os.path.join(HOME, "third")


def add_project_at(driver: Driver, directory: str) -> None:
    """Ctrl+Shift+T on `directory`: its tab, named after the folder, active."""
    driver.press(CONTROL, SHIFT, "t")
    wait_until("the project form", lambda: dialog_open(driver, "new-project"))
    field = driver.find("#project-form input[name=directory]")
    driver.clear(field)
    driver.type_into(field, directory)
    driver.click(driver.find("#project-form button[type=submit]"))
    name = os.path.basename(directory)
    wait_until(f"the tab {name}, active", lambda: (name, True) in tabs(driver))


class NestedProjects(Journey):
    """A project inside another: a session working in the inner folder, or below it, belongs to the
    inner project's tab; one in the outer folder stays in the outer tab."""

    INNER = os.path.join(PROJECT, "inner")

    @classmethod
    def before_app(cls) -> None:
        os.makedirs(os.path.join(cls.INNER, "deep"), exist_ok=True)
        for name, folder in (("outer-one", PROJECT), ("inner-one", cls.INNER), ("deep-one", os.path.join(cls.INNER, "deep"))):
            done = subprocess.run(["kindric-accounts", "run", ACCOUNT, "--bg", "-n", name, "a task work:0"], cwd=folder, capture_output=True, text=True, timeout=30)
            if done.returncode != 0:
                raise AssertionError(f"{name} not started in {folder}: {done.stderr.strip()}")

    def test_deepest_project_wins(self) -> None:
        driver = self.driver
        wait_until("the sessions in the bar", lambda: sorted(chip_titles(driver)) == ["deep-one", "inner-one", "outer-one"] or None)
        add_project_at(driver, PROJECT)
        add_project_at(driver, self.INNER)
        wait_until("the inner tab's sessions", lambda: sorted(chip_titles(driver)) == ["deep-one", "inner-one"] or None)
        driver.press(CONTROL, SHIFT, "h")
        wait_until("the outer tab", lambda: ("project", True) in tabs(driver) or None)
        wait_until("the outer tab's session", lambda: chip_titles(driver) == ["outer-one"] or None)


def stop_hook(command: str, session: dict[str, Any], stop_hook_active: bool = False) -> str:
    """What the Stop hook `kindric <command>` answers Claude Code at the end of the session's turn."""
    folder = re.sub(r"[/.]", "-", str(session["cwd"]))
    given = {"hook_event_name": "Stop", "session_id": session["sessionId"], "cwd": session["cwd"], "stop_hook_active": stop_hook_active,
             "transcript_path": os.path.join(ACCOUNT_DIR, "projects", folder, f"{session['sessionId']}.jsonl")}
    done = subprocess.run(["kindric", command], input=json.dumps(given), capture_output=True, text=True, timeout=30)
    return done.stdout.strip()


class HooksOfTheTurn(Journey):
    """A session started by kindric gets the hooks `overview-start` (SessionStart), which tells it to
    ask with `kindric ask`, and `artifact-check` (Stop): a turn that made an image and kept none is
    asked once to keep it; a turn that kept it, or the second stop of a turn, is let go."""

    started = [("drawer", "draw image:logo.png work:0")]

    def test_overview_start_and_artifact_check(self) -> None:
        wait_until("the turn over", lambda: fake_detail("drawer") == "waiting for you", timeout=30)
        session = next(s for s in fake_sessions() if s.get("name") == "drawer")
        hooks = hook_commands(settings_of(session))
        self.assertTrue(any(command.endswith("overview-start") for command in hooks.get("SessionStart", [])), hooks)
        self.assertTrue(any(command.endswith("artifact-check") for command in hooks.get("Stop", [])), hooks)
        started = subprocess.run(["kindric", "overview-start"], capture_output=True, text=True, timeout=30)
        self.assertIn('kindric ask "<question>" "<context>"', started.stdout)

        answer = json.loads(stop_hook("artifact-check", session))
        self.assertEqual(answer["decision"], "block")
        self.assertIn("logo.png", answer["reason"])
        self.assertEqual(stop_hook("artifact-check", session, stop_hook_active=True), "", "asked twice in a turn")

        attached = subprocess.run(["kindric-accounts", "run", ACCOUNT, "attach", session["id"]], input="draw again image:logo2.png keep:logo2.png work:0\n",
                                  cwd=PROJECT, capture_output=True, text=True, timeout=30)
        self.assertEqual(attached.returncode, 0, attached.stderr)
        wait_until("the second turn over", lambda: len(prompts_of(session["sessionId"])) == 2 and fake_detail("drawer") == "waiting for you", timeout=30)
        self.assertEqual(stop_hook("artifact-check", session), "", "asked although the turn kept its image")


def tab_button(driver: Driver, name: str) -> str:
    found = driver.script("""return [...document.querySelectorAll('#tabs button.tab')].find((tab) => tab.querySelector('.tab-name').textContent === arguments[0]);""", name)
    if not isinstance(found, dict) or ELEMENT_KEY not in found:
        raise AssertionError(f"no tab {name}")
    return str(found[ELEMENT_KEY])


class TabMovedByMouse(Journey):
    """A project tab dragged along the bar takes its new place, which "other" never leaves, and
    keeps it after a restart."""

    SECOND = os.path.join(HOME, "second")

    def names(self) -> list[str]:
        return [name for name, _ in tabs(self.driver)]

    def test_drag_then_kept(self) -> None:
        driver = self.driver
        os.makedirs(self.SECOND, exist_ok=True)
        add_project_at(driver, PROJECT)
        add_project_at(driver, self.SECOND)
        start = self.names()
        self.assertEqual(start[-3:], ["other", "project", "second"])

        driver.drag(tab_button(driver, "second"), tab_button(driver, "other"))
        wait_until("second before project", lambda: self.names()[-3:] == ["other", "second", "project"] or None)
        type(self).restart()
        wait_until("the order after a restart", lambda: self.names()[-3:] == ["other", "second", "project"] or None)


class AltKeysAndAnotherTerminal(Journey):
    """A session in a terminal of its own shows last, with its own mark, and cannot be opened:
    Alt+n on its chip opens nothing, and Alt+L and Alt+H go round the others without it."""

    started = [("one", "a task work:0"), ("two", "a task work:0")]

    @classmethod
    def before_app(cls) -> None:
        subprocess.run(["kindric-accounts", "run", ACCOUNT, "--bg", "-n", "term", "typing in a terminal work:300"], cwd=PROJECT,
                       check=True, capture_output=True, timeout=30, env=dict(os.environ, FAKE_INTERACTIVE="1"))

    def opened(self) -> list[str]:
        chips: list[list[Any]] = self.driver.script(CHIPS)
        return [str(title) for _, title, shown in chips if shown]

    def test_numbers_steps_and_elsewhere(self) -> None:
        driver = self.driver
        wait_until("three chips", lambda: len(chip_titles(driver)) == 3 or None)
        wait_until("one and two at rest", lambda: chip_state(driver, "one") == "waiting" and chip_state(driver, "two") == "waiting" or None, timeout=30)
        titles = chip_titles(driver)
        self.assertEqual(titles[-1], "term", "the session of another terminal not last")
        self.assertEqual(chip_state(driver, "term"), "elsewhere")
        first, second = titles[0], titles[1]

        driver.press(ALT, "2")
        wait_until("chip 2 open", lambda: self.opened() == [second] or None)
        driver.press(ALT, "3")
        time.sleep(1)
        self.assertEqual(self.opened(), [second], "the session of another terminal opened")
        driver.press(ALT, "l")
        wait_until("Alt+L round to chip 1, over the other terminal", lambda: self.opened() == [first] or None)
        driver.press(ALT, "h")
        wait_until("Alt+H round to chip 2", lambda: self.opened() == [second] or None)
        driver.press(ALT, "1")
        wait_until("chip 1 open", lambda: self.opened() == [first] or None)


class OldClaudeCodeWarned(Journey):
    """F1 lists what is left to set up: a Claude Code older than the one kindric was tested with
    gets a warning and the command that updates it, then every shortcut."""

    @classmethod
    def prepare(cls) -> None:
        os.makedirs(os.path.join(CLAUDE_HOME, "fake"), exist_ok=True)
        write(os.path.join(CLAUDE_HOME, "fake/version"), "2.1.300")

    def test_warning_in_f1(self) -> None:
        driver = self.driver
        driver.press("")
        wait_until("the help", lambda: dialog_open(driver, "help"))
        checks = wait_until("Claude Code's line", lambda: [found for found in driver.script("""return [...document.querySelectorAll('#help li.check')].map(
          (item) => [item.querySelector('.check-state').textContent, item.querySelector('.check-text').textContent,
                     [...item.querySelectorAll('.check-copy code')].map((node) => node.textContent)]);""") if str(found[1]).startswith("Claude Code")] or None)
        state, text, copies = checks[0]
        self.assertEqual(state, "warning")
        self.assertTrue(str(text).startswith("Claude Code 2.1.300"), text)
        self.assertEqual(copies, ["claude update"])
        self.assertTrue(driver.find_all("#help kbd"), "the shortcuts after the list")


class DirectoryFieldCompletion(Journey):
    """The directory field of Ctrl+Shift+T lists the folders in use before anything is typed, then
    the folders that can follow what is typed, case ignored, hidden ones only after a dot; Tab
    completes, Down and Enter take one."""

    WORK = os.path.join(HOME, "work")
    # How the field writes it: the home folder as ~.
    SHOWN = "~/work"

    @classmethod
    def prepare(cls) -> None:
        shutil.rmtree(cls.WORK, ignore_errors=True)
        for folder in ("Alpha/sub", "beta", ".hidden", "known"):
            os.makedirs(os.path.join(cls.WORK, folder))

    @classmethod
    def before_app(cls) -> None:
        subprocess.run(["kindric-accounts", "run", ACCOUNT, "--bg", "-n", "in-known", "a task work:0"], cwd=os.path.join(cls.WORK, "known"),
                       check=True, capture_output=True, timeout=30)

    def suggestions(self) -> list[str]:
        return [item.rstrip("/") for item in texts(self.driver, "#new-project .suggestions li")]

    def value(self) -> str:
        return str(self.driver.script("return document.querySelector('#project-form input[name=directory]').value")).rstrip("/")

    def test_known_then_typed_tab_and_enter(self) -> None:
        driver = self.driver
        wait_until("the session in the bar", lambda: chip_state(driver, "in-known"))
        driver.press(CONTROL, SHIFT, "t")
        wait_until("the project form", lambda: dialog_open(driver, "new-project"))
        field = driver.find("#project-form input[name=directory]")
        wait_until("the folders in use", lambda: f"{self.SHOWN}/known" in self.suggestions() or None)

        driver.clear(field)
        driver.type_into(field, f"{self.SHOWN}/a")
        wait_until("the folders that follow, case ignored", lambda: self.suggestions() == [f"{self.SHOWN}/Alpha"] or None)
        driver.type_into(field, TAB)
        wait_until("completed by Tab", lambda: self.value() == f"{self.SHOWN}/Alpha" or None)
        wait_until("its own folders next", lambda: self.suggestions() == [f"{self.SHOWN}/Alpha/sub"] or None)

        driver.clear(field)
        driver.type_into(field, f"{self.SHOWN}/")
        wait_until("the folders of work, no hidden one", lambda: sorted(self.suggestions()) == sorted(f"{self.SHOWN}/{name}" for name in ("Alpha", "beta", "known")) or None)
        driver.type_into(field, ".")
        wait_until("the hidden one after a dot", lambda: self.suggestions() == [f"{self.SHOWN}/.hidden"] or None)
        driver.type_into(field, ARROW_DOWN + ENTER)
        wait_until("taken by Down and Enter", lambda: self.value() == f"{self.SHOWN}/.hidden" or None)
        self.assertTrue(dialog_open(driver, "new-project"), "Enter on a suggestion submitted the form")


class CopyAndPaste(Journey):
    """Text selected with the mouse in a pane, Ctrl+Shift+C copies it; Ctrl+Shift+V pastes it into
    the session of the pane, as one block, and Enter sends it."""

    started = [("pasted", "a task work:0")]

    def select_first_line(self) -> None:
        """Presses the mouse at the start of the pane's first line and lets go at its end."""
        screen = self.driver.find(f"{VISIBLE_PANES}.active .xterm-screen")
        box = self.driver.script("const box = arguments[0].getBoundingClientRect(); return [box.width, box.height];", {ELEMENT_KEY: screen})
        width, height = float(box[0]), float(box[1])
        origin = {ELEMENT_KEY: screen}
        top = int(-height / 2 + 6)
        moves = [{"type": "pointerMove", "origin": origin, "x": int(-width / 2 + 2), "y": top}, {"type": "pointerDown", "button": 0},
                 {"type": "pointerMove", "origin": origin, "x": int(width / 2 - 4), "y": top, "duration": 200}, {"type": "pointerUp", "button": 0}]
        self.driver._command("POST", "/actions", {"actions": [{"type": "pointer", "id": "mouse", "parameters": {"pointerType": "mouse"}, "actions": moves}]})
        self.driver._command("DELETE", "/actions")

    def test_select_copy_paste(self) -> None:
        driver = self.driver
        wait_until("the turn over", lambda: fake_detail("pasted") == "waiting for you", timeout=30)
        open_from_bar(driver, "pasted")
        time.sleep(1)
        self.select_first_line()
        driver.press(CONTROL, SHIFT, "c")
        time.sleep(0.5)
        self.assertNotIn("cannot copy", driver.text(driver.find("#status")))
        driver.press(CONTROL, SHIFT, "v")
        time.sleep(0.5)
        self.assertNotIn("cannot paste", driver.text(driver.find("#status")))
        type_in_pane(driver, "")
        session = session_id_of("pasted")
        last = wait_until("the pasted line sent as a turn", lambda: prompts_of(session)[1:] or None, timeout=15)
        self.assertEqual(last[-1], f"fake claude · pasted · {PROJECT}")


class AwayBackAndDone(Journey):
    """Ctrl+Shift+Y with minutes: the pane's head says until when, and the hook keeps the session
    working. Again with 0, the form's proposal: back, the session may stop. Away once more, then
    the session says `kindric away done`: it may stop, and the head no longer says away."""

    started = [("helper", "a task work:0")]

    def head_away(self) -> str:
        return texts(self.driver, f"{VISIBLE_PANES}.active .head-away")[0]

    def away_for(self, minutes: str | None) -> None:
        """Ctrl+Shift+Y, then `minutes` typed, or the form's proposal kept with None."""
        driver = self.driver
        driver.press(CONTROL, SHIFT, "y")
        wait_until("the away form", lambda: dialog_open(driver, "away"))
        field = driver.find("#away-form input[name=minutes]")
        if minutes is not None:
            driver.clear(field)
            driver.type_into(field, minutes)
        driver.click(driver.find("#away-form button[type=submit]"))
        wait_until("the form closed", lambda: not dialog_open(driver, "away"))

    def test_away_back_done(self) -> None:
        driver = self.driver
        wait_until("the turn over", lambda: fake_detail("helper") == "waiting for you", timeout=30)
        open_from_bar(driver, "helper")
        session = session_id_of("helper")

        self.away_for("30")
        wait_until("away in the pane's head", lambda: self.head_away().startswith("· away until ") or None)
        self.assertIn('"decision":"block"', away_hook("Stop", session))

        driver.press(CONTROL, SHIFT, "y")
        wait_until("the away form", lambda: dialog_open(driver, "away"))
        self.assertEqual(driver.script("return document.querySelector('#away-form input[name=minutes]').value"), "0", "back not proposed")
        driver.press(ESCAPE)
        wait_until("the form closed", lambda: not dialog_open(driver, "away"))
        self.away_for("0")
        wait_until("back, no longer away in the head", lambda: self.head_away() == "" or None)
        self.assertEqual(away_hook("Stop", session), "", "still kept working once back")

        self.away_for("60")
        wait_until("away again", lambda: self.head_away().startswith("· away until ") or None)
        done = subprocess.run(["kindric", "away", "done", session], capture_output=True, text=True, timeout=30)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(away_hook("Stop", session), "", "still kept working after away done")
        wait_until("no longer away in the head", lambda: self.head_away() == "" or None, timeout=30)


class MessagesFromTheChipBadge(Journey):
    """A click on the unread count of a chip shows that session's messages, without opening the
    session in a pane, and reads nothing: the count stays."""

    started = [("writer", "write work:0"), ("reader", "read work:0")]

    def test_badge_click(self) -> None:
        driver = self.driver
        wait_until("both sessions in the bar", lambda: chip_state(driver, "writer") and chip_state(driver, "reader"))
        mcp_call(session_id_of("writer"), "send_message", {"to": "reader", "text": "tests are green"})
        wait_until("the unread count on reader", lambda: chip_badge(driver, "reader") == "1" or None)
        badge = driver.script("""return [...document.querySelectorAll('#chips li.chip:not(.more)')].find(
          (chip) => chip.querySelector('.chip-title').textContent === 'reader').querySelector('.badge');""")
        driver.click(str(badge[ELEMENT_KEY]))
        wait_until("the messages window", lambda: dialog_open(driver, "messages"))
        wait_until("reader's message in it", lambda: "tests are green" in driver.text(driver.find("#messages-list")) or None)
        self.assertFalse((chip(driver, "reader") or ("", True))[1], "reader opened in a pane")
        driver.press(ESCAPE)
        wait_until("the window closed", lambda: not dialog_open(driver, "messages"))
        self.assertEqual(chip_badge(driver, "reader"), "1", "the message marked read")


class AtlasKeysAndSearch(Journey):
    """The memory atlas by the keyboard: an arrow picks the project, Enter goes in, Escape comes
    back; typed letters search the memory, Enter goes to the first file found and shows it, and
    Escape goes back up to the whole map. WebKit's driver drops its connection when it sends keys
    while the atlas draws (a frame takes 60 to 100 ms under Xvfb): the keys are sent from the page."""

    @classmethod
    def prepare(cls) -> None:
        AtlasZoomWithCtrlWheel.prepare()

    def caption(self) -> str:
        return str(self.driver.script("return document.querySelector('#atlas .atlas-caption').textContent"))

    def key(self, name: str, target: str = "#atlas") -> None:
        self.driver.script("""document.querySelector(arguments[1]).dispatchEvent(new KeyboardEvent('keydown', { key: arguments[0], bubbles: true, cancelable: true }));""", name, target)

    def test_keys_search_and_back(self) -> None:
        driver = self.driver
        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "memory")
        driver.click(driver.find("#settings .memory-atlas-open"))
        wait_until("the atlas", lambda: dialog_open(driver, "atlas"))
        wait_until("the memory read", lambda: "2 files" in str(driver.script("return document.querySelector('#atlas .atlas-cartouche-count').textContent")) or None)

        self.key("ArrowRight")
        # The caption names what the arrow picked instead of the hints.
        wait_until("something picked", lambda: not self.caption().startswith("type to search") or None)
        self.key("Enter")
        wait_until("one level down", lambda: self.caption().startswith("click a ") or None)
        self.key("Escape")
        wait_until("the whole map again", lambda: self.caption().startswith("type to search") or None)

        for letter in "chose":
            self.key(letter)
        wait_until("one file found", lambda: "1 files hold “chose”" in self.caption() or None)
        self.key("Enter", "#atlas .atlas-search")
        wait_until("the file shown", lambda: "we chose" in str(driver.script("return document.querySelector('#atlas .atlas-note').textContent")) or None)
        self.assertTrue(self.caption().startswith("esc comes back"), self.caption())
        for _ in range(4):
            self.key("Escape")
            time.sleep(0.3)
        wait_until("the whole map, the search gone", lambda: self.caption() == "type to search · drag or two fingers sideways to turn · pinch or wheel in and out · click a project · esc closes" or None)

    @classmethod
    def tearDownClass(cls) -> None:
        super().tearDownClass()
        shutil.rmtree(MEMORY, ignore_errors=True)


class AtlasRepairsALostNote(Journey):
    """A note its project's INDEX.md does not name, found in the atlas: the reader offers to add
    it, and a click adds its line at the end of INDEX.md, the rest of the file kept (decision 337)."""

    INDEX = "- status/demo.md : where demo is\n- decisions/2026-11-01-001.md : first choice\n"

    @classmethod
    def prepare(cls) -> None:
        AtlasZoomWithCtrlWheel.prepare()
        note("projets/demo/knowledge/ports.md", "---\ndescription: which port each service takes\nupdated: 2026-11-03\n---\n\n- the zebra port\n")

    key = AtlasKeysAndSearch.key

    def text_of(self, selector: str) -> str:
        return str(self.driver.script("const found = document.querySelector(arguments[0]); return found ? found.textContent : '';", selector))

    def test_add_to_the_index(self) -> None:
        driver = self.driver
        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "memory")
        driver.click(driver.find("#settings .memory-atlas-open"))
        wait_until("the atlas", lambda: dialog_open(driver, "atlas"))
        wait_until("the memory read", lambda: "3 files" in self.text_of("#atlas .atlas-cartouche-count") or None)
        for letter in "zebra":
            self.key(letter)
        wait_until("the lost note found", lambda: "1 files hold" in self.text_of("#atlas .atlas-caption") or None)
        self.key("Enter", "#atlas .atlas-search")
        wait_until("the note shown", lambda: "zebra port" in self.text_of("#atlas .atlas-note") or None)
        self.assertEqual(self.text_of("#atlas .atlas-repair"), "not in INDEX.md: add it")
        driver.screenshot(os.path.join(OUT, "AtlasRepairsALostNote-offered.png"))
        driver.script("document.querySelector('#atlas .atlas-repair').click()")
        wait_until("the line added", lambda: "added to INDEX.md" in self.text_of("#atlas .atlas-note") or None)
        with open(os.path.join(MEMORY, "projets/demo/INDEX.md")) as index:
            self.assertEqual(index.read(), self.INDEX + "- knowledge/ports.md : which port each service takes\n")
        self.assertEqual(self.text_of("#atlas .atlas-repair"), "", "no button once repaired")

    @classmethod
    def tearDownClass(cls) -> None:
        super().tearDownClass()
        shutil.rmtree(MEMORY, ignore_errors=True)


class AtlasShowsWhoUsedANote(Journey):
    """A note a session read and one of its agents wrote in the last hour: its planet gets a small
    moon, and its reader says who did what (decision 337), from the account's transcripts. A
    session of the app's list is named by its title; one it does not know by its id."""

    started = [("notes reader", "work:0")]
    UNKNOWN = "6f1c2a40-0000-4000-8000-00000000a5a5"

    @classmethod
    def prepare(cls) -> None:
        AtlasZoomWithCtrlWheel.prepare()

    @classmethod
    def before_app(cls) -> None:
        decision = os.path.join(MEMORY, "projets/demo/decisions/2026-11-01-001.md")
        now = time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())

        def tool(name: str, tool_input: dict[str, Any]) -> str:
            return json.dumps({"type": "assistant", "timestamp": now, "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "t1", "name": name, "input": tool_input}]}}, separators=(",", ":")) + "\n"

        wait_until("the session started", lambda: fake_sessions() or None)
        known = fake_sessions()[0]["sessionId"]
        folder = os.path.join(ACCOUNT_DIR, "projects", "-home-tester-project")
        os.makedirs(os.path.join(folder, known, "subagents"), exist_ok=True)
        with open(os.path.join(folder, f"{known}.jsonl"), "a") as out:
            out.write(tool("Read", {"file_path": decision}))
        with open(os.path.join(folder, known, "subagents", "agent-a1.jsonl"), "w") as out:
            out.write(tool("Edit", {"file_path": decision, "old_string": "we", "new_string": "we"}))
        with open(os.path.join(folder, f"{cls.UNKNOWN}.jsonl"), "w") as out:
            out.write(tool("Bash", {"command": f"cat {decision}"}))

    key = AtlasKeysAndSearch.key
    text_of = AtlasRepairsALostNote.text_of

    def test_who_used_it(self) -> None:
        driver = self.driver
        driver.press(CONTROL, ",")
        wait_until("the settings", lambda: dialog_open(driver, "settings"))
        click_text(driver, "#settings .settings-categories li", "memory")
        driver.click(driver.find("#settings .memory-atlas-open"))
        wait_until("the atlas", lambda: dialog_open(driver, "atlas"))
        wait_until("the memory read", lambda: "2 files" in self.text_of("#atlas .atlas-cartouche-count") or None)
        for letter in "chose":
            self.key(letter)
        wait_until("the decision found", lambda: "1 files hold" in self.text_of("#atlas .atlas-caption") or None)
        self.key("Enter", "#atlas .atlas-search")
        wait_until("who used it", lambda: self.text_of("#atlas .atlas-uses") or None)
        uses = str(driver.script("return [...document.querySelectorAll('#atlas .atlas-uses li')].map((line) => line.textContent)"))
        self.assertIn(f"notes reader (an agent of it) · {ACCOUNT} · wrote ", uses)
        self.assertIn(f"notes reader · {ACCOUNT} · read ", uses)
        self.assertIn(f"{self.UNKNOWN[:8]} · {ACCOUNT} · read ", uses)
        driver.screenshot(os.path.join(OUT, "AtlasShowsWhoUsedANote-reader.png"))

    @classmethod
    def tearDownClass(cls) -> None:
        super().tearDownClass()
        shutil.rmtree(MEMORY, ignore_errors=True)


class SignalWithoutLayerShell(Journey):
    """`kindric-desktop signal` on a screen without Wayland's layer shell (X here): `on` says it
    cannot and exits with 1, `off` with nothing shown does nothing and exits with 0, a wrong
    command line exits with 2."""

    def signal(self, *words: str) -> subprocess.CompletedProcess[str]:
        # A Linux desktop always has XDG_RUNTIME_DIR, where the signal keeps its file; the image has none.
        runtime = os.path.join(HOME, "runtime")
        os.makedirs(runtime, mode=0o700, exist_ok=True)
        environment = {name: value for name, value in os.environ.items() if name != "WAYLAND_DISPLAY"}
        environment["XDG_RUNTIME_DIR"] = runtime
        return subprocess.run([application(), "signal", *words], capture_output=True, text=True, timeout=15, env=environment)

    def test_refused_off_and_usage(self) -> None:
        on = self.signal("on")
        self.assertEqual(on.returncode, 1, on.stderr)
        self.assertTrue(on.stderr.startswith("kindric-desktop: "), on.stderr)
        off = self.signal("off")
        self.assertEqual((off.returncode, off.stderr), (0, ""))
        wrong = self.signal("blink")
        self.assertEqual(wrong.returncode, 2, wrong.stderr)



def shown_tab(driver: Driver) -> str:
    """The name of the tab shown, "0" for the control center."""
    if not overview_hidden(driver):
        return "0"
    return next((name for name, active in tabs(driver) if active), "")


class TabStepAcrossEmptyTabs(Journey):
    """Ctrl+Shift+H and Ctrl+Shift+L go through every tab, the control center included, and wrap
    around, while the project tabs have no session (reported by Louis on 2026-11-16: the keys
    stopped there). First with the welcome list over the empty tabs, then with the list of
    accounts, which takes the focus, once nothing is left to set up."""

    # A session in the test project, which gets no tab: it belongs to "other", and the two
    # project tabs stay empty.
    started = [("elsewhere", "somewhere else work:0")]

    @classmethod
    def prepare(cls) -> None:
        os.makedirs(SECOND_PROJECT, exist_ok=True)
        os.makedirs(THIRD_PROJECT, exist_ok=True)

    def step(self, key: str, expected: str) -> None:
        self.driver.press(CONTROL, SHIFT, key)
        wait_until(f"Ctrl+Shift+{key.upper()} to {expected}", lambda: shown_tab(self.driver) == expected, timeout=5)

    def round_trip(self) -> None:
        for expected in ["second", "other", "0", "third"]:
            self.step("h", expected)
        for expected in ["0", "other", "second", "third"]:
            self.step("l", expected)

    def test_every_tab_in_both_directions(self) -> None:
        driver = self.driver
        add_project_at(driver, SECOND_PROJECT)
        add_project_at(driver, THIRD_PROJECT)
        wait_until("the welcome list over the empty tab", lambda: not driver.script("return document.getElementById('welcome').hidden"))
        self.round_trip()

        carry_messages(driver)
        wait_until("the list of accounts in the empty tab", lambda: not driver.script("return document.getElementById('quick').hidden"))
        self.round_trip()
        # The same once the list of accounts is clicked, as the user would: on its title, since a
        # click on a row starts a session or opens the form.
        driver.click(driver.find("#quick-title"))
        self.round_trip()


def in_all_toggle(driver: Driver, name: str) -> str | None:
    """Whether the dot left of `name` on the control center's side shows it in "all projects":
    "true" or "false"; None for an entry without a dot ("all", "other")."""
    found = driver.script("""const line = [...document.querySelectorAll('.control-project-line')].find(
      (line) => line.querySelector('.control-project-name').textContent === arguments[0]);
      const toggle = line && line.querySelector('button.in-all-toggle');
      return toggle ? toggle.getAttribute('aria-pressed') : null;""", name)
    return None if found is None else str(found)


def kept_out_of_all() -> list[str]:
    """The projects projects.json keeps out of "all projects"."""
    try:
        with open(PROJECTS_FILE) as source:
            projects: list[dict[str, Any]] = json.load(source)
    except (OSError, ValueError):
        return []
    return [str(project["name"]) for project in projects if project.get("out_of_all")]


class ProjectLeftOutOfAll(Journey):
    """The dot left of a project's name on the control center's side leaves it out of "all
    projects": its group goes, what its sessions ask stays in "needs you", its own view still has
    every session, and the next start keeps it out; the dot again brings it back (351)."""

    started = [("calm", "a quiet task work:0"), ("asks", "a task that asks work:0")]

    def all_rows(self) -> list[str]:
        return sorted(texts(self.driver, "article.control-row .row-name"))

    def click_dot(self, name: str) -> None:
        found = self.driver.script("""const line = [...document.querySelectorAll('.control-project-line')].find(
          (line) => line.querySelector('.control-project-name').textContent === arguments[0]);
          return line && line.querySelector('button.in-all-toggle');""", name)
        if not isinstance(found, dict) or ELEMENT_KEY not in found:
            raise AssertionError(f"no dot left of {name} on the side")
        self.driver.click(str(found[ELEMENT_KEY]))

    def control_center(self) -> None:
        self.driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(self.driver))

    def test_left_out_asks_still_and_kept(self) -> None:
        driver = self.driver
        carry_messages(driver)
        add_project(driver)
        self.control_center()
        wait_until("both sessions in all", lambda: self.all_rows() == ["asks", "calm"])
        self.assertEqual(in_all_toggle(driver, "project"), "true", "a project shows in all by default")
        self.assertEqual((in_all_toggle(driver, "all"), in_all_toggle(driver, "other")), (None, None))

        hook = asked_by_hook(session_id_of("asks"), "Bash", {"command": "touch e2e-made.txt", "description": "Create a file"})
        # The page reads the requests every second, after the list says the session asks.
        wait_until("the command in the row", lambda: any("touch e2e-made.txt" in text for text in texts(driver, "article.control-row .request-command")))
        self.click_dot("project")
        wait_until("the project left out", lambda: in_all_toggle(driver, "project") == "false")
        wait_until("only what needs you left in all", lambda: self.all_rows() == ["asks"])
        self.assertEqual(texts(driver, "h2.control-group.is-need"), ["needs you · 1"])
        self.assertNotIn("project ›", texts(driver, ".control-group-name"))
        self.assertEqual(kept_out_of_all(), ["project"])
        driver.screenshot(os.path.join(OUT, "ProjectLeftOutOfAll-out.png"))

        click_text(driver, "article.control-row .request button", "Allow")
        self.assertEqual(decision_of(hook)["behavior"], "allow")
        wait_until("all empty, saying why", lambda: self.all_rows() == [] and any("left out of all" in text for text in texts(driver, ".control-body .overview-note")))

        type(self).restart()
        driver = self.driver
        self.control_center()
        wait_until("still left out after a restart", lambda: in_all_toggle(driver, "project") == "false")
        wait_until("all still without the project", lambda: any("left out of all" in text for text in texts(driver, ".control-body .overview-note")))

        click_text(driver, ".control-project .control-project-name", "project")
        wait_until("the project's own view with both", lambda: self.all_rows() == ["asks", "calm"])
        click_text(driver, ".control-project .control-project-name", "all")
        self.click_dot("project")
        wait_until("the project back in all", lambda: self.all_rows() == ["asks", "calm"])
        self.assertEqual(in_all_toggle(driver, "project"), "true")
        self.assertEqual(kept_out_of_all(), [])

if __name__ == "__main__":
    unittest.main(verbosity=2)
