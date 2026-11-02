use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub use kindric_accounts::registry::DEFAULT_ACCOUNT;
use kindric_accounts::{Paths, launch, registry};
use serde::Deserialize;

use crate::progress;

pub(crate) const LAUNCHER: &str = "kindric-accounts";
pub const MIN_LAUNCHER_VERSION: (u32, u32, u32) = (1, 2, 0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Working,
    Waiting,
    Paused,
    Elsewhere,
}

impl SessionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Paused => "paused",
            Self::Elsewhere => "elsewhere",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub account: String,
    /// Short id from `claude --bg`; only background sessions have one.
    pub short_id: Option<String>,
    pub session_id: String,
    pub name: String,
    pub cwd: String,
    pub state: SessionState,
    /// Account isolated by `kindric-accounts isolate`: kept out of messages.
    pub isolated: bool,
    /// What the session last said it is doing (see `progress`); background sessions only.
    pub progress: Option<String>,
}

impl SessionInfo {
    pub fn can_attach(&self) -> bool {
        self.short_id.is_some()
    }

    /// `name` if set, else the last directory of `cwd`.
    pub fn title(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        Path::new(&self.cwd)
            .file_name()
            .map_or_else(|| self.cwd.clone(), |dir| dir.to_string_lossy().into_owned())
    }
}

#[derive(Deserialize)]
struct RawSession {
    id: Option<String>,
    #[serde(rename = "sessionId", default)]
    session_id: String,
    name: Option<String>,
    kind: Option<String>,
    status: Option<String>,
    pid: Option<u32>,
    cwd: Option<String>,
}

fn state_of(raw: &RawSession) -> SessionState {
    if raw.kind.as_deref() != Some("background") {
        return SessionState::Elsewhere;
    }
    if raw.pid.is_none() {
        return SessionState::Paused;
    }
    match raw.status.as_deref() {
        Some("busy") => SessionState::Working,
        _ => SessionState::Waiting,
    }
}

/// Parses the output of `claude agents --json` for one account.
pub fn parse_agents(account: &str, json: &str) -> serde_json::Result<Vec<SessionInfo>> {
    let raw: Vec<RawSession> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .map(|raw| SessionInfo {
            account: account.to_string(),
            state: state_of(&raw),
            short_id: raw.id,
            session_id: raw.session_id,
            name: raw.name.unwrap_or_default(),
            cwd: raw.cwd.unwrap_or_default(),
            isolated: false,
            progress: None,
        })
        .collect())
}

/// Parses `kindric-accounts 1.2.3` into (1, 2, 3).
pub fn parse_version(output: &str) -> Option<(u32, u32, u32)> {
    let version = output.trim().strip_prefix("kindric-accounts ")?;
    let mut parts = version.split('.').map(str::parse::<u32>);
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(Ok(major)), Some(Ok(minor)), Some(Ok(patch)), None) => Some((major, minor, patch)),
        _ => None,
    }
}

fn launcher_output(args: &[&str]) -> io::Result<String> {
    let output = Command::new(LAUNCHER).args(args).stdin(Stdio::null()).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!("{LAUNCHER} {}: {}", args.join(" "), stderr.trim())));
    }
    String::from_utf8(output.stdout).map_err(io::Error::other)
}

/// The version line of kindric-accounts, `kindric-accounts 2.0.0`, when it is recent enough.
pub fn check_launcher_version() -> io::Result<String> {
    let output = launcher_output(&["--version"])?;
    let version = parse_version(&output)
        .ok_or_else(|| io::Error::other(format!("unexpected version output: {}", output.trim())))?;
    if version < MIN_LAUNCHER_VERSION {
        let (major, minor, patch) = MIN_LAUNCHER_VERSION;
        return Err(io::Error::other(format!(
            "{LAUNCHER} {major}.{minor}.{patch} or later is required, found {}",
            output.trim()
        )));
    }
    Ok(output.trim().to_string())
}

fn library_error(error: kindric_accounts::Error) -> io::Error {
    io::Error::other(error.to_string())
}

