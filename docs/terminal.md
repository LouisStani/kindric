# Terminal interface

`kindric` runs in a terminal, for machines without a display (over SSH, say). It shows the session list and two panes side by side.

It is frozen: it gets no new features, but it is still built and tested. Its binary also runs the MCP server for [messages](messages.md).

```text
┌ sessions ─────────────┐┌ roblox · louis ─────────────┐┌ api · lystriq ──────────────┐
│1 roblox               ││                             ││                             │
│  louis · working      ││  Claude Code                ││  Claude Code                │
│2 api 1 msg            ││                             ││                             │
│  lystriq · waiting    ││ ❯                           ││ ❯                           │
└ Ctrl+P · Ctrl+N ──────┘└─────────────────────────────┘└─────────────────────────────┘
```

The terminal interface leaves out the `default` account (plain `~/.claude`).

## Keys

| Key | Effect |
| --- | --- |
| `Alt+1` … `Alt+9` | Show session 1 to 9 of the list in the active pane |
| `Ctrl+P` | Find a session by title, account or directory. Its last entry redraws the screen. |
| `Ctrl+N` | Start a new background session: account, directory, optional name |
| `Ctrl+H` / `Ctrl+L` | Make the left or right pane active |
| `Ctrl+Q` | Quit. The sessions keep running. |

Every other key goes to the session in the active pane.

kindric keeps these five keys for itself. Claude Code uses `Ctrl+N` and `Ctrl+P` like the arrow keys, which still work. It uses `Ctrl+L` to redraw its screen: use the redraw entry of `Ctrl+P` instead.

## Session states

| State | Meaning |
| --- | --- |
| `working` | Claude is answering |
| `waiting` | Claude waits for you |
| `paused` | Background session with no process. Opening it wakes it up. |
| `elsewhere` | Interactive session running in another terminal. It can receive messages but cannot be shown here. |

`isolated` after the state marks a session of an isolated account. `2 msg` after a title means two unread messages.
