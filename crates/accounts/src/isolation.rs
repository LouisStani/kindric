//! Isolated accounts (a work account, for example) give their MCP servers to no other
//! account and receive none from them, and their sessions get rules against reading what
//! belongs to the others. Those are rules of Claude Code, not a wall: its file tools and the
//! file commands it recognizes obey them, a script that opens a file itself does not.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::error::{Result, fail};
use crate::files;
use crate::paths::{MEMORY_ROOT_VARIABLE, Paths};
use crate::registry;

const RULE_TOOLS: [&str; 2] = ["Read", "Edit"];
const GLOBAL_RULES_DIR: &str = "global";

/// The settings of a session of an isolated account.
#[derive(Debug, PartialEq, Eq)]
pub struct IsolatedSettings {
    /// File to pass with `--settings`, in place of the shared one.
    pub file: PathBuf,
    /// The rules of `global/` in the memory, if there are any.
    pub global_rules: Option<PathBuf>,
    pub warnings: Vec<String>,
}

impl IsolatedSettings {
    /// The options these settings add to a session. The rules of `global/` go only to a launch
    /// that opens a conversation; the settings, which hold what may not be read, go to every one.
    pub fn args(&self, conversation: bool) -> Vec<OsString> {
        let mut args = vec![OsString::from("--settings"), self.file.clone().into_os_string()];
        if let Some(rules) = self.global_rules.as_ref().filter(|_| conversation) {
            args.push(OsString::from("--append-system-prompt-file"));
            args.push(rules.clone().into_os_string());
        }
        args
    }
}

/// `default`, Claude Code's own configuration, can be isolated without being an account:
/// kindric keeps it out of its messages.
fn require_isolatable(paths: &Paths, name: &str) -> Result<()> {
    if name == registry::DEFAULT_ACCOUNT {
        return Ok(());
    }
    registry::require_existing_account(paths, name)
}

/// Returns `false` when the account was already isolated.
pub fn isolate(paths: &Paths, name: &str) -> Result<bool> {
    require_isolatable(paths, name)?;
    if registry::is_isolated(paths, name)? {
        return Ok(false);
    }
    files::create_private_dir(&paths.config_dir)?;
    files::append_line(&paths.isolated_file(), name)?;
    Ok(true)
}

/// Returns `false` when the account was not isolated.
pub fn unisolate(paths: &Paths, name: &str) -> Result<bool> {
    require_isolatable(paths, name)?;
    if !registry::is_isolated(paths, name)? {
        return Ok(false);
    }
    files::remove_line(&paths.isolated_file(), name)?;
    Ok(true)
}

/// What `name` may not read: the other accounts, whatever carries their settings or MCP servers,
/// kindric's memory and data, and the user's own list.
fn protected_paths(paths: &Paths, name: &str, memory_root: Option<PathBuf>) -> Result<(Vec<PathBuf>, Vec<String>)> {
    let mut protected: Vec<PathBuf> = registry::list_accounts(paths)?
        .iter()
        .filter(|account| account.as_str() != name)
        .map(|account| paths.account_dir(account))
        .collect();
    protected.push(paths.data_root.join(".shared-mcp"));
    protected.push(paths.data_root.join(".shared-settings"));
    if name != registry::DEFAULT_ACCOUNT {
        protected.push(paths.shared_root.join("projects"));
    }
    protected.extend(memory_root);
    protected.extend(paths.kindric_data_dir());
    let (listed, warnings) = listed_paths(paths)?;
    protected.extend(listed);
    Ok((protected, warnings))
}

/// The lines of `isolated-deny`: absolute paths or `~/...`; blank lines and `#` lines ignored.
fn listed_paths(paths: &Paths) -> Result<(Vec<PathBuf>, Vec<String>)> {
    let mut listed = Vec::new();
    let mut warnings = Vec::new();
    for line in files::read_lines(&paths.isolated_deny_file())? {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let expanded = match (line.strip_prefix("~/"), paths.home()) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => PathBuf::from(line),
        };
        if expanded.is_absolute() {
            listed.push(expanded);
        } else {
            warnings.push(format!("{}: ignored, not an absolute path: {line}", paths.isolated_deny_file().display()));
        }
    }
    Ok((listed, warnings))
}

