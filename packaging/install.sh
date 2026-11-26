#!/bin/sh
# Installs kindric on Linux or macOS: kindric-desktop, kindric and kindric-accounts, from a release of
# github.com/LouisStani/kindric.
#
#   curl -fsSL https://github.com/LouisStani/kindric/releases/latest/download/install.sh | sh
#
# Options, after `sh -s --` when piped:
#   --version <x.y.z>   install this release instead of the latest
#   --uninstall         remove what this script installed; your accounts, sessions and settings stay
#
# KINDRIC_INSTALL_DIR changes where the programs go (default ~/.local/bin, where Claude Code's own
# installer puts claude). KINDRIC_DOWNLOAD_URL replaces the folder of the release (used by the tests).
set -eu

repo="LouisStani/kindric"
bundle_id="io.github.louisstani.kindric"
desktop_marker="X-Kindric-Installer=true"

say() { printf '%s\n' "$*"; }
fail() { printf 'kindric install: %s\n' "$*" >&2; exit 1; }

version=""
action="install"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail "--version needs a version, as in --version 1.0.0"
            version=${2#v}
            shift 2
            ;;
        --uninstall) action="uninstall"; shift ;;
        -h | --help)
            say "usage: install.sh [--version <x.y.z>] [--uninstall]"
            exit 0
            ;;
        *) fail "unknown option $1 (try --help)" ;;
    esac
done

[ -n "${HOME:-}" ] || fail "HOME is not set"
bin_dir=${KINDRIC_INSTALL_DIR:-$HOME/.local/bin}
data_home=${XDG_DATA_HOME:-$HOME/.local/share}
desktop_file="$data_home/applications/kindric.desktop"
icon_file="$data_home/icons/hicolor/256x256/apps/$bundle_id.png"
# What this script put on the computer, one path a line, and the launcher of the Linux menu entry.
state_dir="$data_home/kindric-installer"
manifest="$state_dir/installed"
launcher="$state_dir/launch"
app_dir="$HOME/Applications/kindric.app"

detect_target() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$arch" in
        x86_64 | amd64) arch="x86_64" ;;
        aarch64 | arm64) arch="aarch64" ;;
        *) fail "no build of kindric for the processor $arch" ;;
    esac
    case "$os" in
        Linux) target="$arch-unknown-linux-gnu" ;;
        Darwin)
            # A shell under Rosetta says x86_64 on an Apple processor.
            if [ "$arch" = "x86_64" ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = "1" ]; then
                arch="aarch64"
            fi
            target="$arch-apple-darwin"
            ;;
        *) fail "this script is for Linux and macOS; on Windows, use install.ps1 (see the README)" ;;
    esac
}

download() {
    if command -v curl > /dev/null 2>&1; then
        curl -fsSL --retry 3 -o "$2" "$1" || fail "could not download $1"
    elif command -v wget > /dev/null 2>&1; then
        wget -q -O "$2" "$1" || fail "could not download $1"
    else
        fail "curl or wget is needed to download kindric"
    fi
}

sha256_of() {
    if command -v sha256sum > /dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum > /dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        echo ""
    fi
}

verify() {
    expected=$(grep " $2\$" "$work/SHA256SUMS" | cut -d ' ' -f 1 || true)
    [ -n "$expected" ] || fail "$2 is not listed in the SHA256SUMS of the release"
    actual=$(sha256_of "$1")
    if [ -z "$actual" ]; then
        say "warning: neither sha256sum nor shasum is installed: the download is not checked"
        return
    fi
    [ "$actual" = "$expected" ] || fail "$2 does not match its SHA256SUMS: the download is damaged, try again"
}

installed() { [ -f "$manifest" ] && grep -qxF "$1" "$manifest"; }

remember() {
    mkdir -p "$state_dir"
    installed "$1" || printf '%s\n' "$1" >> "$manifest"
}

# A file this script did not put there is replaced only when it is a kindric program, by its --version:
# one from the README of kindric-accounts, say. A link, or anything else, is left alone.
may_replace() {
    [ -e "$1" ] || [ -L "$1" ] || return 0
    installed "$1" && return 0
    if [ ! -L "$1" ] && [ "$2" != "kindric-desktop" ]; then
        case "$("$1" --version 2> /dev/null || true)" in
            "$2 "*) return 0 ;;
        esac
    fi
    fail "$1 was not installed by this script: move it away, then run this again"
}

