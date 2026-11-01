//! Starts a background Claude Code session for an account, in a directory.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf, is_separator};
use std::process::{Command, Stdio};

use claude_accounts::{Paths, launch, mcp};

use crate::accounts;

#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub account: String,
    pub directory: PathBuf,
    pub name: String,
}

pub fn expand_home(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(path),
    }
}

/// The directory typed in a new-session form, `~` expanded; refused if it does not exist.
pub fn resolve_directory(text: &str, home: &Path) -> Result<PathBuf, String> {
    let directory = expand_home(text.trim(), home);
    if !directory.is_dir() {
        return Err(format!("no such directory: {}", directory.display()));
    }
    Ok(directory)
}

/// `path` as a person reads it: `~` for the home directory, which `expand_home` reads back.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Completions never list more than this.
const COMPLETIONS: usize = 20;

/// The folders that can follow `text` in a directory field: `~/Dev/Pr` lists the folders of
/// `~/Dev` whose name starts with `Pr`, case ignored, hidden ones only after a `.`. Each comes
/// back as typed plus its name and a separator, so choosing it lists the next level.
pub fn complete_directory(text: &str, home: &Path) -> Vec<String> {
    let text = text.trim_start();
    let (parent, start) = match text.rfind(is_separator) {
        Some(index) => (&text[..=index], &text[index + 1..]),
        None if text == "~" => ("~/", ""),
        None => return Vec::new(),
    };
    let Ok(entries) = fs::read_dir(expand_home(parent, home)) else {
        return Vec::new();
    };
    let start = start.to_lowercase();
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| (start.starts_with('.') || !name.starts_with('.')) && name.to_lowercase().starts_with(&start))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.truncate(COMPLETIONS);
    names.into_iter().map(|name| format!("{parent}{name}{}", std::path::MAIN_SEPARATOR)).collect()
}

/// Reads the short id from `backgrounded · 378ce649 · name (…)`.
pub fn parse_backgrounded_id(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("backgrounded · ")?;
        let id = rest.split_whitespace().next()?;
        id.chars().all(|c| c.is_ascii_alphanumeric()).then(|| id.to_string())
    })
}

pub struct Launched {
    pub short_id: String,
    pub warnings: Vec<String>,
}

fn library_error(error: claude_accounts::Error) -> String {
    error.to_string()
}

fn background_args(name: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["--bg".into()];
    if !name.is_empty() {
        args.push("-n".into());
        args.push(name.into());
    }
    args
}