/// A path as Claude Code matches it: `//` then the absolute path, with Windows paths in POSIX
/// form (`C:\Users` is `//c/Users`).
fn rule_path(path: &str) -> String {
    let slashes = path.replace('\\', "/");
    let trimmed = slashes.trim_end_matches('/');
    let mut letters = trimmed.chars();
    match (letters.next(), letters.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => {
            format!("//{}{}", drive.to_ascii_lowercase(), &trimmed[2..])
        }
        _ => format!("/{trimmed}"),
    }
}

fn deny_rules(protected: &[PathBuf]) -> Vec<String> {
    let mut rules = Vec::new();
    for path in protected {
        let path = rule_path(&path.to_string_lossy());
        rules.extend(RULE_TOOLS.iter().map(|tool| format!("{tool}({path}/**)")));
    }
    rules
}

fn with_unique(mut list: Vec<Value>, items: &[String]) -> Vec<Value> {
    for item in items {
        let item = Value::String(item.clone());
        if !list.contains(&item) {
            list.push(item);
        }
    }
    list
}

/// `~/CLAUDE.md`, which Claude Code loads for any session under the home folder.
fn excluded_instructions(paths: &Paths) -> Vec<String> {
    paths.home().map(|home| home.join("CLAUDE.md").to_string_lossy().replace('\\', "/")).into_iter().collect()
}

/// The shared settings plus the rules, in one file: Claude Code keeps only the last `--settings`.
/// Settings that cannot be read as an object are left out, and the reason comes back.
fn settings_with_rules(shared: Option<&str>, rules: &[String], excluded: &[String]) -> (String, Option<String>) {
    let (mut settings, problem) = match shared.map(serde_json::from_str::<Map<String, Value>>) {
        Some(Ok(settings)) => (settings, None),
        Some(Err(error)) => (Map::new(), Some(error.to_string())),
        None => (Map::new(), None),
    };
    let mut permissions = match settings.remove("permissions") {
        Some(Value::Object(permissions)) => permissions,
        _ => Map::new(),
    };
    let deny = match permissions.remove("deny") {
        Some(Value::Array(deny)) => deny,
        _ => Vec::new(),
    };
    permissions.insert("deny".to_string(), Value::Array(with_unique(deny, rules)));
    settings.insert("permissions".to_string(), Value::Object(permissions));
    let excludes = match settings.remove("claudeMdExcludes") {
        Some(Value::Array(excludes)) => excludes,
        _ => Vec::new(),
    };
    settings.insert("claudeMdExcludes".to_string(), Value::Array(with_unique(excludes, excluded)));
    (format!("{}\n", Value::Object(settings)), problem)
}

fn read_if_present(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn write_settings(file: &Path, content: &str) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        files::create_private_dir(dir)?;
    }
    files::write_atomic(file, content)
}

/// Where kindric's memory is, for an account that may not read it.
fn memory_root(paths: &Paths) -> Option<PathBuf> {
    std::env::var_os(MEMORY_ROOT_VARIABLE)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| paths.default_memory_root())
}

/// The files of `global/`, joined. `None` when there is nothing to give.
fn global_rules_text(memory_root: &Path) -> io::Result<Option<String>> {
    let mut parts = Vec::new();
    for file in files::files_under(&memory_root.join(GLOBAL_RULES_DIR))? {
        let text = fs::read_to_string(file)?;
        if !text.trim().is_empty() {
            parts.push(text.trim().to_string());
        }
    }
    Ok((!parts.is_empty()).then(|| parts.join("\n\n")))
}

