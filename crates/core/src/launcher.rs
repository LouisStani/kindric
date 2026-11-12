//! Starts a background Claude Code session for an account, in a directory.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf, is_separator};
use std::process::{Command, Stdio};

use kindric_accounts::injection;
use kindric_accounts::memory::Memory;
use kindric_accounts::paths::expand_home;
use kindric_accounts::{Paths, isolation, launch, mcp, registry};

use crate::accounts;

#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub account: String,
    pub directory: PathBuf,
    pub name: String,
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

fn library_error(error: kindric_accounts::Error) -> String {
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

/// Whether `error`, from `launch`, is Claude Code refusing a folder the account has not trusted
/// yet, which `attach::trust_command` lets it ask about. The home directory is refused too, but
/// its trust is never saved: asking would not help, and Claude Code's text says so.
pub fn needs_trust(error: &str) -> bool {
    error.contains("Workspace not trusted") && !error.contains("home directory")
}

/// The memory zone a session started in `directory` receives, by name, and the zones that could
/// not be read on the way. An isolated account receives no zone whatever this says.
pub fn memory_zone(directory: &Path) -> Result<(Option<String>, Vec<String>), String> {
    let paths = Paths::from_env().map_err(library_error)?;
    let home = paths.home().ok_or("the shared configuration has no parent")?;
    let root = Memory::default_root_for(&paths).map_err(|error| error.to_string())?;
    let lookup = Memory::at(root).zone_of(directory, home).map_err(|error| error.to_string())?;
    Ok((lookup.zone.map(|zone| zone.name), lookup.warnings))
}

/// Starts a background session the way `kindric-accounts run <account> --bg` does: shared
/// MCP servers (none for an isolated account), shared settings, billing variables removed. With
/// `theme`, the session draws with Claude Code's theme `custom:<theme>`.
pub fn launch(request: &Request, theme: Option<&str>) -> Result<Launched, String> {
    let paths = Paths::from_env().map_err(library_error)?;
    if accounts::is_implicit(&paths, &request.account).map_err(|error| error.to_string())? {
        return launch_default(&paths, request, theme);
    }
    launch::require_account(&paths, &request.account).map_err(library_error)?;
    launch::require_claude().map_err(library_error)?;
    let shared = mcp::prepare(&paths, &request.account).map_err(library_error)?;
    let args = background_args(&request.name);
    let prepared = launch::prepare(&paths, &request.account, shared.file.as_deref(), &args, Some(&request.directory), theme)
        .map_err(library_error)?;
    let short_id = run_backgrounded(prepared.command, &request.directory)?;
    let warnings = shared.warnings.into_iter().chain(prepared.warnings).collect();
    Ok(Launched { short_id, warnings })
}

/// The options of the implicit `default`: its rules when it is isolated, the theme when one is
/// given, in one `--settings` file. Claude Code adds these settings to the ones it reads there. A
/// `--bg` launch opens a conversation, so its rules of `global/` come too.
fn default_settings(paths: &Paths, theme: Option<&str>) -> Result<(Vec<OsString>, Vec<String>), String> {
    let mut isolated = isolation::prepare_settings(paths, registry::DEFAULT_ACCOUNT).map_err(library_error)?;
    let mut warnings = isolated.as_ref().map(|settings| settings.warnings.clone()).unwrap_or_default();
    let mut themed = None;
    if let Some(theme) = theme {
        let source = isolated.as_ref().map(|settings| settings.file.clone());
        match kindric_accounts::theme::write_themed_settings(paths, registry::DEFAULT_ACCOUNT, source.as_deref(), theme) {
            Ok(file) => themed = Some(file),
            Err(error) => warnings.push(format!("theme {theme} not given to the session: {error}")),
        }
    }
    let args = match (isolated.as_mut(), themed) {
        (Some(settings), Some(file)) => {
            settings.file = file;
            settings.args(true)
        },
        (Some(settings), None) => settings.args(true),
        (None, Some(file)) => vec![OsString::from("--settings"), file.into_os_string()],
        (None, None) => Vec::new(),
    };
    Ok((args, warnings))
}

/// `claude --bg` on the implicit `default`: Claude Code reads its own settings and MCP
/// servers there, so only the environment is cleaned.
fn launch_default(paths: &Paths, request: &Request, theme: Option<&str>) -> Result<Launched, String> {
    let claude = launch::require_claude().map_err(library_error)?;
    let (settings, settings_warnings) = default_settings(paths, theme)?;
    let mut command = Command::new(claude);
    for variable in accounts::default_removed_env() {
        command.env_remove(variable);
    }
    let memory = injection::for_session(paths, registry::DEFAULT_ACCOUNT, &request.directory);
    command.args(settings).args(background_args(&request.name)).args(memory.args);
    let short_id = run_backgrounded(command, &request.directory)?;
    let warnings = settings_warnings.into_iter().chain(memory.warnings).collect();
    Ok(Launched { short_id, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_implicit_default_gets_read_rules_only_when_isolated() {
        let root = std::env::temp_dir().join(format!("kindric-default-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let paths = Paths::from_roots(root.join("config"), root.join("data"), root.clone());
        assert_eq!(default_settings(&paths, None).unwrap(), (Vec::new(), Vec::new()));
        kindric_accounts::isolation::isolate(&paths, registry::DEFAULT_ACCOUNT).unwrap();
        let (args, warnings) = default_settings(&paths, None).unwrap();
        let file = paths.shared_settings_file(registry::DEFAULT_ACCOUNT);
        assert_eq!(args, [OsString::from("--settings"), file.clone().into_os_string()]);
        assert!(warnings.is_empty());
        assert!(fs::read_to_string(file).unwrap().contains("deny"));
        fs::remove_dir_all(root).unwrap();
    }

    fn settings_of(args: &[OsString]) -> serde_json::Value {
        assert_eq!(args[0], "--settings");
        serde_json::from_str(&fs::read_to_string(&args[1]).unwrap()).unwrap()
    }

    #[test]
    fn the_implicit_default_gets_the_theme_alone_or_with_its_rules() {
        let root = std::env::temp_dir().join(format!("kindric-default-theme-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let paths = Paths::from_roots(root.join("config"), root.join("data"), root.clone());
        // Claude Code reads the user's settings there itself: only the theme is added.
        let (args, warnings) = default_settings(&paths, Some("kindric")).unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(settings_of(&args), serde_json::json!({ "theme": "custom:kindric" }));
        assert!(warnings.is_empty(), "{warnings:?}");
        kindric_accounts::isolation::isolate(&paths, registry::DEFAULT_ACCOUNT).unwrap();
        let (args, _) = default_settings(&paths, Some("kindric")).unwrap();
        let settings = settings_of(&args);
        assert_eq!(settings["theme"], "custom:kindric");
        assert!(settings["permissions"]["deny"].as_array().is_some_and(|deny| !deny.is_empty()), "{settings}");
        assert_eq!(args[1], kindric_accounts::theme::themed_settings_file(&paths, registry::DEFAULT_ACCOUNT, "kindric").into_os_string());
        fs::remove_dir_all(root).unwrap();
    }

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

    /// The texts of Claude Code 2.1.319, behind `run_backgrounded`'s prefix.
    #[test]
    fn only_a_folder_whose_trust_can_be_saved_needs_trust() {
        let refused = "cannot start the session: Workspace not trusted. Run `claude` in /tmp/x once and accept the trust prompt, then retry.";
        assert!(needs_trust(refused));
        let home = "cannot start the session: Workspace not trusted. /home/u is your home directory, and for security home-directory trust is never saved, so running `claude` here first won't help.";
        assert!(!needs_trust(home));
        assert!(!needs_trust("cannot start the session: error: unknown option"));
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
        let root = std::env::temp_dir().join(format!("kindric-complete-{label}-{}", std::process::id()));
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
        let missing = resolve_directory("~/no-such-dir-kindric", &home);
        assert_eq!(missing, Err(format!("no such directory: {}", home.join("no-such-dir-kindric").display())));
    }
}
