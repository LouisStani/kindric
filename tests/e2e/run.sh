#!/bin/sh
# kindric's end-to-end journeys in a container (decisions 250, 251, 310): the desktop app on a
# screen that does not exist, driven through WebDriver, with a fake claude. Nothing is installed
# on this machine.
#
#   tests/e2e/run.sh                     every journey, on the code of this checkout
#   tests/e2e/run.sh SessionInTheBar     one journey (any unittest name works)
#   tests/e2e/run.sh --rebuild ...       build the image again first (new Rust, new system libs)
#
#   E2E_SCRIPT=measure.py E2E_NO_TRACE=1 tests/e2e/run.sh    what the app costs (see measure.py)
#
# The image is built the first time. The code of the checkout is then copied into the container
# and compiled there, into one volume shared by every checkout: the dependencies are compiled
# once for all of them, and only what changed is compiled again. The programs land in one place of
# that volume, target/release: two runs at once would replace each other's while their journeys
# run (seen on 2026-11-18, decision 382). So a run holds a lock on the volume while it compiles
# and copies its programs into the container, and its journeys use that copy; the journeys of two
# runs still go on at the same time.
# Screenshots go to $E2E_OUT (default /tmp/kindric-e2e-out), one per journey.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
image=kindric-e2e
target=kindric-e2e-target
# Each checkout is copied to its own path and compiled there. Copied, not compiled in place: Tauri's
# build script writes in the source. Cargo still gives two checkouts the same compiled files for
# kindric's crates (verified on 2026-11-18), hence the touch below.
build=/home/tester/build/$(printf %s "$root" | cksum | cut -d ' ' -f 1)
out=${E2E_OUT:-/tmp/kindric-e2e-out}

if ! command -v podman >/dev/null 2>&1; then
    echo "run.sh: podman is needed (rootless; the Docker daemon is not used)" >&2
    exit 2
fi

if [ "${1:-}" = --rebuild ]; then
    shift
    podman volume rm --force "$target" >/dev/null
    podman build --ignorefile "$root/tests/e2e/Dockerfile.dockerignore" -f "$root/tests/e2e/Dockerfile" -t "$image" "$root"
elif ! podman image exists "$image"; then
    podman build --ignorefile "$root/tests/e2e/Dockerfile.dockerignore" -f "$root/tests/e2e/Dockerfile" -t "$image" "$root"
fi

mkdir -p "$out"
# keep-id: the tester of the image is this user, so the screenshots belong to them.
exec podman run --rm --userns=keep-id \
    -e E2E_OUT=/out -e E2E_SCRIPT -e E2E_NO_TRACE -e MEASURE_IDLE -e MEASURE_PANES -e E2E_BUILD="$build" \
    -v "$root:/src:ro,Z" \
    -v "$target:/home/tester/target" \
    -v "$out:/out:Z" \
    "$image" sh -c '
        mkdir -p "$E2E_BUILD"
        # tar keeps the dates of the files, which cargo compares to know what changed.
        (cd /src && tar --exclude=./target --exclude=./.git --exclude=./.claude --exclude=node_modules -cf - .) \
            | tar -xf - -C "$E2E_BUILD"
        # One build at a time in the shared volume, until this run has its own copy of the programs.
        # The lock goes with the shell if the build fails.
        exec 9>/home/tester/target/.e2e-build.lock
        flock 9
        # Cargo gives the crates of kindric in every checkout the same compiled files, and keeps those of
        # the last checkout built while it sees nothing new: their sources are made newer here, so
        # that this run compiles its own. The dependencies stay compiled once for all.
        find "$E2E_BUILD/crates" -type f -exec touch {} +
        (cd "$E2E_BUILD" && cargo build --locked --release -p kindric-desktop -p kindric-accounts -p kindric) >"$HOME/build.log" 2>&1 \
            || { tail -40 "$HOME/build.log"; exit 1; }
        programs="$HOME/programs"
        mkdir -p "$programs"
        for program in kindric-desktop kindric kindric-accounts; do
            cp /home/tester/target/release/$program "$programs/"
        done
        flock -u 9
        exec 9>&-
        export PATH="$programs:$PATH"
        exec /src/tests/e2e/inside.sh "$@"' inside "$@"
