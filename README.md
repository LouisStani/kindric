# kindric

Several Claude Code sessions, across several accounts, side by side in one desktop window, or in one terminal. Sessions can leave each other short messages.

kindric is the companion of [claude-accounts](crates/accounts/README.md), which lives in this repository. claude-accounts runs each account with its own login and a shared setup. kindric shows the sessions of all those accounts in one place.

```text
┌ sessions ─────────────┐┌ roblox · louis ─────────────┐┌ api · lystriq ──────────────┐
│1 roblox               ││                             ││                             │
│  louis · working      ││  Claude Code                ││  Claude Code                │
│2 api 1 msg            ││                             ││                             │
│  lystriq · waiting    ││ ❯                           ││ ❯                           │
└ Ctrl+P · Ctrl+N ──────┘└─────────────────────────────┘└─────────────────────────────┘
```

## What it does

- Lists the Claude Code sessions of every account managed by claude-accounts, with their state. Claude Code's own configuration, `~/.claude`, counts as an account named `default` when it is logged in, so a plain Claude Code setup works without claude-accounts accounts. The terminal interface leaves `default` out.
- Shows two sessions side by side. Each pane is a real Claude Code session: everything you type goes to it.
- Starts new background sessions on the account and in the directory you choose.
- Lets sessions send each other messages through a small MCP server, across accounts.

There are two interfaces. The desktop app (`kindric-desktop`) is the main one: projects as tabs, short names, hidden sessions, messages and progress lines, themes, and every new feature. The terminal interface (`kindric`) stays as it is, for machines without a display (over SSH, say); it gets no new features, but it is still built and tested, and its binary runs the MCP server for messages.

Closing kindric closes nothing: the sessions are Claude Code background sessions and keep running. Open kindric again and they are still there.

## Requirements

- [claude-accounts](crates/accounts/README.md), from this repository, on your `PATH` (installed below). kindric also builds it in as a library, but panes still start through the command.
- Claude Code with background sessions (`claude --bg`, `claude attach`, `claude agents`). Tested with 2.1.312.
- Rust, to build
- Used day to day on Linux. CI builds and tests on Linux, macOS and Windows.

## Installation

```sh
git clone https://github.com/LouisStani/kindric.git kindric
cargo install --locked --path kindric/crates/accounts
cargo install --locked --path kindric/crates/tui
```

