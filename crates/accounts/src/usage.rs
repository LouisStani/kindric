//! How much of its limits an account has used, over five hours and over the week. Claude Code
//! gives these figures (`rate_limits`) only to the command of its status line. The sessions
//! kindric-accounts starts get settings whose status line is `kindric-accounts statusline`: it
//! writes the figures down per account, then runs the user's own status line with the same
//! input, so the user sees no difference. The user's `settings.json` is never written.

use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::files;
use crate::launch;
use crate::paths::Paths;
use crate::registry;

/// The subcommand Claude Code runs for the status line.
pub const SUBCOMMAND: &str = "statusline";
/// Set for the user's status line: a status line that runs kindric-accounts again stops there.
const NESTED: &str = "KINDRIC_ACCOUNTS_STATUSLINE";

#[cfg(windows)]
const PROGRAM_NAMES: [&str; 2] = ["kindric-accounts.exe", "kindric-accounts"];
#[cfg(not(windows))]
const PROGRAM_NAMES: [&str; 1] = ["kindric-accounts"];

/// One window of limits, as Claude Code gives it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub used_percentage: f64,
    /// Unix seconds.
    pub resets_at: Option<i64>,
}

/// The last figures of an account. A window Claude Code left out had just been reset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    /// Unix seconds.
    pub updated_at: i64,
}

/// The share of a window used at `now`: 0 once it has been reset.
pub fn percent(window: Option<&Window>, now: i64) -> f64 {
    match window {
        Some(window) if window.resets_at.is_none_or(|resets_at| resets_at > now) => window.used_percentage,
        _ => 0.0,
    }
}

pub fn usage_file(paths: &Paths, name: &str) -> PathBuf {
    paths.data_root.join(".usage").join(format!("{name}.json"))
}

/// The last figures written down for `name`; `None` when there are none or they cannot be read.
pub fn read(paths: &Paths, name: &str) -> Option<Usage> {
    serde_json::from_str(&std::fs::read_to_string(usage_file(paths, name)).ok()?).ok()
}

/// The figures of the input Claude Code gives a status line; `None` without `rate_limits`
/// (an account billed by API key has none).
fn usage_of(input: &[u8], now: i64) -> Option<Usage> {
    #[derive(Deserialize)]
    struct Input {
        rate_limits: Option<Limits>,
    }
    #[derive(Deserialize)]
    struct Limits {
        five_hour: Option<RawWindow>,
        seven_day: Option<RawWindow>,
    }
    #[derive(Deserialize)]
    struct RawWindow {
        used_percentage: Option<f64>,
        resets_at: Option<f64>,
    }
    let window = |raw: Option<RawWindow>| {
        let raw = raw?;
        let used = raw.used_percentage.filter(|used| used.is_finite())?.clamp(0.0, 100.0);
        // Whole seconds: the cast saturates on a value out of range.
        #[allow(clippy::cast_possible_truncation)]
        let resets_at = raw.resets_at.filter(|at| at.is_finite()).map(|at| at as i64);
        Some(Window { used_percentage: used, resets_at })
    };
    let limits = serde_json::from_slice::<Input>(input).ok()?.rate_limits?;
    Some(Usage { five_hour: window(limits.five_hour), seven_day: window(limits.seven_day), updated_at: now })
}

/// The account a status line runs for: the last part of `CLAUDE_CONFIG_DIR`, when that is the
/// directory of an account of the list.
fn account_of(paths: &Paths, config_dir: Option<&OsStr>) -> Option<String> {
    let dir = Path::new(config_dir?);
    let name = dir.file_name()?.to_str()?.to_string();
    let listed = registry::account_exists(paths, &name).unwrap_or(false);
    (listed && paths.account_dir(&name) == dir).then_some(name)
}

/// Writes down the figures of `input` for the account of `config_dir`. Nothing to write is not
/// an error.
pub fn record(paths: &Paths, config_dir: Option<&OsStr>, input: &[u8], now: i64) -> io::Result<()> {
    let (Some(name), Some(usage)) = (account_of(paths, config_dir), usage_of(input, now)) else {
        return Ok(());
    };
    let file = usage_file(paths, &name);
    if let Some(dir) = file.parent() {
        files::create_private_dir(dir)?;
    }
    files::write_atomic(&file, &serde_json::to_string(&usage).map_err(io::Error::other)?)
}

