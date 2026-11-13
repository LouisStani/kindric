"""The few WebDriver calls kindric's end-to-end tests need, spoken to tauri-driver over HTTP with
the standard library only (decision 310): open the app, find an element, read it, click it, press
keys, run a script, take a screenshot, close the app.

The protocol is the W3C one: https://www.w3.org/TR/webdriver2/
"""
from __future__ import annotations

import base64
import json
import time
import urllib.error
import urllib.request
from typing import Any, Callable, TypeVar

# How the protocol names an element in what it sends back.
ELEMENT_KEY = "element-6066-11e4-a52e-4f735466cecc"
# Keys of the protocol, for key presses (https://www.w3.org/TR/webdriver2/#keyboard-actions).
CONTROL = ""
SHIFT = ""
ALT = ""
ENTER = ""
ESCAPE = ""
TAB = ""

T = TypeVar("T")


class DriverError(Exception):
    """What the WebDriver server refused, with its own words."""


class Driver:
    """One session of the app: opened by start(), closed by quit()."""

    def __init__(self, url: str, session_id: str) -> None:
        self._url = url
        self._session = session_id

    @classmethod
    def start(cls, application: str, url: str = "http://127.0.0.1:4444", wait: float = 30.0) -> Driver:
        """Launches the app through tauri-driver, which must already listen on url."""
        capabilities = {"capabilities": {"alwaysMatch": {"tauri:options": {"application": application}}}}
        deadline = time.monotonic() + wait
        while True:
            try:
                value = _call(f"{url}/session", "POST", capabilities)
                return cls(url, value["sessionId"])
            except (urllib.error.URLError, ConnectionError) as error:
                if time.monotonic() > deadline:
                    raise DriverError(f"tauri-driver does not answer on {url}: {error}") from error
                time.sleep(0.2)

    def quit(self) -> None:
        self._command("DELETE", "")

    def find(self, css: str) -> str:
        """The first element the selector matches; DriverError if none."""
        value = self._command("POST", "/element", {"using": "css selector", "value": css})
        return str(value[ELEMENT_KEY])

    def find_all(self, css: str) -> list[str]:
        value = self._command("POST", "/elements", {"using": "css selector", "value": css})
        return [str(item[ELEMENT_KEY]) for item in value]

    def text(self, element: str) -> str:
        return str(self._command("GET", f"/element/{element}/text"))

    def attribute(self, element: str, name: str) -> str | None:
        value = self._command("GET", f"/element/{element}/attribute/{name}")
        return None if value is None else str(value)

    def click(self, element: str) -> None:
        self._command("POST", f"/element/{element}/click", {})

    def type_into(self, element: str, text: str) -> None:
        self._command("POST", f"/element/{element}/value", {"text": text})

    def press(self, *keys: str) -> None:
        """Presses the keys together, then lets them go in reverse order: press(CONTROL, "n")."""
        down = [{"type": "keyDown", "value": key} for key in keys]
        up = [{"type": "keyUp", "value": key} for key in reversed(keys)]
        action = {"type": "key", "id": "keyboard", "actions": down + up}
        self._command("POST", "/actions", {"actions": [action]})
        self._command("DELETE", "/actions")

    def script(self, body: str, *args: Any) -> Any:
        """Runs a function body in the page; its return value comes back."""
        return self._command("POST", "/execute/sync", {"script": body, "args": list(args)})

    def screenshot(self, path: str) -> None:
        png = base64.b64decode(str(self._command("GET", "/screenshot")))
        with open(path, "wb") as out:
            out.write(png)

    def _command(self, method: str, path: str, body: dict[str, Any] | None = None) -> Any:
        return _call(f"{self._url}/session/{self._session}{path}", method, body)


def wait_until(what: str, check: Callable[[], T | None], timeout: float = 20.0, every: float = 0.25) -> T:
    """Asks check() until it gives something other than None or False; AssertionError after timeout.
    A DriverError while asking counts as not yet (the element may not exist yet)."""
    deadline = time.monotonic() + timeout
    last_error = ""
    while True:
        try:
            result = check()
            if result is not None and result is not False:
                return result
        except DriverError as error:
            last_error = f" (last error: {error})"
        if time.monotonic() > deadline:
            raise AssertionError(f"waited {timeout:g} s for {what}{last_error}")
        time.sleep(every)


def _call(url: str, method: str, body: dict[str, Any] | None) -> Any:
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(url, data=data, method=method, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return json.load(response).get("value")
    except urllib.error.HTTPError as error:
        raise DriverError(_reason(error)) from error


def _reason(error: urllib.error.HTTPError) -> str:
    try:
        value = json.load(error).get("value", {})
        return f"{value.get('error', error.code)}: {value.get('message', '')}".strip()
    except (ValueError, AttributeError):
        return f"HTTP {error.code} on {error.url}"