pub fn paths() -> io::Result<Paths> {
    Paths::from_env().map_err(library_error)
}

pub fn isolated_accounts() -> io::Result<Vec<String>> {
    registry::list_isolated(&paths()?).map_err(library_error)
}

pub fn mark_isolated(sessions: &mut [SessionInfo], isolated: &[String]) {
    for session in sessions {
        session.isolated = isolated.contains(&session.account);
    }
}

/// Sessions that may take part in messages: never those of an isolated account.
pub fn messaging_sessions(sessions: Vec<SessionInfo>) -> Vec<SessionInfo> {
    sessions.into_iter().filter(|session| !session.isolated).collect()
}

/// `default` joins the accounts, first, when Claude Code's own configuration is logged in
/// and no account of kindric-accounts took the name (older versions allowed it). The flag
/// says whether it was added.
pub fn with_default(mut accounts: Vec<String>, logged_in: bool) -> (Vec<String>, bool) {
    let added = logged_in && !accounts.iter().any(|account| account == DEFAULT_ACCOUNT);
    if added {
        accounts.insert(0, DEFAULT_ACCOUNT.to_string());
    }
    (accounts, added)
}

fn is_implicit_default(account: &str, implicit_default: bool) -> bool {
    implicit_default && account == DEFAULT_ACCOUNT
}

/// Whether `account` is the implicit `default`: that name, and no account of
/// kindric-accounts that took it.
pub fn is_implicit(paths: &Paths, account: &str) -> io::Result<bool> {
    Ok(account == DEFAULT_ACCOUNT && !registry::account_exists(paths, account).map_err(library_error)?)
}

/// What `claude` must not inherit on the implicit `default`: `CLAUDE_CONFIG_DIR`, so that
/// it reads `~/.claude` and `~/.claude.json`, and the billing variables.
pub fn default_removed_env() -> Vec<&'static str> {
    std::iter::once("CLAUDE_CONFIG_DIR").chain(launch::STRIPPED_ENV).collect()
}

/// Where an account keeps Claude Code's files: `~/.claude` for the implicit `default`.
pub fn dir_of(paths: &Paths, account: &str, implicit_default: bool) -> PathBuf {
    if is_implicit_default(account, implicit_default) {
        return paths.shared_root.clone();
    }
    paths.account_dir(account)
}

/// `claude` with the billing variables removed, as `kindric-accounts run` does.
fn claude_command(claude: PathBuf) -> Command {
    let mut command = Command::new(claude);
    for variable in launch::STRIPPED_ENV {
        command.env_remove(variable);
    }
    command.stdin(Stdio::null());
    command
}

pub fn parse_logged_in(json: &str) -> bool {
    #[derive(Deserialize)]
    struct AuthStatus {
        #[serde(rename = "loggedIn", default)]
        logged_in: bool,
    }
    serde_json::from_str::<AuthStatus>(json).is_ok_and(|status| status.logged_in)
}

/// Whether Claude Code's own configuration (`~/.claude`) is logged in, from `claude auth
/// status --json` without `CLAUDE_CONFIG_DIR`. It exits with 1 when logged out but still
/// prints its JSON. Any failure counts as logged out.
pub fn default_logged_in() -> bool {
    let Ok(claude) = launch::require_claude() else {
        return false;
    };
    let mut command = claude_command(claude);
    command.env_remove("CLAUDE_CONFIG_DIR").args(["auth", "status", "--json"]);
    command.output().is_ok_and(|output| parse_logged_in(&String::from_utf8_lossy(&output.stdout)))
}

