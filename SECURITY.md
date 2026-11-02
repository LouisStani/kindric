# Reporting a vulnerability

Do not open a public issue if the problem could expose an OAuth token, API key, Claude session, messages between sessions, or a user’s configuration paths.

Contact the repository maintainer privately first, including:

- a description of the issue;
- the affected versions (`kindric-accounts --version`, `kindric --version`), operating system and Claude Code version;
- reproduction steps without real credentials;
- the expected impact.

Never attach the contents of `~/.claude`, `~/.local/share/claude-accounts`, `~/.local/share/kindric`, `~/.config/claude-accounts`, or `%APPDATA%\claude-accounts` and `%APPDATA%\kindric` on Windows.
