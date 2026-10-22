# claude-accounts

<img src="assets/header.svg" alt="claude-accounts: several claude code accounts side by side, one shared setup. no logout, no login." width="100%">

Run several Claude Code accounts side by side, each in its own terminal, with a single shared setup.

No more `/logout` and `/login` every time you switch between your personal and work accounts. Pick an account and Claude starts. Your `CLAUDE.md`, skills, agents, plugins and settings come with it.

## Why

Claude Code keeps one login per configuration directory. Switching accounts means logging out and back in, and you cannot keep two accounts open at once.

claude-accounts gives each account its own directory and links the shared parts back to `~/.claude`. You configure Claude once and every account uses that configuration.

- Keep two accounts open at the same time, in two terminals.
- Share one `CLAUDE.md` and one set of skills, agents, commands, plugins and themes across all accounts.
- Create a shared directory later and existing accounts pick it up on their next launch.
- The script never reads, copies or sends your credentials. It makes no network requests.
- Install one Bash script. `gum` gives you a nicer menu, but it is optional.

## Installation

Requirements:

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) on your `PATH`
- Bash 4 or later
- [`gum`](https://github.com/charmbracelet/gum) (optional). Without it, the menu uses Bash `select`.

```sh
git clone https://github.com/LouisStani/Claude-Accounts.git claude-accounts
cd claude-accounts
install -Dm755 claude-accounts "$HOME/.local/bin/claude-accounts"
```

Make sure `~/.local/bin` is on your `PATH`.

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

Or just run `claude-accounts` to open the menu. From there you can launch, add or remove an account with the arrow keys. Escape cancels.

## Commands

| Command | Effect |
| --- | --- |
| `claude-accounts` | Opens the menu and starts Claude in the current terminal. |
| `claude-accounts add <name>` | Creates an account and its private directory. |
| `claude-accounts list` | Prints account names, one per line. |
| `claude-accounts remove <name>` | Removes an account from the list. Its directory is deleted only after you confirm. |
| `claude-accounts run <name> [args]` | Starts Claude with this account. Extra arguments go to `claude`, for example `--resume`. |

Names use 1 to 32 characters among `a-z`, `0-9`, `_` and `-`.

`add`, `list`, `remove` and `run` work without the menu, so you can use them in scripts and aliases:

```sh
alias cw='claude-accounts run work'
```

## How it works

Each account gets its own `CLAUDE_CONFIG_DIR`.

| Data | Where it lives |
| --- | --- |
| Credentials, sessions, history, project memory | Private to each account |
| `CLAUDE.md`, `skills/`, `agents/`, `commands/`, `plugins/`, `themes/` | Symbolic links to `~/.claude` |
| `settings.json` | Read from `~/.claude/settings.json` and passed with `--settings` |

Links are checked on every launch. Account directories use `700` permissions and the account list uses `600`.

## Security and billing

Before starting Claude Code, the script removes these environment variables:

```text
ANTHROPIC_API_KEY
ANTHROPIC_AUTH_TOKEN
CLAUDE_CODE_OAUTH_TOKEN
```

A key or token exported in your shell therefore cannot override the account you picked, and you are billed on the account you chose.

## Development

The tests never touch a real Claude account. They run in a temporary `HOME` with fake `claude` and `gum` executables.

```sh
./tests.sh
```

Read [CONTRIBUTING.md](CONTRIBUTING.md) before contributing and [SECURITY.md](SECURITY.md) to report a vulnerability.