/// The command of the user's status line, in `~/.claude/settings.json`.
fn user_statusline(paths: &Paths) -> Option<String> {
    let settings: Value = serde_json::from_str(&std::fs::read_to_string(paths.shared_root.join("settings.json")).ok()?).ok()?;
    let command = settings.get("statusLine")?.get("command")?.as_str()?.trim();
    (!command.is_empty()).then(|| command.to_string())
}

#[cfg(not(windows))]
fn shell(command: &str) -> Command {
    let mut shell = Command::new("sh");
    shell.arg("-c").arg(command);
    shell
}

/// Claude Code on Windows runs hooks through Git Bash when it is there; not tried.
#[cfg(windows)]
fn shell(command: &str) -> Command {
    let path = std::env::var_os("PATH").unwrap_or_default();
    match launch::find_in(&path, &["bash.exe"]) {
        Some(bash) => {
            let mut shell = Command::new(bash);
            shell.arg("-c").arg(command);
            shell
        },
        None => {
            let mut shell = Command::new("cmd");
            shell.arg("/C").arg(command);
            shell
        },
    }
}

/// Runs the user's status line with `input` and returns what it prints and its exit code.
/// `None` when there is none to run, or when this is already the user's status line calling
/// kindric-accounts again.
pub fn run_user_statusline(paths: &Paths, input: &[u8]) -> io::Result<Option<(Vec<u8>, i32)>> {
    if std::env::var_os(NESTED).is_some() {
        return Ok(None);
    }
    let Some(command) = user_statusline(paths) else {
        return Ok(None);
    };
    let mut child = shell(&command).env(NESTED, "1").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        // A status line that does not read its input closes it early: not an error.
        let _ = stdin.write_all(input);
    }
    let output = child.wait_with_output()?;
    Ok(Some((output.stdout, output.status.code().unwrap_or(1))))
}

/// `"<kindric-accounts>" statusline`, the status line of the sessions kindric-accounts starts:
/// this program when it is kindric-accounts, else the one in `PATH` when it is recent enough to
/// know `statusline` (an older one would break the status line). `None` otherwise.
pub fn wrapper_command() -> Option<String> {
    let current = std::env::current_exe().ok().filter(|exe| {
        exe.file_stem().is_some_and(|stem| stem == "kindric-accounts")
    });
    let program = match current {
        Some(program) => program,
        None => {
            let path = std::env::var_os("PATH").unwrap_or_default();
            launch::find_in(&path, &PROGRAM_NAMES).filter(|program| knows_statusline(program))?
        },
    };
    Some(format!("\"{}\" {SUBCOMMAND}", program.display()))
}

/// The first version of kindric-accounts with `statusline`.
const STATUSLINE_SINCE: (u32, u32, u32) = (2, 1, 0);

/// Whether `program --version` says `kindric-accounts X.Y.Z` with a version that has `statusline`.
fn knows_statusline(program: &Path) -> bool {
    let mut command = Command::new(program);
    command.arg("--version").stdin(Stdio::null()).stderr(Stdio::null());
    let Ok(output) = command.output() else {
        return false;
    };
    version_of(&String::from_utf8_lossy(&output.stdout)).is_some_and(|version| version >= STATUSLINE_SINCE)
}

/// `kindric-accounts 2.1.0` → (2, 1, 0).
fn version_of(output: &str) -> Option<(u32, u32, u32)> {
    let mut parts = output.trim().strip_prefix("kindric-accounts ")?.split('.').map(str::parse::<u32>);
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(Ok(major)), Some(Ok(minor)), Some(Ok(patch)), None) => Some((major, minor, patch)),
        _ => None,
    }
}

/// `settings` (a `settings.json`) with its status line replaced by `command`; what else it holds
/// is kept, its `padding` too. Settings that do not parse are left out: the second value says why.
pub fn with_statusline(settings: Option<&str>, command: &str) -> (String, Option<String>) {
    with_additions(settings, Some(command), &KindricHooks::default())
}

/// The hooks of `kindric` set on a session, each when the `kindric` in `PATH` has it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KindricHooks {
    /// Tells a session of its unread messages.
    pub unread: Option<String>,
    /// Keeps a session working while the user is away.
    pub away: Option<String>,
}

