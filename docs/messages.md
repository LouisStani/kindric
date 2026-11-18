# Messages between sessions

kindric's MCP server lets sessions leave each other short messages, across accounts, and keep the files they made. To set it up, see [Get started](../README.md#get-started).

## The tools

The server gives every session five tools:

| Tool | Effect |
| --- | --- |
| `list_sessions` | List the sessions of all accounts: title, account, state, id, directory |
| `send_message` | Leave a message for another session, by title or id. `wake` asks it to act now |
| `read_messages` | Return the unread messages of this session, once |
| `keep_artifact` | Keep a file it made, for you (`for=user`) or for later sessions of the project (`for=claude`), in a group if asked |
| `list_artifacts` | List the files kept for this project, with their paths |

## How a session learns of a message

A session reads its messages only when it calls `read_messages`. kindric does not type anything into it, but the sessions kindric-accounts starts get hooks that tell them:

- Before each prompt and after each tool, `kindric unread` tells the session, once per new message, that a message is waiting and who sent it.
- A message sent with `wake` also wakes a session at rest: after each turn, and each time the session starts, `kindric wait-message` waits in the background for such a message. A session at rest for more than a day is not woken until its next turn.

Details are in [Messages of kindric](../crates/accounts/README.md#messages-of-kindric), in the README of kindric-accounts. Sessions started some other way, or before kindric was installed, only see a message when they call `read_messages`.

## Information or request

Without `wake`, a message is information. The tools tell Claude to treat it as such, and to ask you before acting on a request it contains. In testing, a session that received "please create this file" did not create it and asked the user first.

With `wake`, the other session must act now: it wakes up and does what the message asks without waiting for you. The tools tell Claude to keep `wake` for that case, and never for answers, acknowledgements or information. Sending a message still asks for your permission, as any tool does; a woken session may read it without asking.

## Artifacts

`keep_artifact` copies the file into kindric's data folder, in `artifacts/<project>/user/` or `artifacts/<project>/claude/`, under its own name. A file of the same name already there gets a number (`name-2.png`). The desktop app shows those kept for you in its [artifacts window](desktop.md#artifacts), and the control center counts the new ones.

`list_artifacts` gives a later session the files kept for Claude in this project by default, or those kept for you.

At the end of a turn that made pages or images and kept none, kindric-accounts' hook asks the session once to keep those made for you. See [Artifacts, in kindric](../crates/accounts/README.md#artifacts-in-kindric).

## Isolated accounts

An isolated account (`kindric-accounts isolate <account>`, for a work account for example) stays out of messages and artifacts:

- its sessions never appear in `list_sessions`
- they cannot receive messages
- the MCP server refuses to start in them, so they have none of the five tools

They can still be shown in a pane. `kindric-accounts isolate default` does the same for plain `~/.claude`.

## In the desktop app

`Ctrl+Shift+M` shows the messages the session of the active pane received and sent, oldest first, read or not. A click on the message count of a chip does the same for that session.

This view only reads: nothing is marked read, so the session still gets its messages from `read_messages`.

## Storage

Each message is one JSON file in `mailbox/<session id>/`, inside kindric's data folder: `~/.local/share/kindric/` on Linux and macOS (`$XDG_DATA_HOME/kindric/` if set), `%APPDATA%\kindric\` on Windows. The directories are readable only by you. A message that has been read moves to `read/`.