/// `claude agents --json` on one account, with the environment rules of
/// `kindric-accounts run`. The shared configuration is not linked again here: relinking
/// removes then recreates the links, and this runs at every refresh. The implicit
/// `default` runs without `CLAUDE_CONFIG_DIR`: setting it to `~/.claude` would make Claude
/// Code read `~/.claude/.claude.json` instead of its real `~/.claude.json`.
fn account_sessions(paths: &Paths, account: &str, implicit_default: bool) -> io::Result<Vec<SessionInfo>> {
    let dir = if is_implicit_default(account, implicit_default) {
        paths.shared_root.clone()
    } else {
        launch::require_account(paths, account).map_err(library_error)?
    };
    let claude = launch::require_claude().map_err(library_error)?;
    let mut command = claude_command(claude);
    if is_implicit_default(account, implicit_default) {
        command.env_remove("CLAUDE_CONFIG_DIR");
    } else {
        command.env("CLAUDE_CONFIG_DIR", &dir);
    }
    let output = command.args(["agents", "--json"]).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!("claude agents --json: {}", stderr.trim())));
    }
    let mut sessions = parse_agents(account, &String::from_utf8_lossy(&output.stdout)).map_err(io::Error::other)?;
    for session in &mut sessions {
        session.progress = session.short_id.as_deref().and_then(|short_id| progress::progress(&dir, short_id));
    }
    Ok(sessions)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub accounts: Vec<String>,
    pub isolated: Vec<String>,
    /// Sorted by account, then title, so list positions (Alt+1..9) stay stable.
    pub sessions: Vec<SessionInfo>,
    pub errors: Vec<String>,
    /// How long `claude agents --json` took, by account.
    pub timings: Vec<(String, Duration)>,
    /// `default` in `accounts` is Claude Code's own configuration, not an account of
    /// kindric-accounts (see `with_default`).
    pub implicit_default: bool,
}

/// The snapshot without the implicit `default`, for the terminal interface: its panes
/// cannot remove `CLAUDE_CONFIG_DIR`, so they could not attach to its sessions.
pub fn without_implicit_default(mut snapshot: Snapshot) -> Snapshot {
    if snapshot.implicit_default {
        snapshot.accounts.retain(|account| account != DEFAULT_ACCOUNT);
        snapshot.sessions.retain(|session| session.account != DEFAULT_ACCOUNT);
        snapshot.timings.retain(|(account, _)| account != DEFAULT_ACCOUNT);
        snapshot.errors.retain(|error| !error.starts_with(&format!("{DEFAULT_ACCOUNT}: ")));
        snapshot.implicit_default = false;
    }
    snapshot
}

pub fn sort_sessions(sessions: &mut [SessionInfo]) {
    sessions.sort_by_cached_key(|session| {
        (session.account.clone(), session.title().to_lowercase(), session.session_id.clone())
    });
}

/// What `claude agents --json` gave for an account: its sessions, or why it failed.
pub type Found = Result<Vec<SessionInfo>, String>;

/// One account queried, and how long it took.
pub struct Queried {
    pub account: String,
    pub took: Duration,
    pub found: Found,
}

/// The accounts registered, then those kept out of messages: two small files.
pub fn registered(paths: &Paths) -> io::Result<(Vec<String>, Vec<String>)> {
    let accounts = registry::list_accounts(paths).map_err(library_error)?;
    let isolated = registry::list_isolated(paths).map_err(library_error)?;
    Ok((accounts, isolated))
}

/// Queries `accounts` in parallel.
pub fn query_accounts(paths: &Paths, accounts: &[String], implicit_default: bool) -> Vec<Queried> {
    let handles: Vec<_> = accounts
        .iter()
        .cloned()
        .map(|account| {
            let paths = paths.clone();
            thread::spawn(move || {
                let start = Instant::now();
                let found = account_sessions(&paths, &account, implicit_default).map_err(|error| error.to_string());
                (start.elapsed(), found)
            })
        })
        .collect();
    accounts
        .iter()
        .zip(handles)
        .map(|(account, handle)| {
            let (took, found) = handle.join().unwrap_or_else(|_| (Duration::ZERO, Err("account query thread panicked".to_string())));
            Queried { account: account.clone(), took, found }
        })
        .collect()
}

