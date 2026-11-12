//! Starting Claude Code on an account.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Result, fail};
use crate::injection;
use crate::isolation::{self, IsolatedSettings};
use crate::paths::Paths;
use crate::registry;
use crate::shared;
use crate::usage;

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
    find_in(path, &CLAUDE_NAMES)
}

/// The first executable of `names` found in `path` (a `PATH`-style list).
pub(crate) fn find_in(path: &OsString, names: &[&str]) -> Option<PathBuf> {
    std::env::split_paths(path)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
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

/// Arguments placed before the user's: the options of an isolated account, else `--settings`
/// with `settings` (the account's own file, see `usage`) or `~/.claude/settings.json` when it
/// exists, and `--mcp-config=<file>` (the `=` form: the option takes several values and would
/// swallow the next arguments otherwise). `conversation`: the launch opens one, so an isolated
/// account gets its rules of `global/`.
pub fn claude_args(
    paths: &Paths,
    isolated: Option<&IsolatedSettings>,
    settings: Option<&Path>,
    mcp_config: Option<&Path>,
    conversation: bool,
) -> Vec<OsString> {
    let mut args = Vec::new();
    let shared = paths.shared_root.join("settings.json");
    if let Some(isolated) = isolated {
        args.extend(isolated.args(conversation));
    } else if let Some(settings) = settings {
        args.push(OsString::from("--settings"));
        args.push(settings.as_os_str().to_os_string());
    } else if shared.is_file() {
        args.push(OsString::from("--settings"));
        args.push(shared.into_os_string());
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
/// directory may have appeared since `add`), cleans the environment, adds the args. With
/// `conversation_dir`, the directory a conversation starts in, the memory of its zone is added;
/// leave it out for a subcommand of Claude Code.
pub fn prepare(
    paths: &Paths,
    name: &str,
    mcp_config: Option<&Path>,
    user_args: &[OsString],
    conversation_dir: Option<&Path>,
) -> Result<Launch> {
    let dir = require_account(paths, name)?;
    let claude = require_claude()?;
    let mut warnings = shared::link_shared(&paths.shared_root, &dir)?;
    let isolated = isolation::prepare_settings(paths, name)?;
    if let Some(settings) = &isolated {
        warnings.extend(settings.warnings.iter().cloned());
    }
    // The status line that writes down how much of its limits the account used.
    let isolated_file = isolated.as_ref().map(|settings| settings.file.as_path());
    // kindric's hooks (unread messages, the user away); never for an isolated account (039).
    let hooks = if isolated.is_none() { usage::kindric_hooks() } else { usage::KindricHooks::default() };
    let settings = usage::settings_file(paths, name, isolated_file, usage::wrapper_command().as_deref(), &hooks, &mut warnings);
    let args = claude_args(paths, isolated.as_ref(), settings.as_deref(), mcp_config, conversation_dir.is_some());
    let memory = conversation_dir.map(|directory| injection::for_session(paths, name, directory)).unwrap_or_default();
    warnings.extend(memory.warnings);
    let mut command = Command::new(claude);
    for variable in STRIPPED_ENV {
        command.env_remove(variable);
    }
    command.env("CLAUDE_CONFIG_DIR", &dir).args(args).args(memory.args).args(user_args);
    Ok(Launch { command, warnings })
}

/// Ctrl+C reaches every process attached to the console. While waiting for Claude Code,
/// kindric-accounts handles it by doing nothing, so only Claude Code reacts. A handler is
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
        assert!(claude_args(&paths, None, None, None, true).is_empty());
        fs::create_dir_all(&paths.shared_root).unwrap();
        fs::write(paths.shared_root.join("settings.json"), "{}").unwrap();
        let args = claude_args(&paths, None, None, Some(Path::new("/x/work.json")), true);
        let settings = paths.shared_root.join("settings.json").into_os_string();
        assert_eq!(args, [OsString::from("--settings"), settings, OsString::from("--mcp-config=/x/work.json")]);
        // The account's own file, with the status line that follows its usage, comes instead.
        let own = claude_args(&paths, None, Some(Path::new("/x/own.json")), None, true);
        assert_eq!(own, [OsString::from("--settings"), OsString::from("/x/own.json")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_settings_of_an_isolated_account_replace_the_shared_ones() {
        let root = test_dir::fresh("isolated-args");
        let paths = Paths::from_roots(root.join("c"), root.join("d"), root.clone());
        fs::create_dir_all(&paths.shared_root).unwrap();
        fs::write(paths.shared_root.join("settings.json"), "{}").unwrap();
        let isolated = IsolatedSettings {
            file: PathBuf::from("/x/isolated.json"),
            global_rules: Some(PathBuf::from("/x/isolated.md")),
            warnings: Vec::new(),
        };
        let args = claude_args(&paths, Some(&isolated), None, None, true);
        assert_eq!(args, isolated.args(true));
        assert_eq!(claude_args(&paths, Some(&isolated), None, None, false), isolated.args(false));
        assert!(!args.contains(&paths.shared_root.join("settings.json").into_os_string()));
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
