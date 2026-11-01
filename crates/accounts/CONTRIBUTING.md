# Contributing to claude-accounts

Thank you for helping improve the project.

## Before opening a contribution

- Never add an OAuth token, API key, Claude session, or content from `~/.claude` to the repository.
- Keep `add`, `list`, `status`, `remove`, `run`, `isolate`, `unisolate` and `isolated` suitable for scripting: their text, JSON and exit codes are a contract (see Scripting in the README).
- Keep Linux, macOS and Windows working. Code for one system goes behind `#[cfg(...)]`.
- Never overwrite a file or directory that claude-accounts did not create.
- Document every change that affects configuration paths, permissions, or account management.

## Local verification

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release && ./tests.sh
```

`tests.sh` checks the command-line contract on Linux. The tests use a temporary environment and do not touch the machine's Claude accounts. To check that the Windows code compiles from Linux:

```sh
rustup target add x86_64-pc-windows-gnu
cargo clippy --target x86_64-pc-windows-gnu --all-targets
```

## Style

- One responsibility per module and per function.
- Explicit error handling: no `unwrap` outside tests, actionable error messages.
- No global state.
