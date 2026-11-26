"""The picture of the README: the control center with sessions that wait for an answer, made with the
fake claude, so that no real project shows (E2E_SCRIPT=readme_shot.py tests/e2e/run.sh).

Two accounts, one project, five sessions: a permission, a question of AskUserQuestion and a
`kindric ask` waiting in the control center, a session at work and one whose turn is done. The
screenshot goes to $E2E_OUT/readme-control-center.png.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time

from driver import CONTROL, SHIFT, Driver, wait_until
from journeys import (HOME, OUT, application, carry_messages, click_text, dialog_open, forget_everything, hook_input,
                      overview_hidden, page_ready, stop_every_session, tabs, texts)

GAME = os.path.join(HOME, "arcade")
PERSONAL = "personal"
WORK = "work"


def accounts(*words: str) -> str:
    done = subprocess.run(["kindric-accounts", *words], cwd=GAME, capture_output=True, text=True, timeout=30)
    if done.returncode != 0:
        raise AssertionError(f"kindric-accounts {' '.join(words)} exited {done.returncode}: {done.stderr.strip()}")
    return done.stdout


def session_id(account: str, name: str) -> str:
    path = os.path.join(HOME, ".local/share/kindric-accounts", account, "fake/sessions.json")
    with open(path) as source:
        return str(next(session["sessionId"] for session in json.load(source) if session.get("name") == name))


def stop_all() -> None:
    for account in (PERSONAL, WORK):
        path = os.path.join(HOME, ".local/share/kindric-accounts", account, "fake/sessions.json")
        try:
            with open(path) as source:
                sessions = json.load(source)
        except (OSError, ValueError):
            continue
        for session in sessions:
            if session.get("pid"):
                subprocess.run(["kindric-accounts", "run", account, "stop", session["id"]], cwd=GAME, capture_output=True, timeout=30)


def add_project(driver: Driver) -> None:
    driver.press(CONTROL, SHIFT, "t")
    wait_until("the project form", lambda: dialog_open(driver, "new-project"))
    directory = driver.find("#project-form input[name=directory]")
    driver.clear(directory)
    driver.type_into(directory, GAME)
    driver.click(driver.find("#project-form button[type=submit]"))
    wait_until("the project's tab", lambda: ("arcade", True) in tabs(driver))


def asked_by_hook(account: str, name: str, tool: str, tool_input: dict) -> subprocess.Popen:
    """`kindric permission-hook` as Claude Code starts it for a session of this account."""
    hook = subprocess.Popen(["kindric", "permission-hook"], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            text=True, cwd=GAME, env={**os.environ, "CLAUDE_CONFIG_DIR": account_dir(account)})
    assert hook.stdin is not None
    hook.stdin.write(hook_input(session_id(account, name), tool, tool_input))
    hook.stdin.close()
    return hook


def account_dir(account: str) -> str:
    return os.path.join(HOME, ".local/share/kindric-accounts", account)


def write_figures(started: list[tuple[str, str, str]]) -> None:
    """The limits of each account and the context of each session, as kindric-accounts' status
    line writes them down: the picture shows a day of work, not an empty install."""
    usage = os.path.join(HOME, ".local/share/kindric-accounts/.usage")
    now = int(time.time())
    for account, five_hour, seven_day in ((PERSONAL, 62.0, 31.0), (WORK, 18.0, 44.0)):
        os.makedirs(usage, exist_ok=True)
        with open(os.path.join(usage, f"{account}.json"), "w") as out:
            json.dump({"five_hour": {"used_percentage": five_hour, "resets_at": now + 2 * 3600 + 40 * 60},
                       "seven_day": {"used_percentage": seven_day, "resets_at": now + 4 * 86400}, "updated_at": now}, out)
    for (account, name, _), used in zip(started, (38.0, 61.0, 24.0, 47.0, 12.0)):
        folder = os.path.join(usage, "context", account)
        os.makedirs(folder, exist_ok=True)
        with open(os.path.join(folder, f"{session_id(account, name)}.json"), "w") as out:
            json.dump({"used_percentage": used, "updated_at": now}, out)


def main() -> int:
    forget_everything()
    os.makedirs(GAME, exist_ok=True)
    os.makedirs(OUT, exist_ok=True)
    # The fake claude leaves its marks ("work:N") out of what the picture shows.
    os.environ["FAKE_HIDE_MARKS"] = "1"
    accounts("add", PERSONAL)
    accounts("add", WORK)
    started = [
        (PERSONAL, "leaderboard", "add a weekly leaderboard work:0"),
        (PERSONAL, "scores-api", "serve the scores over HTTP work:0"),
        (WORK, "auth", "move the login to sessions work:0"),
        (WORK, "docs", "write the API guide work:900 say:I_write_the_section_on_rate_limits,_with_an_example_per_error_code."),
        (PERSONAL, "sprites", "pack the sprites into one sheet work:1"),
    ]
    for account, name, task in started:
        accounts("run", account, "--bg", "-n", name, task)
    write_figures(started)

    driver = Driver.start(application())
    hooks = []
    try:
        wait_until("the page ready", page_ready)
        # The app opens on the control center; the setup list, with its "add", is on the first tab.
        driver.press(CONTROL, SHIFT, "1")
        wait_until("the first tab shown", lambda: overview_hidden(driver))
        carry_messages(driver)
        add_project(driver)
        driver.press(CONTROL, SHIFT, "0")
        wait_until("the control center", lambda: not overview_hidden(driver))
        wait_until("a row per session", lambda: len(texts(driver, "article.control-row .row-name")) == len(started), timeout=30)

        hooks.append(asked_by_hook(PERSONAL, "leaderboard", "Bash", {"command": "npm test -- leaderboard", "description": "Run the leaderboard tests"}))
        question = {"question": "Where should the scores live?", "header": "Storage", "multiSelect": False,
                    "options": [{"label": "SQLite", "description": "One file next to the game"},
                                {"label": "Postgres", "description": "The server the API already uses"}]}
        hooks.append(asked_by_hook(PERSONAL, "scores-api", "AskUserQuestion", {"questions": [question]}))
        asked = subprocess.run(["kindric", "ask", "Keep the old /login route for a week?",
                                "The new sessions are in and tested. Two mobile builds still call /login. I'm writing the migration notes meanwhile."],
                               cwd=GAME, capture_output=True, text=True, timeout=10,
                               env={**os.environ, "CLAUDE_CODE_SESSION_ID": session_id(WORK, "auth"), "CLAUDE_CONFIG_DIR": account_dir(WORK)})
        if asked.returncode != 0:
            raise AssertionError(f"kindric ask: {asked.stderr.strip()}")
        try:
            wait_until("three questions in the control center", lambda: len(texts(driver, "article.control-row .request")) >= 3, timeout=30)
        except AssertionError:
            driver.screenshot(os.path.join(OUT, "readme-failed.png"))
            print("rows:", texts(driver, "article.control-row"))
            print("requests:", texts(driver, "article.control-row .request"))
            for hook in hooks:
                print("hook", hook.poll())
            overview = os.path.join(HOME, ".local/share/kindric/overview")
            for folder, _, files in os.walk(overview):
                print(folder, files)
            raise
        wait_until("the sprites done", lambda: "1 done, not seen" in str(driver.script("return document.body.innerText;")), timeout=30)
        # What docs does, on the line under its row (decisions 499, 500), once it said it.
        wait_until("what docs does", lambda: any("rate limits" in text for text in texts(driver, "article.control-row .row-task")) or None, timeout=30)
        path = os.path.join(OUT, "readme-control-center.png")
        driver.screenshot(path)
        print(path)
        return 0
    finally:
        for hook in hooks:
            hook.kill()
        driver.quit()
        stop_all()
        stop_every_session()


if __name__ == "__main__":
    sys.exit(main())