The binaries go to `~/.cargo/bin`. Make sure it is on your `PATH`. Install both even if you only use the desktop app: `kindric` runs the MCP server for messages. claude-accounts can also be installed alone, without Rust: see [its README](crates/accounts/README.md#installation).

The desktop app installs the same way:

```sh
cargo install --locked --path kindric/crates/desktop
```

Start it with `kindric-desktop`. It draws with the system web view: on Linux, WebKitGTK 4.1 must be installed (`webkit2gtk-4.1` on Arch, `libwebkit2gtk-4.1-dev` on Debian and Ubuntu). Messages between sessions still go through the `kindric` binary above.

To let sessions send messages, add the MCP server once, in any account, with the user scope:

```sh
claude-accounts run <account> mcp add -s user kindric -- kindric mcp
```

claude-accounts shares it with your other accounts, but not with `default` (Claude Code's own configuration, `~/.claude`), which needs its own:

```sh
claude mcp add -s user kindric -- kindric mcp
```

Sessions started after this have it; sessions already running do not.

The desktop app checks all of this itself. While something is left to set up and no pane of the visible tab shows a session, it lists, in place of the panes: Claude Code (a version older than 2.1.312, the one it was tested with, gets a warning), an account, claude-accounts (needed only for its own accounts), the messages server on each side that lacks it, and a first session. Each missing step comes with the command to copy. For the messages server, an "add" button runs these commands for you (Claude Code writes its own configuration, the app never edits it), skipping isolated accounts; if one fails, its error shows under the button. F1 shows the same list.

## Usage

```sh
kindric
```

| Key | Effect |
| --- | --- |
| `Alt+1` … `Alt+9` | Shows session 1 to 9 of the list in the active pane |
| `Ctrl+P` | Finds a session by title, account or directory. Its last entry redraws the screen. |
| `Ctrl+N` | Starts a new background session: account, directory, optional name |
| `Ctrl+H` / `Ctrl+L` | Makes the left or right pane active |
| `Ctrl+Q` | Quits. The sessions keep running. |

Every other key goes to the session in the active pane.

kindric keeps `Ctrl+H`, `Ctrl+L`, `Ctrl+N`, `Ctrl+P` and `Ctrl+Q` for itself. Claude Code uses `Ctrl+N` and `Ctrl+P` like the arrow keys, which still work. It uses `Ctrl+L` to redraw its screen: use the redraw entry of `Ctrl+P` instead.

### Session states

| State | Meaning |
| --- | --- |
| `working` | Claude is answering |
| `waiting` | Claude waits for you |
| `paused` | Background session with no process. Opening it wakes it up. |
| `elsewhere` | Interactive session running in another terminal. It can receive messages but cannot be shown here. |

`isolated` after the state marks a session of an isolated account.

`2 msg` after a title means the session has two unread messages.

## Messages between sessions

The MCP server gives every session three tools:

| Tool | Effect |
| --- | --- |
| `list_sessions` | Lists the sessions of all accounts: title, account, state, id, directory |
| `send_message` | Leaves a message for another session, by title or id |
| `read_messages` | Returns the unread messages of this session, once |

A session reads its messages only when it calls `read_messages`, for example when you ask it to. Nothing is typed into a session for you.

In the desktop interface, Ctrl+Shift+M shows the messages the session of the active pane received and sent, oldest first, read or not (a click on the message count of a chip does the same for that session). It only reads them: nothing is marked read, so the session still gets its messages from `read_messages`.

Messages are information, not orders. The tools tell Claude to treat them that way and to ask you before acting on a request they contain. In testing, a session that received "please create this file" did not create it and asked the user first.

Isolated accounts (`claude-accounts isolate <account>`, for a work account for example) stay out of messages: their sessions never appear in `list_sessions`, cannot receive messages, and the MCP server refuses to start in them. They can still be shown in a pane.

Messages are stored as one JSON file each in `~/.local/share/kindric/mailbox/<session id>/` (or under `$XDG_DATA_HOME`), in directories readable only by you. A message that has been read moves to `read/`.

## How it works

- Accounts, isolated accounts and their directories come from the claude-accounts library. The session list comes from `claude agents --json` on each account, started with the same rules as `claude-accounts run`.
- A pane runs `claude-accounts run <account> attach <id>` in its own pseudo-terminal. [alacritty_terminal](https://crates.io/crates/alacritty_terminal) keeps its screen and [ratatui](https://ratatui.rs) draws it.
- `Ctrl+N` starts `claude --bg` the way `claude-accounts run` does (shared MCP servers, shared settings, billing variables removed), then opens the new session. Claude Code refuses to start one in a directory the account never trusted: open `claude` there once first.
- `default` is checked with `claude auth status --json` at start and at each full refresh. Its sessions are listed, attached, started and stopped with `claude` alone, without `CLAUDE_CONFIG_DIR` and without the billing variables: Claude Code then reads its own settings and MCP servers, and claude-accounts shares none with it. `claude-accounts isolate default` keeps it out of messages.
- The list refreshes when Claude Code writes to an account's `sessions/`, `daemon/` or `jobs/` directory, or when a message arrives, and every 30 seconds in case a change was missed. A write queries only its account again, a message none (only unread counts are counted again), and the 30-second refresh queries every account. A burst of writes counts once: the list waits until 0.3 s pass without one, but never more than 1 s. Idle, kindric uses about 5% of one CPU core.

kindric never reads credentials. All account handling goes through claude-accounts.

## Limits

- Mouse events are not forwarded to Claude Code.
- An interactive session started in another terminal cannot be shown in a pane: only background sessions can be attached.
- Resuming a background session through `claude-accounts run … --bg --resume` starts a copy of it, because claude-accounts always passes its own options. To continue the session itself, run `claude --bg --resume <id>` with no other option and `CLAUDE_CONFIG_DIR` set to the account directory.

## Development

The workspace has four crates: `crates/accounts` (claude-accounts, a library and a binary, with its own version and [README](crates/accounts/README.md)), `crates/core` (accounts, sessions, messages, refresh, shared by every interface), `crates/tui` (the terminal interface, the `kindric` binary) and `crates/desktop` (the desktop interface, built with [Tauri](https://tauri.app). Each tab is a project: a folder, and the sessions working in it, on any account (in nested projects, the deepest folder wins). The first tab, "other", holds the sessions of no project and stays. Ctrl+Shift+T adds a project, Ctrl+Shift+X removes the project of the visible tab (its sessions keep running); projects are kept in `projects.json` next to the mailbox and come back at the next start. The app also starts on the tab you left, and each tab opens again, in a single pane, the session of its active pane, at its first visit (kept in `workspace.json`; splits are not kept). A bar at the top shows the sessions of the visible project as chips, those that need you first: unread messages, then a finished turn, then waiting, working and paused, and sessions running in another terminal last. A chip shows the state (filled dot waiting, half dot working, ✓ finished, empty ring paused, dashed ring another terminal), a short name and the unread count; hovering it shows Claude Code's whole name, the account, the directory, the id and the session's progress line. That line is what a background session last said it is doing, which Claude Code writes in `jobs/<id>/state.json` (or, once the session stopped, the last such line of `jobs/<id>/timeline.jsonl`); these files are not documented, so the line may disappear with a Claude Code update. It also shows in the head of the session's pane and in Ctrl+P. The short name is the one given with F2 to the session of the active pane (kept in `sessions.json` next to the mailbox, for this app only; an empty name removes it), else Claude Code's name, cut after the last whole word within 24 characters. Ctrl+N's "short name" field names the session in Claude Code itself; Ctrl+P also finds sessions by their short name. Ctrl+Shift+K, once confirmed, stops the session of the active pane (`claude stop`: its conversation is kept) and hides it from the bar, also in `sessions.json`; Ctrl+P still lists hidden sessions, last, and choosing one shows it again and starts it. Chips that do not fit are counted in a "+N" chip that opens Ctrl+P, and errors or notices show at the right end of the bar. Clicking a background session opens it in the active pane of its project's tab, moving there if needed (the pane says "connecting to <name>…" until the session first prints); a tab with a session waiting for you carries its dot. Ctrl+P searches the sessions of the visible tab, hidden ones included. Ctrl+N in a project tab offers the project's folder and the account of the last session started in it (kept in `projects.json`). Directory fields (Ctrl+N, Ctrl+Shift+T) first list the folders already in use, projects then sessions; once you type, they list the folders that can follow (case ignored, hidden folders after a "."): Tab completes, Up/Down choose, Enter takes the chosen one. Ctrl+Shift+D splits the active pane (side by side if it is wider than tall, one above the other otherwise), Ctrl+Shift+W closes it (the session keeps running), Ctrl+H/Ctrl+L move to the previous/next pane, and separators can be dragged. Each tab has its own panes: Ctrl+Shift+H/Ctrl+Shift+L move to the previous/next tab and Ctrl+Shift+1..9 to tab n; sessions in hidden tabs stay attached. A session where Claude Code finished a turn while you were not looking at it (in the active pane of the visible tab) gets a ✓ in the bar and its pane a green border and a "done" mark, until you look at it. Ctrl+N starts a new session and Ctrl+P chooses the session shown, as in the terminal. Ctrl+Shift+C copies the selection of the active pane and Ctrl+Shift+V pastes text into its session; Ctrl+C and Ctrl+V still go to Claude Code, which interrupts with the first and pastes an image with the second. F1 lists what is left to set up, then every shortcut of the app. On macOS, Cmd takes the place of Ctrl in all of them (Cmd+N, Cmd+Shift+T…), copy and paste are Cmd+C and Cmd+V, so Ctrl keys all go to the terminal, and Cmd+[ / Cmd+] move to the previous/next pane, since Cmd+H hides the app there. Alt+1..9 opens the chip at that place in the bar of the visible tab, so Alt+1 is the session there that needs you most, and Alt+H/Alt+L open the chip before/after the session of the active pane (round the ends; sessions in another terminal are skipped); unlike the terminal, the number of a session changes with its state).

The desktop page has no build step. [xterm.js](https://xtermjs.org) 6.0.0 and its fit (0.11.0) and WebGL (0.19.0) addons are copied from npm into `crates/desktop/ui/vendor/`, each with its MIT license. The interface font, [JetBrains Mono](https://www.jetbrains.com/lp/mono/) 2.304 (regular and bold, from the official release), sits next to them with its OFL license, so the app looks the same on every system. The colors and the font are CSS variables at the top of `style.css`. Ctrl+Shift+E moves to the next theme (ember, the default, paper, light, and graphite), kept in `settings.json`. A `theme.json` next to the mailbox adds a "custom" theme: ember with the colors it names, for example `{"background": "#101010", "active": "#c04020"}` (names are the color variables without `--`; only `#rgb`, `#rrggbb` and `#rrggbbaa` values are taken). It is read again each time it is chosen. Claude Code picks its own text colors for a dark background unless you set its light theme (`/theme`), so some output may be pale in paper.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo run -p kindric-desktop
```

On Linux the desktop interface needs WebKitGTK 4.1 (`webkit2gtk-4.1` on Arch).

To find where a delay comes from, start the desktop interface with `KINDRIC_TRACE=1`: it writes `trace.log` next to the mailbox, started over at each launch, one line per timing (time since launch, what, how long): every command, the first output of an attached session, `claude agents --json` per account, each published list, and in the page each shortcut, chip click, render and first output shown.

The icon is drawn in `crates/desktop/icons/icon.svg`. After changing it, regenerate the PNG and the Windows ICO (needs librsvg and ImageMagick):

```sh
cd crates/desktop/icons
rsvg-convert -w 256 -h 256 icon.svg -o icon.png
for s in 16 24 32 48 64 256; do rsvg-convert -w $s -h $s icon.svg -o /tmp/icon-$s.png; done
magick /tmp/icon-{16,24,32,48,64,256}.png icon.ico
```

The tests never start Claude Code and never touch your accounts. CI runs clippy and the tests on Linux, macOS and Windows at every push, plus claude-accounts' command-line tests (`crates/accounts/tests.sh`, Linux) and smoke tests (macOS, Windows). A `v*` tag publishes the claude-accounts binaries for five targets in the release.

Read [CONTRIBUTING.md](CONTRIBUTING.md) before contributing and [SECURITY.md](SECURITY.md) to report a vulnerability.

## License

kindric is licensed under the [GNU General Public License v3.0 only](LICENSE) (`GPL-3.0-only`). You can use it for any purpose, including inside a company, change it and share it. If you distribute it, changed or not, you must share its source code under the same license. Earlier versions were released under CC BY-NC-SA 4.0 and keep that license.