/// Writes the rules of `global/` for a session of `name`. These rules protect nothing, so what
/// goes wrong here is a warning and the session still starts.
fn write_global_rules(paths: &Paths, name: &str, memory_root: &Path, warnings: &mut Vec<String>) -> Option<PathBuf> {
    let file = paths.shared_settings_file(name).with_extension("md");
    let written = global_rules_text(memory_root).and_then(|text| match text {
        Some(text) => write_settings(&file, &text).map(|()| true),
        None => Ok(false),
    });
    match written {
        Ok(true) => Some(file),
        Ok(false) => None,
        Err(error) => {
            warnings.push(format!("rules of {} not loaded: {error}", memory_root.join(GLOBAL_RULES_DIR).display()));
            None
        }
    }
}

fn settings_for(paths: &Paths, name: &str, memory_root: Option<PathBuf>) -> Result<Option<IsolatedSettings>> {
    if !registry::is_isolated(paths, name)? {
        return Ok(None);
    }
    let (protected, mut warnings) = protected_paths(paths, name, memory_root.clone())?;
    let source = paths.shared_root.join("settings.json");
    let (shared, unreadable) = match read_if_present(&source) {
        Ok(text) => (text, None),
        Err(error) => (None, Some(error.to_string())),
    };
    let (content, invalid) = settings_with_rules(shared.as_deref(), &deny_rules(&protected), &excluded_instructions(paths));
    if let Some(problem) = unreadable.or(invalid) {
        warnings.push(format!("{}: left out of this isolated session: {problem}", source.display()));
    }
    let file = paths.shared_settings_file(name);
    if let Err(error) = write_settings(&file, &content) {
        return fail(format!("not starting isolated account {name}: cannot write {}: {error}", file.display()));
    }
    let global_rules = memory_root.and_then(|root| write_global_rules(paths, name, &root, &mut warnings));
    Ok(Some(IsolatedSettings { file, global_rules, warnings }))
}

