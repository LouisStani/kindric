#!/bin/sh
# kindric's end-to-end journeys in a container (decisions 250, 251, 310): the desktop app on a
# screen that does not exist, driven through WebDriver, with a fake claude. Nothing is installed
# on this machine.
#
#   tests/e2e/run.sh                     every journey, on the code of this checkout
#   tests/e2e/run.sh SessionInTheBar     one journey (any unittest name works)
#   tests/e2e/run.sh --rebuild ...       build the image again first (new Rust, new system libs)
#
# The image is built the first time. The code of the checkout is then compiled inside, at each
# run, into a volume that keeps what was compiled: only what changed is compiled again.
# Screenshots go to $E2E_OUT (default /tmp/kindric-e2e-out), one per journey.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
image=kindric-e2e
target=kindric-e2e-target
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
    -e E2E_OUT=/out \
    -v "$root:/src:ro,Z" \
    -v "$target:/home/tester/target" \
    -v "$out:/out:Z" \
    "$image" sh -c '
        cargo build --locked --release -p kindric-desktop -p kindric-accounts -p kindric >"$HOME/build.log" 2>&1 \
            || { tail -40 "$HOME/build.log"; exit 1; }
        exec /src/tests/e2e/inside.sh "$@"' inside "$@"