/// Runs `claude --bg …` in `directory` and reads the short id it prints.
fn run_backgrounded(mut command: Command, directory: &Path) -> Result<String, String> {
    let output =
        command.current_dir(directory).stdin(Stdio::null()).output().map_err(|error| format!("claude: {error}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        return Err(format!("cannot start the session: {}", text.trim()));
    }
    parse_backgrounded_id(&text).ok_or_else(|| format!("no session id in: {}", text.trim()))
}

/// Starts a background session the way `claude-accounts run <account> --bg` does: shared
/// MCP servers (none for an isolated account), shared settings, billing variables removed.
pub fn launch(request: &Request) -> Result<Launched, String> {
    let paths = Paths::from_env().map_err(library_error)?;
    if accounts::is_implicit(&paths, &request.account).map_err(|error| error.to_string())? {
        return launch_default(request);
    }
    launch::require_account(&paths, &request.account).map_err(library_error)?;
    launch::require_claude().map_err(library_error)?;
    let shared = mcp::prepare(&paths, &request.account).map_err(library_error)?;
    let args = background_args(&request.name);
    let prepared =
        launch::prepare(&paths, &request.account, shared.file.as_deref(), &args).map_err(library_error)?;
    let short_id = run_backgrounded(prepared.command, &request.directory)?;
    let warnings = shared.warnings.into_iter().chain(prepared.warnings).collect();
    Ok(Launched { short_id, warnings })
}

/// `claude --bg` on the implicit `default`: Claude Code reads its own settings and MCP
/// servers there, so only the environment is cleaned.
fn launch_default(request: &Request) -> Result<Launched, String> {
    let claude = launch::require_claude().map_err(library_error)?;
    let mut command = Command::new(claude);
    for variable in accounts::default_removed_env() {
        command.env_remove(variable);
    }
    command.args(background_args(&request.name));
    let short_id = run_backgrounded(command, &request.directory)?;
    Ok(Launched { short_id, warnings: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_is_named_only_when_a_name_is_given() {
        let args = |name: &str| background_args(name).into_iter().map(|arg| arg.into_string().unwrap()).collect::<Vec<_>>();
        assert_eq!(args(""), ["--bg"]);
        assert_eq!(args("api"), ["--bg", "-n", "api"]);
    }

    #[test]
    fn backgrounded_id_is_read_from_the_first_line() {
        let output = "backgrounded · 378ce649 · proto-bg-test (idle — send a prompt to start)\n  claude agents  list";
        assert_eq!(parse_backgrounded_id(output), Some("378ce649".into()));
        assert_eq!(parse_backgrounded_id("error: nope"), None);
        assert_eq!(parse_backgrounded_id("backgrounded · ../x · y"), None);
    }

    #[test]
    fn home_is_expanded_only_at_the_start() {
        let home = Path::new("/home/u");
        assert_eq!(expand_home("~/Dev", home), PathBuf::from("/home/u/Dev"));
        assert_eq!(expand_home("~", home), PathBuf::from("/home/u"));
        assert_eq!(expand_home("~other/x", home), PathBuf::from("~other/x"));
        assert_eq!(expand_home("/tmp", home), PathBuf::from("/tmp"));
    }

    #[test]
    fn tilde_shortens_the_home_directory_only() {
        let home = Path::new("/home/u");
        assert_eq!(tilde(Path::new("/home/u"), home), "~");
        assert_eq!(tilde(Path::new("/home/u/Dev/api"), home), format!("~/{}", Path::new("Dev/api").display()));
        assert_eq!(tilde(Path::new("/home/user2"), home), Path::new("/home/user2").display().to_string());
    }

    /// `a/`, `Alpha/`, `b/`, `.cache/` and a file `a.txt`, in a fresh folder.
    fn tree(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("claude-sessions-complete-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for folder in ["a", "Alpha", "b", ".cache"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        fs::write(root.join("a.txt"), b"").unwrap();
        root
    }

    /// The folder names offered, whatever separators the typed text used.
    fn completed(text: &str, home: &Path) -> Vec<String> {
        let name = |path: &str| path.trim_end_matches(is_separator).rsplit(is_separator).next().unwrap_or_default().to_string();
        complete_directory(text, home).iter().map(|path| name(path)).collect()
    }

    #[test]
    fn a_folder_lists_its_visible_folders_in_order() {
        let root = tree("list");
        let typed = format!("{}{}", root.display(), std::path::MAIN_SEPARATOR);
        assert_eq!(completed(&typed, &root), ["a", "Alpha", "b"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_start_of_a_name_is_matched_case_ignored_and_files_are_left_out() {
        let root = tree("start");
        let typed = format!("{}{}A", root.display(), std::path::MAIN_SEPARATOR);
        assert_eq!(completed(&typed, &root), ["a", "Alpha"]);
        assert!(complete_directory(&typed, &root).iter().all(|path| path.starts_with(&typed[..typed.len() - 1])));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hidden_folders_need_a_dot() {
        let root = tree("hidden");
        let typed = format!("{}{}.c", root.display(), std::path::MAIN_SEPARATOR);
        assert_eq!(completed(&typed, &root), [".cache"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn home_is_completed_as_typed() {
        let root = tree("home");
        assert_eq!(complete_directory("~/al", &root), [format!("~/Alpha{}", std::path::MAIN_SEPARATOR)]);
        assert_eq!(completed("~", &root), ["a", "Alpha", "b"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nothing_to_list_gives_nothing() {
        let root = tree("nothing");
        assert!(complete_directory("~/no-such-folder/x", &root).is_empty());
        assert!(complete_directory("Dev", &root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn typed_directory_is_expanded_and_must_exist() {
        let home = std::env::temp_dir();
        assert_eq!(resolve_directory(" ~ ", &home), Ok(home.clone()));
        let missing = resolve_directory("~/no-such-dir-claude-sessions", &home);
        assert_eq!(missing, Err(format!("no such directory: {}", home.join("no-such-dir-claude-sessions").display())));
    }
}
