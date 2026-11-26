# Installing kindric

One command installs the three programs of kindric:

| Program | What it is |
| --- | --- |
| `kindric-desktop` | The app |
| `kindric` | The messages server and the hooks of the sessions, and the terminal interface |
| `kindric-accounts` | The account manager: each account with its own login |

The app needs the other two on your `PATH`: its panes start sessions through `kindric-accounts`, and the sessions call `kindric`. The installer puts them there.

## Before you start

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code), with background sessions (tested with 2.1.332). Codex (0.172.0) and OpenCode (2.4.1) are needed only for their own accounts.
- Linux (x86_64 or arm64, with glibc 2.35 or newer: not Alpine), macOS (Intel or Apple) or Windows 10 and 11 (x86_64).

## Linux and macOS

```sh
curl -fsSL https://github.com/LouisStani/kindric/releases/latest/download/install.sh | sh
```

It downloads the release for your system, checks it against the release's `SHA256SUMS`, and puts the programs in `~/.local/bin`, where Claude Code's own installer puts `claude`. Then:

- **On Linux**, kindric gets an entry in your applications menu. Opened from there, it takes the `PATH` of your shell, so it finds `claude` as a terminal would. The app needs WebKitGTK 4.1: if it is missing, the installer gives the command for your distribution, for example `sudo apt install libwebkit2gtk-4.1-0` on Debian and Ubuntu, or `sudo pacman -S webkit2gtk-4.1` on Arch.
- **On macOS**, kindric goes to `~/Applications/kindric.app`, so it shows in Launchpad and the Dock. Opened from there, it takes the `PATH` of your shell, so it finds `claude` as a terminal would.

If `~/.local/bin` is not on your `PATH`, the installer says so and gives the line to add to your shell's startup file.

## Windows

In PowerShell:

```powershell
irm https://raw.githubusercontent.com/LouisStani/kindric/main/packaging/install.ps1 | iex
```

It downloads the release, checks it against `SHA256SUMS`, puts the programs in `%LOCALAPPDATA%\Programs\kindric`, adds that folder to your user `PATH` and adds kindric to the Start menu. Open a new terminal for the `PATH` to apply.

The app draws with Microsoft Edge WebView2, which Windows 10 and 11 usually have. If it is missing, the installer says where to get it.

Windows support passes the automated tests on every change, but it has not been used day to day yet. Please report any problem.

## First launch

Open kindric from the menu, or run `kindric-desktop`. Until everything is set up, the app lists what is left, with the command for each step: an account, the messages server (a button adds it for you), a first session. `F1` shows that list again.

## Update

Run the same command again. It replaces the programs and leaves your accounts and settings alone. A kindric already running keeps the old programs until it restarts.

To install a given version instead of the latest (the installers came after 1.0.0, so 1.0.0 and older cannot be installed this way):

```sh
curl -fsSL https://github.com/LouisStani/kindric/releases/latest/download/install.sh | sh -s -- --version <x.y.z>
```

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/LouisStani/kindric/main/packaging/install.ps1))) -Version <x.y.z>
```

The sessions already running keep the hooks they started with. Those started after the update get the new ones.

## Uninstall

```sh
curl -fsSL https://github.com/LouisStani/kindric/releases/latest/download/install.sh | sh -s -- --uninstall
```

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/LouisStani/kindric/main/packaging/install.ps1))) -Uninstall
```

This removes what the installer put there: the programs, the menu entry and, on Windows, the `PATH` entry. It keeps your data, and never touches Claude Code. To remove the data too, delete these folders once kindric is closed:

| What | Linux and macOS | Windows |
| --- | --- | --- |
| kindric: projects, messages, artifacts, stats | `~/.local/share/kindric` | `%APPDATA%\kindric` |
| kindric-accounts: your accounts and their logins | `~/.config/kindric-accounts`, `~/.local/share/kindric-accounts` | `%APPDATA%\kindric-accounts` |

On Linux and macOS, `XDG_CONFIG_HOME` and `XDG_DATA_HOME` replace `~/.config` and `~/.local/share` when you set them. Deleting the folder of kindric-accounts logs every account out for good.

## If something goes wrong

- **`kindric-desktop: command not found`.** The install folder is not on your `PATH`: add the line the installer gave, then open a new terminal.
- **The installer warns that another `kindric` comes first.** An older copy, often from `cargo install`, runs instead of the new one. Remove it with `cargo uninstall kindric kindric-desktop kindric-accounts`.
- **macOS says kindric cannot be opened.** That happens when the app was copied from a download made by a browser. Run `xattr -dr com.apple.quarantine ~/Applications/kindric.app`, or install with the command above, which does not have the problem.
- **No menu entry on Linux.** A `kindric.desktop` that the installer did not make was already there: it is left alone, and the installer says so.
- **The installer says a file was not installed by it.** A `kindric`, `kindric-accounts` or `kindric-desktop` it did not put there is in the install folder, a link to a build of your own for example. It never replaces one, except a kindric program whose `--version` says so. Move it away, then run the installer again.

## From source

With [Rust](https://rustup.rs) installed (and WebKitGTK 4.1 with its development files on Linux: `libwebkit2gtk-4.1-dev` on Debian and Ubuntu, `webkit2gtk-4.1` on Arch):

```sh
git clone https://github.com/LouisStani/kindric.git
cargo install --locked --path kindric/crates/accounts
cargo install --locked --path kindric/crates/tui
cargo install --locked --path kindric/crates/desktop
```

The programs go to `~/.cargo/bin`, which must be on your `PATH`. Use one way or the other, not both: two copies on the `PATH` run whichever comes first.
