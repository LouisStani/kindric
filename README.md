# claude-accounts

Use multiple Claude Code accounts in separate terminals without running `/logout` and `/login` whenever you switch.

Each account keeps its own credentials, history, and sessions. Working instructions, skills, agents, and plugins stay shared.

## Overview

```text
claude-accounts
2 accounts ready to launch

Choose an account to open Claude in this terminal.

› lystriq
  louis
  + Add an account
  - Remove an account
  Quit
```

## Requirements

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed and available on your `PATH`
- Bash 4 or later
- [`gum`](https://github.com/charmbracelet/gum), optional: the menu falls back to Bash `select`

## Installation

Clone the repository, then install the script on your `PATH`:

```sh
git clone <URL_DU_DEPOT> claude-accounts
cd claude-accounts
install -Dm755 claude-accounts "$HOME/.local/bin/claude-accounts"
```

For development, create a symbolic link instead of copying the script:

```sh
mkdir -p "$HOME/.local/bin"
ln -s "$(pwd)/claude-accounts" "$HOME/.local/bin/claude-accounts"
```

Make sure `~/.local/bin` is on your `PATH`.

## Quick start

```sh
# Create two isolated profiles.
claude-accounts add personal
claude-accounts add work

# Start Claude Code with a profile.
claude-accounts run personal

# On the first launch, use /login in Claude Code.
# Later launches reuse the same account.
```

Run `claude-accounts` to open the interactive dashboard. It lets you launch, add, or remove an account. Invalid names stay within the menu flow: correct the name or cancel with Escape.

You can open both accounts at the same time in separate terminals:

```sh
claude-accounts run personal
claude-accounts run work
```

## Commands

| Command | Effect |
| --- | --- |
| `claude-accounts` | Opens the dashboard and starts Claude in the current terminal. |
| `claude-accounts add <name>` | Creates a profile and its private directory. |
| `claude-accounts list` | Prints profile names, one per line. |
| `claude-accounts remove <name>` | Removes a profile from the list. Its directory is deleted only after confirmation. |
| `claude-accounts run <name> [args]` | Starts a profile and passes arguments to `claude`, for example `--resume`. |

Names may contain 1 to 32 characters: `a-z`, `0-9`, `_`, and `-`.

## How it works

`claude-accounts` starts Claude Code with a dedicated `CLAUDE_CONFIG_DIR` for each profile.

| Data | Location |
| --- | --- |
| Credentials, sessions, history, and project memory | private to each profile |
| `CLAUDE.md`, `skills/`, `agents/`, `commands/`, `plugins/`, `themes/` | symbolic links to `~/.claude` |
| Shared settings | `~/.claude/settings.json`, passed with `--settings` |

Links to the shared configuration are checked on every launch. If you later add a shared directory such as `agents/`, it becomes available to existing profiles the next time they start.

Profile directories use `700` permissions, and the account registry uses `600`.

## Security and billing

The script never reads, copies, or displays credentials. It makes no network requests.

Before starting Claude Code, it removes these environment variables:

```text
ANTHROPIC_API_KEY
ANTHROPIC_AUTH_TOKEN
CLAUDE_CODE_OAUTH_TOKEN
```

This prevents an API key or token from another account from taking precedence over the selected account.

## Development

The tests never use a real Claude account: they create a temporary `HOME`, a fake `claude` executable, and a fake `gum` executable.

```sh
./tests.sh
```

Read [CONTRIBUTING.md](CONTRIBUTING.md) before contributing, and [SECURITY.md](SECURITY.md) to report a vulnerability.