/// The settings of a session of `name`, if it is isolated. Refuses rather than start an isolated
/// session without its rules.
pub fn prepare_settings(paths: &Paths, name: &str) -> Result<Option<IsolatedSettings>> {
    settings_for(paths, name, memory_root(paths))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::test_dir;

    fn setup(label: &str, accounts: &[&str]) -> (PathBuf, Paths) {
        let root = test_dir::fresh(&format!("isolation-{label}"));
        let paths = Paths::from_roots(root.join("config"), root.join("data"), root.clone());
        files::create_private_dir(&paths.config_dir).unwrap();
        for account in accounts {
            files::append_line(&paths.registry_file(), account).unwrap();
        }
        (root, paths)
    }

    fn deny_of(file: &Path) -> Vec<String> {
        let settings: Value = serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
        settings["permissions"]["deny"].as_array().unwrap().iter().map(|rule| rule.as_str().unwrap().to_string()).collect()
    }

    fn rules_for(path: &Path) -> [String; 2] {
        let path = rule_path(&path.to_string_lossy());
        [format!("Read({path}/**)"), format!("Edit({path}/**)")]
    }

    #[test]
    fn rule_paths_are_absolute_from_the_root_of_the_filesystem() {
        assert_eq!(rule_path("/home/me/.kindric/memory"), "//home/me/.kindric/memory");
        assert_eq!(rule_path("/home/me/notes/"), "//home/me/notes");
        assert_eq!(rule_path(r"C:\Users\me\notes"), "//c/Users/me/notes");
    }

    #[test]
    fn an_account_that_is_not_isolated_gets_nothing_and_nothing_is_written() {
        let (root, paths) = setup("not-isolated", &["work"]);
        assert_eq!(settings_for(&paths, "work", None).unwrap(), None);
        assert!(!paths.shared_settings_file("work").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_isolated_account_may_not_read_what_belongs_to_the_others() {
        let (root, paths) = setup("protected", &["work", "personal"]);
        isolate(&paths, "work").unwrap();
        let memory = root.join("memory");
        let settings = settings_for(&paths, "work", Some(memory.clone())).unwrap().unwrap();
        assert_eq!(settings.file, paths.shared_settings_file("work"));
        assert!(settings.warnings.is_empty());
        let deny = deny_of(&settings.file);
        let expected = [
            paths.account_dir("personal"),
            paths.data_root.join(".shared-mcp"),
            paths.data_root.join(".shared-settings"),
            paths.shared_root.join("projects"),
            memory,
            paths.kindric_data_dir().unwrap(),
        ];
        for path in &expected {
            for rule in rules_for(path) {
                assert!(deny.contains(&rule), "{rule} missing from {deny:?}");
            }
        }
        assert_eq!(deny.len(), expected.len() * 2);
        for rule in rules_for(&paths.account_dir("work")) {
            assert!(!deny.contains(&rule), "its own folder is denied: {rule}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_default_account_keeps_its_own_transcripts() {
        let (root, paths) = setup("default", &[]);
        isolate(&paths, registry::DEFAULT_ACCOUNT).unwrap();
        let settings = settings_for(&paths, registry::DEFAULT_ACCOUNT, None).unwrap().unwrap();
        for rule in rules_for(&paths.shared_root.join("projects")) {
            assert!(!deny_of(&settings.file).contains(&rule));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_shared_settings_are_kept_and_their_rules_with_them() {
        let (root, paths) = setup("merge", &["work"]);
        isolate(&paths, "work").unwrap();
        fs::create_dir_all(&paths.shared_root).unwrap();
        let existing = "Read(//already/**)";
        let shared = serde_json::json!({
            "theme": "dark",
            "permissions": {"allow": ["Bash(ls)"], "deny": [existing]}
        });
        fs::write(paths.shared_root.join("settings.json"), shared.to_string()).unwrap();
        let file = settings_for(&paths, "work", None).unwrap().unwrap().file;
        let settings: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(settings["theme"], "dark");
        assert_eq!(settings["permissions"]["allow"][0], "Bash(ls)");
        assert!(deny_of(&file).iter().any(|rule| rule == existing));
        assert!(deny_of(&file).len() > 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_rule_already_there_is_not_added_twice() {
        let rule = "Read(//x/**)".to_string();
        let shared = format!("{{\"permissions\":{{\"deny\":[\"{rule}\"]}}}}");
        let (content, problem) = settings_with_rules(Some(&shared), std::slice::from_ref(&rule), &[]);
        assert_eq!(problem, None);
        assert_eq!(content.matches(&rule).count(), 1);
    }

    #[test]
    fn invalid_shared_settings_give_the_rules_alone_and_a_warning() {
        let (root, paths) = setup("invalid", &["work"]);
        isolate(&paths, "work").unwrap();
        fs::create_dir_all(&paths.shared_root).unwrap();
        fs::write(paths.shared_root.join("settings.json"), "{broken").unwrap();
        let settings = settings_for(&paths, "work", None).unwrap().unwrap();
        assert_eq!(settings.warnings.len(), 1);
        assert!(settings.warnings[0].contains("settings.json"), "{:?}", settings.warnings);
        assert!(!deny_of(&settings.file).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_list_of_the_user_is_added_and_a_relative_path_is_refused() {
        let (root, paths) = setup("listed", &["work"]);
        isolate(&paths, "work").unwrap();
        let absolute = root.join("private");
        fs::write(
            paths.isolated_deny_file(),
            format!("# comment\n\n~/notes\n{}\nrelative/path\n", absolute.display()),
        )
        .unwrap();
        let settings = settings_for(&paths, "work", None).unwrap().unwrap();
        let deny = deny_of(&settings.file);
        for path in [root.join("notes"), absolute] {
            for rule in rules_for(&path) {
                assert!(deny.contains(&rule), "{rule} missing");
            }
        }
        assert!(!deny.iter().any(|rule| rule.contains("relative") || rule.contains("comment")));
        assert_eq!(settings.warnings.len(), 1);
        assert!(settings.warnings[0].contains("relative/path"));
        fs::remove_dir_all(root).unwrap();
    }

    fn memory_file(memory: &Path, relative: &str, text: &str) {
        let file = memory.join(relative);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }

    #[test]
    fn a_session_gets_global_and_nothing_else_from_the_memory() {
        let (root, paths) = setup("global", &["work"]);
        isolate(&paths, "work").unwrap();
        let memory = root.join("memory");
        memory_file(&memory, "global/a.md", "RULE-A\n");
        memory_file(&memory, "global/sub/b.md", "RULE-B\n");
        memory_file(&memory, "global/empty.md", "  \n");
        memory_file(&memory, "setup/machine.md", "MACHINE");
        memory_file(&memory, "perso/profil.md", "PERSONAL");
        memory_file(&memory, "projets/x/INDEX.md", "PROJECT");
        memory_file(&memory, "skills/s/SKILL.md", "SKILL");
        let settings = settings_for(&paths, "work", Some(memory)).unwrap().unwrap();
        let rules = settings.global_rules.clone().unwrap();
        assert_eq!(fs::read_to_string(&rules).unwrap(), "RULE-A\n\nRULE-B");
        assert_eq!(settings.args(false), [OsString::from("--settings"), settings.file.clone().into_os_string()]);
        assert_eq!(
            settings.args(true),
            [
                OsString::from("--settings"),
                settings.file.clone().into_os_string(),
                OsString::from("--append-system-prompt-file"),
                rules.into_os_string(),
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn without_global_there_is_no_option_and_no_warning() {
        let (root, paths) = setup("no-global", &["work"]);
        isolate(&paths, "work").unwrap();
        let settings = settings_for(&paths, "work", Some(root.join("memory"))).unwrap().unwrap();
        assert_eq!(settings.global_rules, None);
        assert!(settings.warnings.is_empty());
        assert_eq!(settings.args(true).len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn global_that_cannot_be_read_is_a_warning_and_the_session_starts() {
        let (root, paths) = setup("bad-global", &["work"]);
        isolate(&paths, "work").unwrap();
        let memory = root.join("memory");
        memory_file(&memory, "global", "a file, not a folder");
        let settings = settings_for(&paths, "work", Some(memory)).unwrap().unwrap();
        assert_eq!(settings.global_rules, None);
        assert_eq!(settings.warnings.len(), 1);
        assert!(settings.warnings[0].contains("not loaded"), "{:?}", settings.warnings);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_instructions_of_the_home_folder_are_excluded_and_other_exclusions_kept() {
        let (root, paths) = setup("excludes", &["work"]);
        isolate(&paths, "work").unwrap();
        fs::create_dir_all(&paths.shared_root).unwrap();
        fs::write(paths.shared_root.join("settings.json"), r#"{"claudeMdExcludes":["/already/CLAUDE.md"]}"#).unwrap();
        let file = settings_for(&paths, "work", None).unwrap().unwrap().file;
        let settings: Value = serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
        let excludes = settings["claudeMdExcludes"].as_array().unwrap();
        let home = root.join("CLAUDE.md").to_string_lossy().replace('\\', "/");
        assert!(excludes.contains(&Value::String(home)), "{excludes:?}");
        assert!(excludes.contains(&Value::String("/already/CLAUDE.md".to_string())), "{excludes:?}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn without_a_place_to_write_the_rules_the_session_does_not_start() {
        let (root, paths) = setup("unwritable", &["work"]);
        isolate(&paths, "work").unwrap();
        fs::create_dir_all(&paths.data_root).unwrap();
        fs::write(paths.data_root.join(".shared-settings"), "a file, not a folder").unwrap();
        let error = settings_for(&paths, "work", None).unwrap_err();
        assert!(error.0.contains("not starting"), "{}", error.0);
        fs::remove_dir_all(root).unwrap();
    }
}
