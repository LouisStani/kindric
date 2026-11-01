//! Starting Claude Code on an account.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Result, fail};
use crate::paths::Paths;
use crate::registry;
use crate::shared;

/// Removed before starting Claude Code: a key or token exported in the shell must
/// never override the account the user picked, nor bill another one.
pub const STRIPPED_ENV: [&str; 3] = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN"];

#[cfg(windows)]
const CLAUDE_NAMES: [&str; 3] = ["claude.exe", "claude.cmd", "claude"];
#[cfg(not(windows))]
const CLAUDE_NAMES: [&str; 1] = ["claude"];

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// The `claude` executable found in `path` (a `PATH`-style list), like `command -v`.
pub fn find_claude_in(path: &OsString) -> Option<PathBuf> {
    std::env::split_paths(path)
        .flat_map(|dir| CLAUDE_NAMES.iter().map(move |name| dir.join(name)))
        .find(|candidate| is_executable(candidate))
}

pub fn require_claude() -> Result<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    match find_claude_in(&path) {
        Some(claude) => Ok(claude),
        None => fail("claude not found in PATH"),
    }
}

/// The account must be listed and its directory must exist.
pub fn require_account(paths: &Paths, name: &str) -> Result<PathBuf> {
    registry::require_existing_account(paths, name)?;
    let dir = paths.account_dir(name);
    if !dir.is_dir() {
        return fail(format!("missing directory for {name}: {}", dir.display()));
    }
    Ok(dir)
}

/// Arguments placed before the user's: `--settings` when `~/.claude/settings.json`
/// exists, and `--mcp-config=<file>` (the `=` form: the option takes several values
/// and would swallow the next arguments otherwise).
pub fn claude_args(paths: &Paths, mcp_config: Option<&Path>) -> Vec<OsString> {
    let mut args = Vec::new();
    let settings = paths.shared_root.join("settings.json");
    if settings.is_file() {
        args.push(OsString::from("--settings"));
        args.push(settings.into_os_string());
    }
    if let Some(file) = mcp_config {
        let mut option = OsString::from("--mcp-config=");
        option.push(file);
        args.push(option);
    }
    args
}

pub struct Launch {
    pub command: Command,
    pub warnings: Vec<String>,
}

/// Prepares `claude` for `name`: links the shared configuration again (a shared
/// directory may have appeared since `add`), cleans the environment, adds the args.
pub fn prepare(paths: &Paths, name: &str, mcp_config: Option<&Path>, user_args: &[OsString]) -> Result<Launch> {
    let dir = require_account(paths, name)?;
    let claude = require_claude()?;
    let warnings = shared::link_shared(&paths.shared_root, &dir)?;
    let mut command = Command::new(claude);
    for variable in STRIPPED_ENV {
        command.env_remove(variable);
    }
    command.env("CLAUDE_CONFIG_DIR", &dir).args(claude_args(paths, mcp_config)).args(user_args);
    Ok(Launch { command, warnings })
}

/// Ctrl+C reaches every process attached to the console. While waiting for Claude Code,
/// claude-accounts handles it by doing nothing, so only Claude Code reacts. A handler is
/// used rather than ignoring the signal, because ignoring it would be inherited.
#[cfg(windows)]
fn survive_ctrl_c() {
    use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
    unsafe extern "system" fn handler(kind: u32) -> windows_sys::core::BOOL {
        windows_sys::core::BOOL::from(kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT)
    }
    // SAFETY: registers a handler that only reads its argument.
    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

/// Windows has no exec: starts Claude Code, waits, and returns its exit code.
#[cfg(windows)]
pub fn run_and_wait(mut command: Command) -> Result<i32> {
    survive_ctrl_c();
    let status = command.status().map_err(|error| crate::error::Error(format!("cannot start claude: {error}")))?;
    Ok(status.code().unwrap_or(1))
}

/// Replaces this process with Claude Code; returns only if that failed.
#[cfg(unix)]
pub fn exec(mut command: Command) -> crate::error::Error {
    use std::os::unix::process::CommandExt;
    crate::error::Error(format!("cannot start claude: {}", command.exec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::test_dir;
    use std::fs;

    #[test]
    fn settings_and_mcp_config_come_first_in_the_equals_form() {
        let root = test_dir::fresh("args");
        let paths = Paths::from_roots(root.join("c"), root.join("d"), root.clone());
        assert!(claude_args(&paths, None).is_empty());
        fs::create_dir_all(&paths.shared_root).unwrap();
        fs::write(paths.shared_root.join("settings.json"), "{}").unwrap();
        let args = claude_args(&paths, Some(Path::new("/x/work.json")));
        let settings = paths.shared_root.join("settings.json").into_os_string();
        assert_eq!(args, [OsString::from("--settings"), settings, OsString::from("--mcp-config=/x/work.json")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn claude_is_found_only_when_executable() {
        use std::os::unix::fs::PermissionsExt;
        let root = test_dir::fresh("find");
        let (empty, bin) = (root.join("empty"), root.join("bin"));
        fs::create_dir_all(&empty).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("claude"), "#!/bin/sh\n").unwrap();
        let path = std::env::join_paths([&empty, &bin]).unwrap();
        assert_eq!(find_claude_in(&path), None);
        fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_claude_in(&path), Some(bin.join("claude")));
        fs::remove_dir_all(root).unwrap();
    }
}
