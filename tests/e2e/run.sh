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
# once for all of them, and only what changed is compiled again.
# Screenshots go to $E2E_OUT (default /tmp/kindric-e2e-out), one per journey.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
image=kindric-e2e
target=kindric-e2e-target
# Each checkout is compiled from its own path: cargo tells the programs of two checkouts apart by
# their path, so one never runs the other's (mounted at the same path, a build whose sources look
# older than it is kept). Copied, not compiled in place: Tauri's build script writes in the source.
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
        (cd "$E2E_BUILD" && cargo build --locked --release -p kindric-desktop -p kindric-accounts -p kindric) >"$HOME/build.log" 2>&1 \
            || { tail -40 "$HOME/build.log"; exit 1; }
        exec /src/tests/e2e/inside.sh "$@"' inside "$@"
