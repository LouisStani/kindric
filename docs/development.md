# Development

Read [CONTRIBUTING.md](../CONTRIBUTING.md) first.

## Workspace

| Crate | What it is |
| --- | --- |
| `crates/accounts` | kindric-accounts: a library and a binary, with its own version and [README](../crates/accounts/README.md) |
| `crates/core` | Accounts, sessions, messages and refresh, shared by every interface |
| `crates/tui` | The terminal interface, the `kindric` binary |
| `crates/desktop` | The desktop interface, built with [Tauri](https://tauri.app) |

## Build and test

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p kindric-desktop
```

On Linux the desktop interface needs WebKitGTK 4.1 (`webkit2gtk-4.1` on Arch, `libwebkit2gtk-4.1-dev` on Debian and Ubuntu).

The tests never start Claude Code and never touch your accounts.

### End-to-end journeys

`tests/e2e/run.sh` runs the desktop app in a container (podman, rootless), on a screen that does not exist, driven through WebDriver, with a fake `claude` (`tests/e2e/bin/claude`). Each journey starts from nothing and leaves a screenshot in `/tmp/kindric-e2e-out`. The first run builds the image (several minutes); later runs compile only what changed. Pass a journey name to run only it (`tests/e2e/run.sh SessionInTheBar`), `--rebuild` to build the image again. They are not part of CI.

## CI and releases

- At every push, CI runs clippy and the tests on Linux, macOS and Windows.
- It also runs kindric-accounts' command-line tests (`crates/accounts/tests.sh`, Linux) and smoke tests (macOS, Windows).
- A `v*` tag publishes the kindric-accounts binaries for five targets in the release.

## The desktop page

The page in `crates/desktop/ui/` has no build step.

- [xterm.js](https://xtermjs.org) 6.0.0 and its fit (0.11.0), WebGL (0.19.0) and Unicode 11 (0.9.0) addons are copied from npm into `ui/vendor/`, each with its MIT license. Unicode 11 gives emojis two cells, as Claude Code counts them in its tables.
- The font, [JetBrains Mono](https://www.jetbrains.com/lp/mono/) 2.304 (regular and bold, from the official release), sits next to them with its OFL license, so the app looks the same on every system.
- Colors and font are CSS variables at the top of `style.css`. Themes only replace those variables.

## Tracing delays

Start the desktop interface with `KINDRIC_TRACE=1` to find where a delay comes from. It writes `trace.log` in kindric's data folder, started over at each launch.

Each line is one timing: time since launch, what, how long. It covers every command, the first output of an attached session, `claude agents --json` per account, each published list, and in the page each shortcut, chip click, render and first output shown.

## Icon

The icon is drawn in `crates/desktop/icons/icon.svg`. After changing it, regenerate the PNG and the Windows ICO (needs librsvg and ImageMagick):

```sh
cd crates/desktop/icons
rsvg-convert -w 256 -h 256 icon.svg -o icon.png
for s in 16 24 32 48 64 256; do rsvg-convert -w $s -h $s icon.svg -o /tmp/icon-$s.png; done
magick /tmp/icon-{16,24,32,48,64,256}.png icon.ico
```
