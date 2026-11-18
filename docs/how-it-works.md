# How it works

kindric never reads credentials. All account handling goes through [kindric-accounts](../crates/accounts/README.md).

<img src="how-it-works.svg" alt="kindric-desktop starts and attaches sessions through kindric-accounts, which runs Claude Code on each account. The sessions call the kindric command, its MCP server and hooks, which write to kindric's data folder. kindric-desktop watches that folder." width="100%">

## Accounts and sessions

- Accounts, isolated accounts and their directories come from the kindric-accounts library.
- Claude Code's own configuration, `~/.claude`, counts as an account named `default` when it is logged in. A plain Claude Code setup works without any kindric-accounts account.
- The session list comes from `claude agents --json` on each account, started with the same rules as `kindric-accounts run`.
- `default` is checked with `claude auth status --json` at start and at each full refresh. Its sessions are listed, attached, started and stopped with `claude` alone, without `CLAUDE_CONFIG_DIR` and without the billing variables. Claude Code then reads its own settings and MCP servers; kindric-accounts shares none with it.

## Panes

- A pane runs `kindric-accounts run <account> attach <id>` in its own pseudo-terminal. kindric builds kindric-accounts in as a library, but panes still start through the command, so it must be on your `PATH`.
- In the terminal interface, [alacritty_terminal](https://crates.io/crates/alacritty_terminal) keeps the screen and [ratatui](https://ratatui.rs) draws it. The desktop app draws with [xterm.js](https://xtermjs.org).

## New sessions

`Ctrl+N` starts `claude --bg` the way `kindric-accounts run` does (shared MCP servers, shared settings, billing variables removed), then opens the new session.

Before it starts, kindric writes down the state of the session's repository (`starts/` in its data folder), from which [Changed files](desktop.md#changed-files) compares.

Claude Code refuses to start a session in a directory the account never trusted. The form then offers **trust it**: Claude Code asks you in a window of the app, and kindric starts the session again once you press **done**. Your home directory cannot be trusted this way: Claude Code never saves that trust.

## Hooks

The sessions kindric-accounts starts get hooks that run the `kindric` command, when the `kindric` in `PATH` knows them. Claude Code reads them when a session starts, so only sessions started after kindric was installed have them.

| Hook | Event | What it does |
| --- | --- | --- |
| `kindric unread` | before each prompt, after each tool | tells the session a message is waiting |
| `kindric wait-message` | Stop, SessionStart, in the background | wakes the session when a message asks it to act now |
| `kindric away-hook` | Stop, PreToolUse, PermissionRequest | keeps the session working while you are away |
| `kindric permission-hook` | PermissionRequest | puts the question in the control center and gives back your answer |
| `kindric overview-start` | SessionStart | tells the session to ask you with `kindric ask` |
| `kindric artifact-check` | Stop | asks once to keep the files made for you |

`kindric ask "<question>" "<context>"` puts a question in the control center without stopping the session; your answer comes back as a message from you. Isolated accounts get only the hooks of absence and of the control center. Each hook is described in the [README of kindric-accounts](../crates/accounts/README.md#messages-of-kindric).

## Refresh

The list refreshes when:

- Claude Code writes to an account's `sessions/`, `daemon/` or `jobs/` directory: only that account is queried again
- a message arrives: no account is queried, only unread counts are counted again
- every 5 minutes, in case a change was missed (a session killed outright leaves its files as they were): every account is queried. Every 30 seconds when a folder could not be watched.

A burst of writes counts once: the list waits until 0.3 s pass without one, but never more than 1 s. Each query runs `claude agents --json` once per account.

Measured on Linux with two panes showing working sessions, the app's main process used about 5% of one CPU core, and its web view as much.

## Limits

- Mouse events are not forwarded to Claude Code.
- An interactive session started in another terminal cannot be shown in a pane: only background sessions can be attached.
- Resuming a background session with `kindric-accounts run … --bg --resume` starts a copy of it, because kindric-accounts always passes its own options. To continue the session itself, run `claude --bg --resume <id>` with no other option and `CLAUDE_CONFIG_DIR` set to the account directory.
