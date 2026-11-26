#!/bin/sh
# Starts kindric-desktop with the PATH of the user's shell. Opened from a menu, the Finder or the Dock, an
# app gets the PATH of the desktop session (on macOS, launchd's /usr/bin:/bin:/usr/sbin:/sbin), where
# claude, kindric and kindric-accounts may be missing. On macOS this is the program of kindric.app; on
# Linux, the installer's menu entry runs it.
#
# It starts the kindric-desktop next to it. When that is a link, as the Linux installer makes it, the
# folder it points to is put on the PATH too; otherwise ~/.local/bin, where the installer puts the programs.
here=$(cd "$(dirname "$0")" && pwd)
program="$here/kindric-desktop"
programs="$HOME/.local/bin"
if [ -L "$program" ]; then
    target=$(readlink "$program")
    case "$target" in
        /*) programs=$(dirname "$target") ;;
    esac
fi

# The PATH of an interactive login shell, as a terminal gives it. A marker, since startup files may print
# something of their own, and at most 5 seconds, since one may wait for input.
out=$(mktemp "${TMPDIR:-/tmp}/kindric-path.XXXXXX" 2> /dev/null || true)
if [ -n "$out" ]; then
    (exec "${SHELL:-/bin/sh}" -ilc 'printf "\n__kindric_path__%s\n" "$PATH"' > "$out" 2> /dev/null < /dev/null) &
    reader=$!
    (sleep 5 && kill -KILL "$reader" 2> /dev/null) &
    watchdog=$!
    wait "$reader" 2> /dev/null || true
    # Waited for, so that the shell does not report the end of the watchdog (bash 3.2 does, on macOS).
    { kill "$watchdog" && wait "$watchdog"; } 2> /dev/null || true
    found=$(sed -n 's/^__kindric_path__//p' "$out" | tail -n 1)
    rm -f "$out"
    if [ -n "$found" ]; then
        PATH=$found
    fi
fi
case ":$PATH:" in
    *":$programs:"*) ;;
    *) PATH="$programs:$PATH" ;;
esac
export PATH
exec "$program" "$@"
