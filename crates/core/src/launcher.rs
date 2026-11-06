//! Starts a background Claude Code session for an account, in a directory.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf, is_separator};
use std::process::{Command, Stdio};

use kindric_accounts::injection::{self, Injection};
use kindric_accounts::memory::Memory;
use kindric_accounts::paths::expand_home;
use kindric_accounts::{Paths, isolation, launch, mcp, registry};

use crate::accounts;
use crate::mailbox;

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

fn not_loaded(error: impl std::fmt::Display) -> Injection {
    Injection { args: Vec::new(), warnings: vec![format!("memory not loaded: {error}")] }
}

/// What the memory adds to this session: the zone whose folders hold its directory. An isolated
/// account receives nothing here: it gets `global/` alone from `kindric_accounts::isolation`, with
/// its rules against reading the rest, and no zone is even looked up. A directory outside every
/// zone receives nothing either. Nothing here can stop the session from starting.
fn injection_for(memory: &Memory, data_dir: &Path, home: &Path, request: &Request, isolated: bool) -> Injection {
    if isolated {
        return Injection::default();
    }
    let lookup = match memory.zone_of(&request.directory, home) {
        Ok(lookup) => lookup,
        Err(error) => return not_loaded(error),
    };
    let Some(zone) = lookup.zone else {
        return Injection { args: Vec::new(), warnings: lookup.warnings };
    };
    let mut injection = injection::prepare(memory, data_dir, &zone, &request.directory).unwrap_or_else(not_loaded);
    injection.warnings.splice(0..0, lookup.warnings);
    injection
}

/// When it cannot be told whether the account is isolated, the account counts as isolated.
fn memory_injection(paths: &Paths, request: &Request) -> Injection {
    let isolated = registry::is_isolated(paths, &request.account).unwrap_or(true);
    let Some(home) = paths.home() else {
        return not_loaded("the shared configuration has no parent");
    };
    match (Memory::default_root(), mailbox::data_dir()) {
        (Ok(root), Ok(data_dir)) => injection_for(&Memory::at(root), &data_dir, home, request, isolated),
        (Err(error), _) | (_, Err(error)) => not_loaded(error),
    }
}

/// Starts a background session the way `kindric-accounts run <account> --bg` does: shared
/// MCP servers (none for an isolated account), shared settings, billing variables removed.
pub fn launch(request: &Request) -> Result<Launched, String> {
    let paths = Paths::from_env().map_err(library_error)?;
    if accounts::is_implicit(&paths, &request.account).map_err(|error| error.to_string())? {
        return launch_default(&paths, request);
    }
    launch::require_account(&paths, &request.account).map_err(library_error)?;
    launch::require_claude().map_err(library_error)?;
    let shared = mcp::prepare(&paths, &request.account).map_err(library_error)?;
    let memory = memory_injection(&paths, request);
    let mut args = background_args(&request.name);
    args.extend(memory.args);
    let prepared =
        launch::prepare(&paths, &request.account, shared.file.as_deref(), &args).map_err(library_error)?;
    let short_id = run_backgrounded(prepared.command, &request.directory)?;
    let warnings = shared.warnings.into_iter().chain(prepared.warnings).chain(memory.warnings).collect();
    Ok(Launched { short_id, warnings })
}

/// The options of the implicit `default`, when it is isolated. Claude Code adds these settings to
/// the ones it reads there.
fn default_settings(paths: &Paths) -> Result<(Vec<OsString>, Vec<String>), String> {
    let settings = isolation::prepare_settings(paths, registry::DEFAULT_ACCOUNT).map_err(library_error)?;
    Ok(settings.map_or_else(Default::default, |settings| (settings.args(), settings.warnings)))
}