# Replaces a program at once, so that a kindric already running keeps its own file.
put_program() {
    may_replace "$bin_dir/$2" "$2"
    cp "$1" "$bin_dir/.$2.new"
    chmod 755 "$bin_dir/.$2.new"
    mv -f "$bin_dir/.$2.new" "$bin_dir/$2"
    remember "$bin_dir/$2"
}

ours_desktop_file() { [ -f "$desktop_file" ] && grep -qx "$desktop_marker" "$desktop_file"; }
ours_app() { [ -f "$app_dir/Contents/Info.plist" ] && grep -q "<string>$bundle_id</string>" "$app_dir/Contents/Info.plist"; }

install_linux_menu() {
    if [ -e "$desktop_file" ] && ! ours_desktop_file; then
        say "warning: $desktop_file was not made by this script: left as it is, kindric gets no menu entry"
        return
    fi
    # The menu runs the launcher, which gives the app the PATH of the user's shell; the link next to it
    # tells it where the programs are.
    mkdir -p "$state_dir" "$(dirname "$desktop_file")" "$(dirname "$icon_file")"
    cp "$1/kindric-launch" "$launcher.new"
    chmod 755 "$launcher.new"
    mv -f "$launcher.new" "$launcher"
    ln -sfn "$bin_dir/kindric-desktop" "$state_dir/kindric-desktop"
    cp "$1/kindric.png" "$icon_file"
    cat > "$desktop_file" <<EOF
[Desktop Entry]
Type=Application
Name=kindric
Comment=Your Claude Code sessions, every account, one window
Exec="$launcher"
Icon=$bundle_id
Terminal=false
Categories=Development;
StartupWMClass=kindric-desktop
$desktop_marker
EOF
    if command -v update-desktop-database > /dev/null 2>&1; then
        update-desktop-database "$(dirname "$desktop_file")" > /dev/null 2>&1 || true
    fi
}

install_macos_app() {
    if [ -e "$app_dir" ] && ! ours_app; then
        fail "$app_dir is another app than kindric: move it away, then run this again"
    fi
    mkdir -p "$HOME/Applications"
    rm -rf "$app_dir.new"
    cp -R "$1/kindric.app" "$app_dir.new"
    rm -rf "$app_dir"
    mv "$app_dir.new" "$app_dir"
    # curl does not mark the file as downloaded, but a copy of the script saved by a browser might.
    xattr -dr com.apple.quarantine "$app_dir" 2> /dev/null || true
    remember "$app_dir"
    # kindric-desktop in a terminal starts the program of the app, by its full path.
    may_replace "$bin_dir/kindric-desktop" kindric-desktop
    printf '#!/bin/sh\n# Made by the kindric installer.\nexec "%s/Contents/MacOS/kindric" "$@"\n' "$app_dir" > "$bin_dir/.kindric-desktop.new"
    chmod 755 "$bin_dir/.kindric-desktop.new"
    mv -f "$bin_dir/.kindric-desktop.new" "$bin_dir/kindric-desktop"
    remember "$bin_dir/kindric-desktop"
}

check_webkit() {
    for dir in /usr/lib /usr/lib64 /usr/lib/x86_64-linux-gnu /usr/lib/aarch64-linux-gnu /usr/local/lib; do
        if [ -e "$dir/libwebkit2gtk-4.1.so.0" ]; then
            return
        fi
    done
    if { ldconfig -p 2> /dev/null || /sbin/ldconfig -p 2> /dev/null || true; } | grep -q 'libwebkit2gtk-4.1\.so\.0'; then
        return
    fi
    distro=$( (. /etc/os-release 2> /dev/null && echo "$ID ${ID_LIKE:-}") || echo "")
    case " $distro " in
        *" arch "*) command="sudo pacman -S webkit2gtk-4.1" ;;
        *" debian "* | *" ubuntu "*) command="sudo apt install libwebkit2gtk-4.1-0" ;;
        *" fedora "* | *" rhel "*) command="sudo dnf install webkit2gtk4.1" ;;
        *" opensuse"* | *" suse "*) command="sudo zypper install libwebkit2gtk-4_1-0" ;;
        *) command="" ;;
    esac
    say ""
    say "The desktop app needs WebKitGTK 4.1, which is not installed."
    if [ -n "$command" ]; then
        say "Install it with: $command"
    else
        say "Install the WebKitGTK 4.1 package of your distribution (libwebkit2gtk-4.1)."
    fi
}

