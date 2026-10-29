# claude-accounts

<img src="assets/header.svg" alt="claude-accounts: several claude code accounts side by side, one shared setup. no logout, no login." width="100%">

Run several Claude Code accounts side by side, each in its own terminal, with a single shared setup.

No more `/logout` and `/login` every time you switch between your personal and work accounts. Pick an account and Claude starts. Your `CLAUDE.md`, skills, agents, plugins and settings come with it.

## Why

Claude Code keeps one login per configuration directory. Switching accounts means logging out and back in, and you cannot keep two accounts open at once.

claude-accounts gives each account its own directory and links the shared parts back to `~/.claude`. You configure Claude once and every account uses that configuration.

- Keep two accounts open at the same time, in two terminals.
- Share one `CLAUDE.md` and one set of skills, agents, commands, plugins and themes across all accounts, plus the MCP servers you add to any of them.
- Create a shared directory later and existing accounts pick it up on their next launch.
- The script never reads, copies or sends your credentials. It makes no network requests.
- Install one Bash script. `gum` gives you a nicer menu, but it is optional.

## Installation

Requirements:

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) on your `PATH`
- Bash 4 or later
- [`gum`](https://github.com/charmbracelet/gum) (optional). Without it, the menu uses Bash `select`.
- [`jq`](https://jqlang.org) (optional). Without it, MCP servers are not shared.

Download the script from the latest release:

```sh
curl -fLo "$HOME/.local/bin/claude-accounts" --create-dirs \
  https://github.com/LouisStani/Claude-Accounts/releases/latest/download/claude-accounts
chmod +x "$HOME/.local/bin/claude-accounts"
claude-accounts --version
```

Make sure `~/.local/bin` is on your `PATH`. Run the same commands again to update.

Or install from source:

```sh
git clone https://github.com/LouisStani/Claude-Accounts.git claude-accounts
cd claude-accounts
install -Dm755 claude-accounts "$HOME/.local/bin/claude-accounts"
```

## Quick start

```sh
claude-accounts add personal
claude-accounts add work

claude-accounts run personal   # first launch: run /login inside Claude
```

Each later launch reuses the account you logged in with. Open a second terminal and run the other account at the same time:

```sh
claude-accounts run work
```

Or just run `claude-accounts` to open the menu. It shows whether each account is logged in, its plan and how many Claude sessions are open on it. From there you can launch, add or remove an account with the arrow keys. Escape cancels.

## Commands

| Command | Effect |
| --- | --- |
| `claude-accounts` | Opens the menu and starts Claude in the current terminal. |
| `claude-accounts add <name>` | Creates an account and its private directory. |
| `claude-accounts list [--json]` | Prints account names, one per line. |
| `claude-accounts status [--json]` | Shows whether each account is logged in, its plan (`pro`, `max`…) and its active sessions. |
| `claude-accounts remove <name> [--delete-dir \| --keep-dir]` | Removes an account from the list. Its directory is deleted only after you confirm, or as the option says. |
| `claude-accounts run <name> [args]` | Starts Claude with this account. Extra arguments go to `claude`, for example `--resume`. |
| `claude-accounts --version` | Prints the installed version. |

Names use 1 to 32 characters among `a-z`, `0-9`, `_` and `-`.

`add`, `list`, `remove` and `run` work without the menu, so you can use them in scripts and aliases:

```sh
alias cw='claude-accounts run work'
```

## Scripting

Other programs can drive claude-accounts through the commands below. Their output stays stable across all 1.x versions: new fields may be added, existing ones are not renamed or removed.

| Command | Output |
| --- | --- |
| `list` | Account names only, one per line. Nothing on stdout when there are no accounts. |
| `list --json` | `["personal","work"]`, or `[]` |
| `status --json` | An array of accounts, see below. `[]` when there are no accounts. |
| `remove <name> --delete-dir` | Removes the account and deletes its directory, without asking. |
| `remove <name> --keep-dir` | Removes the account and keeps its directory, without asking. |
| `run <name> [args]` | Replaces itself with `claude` running on this account. |
| `--version` | `claude-accounts 1.1.0` |

```json
[
  {"name": "personal", "state": "logged-in", "sessions": 1, "plan": "pro", "dir": "/home/me/.local/share/claude-accounts/personal"},
  {"name": "work", "state": "logged-out", "sessions": 0, "plan": null, "dir": "/home/me/.local/share/claude-accounts/work"}
]
```

- `state` is `logged-in`, `logged-out` or `unknown` (Claude Code did not answer).
- `sessions` is the number of running Claude sessions on the account.
- `plan` is the subscription type reported by Claude Code, or `null`.
- `dir` is the account's `CLAUDE_CONFIG_DIR`.

On error, the exit code is not 0 and stderr explains why: one line starting with `claude-accounts:`, or the usage text for an unknown command or option. The JSON is written by the script itself: `jq` is not needed.

## How it works

Each account gets its own `CLAUDE_CONFIG_DIR`.

| Data | Where it lives |
| --- | --- |
| Credentials, sessions, history, project memory | Private to each account |
| `CLAUDE.md`, `skills/`, `agents/`, `commands/`, `plugins/`, `themes/` | Symbolic links to `~/.claude` |
| `settings.json` | Read from `~/.claude/settings.json` and passed with `--settings` |
| MCP servers of the other accounts | Merged at launch and passed with `--mcp-config` |

Links are checked on every launch. Account directories use `700` permissions and the account list uses `600`.

## Shared MCP servers

Add an MCP server once, in any account, with the user scope:

```sh
claude mcp add -s user --transport http sentry https://mcp.sentry.dev/mcp
```

Every other account gets it on its next launch. Remove it from the account where you added it and it disappears everywhere on the next launch.

At each launch, the script reads the `mcpServers` of the other accounts, merges them into its own file and passes it with `--mcp-config`. It never writes to Claude Code's files.

- A server name the launched account already defines, at any scope, is not passed: the account keeps its own server and its own authentication.
- If two other accounts define the same name, the first account in `claude-accounts list` wins.
- If an account's `.claude.json` cannot be read, that account is skipped with a warning. If the launched account's own file cannot be read, or `jq` is missing, Claude starts without shared servers.
- Headers and environment variables of a server, API keys included, become available to every account.

## Security and billing

Before starting Claude Code, the script removes these environment variables:

```text
ANTHROPIC_API_KEY
ANTHROPIC_AUTH_TOKEN
CLAUDE_CODE_OAUTH_TOKEN
```

A key or token exported in your shell therefore cannot override the account you picked, and you are billed on the account you chose.

Login status comes from `claude auth status`, run with the same variables removed. Claude Code reads its own credentials locally; the script only reads the exit code and the plan name.

Active sessions are counted from the `sessions/` directory of each account, keeping only the processes that are still running `claude`. This directory is internal to Claude Code: if its layout changes, the count disappears and nothing else breaks.

## Development

The tests never touch a real Claude account. They run in a temporary `HOME` with fake `claude` and `gum` executables.

```sh
./tests.sh
```

Read [CONTRIBUTING.md](CONTRIBUTING.md) before contributing and [SECURITY.md](SECURITY.md) to report a vulnerability.