/// `claude --bg` on the implicit `default`: Claude Code reads its own settings and MCP
/// servers there, so only the environment is cleaned.
fn launch_default(paths: &Paths, request: &Request) -> Result<Launched, String> {
    let claude = launch::require_claude().map_err(library_error)?;
    let (settings, settings_warnings) = default_settings(paths)?;
    let mut command = Command::new(claude);
    for variable in accounts::default_removed_env() {
        command.env_remove(variable);
    }
    let memory = memory_injection(paths, request);
    command.args(settings).args(background_args(&request.name)).args(memory.args);
    let short_id = run_backgrounded(command, &request.directory)?;
    let warnings = settings_warnings.into_iter().chain(memory.warnings).collect();
    Ok(Launched { short_id, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A memory with a zone for `p` holding a rule and instructions, and an empty project folder.
    fn memory_setup(label: &str) -> (PathBuf, Memory, PathBuf) {
        let root = std::env::temp_dir().join(format!("kindric-launcher-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let memory = Memory::at(root.join("memory"));
        memory.ensure_project("p").unwrap();
        fs::write(root.join("memory/global/rules.md"), "RULE").unwrap();
        let directory = root.join("project");
        let instructions = format!("---\ndirectory: {}\n---\nPROJECT RULES", directory.display());
        fs::write(root.join("memory/projets/p/claude.md"), instructions).unwrap();
        fs::create_dir_all(&directory).unwrap();
        (root, memory, directory)
    }

    fn request(directory: PathBuf) -> Request {
        Request { account: "a".into(), directory, name: String::new() }
    }

    fn injection(root: &Path, memory: &Memory, data_dir: &Path, directory: &Path, isolated: bool) -> Injection {
        injection_for(memory, data_dir, &root.join("home"), &request(directory.to_path_buf()), isolated)
    }

    #[test]
    fn the_implicit_default_gets_read_rules_only_when_isolated() {
        let root = std::env::temp_dir().join(format!("kindric-default-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let paths = Paths::from_roots(root.join("config"), root.join("data"), root.clone());
        assert_eq!(default_settings(&paths).unwrap(), (Vec::new(), Vec::new()));
        kindric_accounts::isolation::isolate(&paths, registry::DEFAULT_ACCOUNT).unwrap();
        let (args, warnings) = default_settings(&paths).unwrap();
        let file = paths.shared_settings_file(registry::DEFAULT_ACCOUNT);
        assert_eq!(args, [OsString::from("--settings"), file.clone().into_os_string()]);
        assert!(warnings.is_empty());
        assert!(fs::read_to_string(file).unwrap().contains("deny"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_session_of_a_project_with_a_zone_receives_the_memory() {
        let (root, memory, directory) = memory_setup("zone");
        fs::create_dir_all(directory.join("src")).unwrap();
        let injection = injection(&root, &memory, &root.join("data"), &directory.join("src"), false);
        assert_eq!(injection.args.len(), 2);
        assert!(injection.warnings.is_empty(), "{:?}", injection.warnings);
        assert!(directory.join("src/CLAUDE.md").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_zone_that_cannot_be_read_is_a_warning_of_the_session() {
        let (root, memory, directory) = memory_setup("broken-zone");
        memory.ensure_project("broken").unwrap();
        fs::write(root.join("memory/projets/broken/claude.md"), "---\ndirectory: ~/x\n").unwrap();
        let injection = injection(&root, &memory, &root.join("data"), &directory, false);
        assert_eq!(injection.args.len(), 2);
        assert_eq!(injection.warnings.len(), 1);
        assert!(injection.warnings[0].starts_with("projets/broken/claude.md: "));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_isolated_account_receives_nothing_and_nothing_is_written() {
        let (root, memory, directory) = memory_setup("isolated");
        let data = root.join("data");
        assert_eq!(injection(&root, &memory, &data, &directory, true), Injection::default());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        assert!(!data.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_directory_outside_every_zone_receives_nothing_and_nothing_is_written() {
        let (root, memory, _) = memory_setup("no-zone");
        let data = root.join("data");
        let elsewhere = root.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        assert_eq!(injection(&root, &memory, &data, &elsewhere, false), Injection::default());
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
        assert!(!data.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_memory_that_fails_is_a_warning_and_the_session_still_starts() {
        let (root, memory, directory) = memory_setup("fails");
        let blocked = root.join("data-is-a-file");
        fs::write(&blocked, "x").unwrap();
        let injection = injection(&root, &memory, &blocked, &directory, false);
        assert!(injection.args.is_empty());
        assert!(injection.warnings[0].starts_with("memory not loaded: "));
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
