#!/usr/bin/env bash
# Opens kindric-desktop on the screen of this macOS machine, as nobody did by hand: waits for its page
# to be ready (written in trace.log), then takes a screenshot, whether it came or not.
#
# usage: packaging/ci/open-app.sh <dir with the programs> <dir for the screenshot and the app's output>
set -euo pipefail

bin=$(cd "$1" && pwd)
mkdir -p "$2"
out=$(cd "$2" && pwd)
trace="$HOME/.local/share/kindric/trace.log"
rm -f "$trace"

KINDRIC_TRACE=1 PATH="$bin:$PATH" "$bin/kindric-desktop" > "$out/app.log" 2>&1 &
app=$!
ready=0
for _ in $(seq 120); do
    if [ -f "$trace" ] && grep -q "page ready" "$trace"; then
        ready=1
        break
    fi
    kill -0 "$app" 2> /dev/null || break
    sleep 0.5
done
# The page draws a moment after it says it is ready.
sleep 3
screencapture -x "$out/macos.png"
kill "$app" 2> /dev/null || true

if [ "$ready" = 1 ]; then
    echo "the page is ready:"
    grep "page ready" "$trace"
else
    echo "no \"page ready\" in $trace after 60 s" >&2
    cat "$out/app.log" >&2
    exit 1
fi