impl KindricHooks {
    fn is_empty(&self) -> bool {
        self.unread.is_none() && self.away.is_none()
    }
}

/// `settings` with the status line replaced by `statusline` and kindric's `hooks` added, each when
/// given. What else the settings hold is kept, the user's own hooks too.
pub fn with_additions(settings: Option<&str>, statusline: Option<&str>, hooks: &KindricHooks) -> (String, Option<String>) {
    let (mut settings, problem) = match settings.map(serde_json::from_str::<Map<String, Value>>) {
        Some(Ok(settings)) => (settings, None),
        Some(Err(error)) => (Map::new(), Some(error.to_string())),
        None => (Map::new(), None),
    };
    if let Some(command) = statusline {
        let mut line = Map::new();
        line.insert("type".into(), Value::from("command"));
        line.insert("command".into(), Value::from(command));
        if let Some(padding) = settings.get("statusLine").and_then(|line| line.get("padding")) {
            line.insert("padding".into(), padding.clone());
        }
        settings.insert("statusLine".into(), Value::Object(line));
    }
    if let Some(command) = &hooks.unread {
        add_hook(&mut settings, &UNREAD_EVENTS, command);
    }
    if let Some(command) = &hooks.away {
        add_hook(&mut settings, &AWAY_EVENTS, command);
    }
    let text = serde_json::to_string_pretty(&Value::Object(settings)).unwrap_or_default();
    (text, problem)
}

/// The events the hook of kindric's messages runs on: before each prompt, after each tool.
const UNREAD_EVENTS: [&str; 2] = ["UserPromptSubmit", "PostToolUse"];

/// The event the hook of the user being away runs on: when a session ends a turn.
const AWAY_EVENTS: [&str; 1] = ["Stop"];

/// Adds `command` to the hooks of `events`, after the user's. Hooks that are not in Claude Code's
/// form are left as they are, without this one.
fn add_hook(settings: &mut Map<String, Value>, events: &[&str], command: &str) {
    let hooks = settings.entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(hooks) = hooks else {
        return;
    };
    for &event in events {
        let groups = hooks.entry(event).or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(groups) = groups {
            groups.push(serde_json::json!({ "hooks": [{ "type": "command", "command": command }] }));
        }
    }
}

#[cfg(windows)]
const KINDRIC_NAMES: [&str; 2] = ["kindric.exe", "kindric"];
#[cfg(not(windows))]
const KINDRIC_NAMES: [&str; 1] = ["kindric"];

/// kindric's hooks for a session: `"<kindric>" unread` and `"<kindric>" away-hook`, each when
/// the `kindric` in `PATH` has it. An older one would answer with its usage and exit code 2, which
/// Claude Code shows to Claude after every tool: each is tried once, with no input, and must exit 0.
pub fn kindric_hooks() -> KindricHooks {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let Some(kindric) = launch::find_in(&path, &KINDRIC_NAMES) else {
        return KindricHooks::default();
    };
    let answering = |subcommand: &str| {
        let mut probe = Command::new(&kindric);
        probe.arg(subcommand).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let answers = probe.status().is_ok_and(|status| status.success());
        answers.then(|| format!("\"{}\" {subcommand}", kindric.display()))
    };
    KindricHooks { unread: answering("unread"), away: answering("away-hook") }
}

