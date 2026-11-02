# kindric-accounts

<img src="assets/header.svg" alt="kindric-accounts: several claude code accounts side by side, one shared setup. no logout, no login." width="100%">

kindric-accounts lives in the [kindric](https://github.com/LouisStani/kindric) repository, in `crates/accounts`. Its binaries are attached to that repository's releases.

Run several Claude Code accounts side by side, each in its own terminal, with a single shared setup.

No more `/logout` and `/login` every time you switch between your personal and work accounts. Pick an account and Claude starts. Your `CLAUDE.md`, skills, agents, plugins and settings come with it.

## Why

Claude Code keeps one login per configuration directory. Switching accounts means logging out and back in, and you cannot keep two accounts open at once.

kindric-accounts gives each account its own directory and links the shared parts back to `~/.claude`. You configure Claude once and every account uses that configuration.

- Keep two accounts open at the same time, in two terminals.
- Share one `CLAUDE.md` and one set of skills, agents, commands, plugins and themes across all accounts, plus the MCP servers you add to any of them.
- Create a shared directory later and existing accounts pick it up on their next launch.
- kindric-accounts never reads, copies or sends your credentials. It makes no network requests.
- One self-contained binary for Linux, macOS and Windows. Nothing else to install.

## Installation

You need [Claude Code](https://docs.anthropic.com/en/docs/claude-code) on your `PATH`.

**Linux and macOS.** Download the binary for your system from the latest release:

```sh
# x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, x86_64-apple-darwin or aarch64-apple-darwin
target=x86_64-unknown-linux-gnu
curl -fLo "$HOME/.local/bin/kindric-accounts" --create-dirs \
  "https://github.com/LouisStani/kindric/releases/latest/download/kindric-accounts-$target"
chmod +x "$HOME/.local/bin/kindric-accounts"
kindric-accounts --version
```

Make sure `~/.local/bin` is on your `PATH`. Run the same commands again to update.

**Windows** (PowerShell):

```powershell
$dir = "$env:LOCALAPPDATA\Programs\kindric-accounts"
New-Item -ItemType Directory -Force $dir | Out-Null
Invoke-WebRequest -OutFile "$dir\kindric-accounts.exe" `
  https://github.com/LouisStani/kindric/releases/latest/download/kindric-accounts-x86_64-pc-windows-msvc.exe
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
[Environment]::SetEnvironmentVariable('Path', "$userPath;$dir", 'User')
```

Open a new terminal, then run `kindric-accounts --version`. Windows support is new: it passes the automated tests but has not been used day to day yet. Please report any problem.

**From source**, with Rust installed:

```sh
cargo install --locked --git https://github.com/LouisStani/kindric kindric-accounts
```

**Upgrading from claude-accounts.** kindric-accounts is claude-accounts under a new name. It reads the same files, so your accounts work as they are. Remove the old command (`cargo uninstall claude-accounts`, or the 1.x Bash script), then install this one. Bash, `jq` and `gum` are no longer needed.

## Quick start

```sh
kindric-accounts add personal
kindric-accounts add work

kindric-accounts run personal   # first launch: run /login inside Claude
```

Each later launch reuses the account you logged in with. Open a second terminal and run the other account at the same time:

```sh
kindric-accounts run work
```

Or just run `kindric-accounts` to open the menu. It shows whether each account is logged in, its plan and how many Claude sessions are open on it. Type to filter, use the arrow keys and Enter to launch, add or remove an account. Escape cancels.

## Commands

| Command | Effect |
| --- | --- |
| `kindric-accounts` | Opens the menu and starts Claude in the current terminal. |
| `kindric-accounts add <name>` | Creates an account and its private directory. |
| `kindric-accounts list [--json]` | Prints account names, one per line. |
| `kindric-accounts status [--json]` | Shows whether each account is logged in, its plan (`pro`, `max`…) and its active sessions. |
| `kindric-accounts remove <name> [--delete-dir \| --keep-dir]` | Removes an account from the list. Its directory is deleted only after you confirm, or as the option says. |
| `kindric-accounts run <name> [args]` | Starts Claude with this account. Extra arguments go to `claude`, for example `--resume`. |
| `kindric-accounts isolate <name>` | Stops sharing MCP servers with and from this account. See [Isolated accounts](#isolated-accounts). |
| `kindric-accounts unisolate <name>` | Shares MCP servers again from the next launch. |
| `kindric-accounts isolated [--json]` | Lists isolated accounts. |
| `kindric-accounts --version` | Prints the installed version. |

Names use 1 to 32 characters among `a-z`, `0-9`, `_` and `-`. `default` is reserved: kindric uses it for Claude Code's own configuration, `~/.claude`.

`add`, `list`, `remove` and `run` work without the menu, so you can use them in scripts and aliases:

```sh
alias cw='kindric-accounts run work'
```

## Scripting

Other programs can drive kindric-accounts through the commands below. Their output stays stable across all 1.x versions: new fields may be added, existing ones are not renamed or removed.

[kindric](https://github.com/LouisStani/kindric) uses them to show the Claude sessions of all your accounts side by side in one terminal, and to let them send each other messages.

| Command | Output |
| --- | --- |
| `list` | Account names only, one per line. Nothing on stdout when there are no accounts. |
| `list --json` | `["personal","work"]`, or `[]` |
| `status --json` | An array of accounts, see below. `[]` when there are no accounts. |
| `remove <name> --delete-dir` | Removes the account and deletes its directory, without asking. |
| `remove <name> --keep-dir` | Removes the account and keeps its directory, without asking. |
| `run <name> [args]` | Replaces itself with `claude` running on this account. |
| `isolated --json` | `["work"]`, or `[]` |
| `--version` | `kindric-accounts 2.0.0` |

```json
[
  {"name": "personal", "state": "logged-in", "sessions": 1, "plan": "pro", "dir": "/home/me/.local/share/claude-accounts/personal", "isolated": false},
  {"name": "work", "state": "logged-out", "sessions": 0, "plan": null, "dir": "/home/me/.local/share/claude-accounts/work", "isolated": true}
]
```

- `state` is `logged-in`, `logged-out` or `unknown` (Claude Code did not answer).
- `sessions` is the number of running Claude sessions on the account.
- `plan` is the subscription type reported by Claude Code, or `null`.
- `dir` is the account's `CLAUDE_CONFIG_DIR`.
- `isolated` is `true` for an isolated account (since 1.2.0).

On error, the exit code is not 0 and stderr explains why: one line starting with `kindric-accounts:`, or the usage text for an unknown command or option. `jq` is not needed.

## How it works

Each account gets its own `CLAUDE_CONFIG_DIR`.

| Data | Where it lives |
| --- | --- |
| Credentials, sessions, history, project memory | Private to each account |
| `CLAUDE.md`, `skills/`, `agents/`, `commands/`, `plugins/`, `themes/` | Symbolic links to `~/.claude` (see Windows below) |
| `settings.json` | Read from `~/.claude/settings.json` and passed with `--settings` |
| MCP servers of the other accounts | Merged at launch and passed with `--mcp-config` |

Links are checked on every launch. On Linux and macOS, account directories use `700` permissions and the account list uses `600`.

On Windows, a symbolic link needs developer mode. Without it, folders are shared through junctions and `CLAUDE.md` is copied, then refreshed from `~/.claude` at each launch. If you edit that copy inside an account, it is no longer refreshed and kindric-accounts warns you: copy your changes to `~/.claude/CLAUDE.md`. Accounts live in `%APPDATA%\claude-accounts`.

## Shared MCP servers

Add an MCP server once, in any account, with the user scope:

```sh
claude mcp add -s user --transport http sentry https://mcp.sentry.dev/mcp
```

Every other account gets it on its next launch. Remove it from the account where you added it and it disappears everywhere on the next launch.

At each launch, kindric-accounts reads the `mcpServers` of the other accounts, merges them into its own file and passes it with `--mcp-config`. It never writes to Claude Code's files.

- A server name the launched account already defines, at any scope, is not passed: the account keeps its own server and its own authentication.
- If two other accounts define the same name, the first account in `kindric-accounts list` wins.
- If an account's `.claude.json` cannot be read, that account is skipped with a warning. If the launched account's own file cannot be read, Claude starts without shared servers.
- Headers and environment variables of a server, API keys included, become available to every account. Isolate an account to keep it out of this.

## Isolated accounts

Keep a work account apart from your personal ones:

```sh
kindric-accounts isolate work
```

An isolated account gives its MCP servers to no other account and receives none from them. Its own servers, added in that account, still work. It keeps the shared `CLAUDE.md`, skills, agents, commands, plugins, themes and settings.

Isolation applies from the next launch. Sessions already running keep the servers they loaded: restart them, on the isolated account and on the others.

[kindric](https://github.com/LouisStani/kindric) also keeps isolated accounts out of its messages between sessions. It shows Claude Code's own configuration, `~/.claude`, as an account named `default`: `kindric-accounts isolate default` keeps it out of messages too, although it is not an account of this list. kindric-accounts shares no MCP servers with it either way.

## Security and billing

Before starting Claude Code, kindric-accounts removes these environment variables:

```text
ANTHROPIC_API_KEY
ANTHROPIC_AUTH_TOKEN
CLAUDE_CODE_OAUTH_TOKEN
```

A key or token exported in your shell therefore cannot override the account you picked, and you are billed on the account you chose.

Login status comes from `claude auth status`, run with the same variables removed. Claude Code reads its own credentials locally; kindric-accounts only reads the exit code and the plan name.

Active sessions are counted from the `sessions/` directory of each account, keeping only the processes that are still running `claude`. This directory is internal to Claude Code: if its layout changes, the count disappears and nothing else breaks.

## Development

The tests never touch a real Claude account. They run in a temporary `HOME` with a fake `claude` executable.

```sh
cargo test
cargo clippy --all-targets
cargo build --release -p kindric-accounts && ./tests.sh   # Linux: the full command-line contract
```

On macOS and Windows, the CI runs `ci/smoke.sh` and `ci/smoke.ps1` instead of `tests.sh`, which needs GNU tools.

Read [CONTRIBUTING.md](../../CONTRIBUTING.md) before contributing and [SECURITY.md](../../SECURITY.md) to report a vulnerability.

## License

kindric-accounts is licensed under the [GNU General Public License v3.0 only](../../LICENSE) (`GPL-3.0-only`). You can use it for any purpose, including inside a company, change it and share it. If you distribute it, changed or not, you must share its source code under the same license. Versions 1.2.1 to 2.0.0 were released under CC BY-NC-SA 4.0, and versions up to 1.2.0 under the Apache License 2.0; they keep their license.
