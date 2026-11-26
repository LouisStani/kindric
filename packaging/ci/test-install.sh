#!/usr/bin/env bash
# Runs packaging/install.sh against an archive served on localhost, in a home of its own:
# a damaged download, an install, an update, a menu entry that is not ours, the uninstall.
#
# usage: packaging/ci/test-install.sh <dir with kindric-<target>.tar.gz and SHA256SUMS>
set -euo pipefail

dist=$(cd "$1" && pwd)
root=$(cd "$(dirname "$0")/../.." && pwd)
installer="$root/packaging/install.sh"
port=8765
failures=0

check() {
    if eval "$1"; then
        echo "ok   $2"
    else
        echo "FAIL $2"
        failures=$((failures + 1))
    fi
}

# ASCII only: the /bin/sh of macOS, bash 3.2, reads the bytes of a character such as an ellipsis right
# after "$target" as part of the name of the variable, in a locale other than UTF-8.
non_ascii=$(cat "$installer" "$root/packaging/launch.sh" | LC_ALL=C tr -d '\000-\177' | wc -c | tr -d ' ')
check '[ "$non_ascii" = 0 ]' "the shell scripts are ASCII only ($non_ascii other bytes)"

python3 -m http.server "$port" --bind 127.0.0.1 --directory "$dist" > /dev/null 2>&1 &
server=$!
home=$(mktemp -d)
trap 'kill "$server" 2> /dev/null || true; rm -rf "$home"' EXIT
tries=0
until curl -fs "http://127.0.0.1:$port/SHA256SUMS" > /dev/null; do
    tries=$((tries + 1))
    if [ "$tries" -gt 50 ]; then
        echo "the test server did not start" >&2
        exit 1
    fi
    sleep 0.1
done

# The real XDG folders must not be touched: the installer writes its menu entry under XDG_DATA_HOME.
run_installer() {
    env -u XDG_DATA_HOME -u XDG_CONFIG_HOME -u XDG_STATE_HOME -u XDG_CACHE_HOME -u KINDRIC_INSTALL_DIR \
        HOME="$home" PATH="$home/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
        KINDRIC_DOWNLOAD_URL="http://127.0.0.1:$port" sh "$installer" "$@"
}

bin="$home/.local/bin"

# The launcher, with kindric-desktop replaced by a script that prints its PATH: the programs are on it,
# even from a bare session, and a startup file of the shell that hangs does not keep the app from starting.
check_launcher() {
    launched=$(env HOME="$home" PATH=/usr/bin:/bin SHELL=/bin/sh "$1")
    check 'case ":$launched:" in *":$bin:"*) true ;; *) false ;; esac' "the launcher puts the programs on the PATH"
    printf 'sleep 30\n' > "$home/.bash_profile"
    started=$(date +%s)
    launched=$(env HOME="$home" PATH=/usr/bin:/bin SHELL=/bin/bash "$1")
    elapsed=$(($(date +%s) - started))
    rm "$home/.bash_profile"
    check '[ "$elapsed" -lt 15 ] && case ":$launched:" in *":$bin:"*) true ;; *) false ;; esac' "a shell that hangs at startup is given up after 5 s (took ${elapsed} s)"
}
case "$(uname -s)" in
    Darwin) macos=1 ;;
    *) macos=0 ;;
esac

# A damaged download is refused, and nothing is installed.
cp "$dist/SHA256SUMS" "$home/SHA256SUMS.good"
sed 's/^[0-9a-f]*/0000/' "$home/SHA256SUMS.good" > "$dist/SHA256SUMS"
if output=$(run_installer 2>&1); then status=0; else status=$?; fi
cp "$home/SHA256SUMS.good" "$dist/SHA256SUMS"
check '[ "$status" -ne 0 ]' "a damaged download fails"
check 'grep -q "does not match its SHA256SUMS" <<< "$output"' "and says why"
check '[ ! -e "$bin/kindric" ]' "and installs nothing"

# A menu entry made by someone else is left as it is.
if [ "$macos" = 0 ]; then
    mkdir -p "$home/.local/share/applications"
    printf '[Desktop Entry]\nName=mine\n' > "$home/.local/share/applications/kindric.desktop"
fi
mkdir -p "$home/.local/share/kindric"
echo keep > "$home/.local/share/kindric/projects.json"

