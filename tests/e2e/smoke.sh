#!/bin/sh
# Inside the image: kindric starts on the screen that does not exist, finds the fake claude's
# account and shows its session. Prints what the trace says; exits 1 if the list never came.
set -u
Xvfb :99 -screen 0 1400x900x24 >/dev/null 2>&1 &
export DISPLAY=:99
command -v tauri-driver >/dev/null && echo "tauri-driver: $(command -v tauri-driver)"
command -v WebKitWebDriver >/dev/null && echo "WebKitWebDriver: $(command -v WebKitWebDriver)"
kindric-accounts add work >/dev/null
mkdir -p "$HOME/project" && cd "$HOME/project"
kindric-accounts run work --bg -n smoke "smoke test work:30"
KINDRIC_TRACE=1 dbus-launch kindric-desktop >"$HOME/desktop.log" 2>&1 &
app=$!
sleep 10
trace="$HOME/.local/share/kindric/trace.log"
grep -E "page ready|list published|agents work" "$trace" | head -5
kill "$app" 2>/dev/null
kindric-accounts run work stop smoke >/dev/null 2>&1
grep -q "list published (1 sessions)" "$trace" || { echo "no list"; tail -20 "$HOME/desktop.log"; exit 1; }
echo "smoke ok"
