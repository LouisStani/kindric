# How it works

kindric never reads credentials. All account handling goes through [kindric-accounts](../crates/accounts/README.md).

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

Claude Code refuses to start a session in a directory the account never trusted: open `claude` there once first.

## Refresh

The list refreshes when:

- Claude Code writes to an account's `sessions/`, `daemon/` or `jobs/` directory: only that account is queried again
- a message arrives: no account is queried, only unread counts are counted again
- every 30 seconds, in case a change was missed: every account is queried

A burst of writes counts once: the list waits until 0.3 s pass without one, but never more than 1 s. Idle, kindric uses about 5% of one CPU core.

## Limits

- Mouse events are not forwarded to Claude Code.
- An interactive session started in another terminal cannot be shown in a pane: only background sessions can be attached.
- Resuming a background session with `kindric-accounts run … --bg --resume` starts a copy of it, because kindric-accounts always passes its own options. To continue the session itself, run `claude --bg --resume <id>` with no other option and `CLAUDE_CONFIG_DIR` set to the account directory.
