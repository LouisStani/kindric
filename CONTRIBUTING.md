# Contributing to claude-accounts

Thank you for helping improve the project.

## Before opening a contribution

- Never add an OAuth token, API key, Claude session, or content from `~/.claude` to the repository.
- Preserve the behavior without `gum`: the menu must always have a fallback using standard Bash tools.
- Keep `add`, `list`, `remove`, and `run` suitable for scripting.
- Document every change that affects configuration paths, permissions, or account management.

## Local verification

```sh
./tests.sh
```

The tests use a temporary environment and do not touch the machine’s Claude accounts.

## Style

- Use strict Bash (`set -euo pipefail`).
- Prefer small functions and actionable error messages.
- Do not assume that `gum` is installed.
