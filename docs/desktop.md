# Desktop app

`kindric-desktop` is the main interface. It draws with the system web view and [xterm.js](https://xtermjs.org).

- [Setup checklist](#setup-checklist)
- [Projects and tabs](#projects-and-tabs)
- [The session bar](#the-session-bar)
- [Panes](#panes)
- [Starting, naming and hiding sessions](#starting-naming-and-hiding-sessions)
- [Shortcuts](#shortcuts)
- [Themes](#themes)
- [Files kept by the app](#files-kept-by-the-app)

## Setup checklist

While something is left to set up and no pane of the visible tab shows a session, the app lists in place of the panes:

- Claude Code (a version older than 2.1.312 gets a warning)
- an account
- kindric-accounts (needed only for its own accounts)
- the messages server, on each side that lacks it
- a first session

Each missing step comes with the command to copy. For the messages server, an **add** button runs the commands for you, skipping isolated accounts. Claude Code writes its own configuration: the app never edits it. If a command fails, its error shows under the button.

`F1` shows the same list, followed by every shortcut.

## Projects and tabs

Each tab is a project: a folder, and the sessions working in it, on any account. In nested projects, the deepest folder wins.

- The first tab, **other**, holds the sessions of no project. It always stays.
- `Ctrl+Shift+T` adds a project. `Ctrl+Shift+X` removes the project of the visible tab; its sessions keep running.
- Each tab has its own panes. Sessions in hidden tabs stay attached.
- A tab with a session waiting for you carries a dot.
- The app starts again on the tab you left. At its first visit, each tab opens the session of its active pane again, in a single pane (splits are not kept).

## The session bar

The bar at the top shows the sessions of the visible project as chips. Sessions that need you come first:

1. unread messages
2. a finished turn
3. waiting
4. working
5. paused
6. running in another terminal

| Mark | State |
| --- | --- |
| filled dot | waiting for you |
| half dot | Claude is working |
| ✓ | finished a turn while you were not looking |
| empty ring | paused (no process; opening it wakes it up) |
| dashed ring | interactive session in another terminal |

A chip shows the state, a short name and the unread count. Hovering it shows Claude Code's full name, the account, the directory, the id and the session's progress line.

The **progress line** is what a background session last said it is doing. Claude Code writes it in `jobs/<id>/state.json` (or, once the session stopped, the last such line of `jobs/<id>/timeline.jsonl`). These files are not documented, so the line may disappear with a Claude Code update. It also shows in the pane head and in `Ctrl+P`.

Chips that do not fit are counted in a **+N** chip that opens `Ctrl+P`. Errors and notices show at the right end of the bar.

Clicking a chip opens the session in the active pane of its project's tab, moving there if needed. The pane says "connecting to <name>…" until the session first prints.

## Panes

- `Ctrl+Shift+D` splits the active pane: side by side if it is wider than tall, one above the other otherwise.
- `Ctrl+Shift+W` closes the pane. The session keeps running.
- Separators can be dragged.
- When Claude Code finishes a turn in a session you are not looking at, its chip gets a ✓ and its pane a green border and a **done** mark, until you look at it.
- `Ctrl+Shift+C` copies the selection, `Ctrl+Shift+V` pastes text. `Ctrl+C` and `Ctrl+V` still go to Claude Code, which interrupts with the first and pastes an image with the second.
- `Ctrl+Shift+M` shows the messages of the active pane's session. See [Messages](messages.md#in-the-desktop-app).

## Starting, naming and hiding sessions

**Start.** `Ctrl+N` asks for an account, a directory and an optional short name. In a project tab, it offers the project's folder and the account of the last session started there. The short name field names the session in Claude Code itself.

**Directory fields** (`Ctrl+N`, `Ctrl+Shift+T`) first list the folders already in use, projects then sessions. Once you type, they list the folders that can follow (case ignored, hidden folders after a `.`). `Tab` completes, `Up` / `Down` choose, `Enter` takes the chosen one.

**Find.** `Ctrl+P` searches the sessions of the visible tab by title, short name, account or directory, hidden sessions included.

**Name.** `F2` gives a short name to the session of the active pane, for this app only. An empty name removes it. Without one, the bar shows Claude Code's name, cut after the last whole word within 24 characters.

**Hide.** `Ctrl+Shift+K`, once confirmed, stops the session of the active pane (`claude stop`, its conversation is kept) and hides it from the bar. `Ctrl+P` still lists hidden sessions, last. Choosing one shows it again and starts it.

## Shortcuts

| Key | Effect |
| --- | --- |
| **Sessions** | |
| `Ctrl+N` | Start a new session |
| `Ctrl+P` | Find a session of this tab |
| `Alt+1` … `Alt+9` | Open chip n of the bar |
| `Alt+H` / `Alt+L` | Open the previous / next chip (wraps around, skips sessions in another terminal) |
| `F2` | Give a short name |
| `Ctrl+Shift+K` | Stop and hide (`Ctrl+P` brings it back) |
| `Ctrl+Shift+M` | Read its messages |
| **Panes** | |
| `Ctrl+Shift+D` | Split the pane |
| `Ctrl+Shift+W` | Close the pane (the session keeps running) |
| `Ctrl+H` / `Ctrl+L` | Previous / next pane |
| `Ctrl+Shift+C` | Copy the selection |
| `Ctrl+Shift+V` | Paste text |
| **Tabs** | |
| `Ctrl+Shift+T` | Add a project |
| `Ctrl+Shift+X` | Remove this project (its sessions keep running) |
| `Ctrl+Shift+H` / `Ctrl+Shift+L` | Previous / next tab |
| `Ctrl+Shift+1` … `Ctrl+Shift+9` | Tab n |
| **App** | |
| `Ctrl+,` | Settings: accounts |
| `Ctrl+Shift+E` | Next theme |
| `F1` | Setup checklist and these shortcuts |

Unlike the terminal interface, the number of a session changes with its state: `Alt+1` is always the session that needs you most.

**On macOS**, `Cmd` replaces `Ctrl` everywhere (`Cmd+N`, `Cmd+Shift+T`…), so all `Ctrl` keys go to the terminal. Copy and paste are `Cmd+C` and `Cmd+V`. Previous / next pane are `Cmd+[` and `Cmd+]`, since `Cmd+H` hides the app.

## Settings

`Ctrl+,` lists the accounts of kindric-accounts: logged in or not, plan, running sessions, isolated. **add** creates an account and opens Claude Code's login in a window of the app. **log in** does the same for an account that is logged out. **remove** takes an account off the list after you confirm; its folder stays on disk.

The memory section sets the size above which a session is warned about the memory it opens at startup: 30 KB by default. It is a warning, nothing is cut. The value is kept in `~/.config/claude-accounts/memory-warning`.

## Themes

`Ctrl+Shift+E` cycles through **ember** (the default), **paper** (light) and **graphite**. The choice is kept in `settings.json`.

A `theme.json` next to the mailbox adds a **custom** theme: ember with the colors it names.

```json
{"background": "#101010", "active": "#c04020"}
```

Names are the color variables at the top of `crates/desktop/ui/style.css`, without `--`. Only `#rgb`, `#rrggbb` and `#rrggbbaa` values are taken. The file is read again each time the theme is chosen.

Claude Code picks its own text colors for a dark background unless you set its light theme (`/theme`), so some output may look pale in paper.

## Files kept by the app

All in kindric's data folder, next to the mailbox: `~/.local/share/kindric/` on Linux and macOS (`$XDG_DATA_HOME/kindric/` if set), `%APPDATA%\kindric\` on Windows.

| File | Holds |
| --- | --- |
| `projects.json` | Projects, and the account last used in each |
| `workspace.json` | The tab you left and the session of each tab's active pane |
| `sessions.json` | Short names and hidden sessions |
| `settings.json` | The theme |
| `theme.json` | Your custom theme, if you write one |
