#!/bin/sh
# Inside the image: a screen that does not exist, a session bus, tauri-driver, then the journeys.
# Exits with the journeys' code. Everything it starts is stopped before it returns.
set -u
here=$(dirname "$0")
Xvfb :99 -screen 0 1400x900x24 >/dev/null 2>&1 &
xvfb=$!
export DISPLAY=:99
eval "$(dbus-launch --sh-syntax)"
# The journeys wait for "page ready" in the trace; measurements run without it (its frame watch costs).
[ -n "${E2E_NO_TRACE:-}" ] || export KINDRIC_TRACE=1
# Every call of the fake claude, for measure.py: the app gets the environment of tauri-driver.
export FAKE_CLAUDE_CALLS="$HOME/claude-calls.log"
tauri-driver >"$HOME/tauri-driver.log" 2>&1 &
driver=$!

python3 "$here/${E2E_SCRIPT:-journeys.py}" "$@"
code=$?

kill "$driver" "$DBUS_SESSION_BUS_PID" "$xvfb" 2>/dev/null
[ "$code" -eq 0 ] || { echo "--- tauri-driver.log"; tail -30 "$HOME/tauri-driver.log"; }
exit "$code"
