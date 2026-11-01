#!/usr/bin/env bash
# Smoke test of a built binary, for systems where tests.sh cannot run (macOS: no GNU
# stat, BSD script). Temporary HOME, fake claude, no real account, no network.
# Usage: ci/smoke.sh <path to claude-accounts>
set -euo pipefail

bin="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
export HOME="$root/home"
unset XDG_CONFIG_HOME XDG_DATA_HOME
mkdir -p "$HOME/.claude/skills" "$root/bin"
printf '# shared\n' >"$HOME/.claude/CLAUDE.md"
cat >"$root/bin/claude" <<'EOF'
#!/bin/sh
printf 'CONFIG=%s\nAPIKEY=%s\nARGS=%s\n' "$CLAUDE_CONFIG_DIR" "${ANTHROPIC_API_KEY:-unset}" "$*"
EOF
chmod +x "$root/bin/claude"
export PATH="$root/bin:$PATH"

fail() {
	printf 'FAIL: %s\n' "$*"
	exit 1
}

"$bin" add ci1 >/dev/null
[ "$("$bin" list --json)" = '["ci1"]' ] || fail "list --json"
dir="$HOME/.local/share/claude-accounts/ci1"
[ -L "$dir/skills" ] || fail "skills is not linked"
[ "$(cat "$dir/CLAUDE.md")" = '# shared' ] || fail "CLAUDE.md is not shared"
out=$(ANTHROPIC_API_KEY=secret "$bin" run ci1 --resume)
printf '%s\n' "$out" | grep -qx "CONFIG=$dir" || fail "wrong CLAUDE_CONFIG_DIR: $out"
printf '%s\n' "$out" | grep -qx 'APIKEY=unset' || fail "ANTHROPIC_API_KEY not removed: $out"
printf '%s\n' "$out" | grep -qx 'ARGS=--resume' || fail "arguments not passed: $out"
"$bin" isolate ci1 >/dev/null
[ "$("$bin" isolated --json)" = '["ci1"]' ] || fail "isolated --json"
printf 'smoke test OK\n'
