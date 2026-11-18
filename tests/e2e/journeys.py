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
import time
import unittest
from typing import Any

from driver import ALT, ARROW_DOWN, CONTROL, DELETE, ELEMENT_KEY, ENTER, ESCAPE, F2, SHIFT, Driver, wait_until

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


def away_hook(event: str, session_id: str, command: str | None = None) -> str:
    """What `kindric away-hook` answers Claude Code for this event of the session."""
    asked: dict[str, Any] = {"hook_event_name": event, "session_id": session_id}
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
        wait_until("away until, in the bar's end", lambda: "away until" in driver.text(driver.find("#status")))
        wait_until("the session told in the user's name", lambda: next((str(s.get("detail", "")) for s in sessions_of(ISOLATED) if s.get("name") == "alone"), "").startswith("working on: I am away for the next 60 minutes"))

        session_id = str(session["sessionId"])
        self.assertIn('"decision":"block"', away_hook("Stop", session_id))
        self.assertIn('"permissionDecision":"deny"', away_hook("PreToolUse", session_id, "git push"))
        self.assertEqual(away_hook("PreToolUse", session_id, "cargo test"), "")

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
        wait_until("taken keys refused", lambda: texts(driver, "#settings .shortcuts-message")[0].startswith('Ctrl+Shift+K is already "stop and hide') or None)
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


if __name__ == "__main__":
    unittest.main(verbosity=2)
