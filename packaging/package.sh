#!/usr/bin/env bash
# Puts the three programs of one target, built in release, into the archive the installers download:
# kindric-<target>.tar.gz on Linux and macOS, kindric-<target>.zip on Windows.
#
# usage: packaging/package.sh <target> <release dir> <out dir>
#   <release dir> holds kindric, kindric-accounts and kindric-desktop (target/<target>/release)
set -euo pipefail

if [ "$#" -ne 3 ]; then
    echo "usage: $0 <target> <release dir> <out dir>" >&2
    exit 2
fi
target=$1
release_dir=$2
out_dir=$3
root=$(cd "$(dirname "$0")/.." && pwd)
name="kindric-$target"

mkdir -p "$out_dir"
out_dir=$(cd "$out_dir" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
stage="$work/$name"
mkdir -p "$stage"

copy_program() {
    local file="$release_dir/$1"
    if [ ! -f "$file" ]; then
        echo "package.sh: $file is missing: build it first (cargo build --release -p $2 --target $target)" >&2
        exit 1
    fi
    cp "$file" "$stage/"
}

# An app bundle, so that macOS shows kindric in the Dock and in Launchpad. Its program is a small script that
# takes the PATH of the user's shell: an app opened from the Finder gets only /usr/bin:/bin:/usr/sbin:/sbin,
# where neither claude nor kindric-accounts is.
make_app() {
    local app="$stage/kindric.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    mv "$stage/kindric-desktop" "$app/Contents/MacOS/kindric-desktop"
    cp "$root/packaging/launch.sh" "$app/Contents/MacOS/kindric"
    chmod +x "$app/Contents/MacOS/kindric"
    sed "s/@VERSION@/$(version)/" "$root/packaging/macos/Info.plist" > "$app/Contents/Info.plist"
    make_icns "$app/Contents/Resources/kindric.icns"
}

make_icns() {
    local iconset="$work/kindric.iconset" size
    mkdir -p "$iconset"
    for size in 16 32 128 256 512; do
        sips -z "$size" "$size" "$root/crates/desktop/icons/icon.png" --out "$iconset/icon_${size}x${size}.png" > /dev/null
    done
    iconutil -c icns "$iconset" -o "$1"
}

version() {
    sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -n 1
}

cp "$root/LICENSE" "$stage/"
case "$target" in
    *windows*)
        copy_program kindric.exe kindric
        copy_program kindric-accounts.exe kindric-accounts
        copy_program kindric-desktop.exe kindric-desktop
        (cd "$work" && 7z a -tzip -bso0 "$out_dir/$name.zip" "$name")
        echo "$out_dir/$name.zip"
        ;;
    *apple*)
        copy_program kindric kindric
        copy_program kindric-accounts kindric-accounts
        copy_program kindric-desktop kindric-desktop
        make_app
        tar -C "$work" -czf "$out_dir/$name.tar.gz" "$name"
        echo "$out_dir/$name.tar.gz"
        ;;
    *)
        copy_program kindric kindric
        copy_program kindric-accounts kindric-accounts
        copy_program kindric-desktop kindric-desktop
        cp "$root/crates/desktop/icons/icon.png" "$stage/kindric.png"
        cp "$root/packaging/launch.sh" "$stage/kindric-launch"
        tar -C "$work" -czf "$out_dir/$name.tar.gz" "$name"
        echo "$out_dir/$name.tar.gz"
        ;;
esac