/// The list of every account in `accounts`, from what each gave last. An account that failed
/// is reported in `errors` and skipped, so one broken account never hides the others.
pub fn assemble(
    accounts: Vec<String>,
    isolated: Vec<String>,
    found: &BTreeMap<String, Found>,
    timings: Vec<(String, Duration)>,
) -> Snapshot {
    let mut sessions = Vec::new();
    let mut errors = Vec::new();
    for account in &accounts {
        match found.get(account) {
            Some(Ok(listed)) => sessions.extend(listed.iter().cloned()),
            Some(Err(error)) => errors.push(format!("{account}: {error}")),
            None => {},
        }
    }
    mark_isolated(&mut sessions, &isolated);
    sort_sessions(&mut sessions);
    Snapshot { accounts, isolated, sessions, errors, timings, implicit_default: false }
}

/// Sessions of every account, `default` included when logged in, accounts queried in parallel.
pub fn snapshot() -> io::Result<Snapshot> {
    let paths = paths()?;
    let (registry, isolated) = registered(&paths)?;
    let (accounts, implicit_default) = with_default(registry, default_logged_in());
    let queried = query_accounts(&paths, &accounts, implicit_default);
    let timings = queried.iter().map(|query| (query.account.clone(), query.took)).collect();
    let found = queried.into_iter().map(|query| (query.account, query.found)).collect();
    let mut snapshot = assemble(accounts, isolated, &found, timings);
    snapshot.implicit_default = implicit_default;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENTS: &str = r#"[
        {"id":"a40b6e33","name":"boss-fight","kind":"background","status":"busy","pid":170869,
         "cwd":"/home/u/Roblox/frieren-rpg","sessionId":"a40b6e33-cde6"},
        {"id":"bc582e37","name":"Router","kind":"background","cwd":"/home/u","sessionId":"bc582e37-1715"},
        {"id":"378ce649","name":"","kind":"background","status":"idle","pid":190760,
         "cwd":"/home/u/proto","sessionId":"378ce649-5ad1"},
        {"name":"frieren-rpg-8d","kind":"interactive","status":"busy","pid":188510,
         "cwd":"/home/u/frieren","sessionId":"25305016-fd5d"}
    ]"#;

    #[test]
    fn agents_json_maps_to_states() {
        let sessions = parse_agents("louis", AGENTS).unwrap();
        let states: Vec<_> = sessions.iter().map(|s| s.state).collect();
        assert_eq!(
            states,
            [SessionState::Working, SessionState::Paused, SessionState::Waiting, SessionState::Elsewhere]
        );
        assert!(sessions.iter().all(|s| s.account == "louis"));
        assert_eq!(sessions[0].short_id.as_deref(), Some("a40b6e33"));
        assert!(!sessions[3].can_attach());
    }

    #[test]
    fn title_falls_back_to_directory() {
        let sessions = parse_agents("lystriq", AGENTS).unwrap();
        assert_eq!(sessions[0].title(), "boss-fight");
        assert_eq!(sessions[2].title(), "proto");
    }

    #[test]
    fn sessions_sort_by_account_then_title() {
        let mut sessions = parse_agents("pro", AGENTS).unwrap();
        sessions.extend(parse_agents("louis", AGENTS).unwrap());
        sort_sessions(&mut sessions);
        let order: Vec<_> = sessions.iter().map(|s| (s.account.as_str(), s.title())).collect();
        assert_eq!(order[0], ("louis", "boss-fight".to_string()));
        assert_eq!(order[3], ("louis", "Router".to_string()));
        assert_eq!(order[4].0, "pro");
    }

    #[test]
    fn isolated_accounts_are_marked_and_kept_out_of_messages() {
        let mut sessions = parse_agents("louis", AGENTS).unwrap();
        sessions.extend(parse_agents("pro", AGENTS).unwrap());
        mark_isolated(&mut sessions, &["pro".to_string()]);
        assert!(sessions.iter().all(|s| s.isolated == (s.account == "pro")));
        let messaging = messaging_sessions(sessions);
        assert_eq!(messaging.len(), 4);
        assert!(messaging.iter().all(|s| s.account == "louis"));
    }

    #[test]
    fn assembling_keeps_registry_order_marks_isolated_and_reports_failures() {
        let found = BTreeMap::from([
            ("pro".to_string(), Ok(parse_agents("pro", AGENTS).unwrap())),
            ("louis".to_string(), Ok(parse_agents("louis", AGENTS).unwrap())),
            ("broken".to_string(), Err("claude agents --json: no".to_string())),
            ("gone".to_string(), Ok(parse_agents("gone", AGENTS).unwrap())),
        ]);
        let accounts = ["louis", "broken", "pro"].map(String::from).to_vec();
        let snapshot = assemble(accounts.clone(), vec!["pro".to_string()], &found, Vec::new());
        assert_eq!(snapshot.accounts, accounts);
        assert_eq!(snapshot.errors, ["broken: claude agents --json: no"]);
        assert_eq!(snapshot.sessions.len(), 8, "only the accounts registered count");
        assert_eq!(snapshot.sessions[0].account, "louis");
        assert!(snapshot.sessions.iter().all(|s| s.isolated == (s.account == "pro")));
    }

    #[test]
    fn auth_status_says_whether_default_is_logged_in() {
        assert!(parse_logged_in("{\n  \"loggedIn\": true,\n  \"authMethod\": \"claude.ai\"\n}"));
        assert!(!parse_logged_in("{\n  \"loggedIn\": false,\n  \"authMethod\": \"none\"\n}"));
        assert!(!parse_logged_in("{}"));
        assert!(!parse_logged_in(""));
        assert!(!parse_logged_in("not json"));
    }

    #[test]
    fn default_comes_first_only_when_logged_in_and_not_taken() {
        let registry = ["louis", "lystriq"].map(String::from).to_vec();
        assert_eq!(with_default(registry.clone(), false), (registry.clone(), false));
        let (accounts, added) = with_default(registry.clone(), true);
        assert!(added);
        assert_eq!(accounts, ["default", "louis", "lystriq"]);
        let taken = ["louis", "default"].map(String::from).to_vec();
        assert_eq!(with_default(taken.clone(), true), (taken, false), "an account named default wins");
    }

    #[test]
    fn the_implicit_default_lives_in_the_shared_configuration() {
        let paths = Paths::from_roots("/c".into(), "/d".into(), "/h".into());
        assert_eq!(dir_of(&paths, "default", true), PathBuf::from("/h/.claude"));
        assert_eq!(dir_of(&paths, "default", false), PathBuf::from("/d/claude-accounts/default"));
        assert_eq!(dir_of(&paths, "louis", true), PathBuf::from("/d/claude-accounts/louis"));
    }

    #[test]
    fn the_terminal_interface_drops_only_the_implicit_default() {
        let mut sessions = parse_agents("default", AGENTS).unwrap();
        sessions.extend(parse_agents("louis", AGENTS).unwrap());
        let snapshot = Snapshot {
            accounts: ["default", "louis"].map(String::from).to_vec(),
            sessions,
            errors: vec!["default: claude agents --json: no".to_string(), "louis: slow".to_string()],
            timings: vec![("default".to_string(), Duration::ZERO), ("louis".to_string(), Duration::ZERO)],
            implicit_default: true,
            ..Snapshot::default()
        };
        let kept = without_implicit_default(snapshot.clone());
        assert_eq!(kept.accounts, ["louis"]);
        assert!(kept.sessions.iter().all(|s| s.account == "louis"));
        assert_eq!(kept.sessions.len(), 4);
        assert_eq!(kept.errors, ["louis: slow"]);
        assert_eq!(kept.timings.len(), 1);
        let registered = Snapshot { implicit_default: false, ..snapshot.clone() };
        assert_eq!(without_implicit_default(registered.clone()), registered, "an account named default stays");
    }

    #[test]
    fn broken_json_is_an_error() {
        assert!(parse_agents("x", "[{").is_err());
    }

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version("kindric-accounts 1.1.0\n"), Some((1, 1, 0)));
        assert_eq!(parse_version("kindric-accounts 1.0"), None);
        assert_eq!(parse_version("other 1.1.0"), None);
        assert!(parse_version("kindric-accounts 1.0.9").unwrap() < MIN_LAUNCHER_VERSION);
        assert!(parse_version("kindric-accounts 1.10.0").unwrap() > MIN_LAUNCHER_VERSION);
    }
}