/// The settings file of a session of `name`, written to `.shared-settings/<name>.json`: `base`
/// (an isolated account's file, already written there) or else `~/.claude/settings.json`, with
/// the status line `wrapper` and kindric's `hooks`, each when given. `None` leaves the session as
/// before, with the reason in `warnings` when something went wrong.
pub fn settings_file(
    paths: &Paths,
    name: &str,
    base: Option<&Path>,
    wrapper: Option<&str>,
    hooks: &KindricHooks,
    warnings: &mut Vec<String>,
) -> Option<PathBuf> {
    if wrapper.is_none() && hooks.is_empty() {
        return None;
    }
    let source = base.map_or_else(|| paths.shared_root.join("settings.json"), Path::to_path_buf);
    let text = match std::fs::read_to_string(&source) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            warnings.push(format!("usage of {name} not followed: cannot read {}: {error}", source.display()));
            return None;
        },
    };
    let (content, problem) = with_additions(text.as_deref(), wrapper, hooks);
    if let Some(problem) = problem {
        warnings.push(format!("usage of {name} not followed: {}: {problem}", source.display()));
        return None;
    }
    let file = paths.shared_settings_file(name);
    let written = file.parent().map_or(Ok(()), files::create_private_dir).and_then(|()| files::write_atomic(&file, &content));
    match written {
        Ok(()) => Some(file),
        Err(error) => {
            warnings.push(format!("usage of {name} not followed: cannot write {}: {error}", file.display()));
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::files::test_dir;

    fn setup(label: &str, accounts: &[&str]) -> (PathBuf, Paths) {
        let root = test_dir::fresh(&format!("usage-{label}"));
        let paths = Paths::from_roots(root.join("config"), root.join("data"), root.join("home").join(".claude"));
        files::create_private_dir(&paths.config_dir).unwrap();
        for account in accounts {
            files::append_line(&paths.registry_file(), account).unwrap();
        }
        (root, paths)
    }

    const INPUT: &str = r#"{"model":{"id":"x"},"rate_limits":{"five_hour":{"used_percentage":34.5,"resets_at":1794409800},"seven_day":{"used_percentage":16,"resets_at":1794800000}}}"#;

    #[test]
    fn the_figures_are_read_from_the_status_line_input() {
        let usage = usage_of(INPUT.as_bytes(), 1_794_400_000).unwrap();
        assert_eq!(usage.five_hour, Some(Window { used_percentage: 34.5, resets_at: Some(1_794_409_800) }));
        assert_eq!(usage.seven_day, Some(Window { used_percentage: 16.0, resets_at: Some(1_794_800_000) }));
        assert_eq!(usage.updated_at, 1_794_400_000);
        let reset = usage_of(br#"{"rate_limits":{"seven_day":{"used_percentage":3}}}"#, 0).unwrap();
        assert_eq!((reset.five_hour, reset.seven_day.map(|window| window.used_percentage)), (None, Some(3.0)));
        assert_eq!(usage_of(br#"{"model":{}}"#, 0), None, "no limits: billed by API key");
        assert_eq!(usage_of(b"{broken", 0), None);
    }

    #[test]
    fn only_a_recent_kindric_accounts_gets_the_status_line() {
        assert_eq!(version_of("kindric-accounts 2.1.0\n"), Some((2, 1, 0)));
        assert!(version_of("kindric-accounts 2.0.0").is_some_and(|version| version < STATUSLINE_SINCE));
        assert!(version_of("kindric-accounts 2.10.3").is_some_and(|version| version >= STATUSLINE_SINCE));
        assert_eq!(version_of("claude 2.1.0"), None);
        assert_eq!(version_of("kindric-accounts 2.1"), None);
    }

    #[test]
    fn a_window_counts_until_it_is_reset() {
        let window = Window { used_percentage: 34.0, resets_at: Some(100) };
        assert_eq!(percent(Some(&window), 99), 34.0);
        assert_eq!(percent(Some(&window), 100), 0.0);
        assert_eq!(percent(None, 0), 0.0);
    }

    #[test]
    fn figures_are_written_for_an_account_of_the_list_only() {
        let (root, paths) = setup("record", &["work"]);
        let work = paths.account_dir("work").into_os_string();
        record(&paths, Some(&work), INPUT.as_bytes(), 1_794_400_000).unwrap();
        assert_eq!(read(&paths, "work").and_then(|usage| usage.five_hour).map(|window| window.used_percentage), Some(34.5));

        let stranger = root.join("elsewhere").join("work").into_os_string();
        let other = paths.account_dir("other").into_os_string();
        for config_dir in [Some(stranger.as_os_str()), Some(other.as_os_str()), None] {
            record(&paths, config_dir, INPUT.as_bytes(), 1).unwrap();
        }
        assert_eq!(read(&paths, "work").map(|usage| usage.updated_at), Some(1_794_400_000));
        assert_eq!(read(&paths, "other"), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_unread_hook_comes_after_the_users_own_hooks() {
        let settings = r#"{"hooks":{"PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"mine"}]}],"Stop":[]}}"#;
        let hooks = KindricHooks { unread: Some("\"/bin/kindric\" unread".into()), away: Some("\"/bin/kindric\" away-hook".into()) };
        let (text, problem) = with_additions(Some(settings), None, &hooks);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(problem, None);
        assert_eq!(value["hooks"]["PostToolUse"][0]["hooks"][0]["command"], "mine");
        assert_eq!(value["hooks"]["PostToolUse"][1]["hooks"][0]["command"], "\"/bin/kindric\" unread");
        assert_eq!(value["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"], "\"/bin/kindric\" unread");
        assert_eq!(value["hooks"]["Stop"][0]["hooks"][0]["command"], "\"/bin/kindric\" away-hook");
        assert_eq!(value["hooks"]["Stop"].as_array().map(Vec::len), Some(1));
        assert!(value.get("statusLine").is_none(), "no status line asked");
        let (odd, _) = with_additions(Some(r#"{"hooks":"not hooks"}"#), None, &hooks);
        assert_eq!(serde_json::from_str::<Value>(&odd).unwrap()["hooks"], "not hooks", "left as it is");
    }

    #[test]
    fn the_hook_alone_is_enough_for_a_settings_file() {
        let (root, paths) = setup("hook-only", &["work"]);
        let mut warnings = Vec::new();
        let hooks = KindricHooks { unread: Some("k unread".into()), away: None };
        let file = settings_file(&paths, "work", None, None, &hooks, &mut warnings).unwrap();
        let value: Value = serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
        assert_eq!(value["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"], "k unread");
        assert!(value.get("statusLine").is_none());
        assert!(value["hooks"].get("Stop").is_none(), "no away hook given");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_status_line_is_replaced_and_the_rest_kept() {
        let settings = r#"{"model":"opus","statusLine":{"type":"command","command":"node hud.mjs","padding":0}}"#;
        let (text, problem) = with_statusline(Some(settings), "\"/bin/kindric-accounts\" statusline");
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(problem, None);
        assert_eq!(value["model"], "opus");
        assert_eq!(value["statusLine"]["command"], "\"/bin/kindric-accounts\" statusline");
        assert_eq!(value["statusLine"]["padding"], 0);
        let (text, _) = with_statusline(None, "k statusline");
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["statusLine"]["type"], "command");
        assert!(with_statusline(Some("{broken"), "k statusline").1.is_some());
    }

    #[test]
    fn a_session_gets_its_own_settings_file_with_the_wrapper() {
        let (root, paths) = setup("settings", &["work"]);
        let mut warnings = Vec::new();
        assert_eq!(settings_file(&paths, "work", None, None, &KindricHooks::default(), &mut warnings), None, "no wrapper, nothing written");
        assert!(!paths.shared_settings_file("work").exists());

        fs::create_dir_all(&paths.shared_root).unwrap();
        fs::write(paths.shared_root.join("settings.json"), r#"{"model":"opus"}"#).unwrap();
        let file = settings_file(&paths, "work", None, Some("k statusline"), &KindricHooks::default(), &mut warnings).unwrap();
        assert_eq!(file, paths.shared_settings_file("work"));
        let value: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!((value["model"].as_str(), value["statusLine"]["command"].as_str()), (Some("opus"), Some("k statusline")));

        // An isolated account's file keeps its rules.
        fs::write(&file, r#"{"permissions":{"deny":["Read(//x/**)"]}}"#).unwrap();
        settings_file(&paths, "work", Some(&file), Some("k statusline"), &KindricHooks::default(), &mut warnings).unwrap();
        let value: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(value["permissions"]["deny"][0], "Read(//x/**)");
        assert_eq!(value["statusLine"]["command"], "k statusline");

        fs::write(paths.shared_root.join("settings.json"), "{broken").unwrap();
        assert_eq!(settings_file(&paths, "work", None, Some("k statusline"), &KindricHooks::default(), &mut warnings), None);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_user_status_line_gets_the_same_input() {
        let (root, paths) = setup("run", &[]);
        assert_eq!(run_user_statusline(&paths, b"x").unwrap(), None, "no status line of the user");
        fs::create_dir_all(&paths.shared_root).unwrap();
        fs::write(paths.shared_root.join("settings.json"), r#"{"statusLine":{"type":"command","command":"cat; printf ' seen'"}}"#).unwrap();
        assert_eq!(run_user_statusline(&paths, b"line").unwrap(), Some((b"line seen".to_vec(), 0)));
        fs::remove_dir_all(root).unwrap();
    }
}
