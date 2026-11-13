#!/bin/sh
# Inside the image: a screen that does not exist, a session bus, tauri-driver, then the journeys.
# Exits with the journeys' code. Everything it starts is stopped before it returns.
set -u
here=$(dirname "$0")
Xvfb :99 -screen 0 1400x900x24 >/dev/null 2>&1 &
xvfb=$!
export DISPLAY=:99
eval "$(dbus-launch --sh-syntax)"
export KINDRIC_TRACE=1
tauri-driver >"$HOME/tauri-driver.log" 2>&1 &
driver=$!

python3 "$here/journeys.py" "$@"
code=$?

kill "$driver" "$DBUS_SESSION_BUS_PID" "$xvfb" 2>/dev/null
[ "$code" -eq 0 ] || { echo "--- tauri-driver.log"; tail -30 "$HOME/tauri-driver.log"; }
exit "$code"
