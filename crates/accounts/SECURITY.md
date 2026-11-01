# Reporting a vulnerability

Do not open a public issue if the problem could expose an OAuth token, API key, Claude session, or a user’s configuration paths.

Contact the repository maintainer privately first, including:

- a description of the issue;
- the affected claude-accounts version (`claude-accounts --version`), operating system and Claude Code version;
- reproduction steps without real credentials;
- the expected impact.

Never attach the contents of `~/.claude`, `~/.local/share/claude-accounts`, `~/.config/claude-accounts`, or `%APPDATA%\claude-accounts` on Windows.
