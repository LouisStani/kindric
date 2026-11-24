# Desktop app

`kindric-desktop` is the main interface. It draws with the system web view and [xterm.js](https://xtermjs.org).

- [Setup checklist](#setup-checklist)
- [Projects and tabs](#projects-and-tabs)
- [Control center](#control-center)
- [The session bar](#the-session-bar)
- [Panes](#panes)
- [Starting, naming and hiding sessions](#starting-naming-and-hiding-sessions)
- [Away](#away)
- [Changed files](#changed-files)
- [Worktrees](#worktrees)
- [Artifacts](#artifacts)
- [Shortcuts](#shortcuts)
- [Settings](#settings)
- [Themes](#themes)
- [Files kept by the app](#files-kept-by-the-app)
- [Signal](#signal)
- [Codex accounts](#codex-accounts)

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
- A tab with a session waiting for you carries a dot: `?` when one asks you something, before anything else.
- The app starts again on the tab you left. At its first visit, each tab opens the session of its active pane again, in a single pane (splits are not kept).

## Control center

Tab **0**, `Ctrl+Shift+0`, is the page the app opens on. It shows every session without opening any of them.

- On the left, **all**, then each project with a dot per session and the number that need you. Below, each account with where its model comes from (`claude`, `openrouter qwen/qwen3-coder`, `codex`…), its 5 h and 7 day limits and when the 5 h window starts again; a frame marks one past 80%. An account whose model does not come from a Claude subscription has no such limits.
- **all** lists the sessions that need you first, whatever their project, then a group per project. A project's name opens it.
- The dot left of a project's name, on the left, leaves it out of **all** or brings it back. Left out, its name is grey and **all** shows only its sessions that need you; the project's own page still shows them all. The choice is kept in `projects.json`. **other** always shows.
- A project lists its sessions, the paused ones last with **resume**. Its head counts the files its sessions changed (each session since it started, so a file two sessions touched counts twice), its worktrees and its artifacts; a click opens the window of each. A row's **N files** opens the changes of its session.
- A session with a task list (Claude Code's TaskCreate and TaskUpdate, which kindric-accounts has a long turn make) shows how far it is on its row (`3/5 · ~4 min left`, the time once a task is done, at the pace of those done), and the task in progress with what it is about on a line under it; its detail lists every task.
- Each row gives what the session does, the files it kept for you (`3 kept · 2 new`, a click opens the artifacts window), how full its context is, and its account. A permission, a question or a `kindric ask` is answered in the row. A `kindric ask` is also over once you type any message into its session, in a pane or another terminal: kindric reads it in the session's transcript and drops the question. A question a session writes in its reply comes too, at the end of its turn, when Claude Code classes the turn as waiting for you; it is answered the same way. A permission or a question of AskUserQuestion leaves the page within 10 s once nobody waits for its answer any more (its session ended, or Claude Code stopped the hook); answer it on the session's screen if it is still there. A `kindric ask` or a question of a reply nobody answered for 18 h leaves too, and everything a session asked leaves once you hide it (`Ctrl+Shift+K`).
- A row picked shows where its session is, its last steps read from its transcript (tools, results, what Claude wrote), and the files it kept for you, the new ones framed; a click on one opens it with the system's app.
- A click picks a row and opens its field: what you type there goes to the session as your message, even while it works (it reads it at the end of its step). **open in a pane** shows the session itself.
- **prompt all N**, in a project, types the same message into each of its sessions; the first send asks to confirm.
- `↑` `↓` pick a row, `Enter` goes into its field, `o` opens it in a pane, `a` / `d` allow or deny its permission, `n` goes to the next session that asks, `Esc` leaves the field, then goes back to **all**.

## The session bar

The bar at the top shows the sessions of the visible project as chips. Sessions that need you come first:

1. a question: a permission or a question of Claude Code on its screen, or a `kindric ask`
2. unread messages
3. a finished turn
4. waiting
5. working
6. paused
7. running in another terminal

| Mark | State |
| --- | --- |
| `?` (amber) | asks you something: answer it in the session or in the control center |
| filled dot | waiting for you |
| half dot | Claude is working |
| ✓ | finished a turn while you were not looking (never while it asks) |
| empty ring | paused (no process; opening it wakes it up) |
| dashed ring | interactive session in another terminal |

A chip shows the state, a short name and the unread count. Hovering it shows Claude Code's full name, the account, the directory, the id and the session's progress line.

The **progress line** is what a background session last said it is doing. Claude Code writes it in `jobs/<id>/state.json` (or, once the session stopped, the last such line of `jobs/<id>/timeline.jsonl`). These files are not documented, so the line may disappear with a Claude Code update. It also shows in the pane head and in `Ctrl+P`.

Chips that do not fit are counted in a **+N** chip that opens `Ctrl+P`. Errors and notices show at the right end of the bar.

Clicking a chip opens the session in the active pane of its project's tab, moving there if needed. Until the session first prints (about a quarter of a second, the time `claude attach` takes), the pane shows, dimmed, the last screen it had in this window at the same width; otherwise it says "connecting to <name>…".

## Panes

- `Ctrl+Shift+D` splits the active pane: side by side if it is wider than tall, one above the other otherwise.
- `Ctrl+Shift+W` closes the pane. The session keeps running.
- Separators can be dragged.
- The head of a pane names its session and its account, then its progress line, the number of files it changed (a click opens [Changed files](#changed-files)) and, while you are [away](#away), until when.
- When the last turn of a session cites image files (`.png`, `.jpg`, `.gif`, `.webp`), its pane shows them in a band, up to 6. kindric reads them in the session's transcript, a file Claude Code does not document: if its layout changes, the band stays empty.
- When Claude Code finishes a turn in a session you are not looking at, its chip gets a ✓ and its pane a green border and a **done** mark, until you look at it.
- `Ctrl+Shift+C` copies the selection, `Ctrl+Shift+V` pastes text. `Ctrl+C` and `Ctrl+V` still go to Claude Code, which interrupts with the first and pastes an image with the second.
- `Ctrl+Shift+M` shows the messages of the active pane's session. See [Messages](messages.md#in-the-desktop-app).

## Starting, naming and hiding sessions

**Start.** In a project tab, `Ctrl+N` lists the accounts, the one of the last session started there first. `Enter` or a click starts a session at once in the project's folder. A project tab with no session shows the same list. Its last line, "other folder or name…", opens the full form. Elsewhere, `Ctrl+N` opens the form directly. It asks for an account, a directory and an optional short name. The short name names the session in Claude Code itself.

**Directory fields** (`Ctrl+N`, `Ctrl+Shift+T`) first list the folders already in use, projects then sessions. Once you type, they list the folders that can follow (case ignored, hidden folders after a `.`). `Tab` completes, `Up` / `Down` choose, `Enter` takes the chosen one.

**Find.** `Ctrl+P` searches the sessions of the visible tab by title, short name, account or directory, hidden sessions included. They are listed by day of last activity, the most recent first, under the heads "today", "yesterday", then the date, each with its time. A session not used yet counts as today. Hidden sessions come last, under "hidden", each with its day.

**Name.** `F2` gives a short name to the session of the active pane, for this app only. An empty name removes it. Without one, the bar shows Claude Code's name, cut after the last whole word within 24 characters.

**Hide.** `Ctrl+Shift+K`, once confirmed, stops the session of the active pane (`claude stop`, its conversation is kept) and hides it from the bar. `Ctrl+P` still lists hidden sessions, last. Choosing one shows it again and starts it.

## Away

`Ctrl+Shift+Y` tells the session of the active pane that you are away for a number of minutes (60 by default, up to 1440). kindric types a message in your name into its pane, and until that time the session keeps working without waiting for your approval. It must not push, merge, delete, install or use another account: kindric-accounts' hook refuses those commands and answers the other questions for you (see [Away, in kindric](../crates/accounts/README.md#away-in-kindric)).

- The pane head shows "away until 14:30".
- `0` means you are back: the session may stop again.
- A session that has finished its task says so with `kindric away done <session id>` and stops.
- A session that cannot go on without you first has an agent review its work. Then it runs `kindric away blocked <session id> "<what it needs>"`, which puts what it needs in the control center, and stops. Once you answer, it works again, and it is kept working until you are back.
- A session told to keep working that then runs nothing stops too, its last words in the control center. It is relaunched once at most for nothing.
- Only sessions started after kindric was installed have the hook.

## Changed files

`Ctrl+Shift+G`, or a click on **N files** in a pane head, lists the files the session of the active pane changed. There is one block per working copy, in the order the session went into them: the folder it started in, then each worktree. Each is compared with its state when the session started.

- The head of a block says what it is compared with: since the start, since the branch was created, or estimated since the start (for a session started before kindric wrote down its starting state).
- In the folder it started in, commits count only when their git output is in the session's conversation. Commits of other sessions are left out.
- `↑` `↓` choose a file, `Enter` shows its diff, `Esc` goes back to the list, then closes.

kindric reads the folders in the session's transcript, a file Claude Code does not document: what it lacks gives fewer blocks, never an error.

## Worktrees

`Ctrl+Shift+B` lists the git worktrees of the visible tab's project: merged into the branch of the main checkout or not, how many commits ahead, the age of the last commit, whether the branch is also on `origin`.

`Delete`, then `Enter`, removes the folder of the worktree chosen. Its branch goes with it only when git sees it merged: after a squash merge, or when it is not merged, the branch stays. A branch on `origin` is never touched. kindric refuses to remove:

- the main checkout
- a worktree a session works in
- a worktree with uncommitted changes
- a locked one (`git worktree unlock` first)
- one whose folder is already gone (`git worktree prune` forgets it)

## Artifacts

Sessions keep the files they made for you (a page, an image, a PDF) with the `keep_artifact` tool of the [messages server](messages.md). kindric copies them into its data folder, by project.

`Ctrl+Shift+A` opens the artifacts of the visible tab's project: unsorted first, then one block per group. The files new since the last opening are marked **new**; the control center counts them too.

| Key | Effect |
| --- | --- |
| `Enter` | Open: an image shows in the window, anything else opens with the system's app |
| `a` | Add files from the system's picker, into the group chosen |
| `m` | Move the file to a group, or to a new one |
| `n` | New group |
| `r` | Rename the group |
| `Delete` | Delete the file, or remove the group (its files go back to unsorted), after `Enter` |

A file can also be dragged onto the head of a group.

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
| `Ctrl+Shift+G` | Files it changed |
| `Ctrl+Shift+Y` | You are away: it keeps working until then |
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
| `Ctrl+Shift+0` | Control center |
| **App** | |
| `Ctrl+Shift+A` | Artifacts of this project |
| `Ctrl+Shift+B` | Worktrees of this project |
| `Ctrl+,` | Settings: accounts, memory, sessions, shortcuts |
| `Ctrl+Shift+E` | Next theme |
| `Ctrl+Alt+E` | Next theme of this project, then kindric's again |
| `F1` | Setup checklist and these shortcuts |

Unlike the terminal interface, the number of a session changes with its state: `Alt+1` is always the session that needs you most.

**On macOS**, `Cmd` replaces `Ctrl` everywhere (`Cmd+N`, `Cmd+Shift+T`…), so all `Ctrl` keys go to the terminal. Copy and paste are `Cmd+C` and `Cmd+V`. Previous / next pane are `Cmd+[` and `Cmd+]`, since `Cmd+H` hides the app.

## Settings

`Ctrl+,` opens the settings with the cursor in a search field: type to find a setting in every category, or pick a category on the left (Up and Down from the search). Escape empties the search, then closes.

The accounts section lists the accounts of kindric-accounts: logged in or not, plan or where the model comes from, running sessions, isolated. **add** creates an account. Its **model from** says where: a Claude login (Claude Code's login then opens in a window of the app), an Anthropic API key, OpenRouter, Ollama on this computer (its model is picked among those installed) or another gateway (its address), or **codex**: the account runs OpenAI's Codex instead of Claude Code, and Codex's login opens in a window of the app. Each asks for what it needs: a model, a key. **set key** and **change key** give such an account its key, which kindric-accounts keeps (see [How it works](how-it-works.md)). **log in** opens the login of a Claude account that is logged out, or of a Codex account. **remove** takes an account off the list after you confirm; its folder stays on disk.

The memory section sets the size above which a session is warned about the memory it opens at startup: 30 KB by default. It is a warning, nothing is cut. The value is kept in `~/.config/kindric-accounts/memory-warning`. **open the memory atlas** draws the memory of kindric (`~/.kindric/memory`) in 3D: each project is a system around its status, each file a planet. A pale sun is a status older than the project's last decision, a red planet a lesson given again, a grey rock a file its `INDEX.md` does not name. Drag to turn, pinch or use the wheel to go in and out, type to search. A click goes one level down, from the whole memory to a project, a system, then a planet to read; `Esc` comes back up.

The sessions section purges the sessions idle for more than 31 or 7 days, hidden or not. kindric first says how many will go, by account, and asks to confirm. Each one is removed with `claude rm`: it leaves kindric, its conversation stays in `claude --resume`. A session that works or asks you something stays, and so does one whose last activity is unknown. When Claude Code refuses one, the reason it gives is shown under the button.

The shortcuts section changes the key of an action: press **change**, then the new keys. A key needs Ctrl or Alt (F1 to F12 alone are fine), and one already taken is refused with the name of its action. **reset** gives the action its own key back. Changed keys are kept in `settings.json`.

## Themes

`Ctrl+Shift+E` cycles through **ember** (the default), **paper** (light) and **graphite**. The choice is kept in `settings.json`.

A project can have a theme of its own: `Ctrl+Alt+E`, in its tab, goes through the themes, then back to kindric's. The panes of its sessions show in it, wherever they are open; the rest of the window keeps kindric's theme. The choice is kept in `projects.json`.

A `theme.json` next to the mailbox adds a **custom** theme: ember with the colors it names.

```json
{"background": "#101010", "active": "#c04020"}
```

Names are the color variables at the top of `crates/desktop/ui/style.css`, without `--`. Only `#rgb`, `#rrggbb` and `#rrggbbaa` values are taken. The file is read again each time the theme is chosen.

Claude Code draws for the background of the theme. kindric writes Claude Code themes in `~/.claude/themes`: `kindric.json`, and one per project (`kindric-` and 8 characters), light or dark after the terminal color of the theme they show in, in the variant you chose for Claude Code (daltonized, ANSI colors only). The sessions kindric starts use the theme of their project, else `kindric`, and Claude Code reloads it when the theme changes, without a restart. Sessions started before this version, or outside kindric, keep your own Claude Code theme.

## Files kept by the app

All in kindric's data folder, next to the mailbox: `~/.local/share/kindric/` on Linux and macOS (`$XDG_DATA_HOME/kindric/` if set), `%APPDATA%\kindric\` on Windows.

| File | Holds |
| --- | --- |
| `projects.json` | Projects, the account last used in each, and their theme |
| `workspace.json` | The tab you left and the session of each tab's active pane |
| `sessions.json` | Short names and hidden sessions |
| `settings.json` | The theme and the shortcuts you changed |
| `theme.json` | Your custom theme, if you write one |
| `artifacts/<project>/` | The files sessions kept (`user/`, `claude/`), and `origins.json`: the session that kept each one, followed when the artifacts window moves it |
| `starts/` | The state of each session's repository when it started, for [Changed files](#changed-files) |
| `overview/` | The questions sessions ask you in the control center, and your answers |
| `away/` | Until when you are away from each session |
| `wait/` | The latest wait of each session for a message that wakes it |
| `attached/` | One lock per session shown, so that two kindric windows never show the same one |
| `mailbox/` | The messages, see [Messages](messages.md#storage) |

## Signal

`kindric-desktop signal on` covers the edges of the screens with a light haze of the theme's colour, over every window, with a short message at the top: "Claude borrows a window, hands off for a second", in English, French, Spanish, German, Italian or Portuguese after the system's language. `--message "…"` shows another text. Clicks and keys go through it. `kindric-desktop signal off` removes it. A skill calls it before it takes a window away for a moment, so you know not to touch anything.

- `on` returns once the haze shows. Called again, it keeps the haze up.
- The haze goes by itself 10 seconds after the last `on`, so a skill that fails cannot leave it up.
- `off` with no haze shown does nothing. Both exit with 0; an error exits with 1.
- It needs Wayland with the layer shell (Hyprland, Sway, KDE). Elsewhere, `on` says it is not available and exits with 1. Virtual screens (`HEADLESS-…`) stay bare.

## Codex accounts

A Codex account (added in the settings with **model from** set to codex, or with `kindric-accounts add <name> --agent codex` then `kindric-accounts run <name> login`) shows its sessions next to those of Claude Code:

- In the bar, the control center and `Ctrl+P`, with their state. A session waiting for an approval carries the `?`: the control center shows the command or the change it asks for, with **Allow** and **Deny** (`a` and `d`); its pane asks too.
- In a pane: the Codex TUI joins the session, in its folder. Closing the pane leaves the session, and a turn it runs, going.
- `Ctrl+N` on a Codex account starts a Codex session in the folder.
- A message typed in the control center reaches the session as yours: Codex starts a turn with it as soon as the session is free.
- `Ctrl+Shift+K` stops the turn running and hides the session. The purge of the settings archives a Codex session: `codex unarchive <id>` brings it back.
- Its sessions send and read messages and keep files, like those of Claude Code: kindric-accounts gives the account kindric's MCP server. A message sent with `wake` waits for its next prompt or tool.

Not yet: answering a question of the model in the control center, away mode, its last steps, images, context and limits. How it works: [Codex accounts](how-it-works.md#codex-accounts).

## OpenCode accounts

An OpenCode account is added with `kindric-accounts add <name> --agent opencode`, then signed in with **log in** in the settings, which runs `kindric-accounts run <name> auth login`. Its card says `opencode`. Its sessions show next to the others:

- In the bar, the control center and `Ctrl+P`, with their state. A session waiting for a permission or for an answer to a question of the model carries the `?`, and the control center shows what it asks: a command with **Allow** and **Deny**, a question with its choices, as for Claude Code. Allow lets it run once. A form of another kind is answered in the pane. A session at rest is paused, as in OpenCode nothing runs for it.
- In a pane: the OpenCode TUI joins the session, in its folder. Closing the pane leaves the session, and a turn it runs, going.
- `Ctrl+N` on an OpenCode account starts an OpenCode session in the folder.
- A message typed in the control center reaches the session as yours: OpenCode starts a turn with it, or after the one running.
- `Ctrl+Shift+K` stops the turn running and hides the session. The purge of the settings archives an OpenCode session: OpenCode keeps it, archived, not deleted.
- Its sessions send and read messages and keep files: kindric-accounts gives the account kindric's MCP server. A message reaches a session at rest too, and wakes it up to read it.

The sessions of an account live in its OpenCode service. kindric starts it when it does not run, and leaves it running: about 370 MB of memory per OpenCode account. `kindric-accounts run <name> service stop` stops it, until kindric needs it again.

Not yet: away mode, its last steps, images, context and limits. How it works: [OpenCode accounts](how-it-works.md#opencode-accounts).