# A program it did not install is left alone, unless it is a kindric program (from the README of
# kindric-accounts, say).
mkdir -p "$bin"
ln -s /bin/sh "$bin/kindric"
if output=$(run_installer 2>&1); then status=0; else status=$?; fi
check '[ "$status" -ne 0 ] && grep -q "was not installed by this script" <<< "$output"' "a program it did not install is refused"
check '[ "$(readlink "$bin/kindric")" = /bin/sh ]' "and left as it is"
rm "$bin/kindric"
tar -xzf "$dist"/kindric-*.tar.gz -C "$home" --strip-components 1 "$(tar -tzf "$dist"/kindric-*.tar.gz | grep '/kindric-accounts$')"
mv "$home/kindric-accounts" "$bin/kindric-accounts"

output=$(run_installer 2>&1)
echo "$output"
check '[ -x "$bin/kindric" ] && [ -x "$bin/kindric-accounts" ] && [ -x "$bin/kindric-desktop" ]' "the three programs are installed"
check '"$bin/kindric" --version | grep -q "^kindric "' "kindric runs"
check 'HOME="$home" "$bin/kindric-accounts" --version | grep -q "^kindric-accounts "' "kindric-accounts runs"
check 'grep -q "installed in $bin" <<< "$output"' "it says where"
if [ "$macos" = 1 ]; then
    app="$home/Applications/kindric.app"
    check '[ -x "$app/Contents/MacOS/kindric" ] && [ -x "$app/Contents/MacOS/kindric-desktop" ]' "kindric.app holds its programs"
    check 'plutil -lint "$app/Contents/Info.plist" > /dev/null' "its Info.plist is valid"
    check '[ -f "$app/Contents/Resources/kindric.icns" ]' "it has its icon"
    # kindric-desktop in a terminal goes through the app's launcher, which puts the programs on the PATH.
    printf '#!/bin/sh\necho "$PATH"\n' > "$app/Contents/MacOS/kindric-desktop"
    check_launcher "$bin/kindric-desktop"
else
    check 'grep -qx "Name=mine" "$home/.local/share/applications/kindric.desktop"' "a menu entry not made by the installer is kept"
    check 'grep -q "was not made by this script" <<< "$output"' "and the installer says so"
    rm "$home/.local/share/applications/kindric.desktop"
    output=$(run_installer 2>&1)
    check 'grep -qx "X-Kindric-Installer=true" "$home/.local/share/applications/kindric.desktop"' "the menu entry is made"
    launcher="$home/.local/share/kindric-installer/launch"
    check 'grep -qx "Exec=\"$launcher\"" "$home/.local/share/applications/kindric.desktop"' "it starts the launcher"
    # The launcher starts kindric-desktop with the programs on its PATH, even from a bare session.
    cp "$bin/kindric-desktop" "$home/kindric-desktop.real"
    printf '#!/bin/sh\necho "$PATH"\n' > "$bin/kindric-desktop"
    check_launcher "$launcher"
    cp "$home/kindric-desktop.real" "$bin/kindric-desktop"
    check '[ -f "$home/.local/share/icons/hicolor/256x256/apps/io.github.louisstani.kindric.png" ]' "with its icon"
fi

# Running it again updates in place, here with the checksums written in binary mode ("hash *name").
sed 's/  / */' "$home/SHA256SUMS.good" > "$dist/SHA256SUMS"
check 'run_installer > /dev/null 2>&1' "a second run updates, with checksums in binary mode"
cp "$home/SHA256SUMS.good" "$dist/SHA256SUMS"
check '"$bin/kindric" --version > /dev/null' "kindric still runs"

output=$(run_installer --uninstall 2>&1)
echo "$output"
check '[ ! -e "$bin/kindric" ] && [ ! -e "$bin/kindric-accounts" ] && [ ! -e "$bin/kindric-desktop" ]' "uninstall removes the programs"
if [ "$macos" = 1 ]; then
    check '[ ! -e "$home/Applications/kindric.app" ]' "and kindric.app"
else
    check '[ ! -e "$home/.local/share/applications/kindric.desktop" ]' "and the menu entry"
    check '[ ! -e "$home/.local/share/icons/hicolor/256x256/apps/io.github.louisstani.kindric.png" ]' "and the icon"
fi
check 'grep -qx keep "$home/.local/share/kindric/projects.json"' "kindric's data stays"
check '[ ! -e "$home/.local/share/kindric-installer" ]' "the installer's own folder goes"
check 'run_installer --uninstall 2>&1 | grep -q "Nothing installed by this script"' "a second uninstall finds nothing"

if [ "$failures" -gt 0 ]; then
    echo "$failures check(s) failed"
    exit 1
fi
echo "all checks passed"
