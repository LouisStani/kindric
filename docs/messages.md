# Messages between sessions

kindric's MCP server lets sessions leave each other short messages, across accounts. To set it up, see [Get started](../README.md#get-started).

## The tools

The server gives every session three tools:

| Tool | Effect |
| --- | --- |
| `list_sessions` | List the sessions of all accounts: title, account, state, id, directory |
| `send_message` | Leave a message for another session, by title or id |
| `read_messages` | Return the unread messages of this session, once |

A session reads its messages only when it calls `read_messages`, for example when you ask it to. Nothing is typed into a session for you.

## Messages are information, not orders

The tools tell Claude to treat messages as information, and to ask you before acting on a request they contain. In testing, a session that received "please create this file" did not create it and asked the user first.

## Isolated accounts

An isolated account (`kindric-accounts isolate <account>`, for a work account for example) stays out of messages:

- its sessions never appear in `list_sessions`
- they cannot receive messages
- the MCP server refuses to start in them

They can still be shown in a pane. `kindric-accounts isolate default` does the same for plain `~/.claude`.

## In the desktop app

`Ctrl+Shift+M` shows the messages the session of the active pane received and sent, oldest first, read or not. A click on the message count of a chip does the same for that session.

This view only reads: nothing is marked read, so the session still gets its messages from `read_messages`.

## Storage

Each message is one JSON file in `mailbox/<session id>/`, inside kindric's data folder: `~/.local/share/kindric/` on Linux and macOS (`$XDG_DATA_HOME/kindric/` if set), `%APPDATA%\kindric\` on Windows. The directories are readable only by you. A message that has been read moves to `read/`.
