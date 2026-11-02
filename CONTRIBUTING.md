# Contributing

Thank you for helping improve the project. This repository holds two tools: claude-accounts (`crates/accounts`) and kindric (`crates/core`, `crates/tui`, `crates/desktop`).

## Before opening a contribution

- Never add an OAuth token, API key, Claude session, or content from `~/.claude` to the repository.
- Keep claude-accounts' `add`, `list`, `status`, `remove`, `run`, `isolate`, `unisolate` and `isolated` suitable for scripting: their text, JSON and exit codes are a contract (see Scripting in [its README](crates/accounts/README.md)).
- Keep Linux, macOS and Windows working. Code for one system goes behind `#[cfg(...)]`.
- Never overwrite a file or directory that the tools did not create.
- Document every change that affects configuration paths, permissions, or account management.

## Local verification

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p claude-accounts && crates/accounts/tests.sh
```

`tests.sh` checks claude-accounts' command-line contract on Linux. The tests use a temporary environment and do not touch the machine's Claude accounts. To check that the Windows code compiles from Linux:

```sh
rustup target add x86_64-pc-windows-gnu
cargo clippy -p claude-accounts -p kindric-core -p kindric --target x86_64-pc-windows-gnu --all-targets
```

The desktop app cannot be checked this way: on Windows, Tauri needs the MinGW resource compiler. The CI checks it on Windows.

## Style

- One responsibility per module and per function.
- Explicit error handling: no `unwrap` outside tests, actionable error messages.
- No global state.