check_path() {
    case ":$PATH:" in
        *":$bin_dir:"*) ;;
        *)
            say ""
            say "$bin_dir is not on your PATH. Add this line to your shell's startup file (~/.bashrc, ~/.zshrc…):"
            say "  export PATH=\"$bin_dir:\$PATH\""
            return
            ;;
    esac
    for name in kindric kindric-accounts kindric-desktop; do
        found=$(command -v "$name" 2> /dev/null || true)
        if [ -n "$found" ] && [ "$found" != "$bin_dir/$name" ]; then
            say ""
            say "warning: $found comes before $bin_dir/$name in your PATH and will run instead."
            say "Remove the old one (cargo uninstall $name, if it came from cargo)."
        fi
    done
}

check_claude() {
    if ! command -v claude > /dev/null 2>&1 && [ ! -x "$HOME/.local/bin/claude" ]; then
        say ""
        say "Claude Code is not installed: kindric shows its sessions, so install it too."
        say "See https://docs.anthropic.com/en/docs/claude-code"
    fi
}

install() {
    detect_target
    if [ -n "${KINDRIC_DOWNLOAD_URL:-}" ]; then
        base=$KINDRIC_DOWNLOAD_URL
    elif [ -n "$version" ]; then
        base="https://github.com/$repo/releases/download/v$version"
    else
        base="https://github.com/$repo/releases/latest/download"
    fi
    archive="kindric-$target.tar.gz"
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    say "Downloading kindric for $target…"
    download "$base/SHA256SUMS" "$work/SHA256SUMS"
    download "$base/$archive" "$work/$archive"
    verify "$work/$archive" "$archive"
    tar -xzf "$work/$archive" -C "$work"
    unpacked="$work/kindric-$target"

    mkdir -p "$bin_dir"
    put_program "$unpacked/kindric" kindric
    put_program "$unpacked/kindric-accounts" kindric-accounts
    case "$target" in
        *apple*) install_macos_app "$unpacked" ;;
        *)
            put_program "$unpacked/kindric-desktop" kindric-desktop
            install_linux_menu "$unpacked"
            ;;
    esac

    installed=$("$bin_dir/kindric" --version 2> /dev/null || echo "kindric")
    say "$installed installed in $bin_dir."
    case "$target" in
        *apple*) say "Open kindric from Launchpad or ~/Applications, or run kindric-desktop." ;;
        *)
            check_webkit
            say "Open kindric from your applications menu, or run kindric-desktop."
            ;;
    esac
    check_claude
    check_path
    say ""
    say "On first launch, kindric lists what is left to set up. Run this script again to update."
}

uninstall() {
    # Only what this script put there: the programs and the app it wrote down, the menu entry with its mark.
    removed=""
    if [ -f "$manifest" ]; then
        while IFS= read -r file; do
            if [ "$file" = "$app_dir" ]; then
                ours_app && rm -rf "$app_dir" && removed="$removed kindric.app"
            elif [ -e "$file" ] || [ -L "$file" ]; then
                rm -f "$file"
                removed="$removed $(basename "$file")"
            fi
        done < "$manifest"
    fi
    if ours_desktop_file; then
        rm -f "$desktop_file" "$icon_file"
        removed="$removed menu-entry"
    fi
    rm -f "$manifest" "$launcher" "$state_dir/kindric-desktop"
    rmdir "$state_dir" 2> /dev/null || true
    if [ -z "$removed" ]; then
        say "Nothing installed by this script was found."
    else
        say "Removed:$removed."
    fi
    say "Your accounts, sessions, messages and settings stay, in kindric's and kindric-accounts' data folders"
    say "(see the installation guide to remove them too). Claude Code is not touched."
}

"$action"
