use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub use kindric_accounts::registry::DEFAULT_ACCOUNT;
pub use kindric_accounts::status::AccountStatus;
use kindric_accounts::agent::{self, Agent};
use kindric_accounts::provider::{self, Kind, Provider};
use kindric_accounts::{Paths, account, codex, launch, memory, registry, status, usage};
use serde::{Deserialize, Serialize};

use crate::progress;
use crate::task_list;

pub(crate) const LAUNCHER: &str = "kindric-accounts";
/// 2.2.0: `token`, the key of the accounts whose model comes from an API key or a gateway.
pub const MIN_LAUNCHER_VERSION: (u32, u32, u32) = (2, 3, 0);

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
    /// How far it is in its task list (see `task_list`), while a task is left to do.
    pub tasks: Option<task_list::TaskProgress>,
    /// What the session waits for from the user, in Claude Code's words ("permission prompt",
    /// "input needed"), while a question of Claude Code is on its screen.
    pub waiting_for: Option<String>,
    /// The program of its account: Claude Code, or Codex, whose sessions are attached and started
    /// otherwise.
    pub agent: Agent,
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

/// `file` is what Claude Code's file for the process says: `agents --json` reports `busy` both
/// for a turn running and for a turn over with only background shells left.
fn state_of(raw: &RawSession, file: &SessionFile) -> SessionState {
    if raw.kind.as_deref() != Some("background") {
        return SessionState::Elsewhere;
    }
    if raw.pid.is_none() {
        return SessionState::Paused;
    }
    match raw.status.as_deref() {
        Some("busy") if file.status.as_deref() != Some("shell") => SessionState::Working,
        _ => SessionState::Waiting,
    }
}

/// Parses the output of `claude agents --json` for one account.
pub fn parse_agents(account: &str, json: &str) -> serde_json::Result<Vec<SessionInfo>> {
    parse_agents_with(account, json, |_| SessionFile::default())
}

/// What Claude Code's own file for a running process, `sessions/<pid>.json`, says.
/// - `status`: `busy` (a turn is running), `shell` (the turn is over, background shells still
///   run: a server, a database, `docker compose up`), `idle`, or `waiting` (a question of
///   Claude Code is on the screen).
/// - `waitingFor`: only while waiting, what for ("permission prompt", "input needed").
#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
struct SessionFile {
    status: Option<String>,
    #[serde(rename = "waitingFor")]
    waiting_for: Option<String>,
}

/// The file of the process `pid` of the account in `account_dir`. One that cannot be read says
/// nothing: the session is then as `agents --json` says.
fn read_session_file(account_dir: &Path, pid: u32) -> SessionFile {
    let file = account_dir.join("sessions").join(format!("{pid}.json"));
    std::fs::read_to_string(file).ok().and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

fn parse_agents_with(account: &str, json: &str, session_file: impl Fn(u32) -> SessionFile) -> serde_json::Result<Vec<SessionInfo>> {
    let raw: Vec<RawSession> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .map(|raw| {
            let file = raw.pid.map(&session_file).unwrap_or_default();
            SessionInfo {
                account: account.to_string(),
                state: state_of(&raw, &file),
                short_id: raw.id,
                session_id: raw.session_id,
                name: raw.name.unwrap_or_default(),
                cwd: raw.cwd.unwrap_or_default(),
                isolated: false,
                progress: None,
                tasks: None,
                waiting_for: file.waiting_for,
                agent: Agent::ClaudeCode,
            }
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

/// How much of its limits an account used at `now`, from the last figures its status line wrote
/// down (see `kindric_accounts::usage`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AccountUsage {
    pub name: String,
    pub five_hour: f64,
    pub seven_day: f64,
    /// When the five-hour window starts again, Unix seconds; `None` when unknown or already past.
    pub five_hour_resets_at: Option<i64>,
    /// When the figures were written down, Unix seconds.
    pub updated_at: i64,
}

/// The figures of each of `names` that has some.
pub fn usages(paths: &Paths, names: &[String], now: i64) -> Vec<AccountUsage> {
    names
        .iter()
        .filter_map(|name| {
            let figures = usage::read(paths, name)?;
            Some(AccountUsage {
                name: name.clone(),
                five_hour: usage::percent(figures.five_hour.as_ref(), now),
                seven_day: usage::percent(figures.seven_day.as_ref(), now),
                five_hour_resets_at: figures.five_hour.and_then(|window| window.resets_at).filter(|resets_at| *resets_at > now),
                updated_at: figures.updated_at,
            })
        })
        .collect()
}

/// A session, by its account and the id Claude Code gives it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct SessionRef {
    pub account: String,
    pub session_id: String,
}

/// How full the context window of each of `sessions` was when its status line last ran, by
/// session id; a session without figures is left out.
pub fn contexts(paths: &Paths, sessions: &[SessionRef]) -> BTreeMap<String, f64> {
    sessions
        .iter()
        .filter_map(|session| {
            let context = usage::read_context(paths, &session.account, &session.session_id)?;
            Some((session.session_id.clone(), context.used_percentage))
        })
        .collect()
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

/// The arguments of `claude agents`. Without `--all`, Claude Code 2.1.322 lists only the
/// sessions that have a process: a stopped one (hidden, or ended) would be found nowhere.
fn agents_args(include_stopped: bool) -> &'static [&'static str] {
    if include_stopped { &["agents", "--json", "--all"] } else { &["agents", "--json"] }
}

/// `claude agents --json` on one account, with the environment rules of
/// `kindric-accounts run`. The shared configuration is not linked again here: relinking
/// removes then recreates the links, and this runs at every refresh. The implicit
/// `default` runs without `CLAUDE_CONFIG_DIR`: setting it to `~/.claude` would make Claude
/// Code read `~/.claude/.claude.json` instead of its real `~/.claude.json`.
fn account_sessions(paths: &Paths, account: &str, implicit_default: bool, include_stopped: bool) -> io::Result<Vec<SessionInfo>> {
    let dir = if is_implicit_default(account, implicit_default) {
        paths.shared_root.clone()
    } else {
        launch::require_account(paths, account).map_err(library_error)?
    };
    if !is_implicit_default(account, implicit_default) && is_codex(paths, account) {
        return codex_sessions(account, &dir, include_stopped);
    }
    let claude = launch::require_claude().map_err(library_error)?;
    let mut command = claude_command(claude);
    if is_implicit_default(account, implicit_default) {
        command.env_remove("CLAUDE_CONFIG_DIR");
    } else {
        command.env("CLAUDE_CONFIG_DIR", &dir);
    }
    let output = command.args(agents_args(include_stopped)).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!("claude agents --json: {}", stderr.trim())));
    }
    let json = String::from_utf8_lossy(&output.stdout);
    let mut sessions =
        parse_agents_with(account, &json, |pid| read_session_file(&dir, pid)).map_err(io::Error::other)?;
    for session in &mut sessions {
        session.progress = session.short_id.as_deref().and_then(|short_id| progress::progress(&dir, short_id));
        session.tasks = task_list::progress(&dir, &session.session_id);
    }
    Ok(sessions)
}

/// A session of Codex as kindric shows it. Its id serves both to attach (`codex resume`) and to
/// address it. Waiting for a permission or an answer reads as Claude Code's words, so it asks the
/// user the same way.
fn codex_session(account: &str, thread: codex::Thread) -> SessionInfo {
    let (state, waiting_for) = match thread.status {
        codex::ThreadStatus::Working => (SessionState::Working, None),
        codex::ThreadStatus::Idle | codex::ThreadStatus::SystemError => (SessionState::Waiting, None),
        codex::ThreadStatus::WaitingOnApproval => (SessionState::Waiting, Some("permission prompt".to_string())),
        codex::ThreadStatus::WaitingOnUserInput => (SessionState::Waiting, Some("input needed".to_string())),
        codex::ThreadStatus::NotLoaded => (SessionState::Paused, None),
    };
    SessionInfo {
        account: account.to_string(),
        short_id: Some(thread.id.clone()),
        session_id: thread.id,
        name: thread.name.unwrap_or_default(),
        cwd: thread.cwd,
        state,
        isolated: false,
        progress: None,
        tasks: None,
        waiting_for,
        agent: Agent::Codex,
    }
}

/// The sessions of the Codex account in `dir`, read from its daemon (see `codex::sessions`).
fn codex_sessions(account: &str, dir: &Path, include_stopped: bool) -> io::Result<Vec<SessionInfo>> {
    let threads = codex::sessions(dir, include_stopped)?;
    Ok(threads.into_iter().map(|thread| codex_session(account, thread)).collect())
}

/// The snapshot without the sessions of Codex accounts, for the terminal interface: frozen, it
/// gets no new kind of session.
pub fn without_codex(mut snapshot: Snapshot) -> Snapshot {
    snapshot.sessions.retain(|session| session.agent != Agent::Codex);
    snapshot
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

/// Login state, plan, sessions and isolation of every account of kindric-accounts, queried in
/// parallel: the accounts of the settings.
pub fn statuses(paths: &Paths) -> io::Result<Vec<AccountStatus>> {
    let names = registry::list_accounts(paths).map_err(library_error)?;
    status::collect(paths, &names).map_err(library_error)
}

/// Creates the account `name`, as `kindric-accounts add` does; returns what it warned about
/// while linking the shared setup.
pub fn add_account(paths: &Paths, name: &str) -> io::Result<Vec<String>> {
    account::add(paths, name).map(|created| created.warnings).map_err(library_error)
}

/// Where a new account's model comes from, when it is not a Claude login: `kind` among
/// `provider::Kind::NAMES`, the address and the model as the user typed them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct NewProvider {
    pub kind: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

fn provider_of(new: &NewProvider) -> io::Result<Provider> {
    let Some(kind) = Kind::parse(new.kind.trim()) else {
        return Err(io::Error::other(format!("unknown provider {:?}: use {}", new.kind, Kind::NAMES.join(", "))));
    };
    Provider::new(kind, new.url.as_deref(), new.model.as_deref()).map_err(library_error)
}

/// Creates the account `name` whose requests go to `provider`, and keeps `key` if one is given,
/// as `kindric-accounts add --provider` then `token` do. Nothing is created when the provider or
/// the key is refused: they are checked first.
pub fn add_provider_account(paths: &Paths, name: &str, new: &NewProvider, key: Option<&str>) -> io::Result<Vec<String>> {
    let provider = provider_of(new)?;
    let key = key.map(str::trim).filter(|key| !key.is_empty());
    if let Some(key) = key
        && key.chars().any(char::is_control)
    {
        return Err(io::Error::other("the key holds a line break or a control character: paste it on a single line"));
    }
    let created = account::add_with(paths, name, Some(&provider)).map_err(library_error)?;
    if let Some(key) = key {
        provider::save_token(&created.dir, key).map_err(library_error)?;
    }
    Ok(created.warnings)
}

/// Keeps `key` for the provider account `name`, in place of the one it had.
pub fn set_account_key(paths: &Paths, name: &str, key: &str) -> io::Result<()> {
    let dir = launch::require_account(paths, name).map_err(library_error)?;
    if is_codex(paths, name) {
        return Err(io::Error::other(format!("{name} is a Codex account: Codex keeps its own key, give it with log in")));
    }
    if provider::read(&dir).map_err(library_error)?.is_none() {
        return Err(io::Error::other(format!("{name} is a Claude account: it signs in with log in, not with a key")));
    }
    provider::save_token(&dir, key).map_err(library_error)
}

/// Whether `name` is an account of kindric-accounts that runs Codex; an unreadable file counts as
/// Claude Code, which every older account runs.
pub fn is_codex(paths: &Paths, name: &str) -> bool {
    agent::read(&paths.account_dir(name)).is_ok_and(|agent| agent == Agent::Codex)
}

/// `openrouter qwen/qwen3-coder` for each of `names` whose model comes from a provider, `codex` for
/// a Codex account: what the overview shows in place of the limits of a Claude subscription. Read
/// from small files, no call to Claude Code.
pub fn provider_labels(paths: &Paths, names: &[String]) -> BTreeMap<String, String> {
    names
        .iter()
        .filter_map(|name| {
            if is_codex(paths, name) {
                return Some((name.clone(), Agent::Codex.name().to_string()));
            }
            let provider = provider::read(&paths.account_dir(name)).ok()??;
            let label = match &provider.model {
                Some(model) => format!("{} {model}", provider.provider.name()),
                None => provider.provider.name().to_string(),
            };
            Some((name.clone(), label))
        })
        .collect()
}

/// Takes `name` off the list of accounts; its directory stays.
pub fn forget_account(paths: &Paths, name: &str) -> io::Result<()> {
    account::forget(paths, name).map_err(library_error)
}

/// The size, in KB, above which sessions are warned about the memory they open at startup; the
/// default when the file that sets it cannot be used.
pub fn memory_warning_kb(paths: &Paths) -> u64 {
    memory::warning_bytes(paths).0 / 1024
}

/// Sets that size for the sessions started from now on.
pub fn set_memory_warning_kb(paths: &Paths, kb: u64) -> io::Result<()> {
    memory::set_warning_kb(paths, kb)
}

/// Queries `accounts` in parallel; `include_stopped` adds the sessions that have no process.
pub fn query_accounts(paths: &Paths, accounts: &[String], implicit_default: bool, include_stopped: bool) -> Vec<Queried> {
    let handles: Vec<_> = accounts
        .iter()
        .cloned()
        .map(|account| {
            let paths = paths.clone();
            thread::spawn(move || {
                let start = Instant::now();
                let found =
                    account_sessions(&paths, &account, implicit_default, include_stopped).map_err(|error| error.to_string());
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

/// Sessions of every account that have a process, `default` included when logged in, accounts
/// queried in parallel.
pub fn snapshot() -> io::Result<Snapshot> {
    let paths = paths()?;
    let (registry, isolated) = registered(&paths)?;
    let (accounts, implicit_default) = with_default(registry, default_logged_in());
    let queried = query_accounts(&paths, &accounts, implicit_default, false);
    let timings = queried.iter().map(|query| (query.account.clone(), query.took)).collect();
    let found = queried.into_iter().map(|query| (query.account, query.found)).collect();
    let mut snapshot = assemble(accounts, isolated, &found, timings);
    snapshot.implicit_default = implicit_default;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Accounts under a fresh folder, nothing read from the environment.
    fn scratch_paths(label: &str) -> (PathBuf, Paths) {
        scratch_paths_in(&std::env::temp_dir(), label)
    }

    /// The account directories under `base`. A Codex account needs a short path: the socket of its
    /// daemon must fit in 103 bytes, which the temporary folder of macOS leaves no room for.
    fn scratch_paths_in(base: &Path, label: &str) -> (PathBuf, Paths) {
        let root = base.join(format!("kindric-accounts-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        std::fs::create_dir_all(&home).expect("home");
        let paths = Paths::from_roots(root.join("config"), root.join("data"), home);
        (root, paths)
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("folder");
        std::fs::write(path, text).expect("write");
    }

    #[test]
    fn the_five_hour_reset_is_given_until_it_has_passed() {
        let (root, paths) = scratch_paths("resets");
        let window = |resets_at: i64| format!(r#"{{"used_percentage": 52.0, "resets_at": {resets_at}}}"#);
        write(&usage::usage_file(&paths, "louis"), &format!(r#"{{"five_hour": {}, "seven_day": null, "updated_at": 1}}"#, window(2_000)));
        let names = ["louis".to_string()];
        assert_eq!(usages(&paths, &names, 1_000)[0].five_hour_resets_at, Some(2_000));
        assert_eq!(usages(&paths, &names, 3_000)[0].five_hour_resets_at, None, "a reset already past is not shown");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_context_of_each_session_is_read_and_one_without_figures_left_out() {
        let (root, paths) = scratch_paths("contexts");
        let context = usage::context_file(&paths, "louis", "aaaa-1111").expect("a session id");
        write(&context, r#"{"used_percentage": 41.5, "updated_at": 1}"#);
        let session = |account: &str, session_id: &str| SessionRef { account: account.into(), session_id: session_id.into() };
        let read = contexts(&paths, &[session("louis", "aaaa-1111"), session("louis", "bbbb-2222"), session("pro", "aaaa-1111")]);
        assert_eq!(read.into_iter().collect::<Vec<_>>(), [("aaaa-1111".to_string(), 41.5)]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_added_account_is_listed_and_a_forgotten_one_keeps_its_directory() {
        let (root, paths) = scratch_paths("add-forget");
        add_account(&paths, "essai").expect("add");
        assert_eq!(registry::list_accounts(&paths).expect("list"), ["essai"]);
        let dir = paths.account_dir("essai");
        assert!(dir.is_dir());
        forget_account(&paths, "essai").expect("forget");
        assert!(registry::list_accounts(&paths).expect("list").is_empty());
        assert!(dir.is_dir(), "the directory stays");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_provider_account_keeps_its_provider_and_key_and_gets_a_label() {
        let (root, paths) = scratch_paths("add-provider");
        let router = NewProvider { kind: "openrouter".into(), url: None, model: Some("qwen/qwen3-coder".into()) };
        add_provider_account(&paths, "router", &router, Some(" sk-or-1 ")).expect("add");
        add_account(&paths, "plain").expect("add");
        let dir = paths.account_dir("router");
        assert_eq!(provider::read_token(&dir).expect("token").as_deref(), Some("sk-or-1"));
        let names = registry::list_accounts(&paths).expect("list");
        let labels = provider_labels(&paths, &names);
        assert_eq!(labels.into_iter().collect::<Vec<_>>(), [("router".to_string(), "openrouter qwen/qwen3-coder".to_string())]);
        set_account_key(&paths, "router", "sk-or-2").expect("new key");
        assert_eq!(provider::read_token(&dir).expect("token").as_deref(), Some("sk-or-2"));
        let claude = set_account_key(&paths, "plain", "x").expect_err("a Claude account").to_string();
        assert!(claude.contains("log in"), "{claude}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_codex_session_reads_as_a_session_of_kindric() {
        let thread = |status| codex::Thread { id: "01a1ff4a".into(), name: None, cwd: "/home/u/api".into(), status };
        let session = codex_session("cx", thread(codex::ThreadStatus::Working));
        assert_eq!((session.short_id.as_deref(), session.session_id.as_str()), (Some("01a1ff4a"), "01a1ff4a"));
        assert_eq!((session.agent, session.title()), (Agent::Codex, "api".to_string()));
        assert!(session.can_attach());
        let state = |status| {
            let session = codex_session("cx", thread(status));
            (session.state, session.waiting_for)
        };
        assert_eq!(state(codex::ThreadStatus::Working), (SessionState::Working, None));
        assert_eq!(state(codex::ThreadStatus::Idle), (SessionState::Waiting, None));
        assert_eq!(state(codex::ThreadStatus::SystemError), (SessionState::Waiting, None));
        assert_eq!(state(codex::ThreadStatus::WaitingOnApproval), (SessionState::Waiting, Some("permission prompt".into())));
        assert_eq!(state(codex::ThreadStatus::WaitingOnUserInput), (SessionState::Waiting, Some("input needed".into())));
        assert_eq!(state(codex::ThreadStatus::NotLoaded), (SessionState::Paused, None));
        let named = codex_session("cx", codex::Thread { name: Some("review".into()), ..thread(codex::ThreadStatus::Idle) });
        assert_eq!(named.title(), "review");
    }

    #[test]
    fn a_codex_account_gets_its_label_and_no_claude_query() {
        let base = if cfg!(unix) { PathBuf::from("/tmp") } else { std::env::temp_dir() };
        let (root, paths) = scratch_paths_in(&base, "cx");
        account::add_codex(&paths, "cx").expect("add");
        add_account(&paths, "plain").expect("add");
        assert!(is_codex(&paths, "cx"));
        assert!(!is_codex(&paths, "plain"));
        let names = registry::list_accounts(&paths).expect("list");
        let labels = provider_labels(&paths, &names);
        assert_eq!(labels.into_iter().collect::<Vec<_>>(), [("cx".to_string(), "codex".to_string())]);
        // No claude is called for it: an empty list even where claude is missing.
        assert_eq!(account_sessions(&paths, "cx", false, false).expect("no session yet"), Vec::new());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_refused_provider_or_key_creates_nothing() {
        let (root, paths) = scratch_paths("add-provider-refused");
        let unknown = NewProvider { kind: "gpt".into(), url: None, model: Some("m".into()) };
        assert!(add_provider_account(&paths, "a", &unknown, None).is_err());
        let no_model = NewProvider { kind: "ollama".into(), url: None, model: None };
        assert!(add_provider_account(&paths, "b", &no_model, None).is_err());
        let ollama = NewProvider { kind: "ollama".into(), url: None, model: Some("qwen3".into()) };
        assert!(add_provider_account(&paths, "c", &ollama, Some("two\nlines")).is_err());
        assert!(registry::list_accounts(&paths).expect("list").is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn adding_twice_default_or_a_bad_name_is_an_error_saying_why() {
        let (root, paths) = scratch_paths("add-errors");
        add_account(&paths, "essai").expect("add");
        let twice = add_account(&paths, "essai").expect_err("twice").to_string();
        assert!(twice.contains("already exists"), "{twice}");
        let default = add_account(&paths, DEFAULT_ACCOUNT).expect_err("default").to_string();
        assert!(default.contains("reserved"), "{default}");
        assert!(add_account(&paths, "../out").is_err());
        assert!(forget_account(&paths, "absent").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

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

    /// What `claude agents --json --all` gave on 2026-11-12 (Claude Code 2.1.322) for a stopped
    /// session: no pid, no status. It must come out paused, the state Ctrl+P finds.
    #[test]
    fn stopped_sessions_are_asked_for_and_come_out_paused() {
        assert_eq!(agents_args(true), ["agents", "--json", "--all"]);
        assert_eq!(agents_args(false), ["agents", "--json"]);
        let stopped = r#"[{"cwd":"/home/u/p","id":"61f9b7da","kind":"background","name":"e2e-c",
            "sessionId":"61f9b7da-1390","startedAt":1794487000000,"state":"stopped"}]"#;
        let sessions = parse_agents("lystriq", stopped).unwrap();
        assert_eq!(sessions[0].state, SessionState::Paused);
        assert_eq!(sessions[0].short_id.as_deref(), Some("61f9b7da"));
    }

    /// A turn that ended with a server left running in the background: `busy` in `agents --json`.
    #[test]
    fn a_turn_over_with_only_shells_left_is_not_working() {
        let shell = |pid| SessionFile { status: (pid == 170869).then(|| "shell".to_string()), waiting_for: None };
        let sessions = parse_agents_with("louis", AGENTS, shell).unwrap();
        assert_eq!(sessions[0].state, SessionState::Waiting);
        assert_eq!(sessions[3].state, SessionState::Elsewhere);
    }

    /// What Claude Code 2.1.319 wrote for a session whose turn was over with `sleep 600` left
    /// running in the background, and for one in the middle of its turn.
    #[test]
    fn the_session_file_says_shell_once_only_shells_run() {
        let root = std::env::temp_dir().join(format!("kindric-only-shells-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        let file = |pid: u32, status: &str| {
            let json = format!(r#"{{"pid":{pid},"kind":"bg","jobId":"2b388db2","status":"{status}","updatedAt":1794222092298}}"#);
            std::fs::write(root.join("sessions").join(format!("{pid}.json")), json).unwrap();
        };
        file(1892552, "shell");
        file(1770477, "busy");
        assert_eq!(read_session_file(&root, 1892552).status.as_deref(), Some("shell"));
        assert_eq!(read_session_file(&root, 1770477).status.as_deref(), Some("busy"));
        assert_eq!(read_session_file(&root, 4242), SessionFile::default(), "no file: as agents --json says");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// What Claude Code 2.1.325 wrote on 2026-11-16 for a session asking a permission, and for
    /// one asking a question (AskUserQuestion): `waitingFor` is there only while it waits.
    #[test]
    fn a_session_waiting_for_the_user_says_what_for() {
        let root = std::env::temp_dir().join(format!("kindric-waiting-for-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        let write = |pid: u32, json: &str| std::fs::write(root.join("sessions").join(format!("{pid}.json")), json).unwrap();
        write(170869, r#"{"pid":170869,"kind":"bg","status":"waiting","waitingFor":"permission prompt","statusUpdatedAt":1794832127865}"#);
        write(190760, r#"{"pid":190760,"kind":"bg","status":"waiting","waitingFor":"input needed","statusUpdatedAt":1794832128000}"#);
        write(188510, r#"{"pid":188510,"kind":"interactive","status":"busy","statusUpdatedAt":1794832126353}"#);
        let sessions = parse_agents_with("louis", AGENTS, |pid| read_session_file(&root, pid)).unwrap();
        let waiting: Vec<_> = sessions.iter().map(|s| s.waiting_for.as_deref()).collect();
        assert_eq!(waiting, [Some("permission prompt"), None, Some("input needed"), None]);
        assert_eq!(sessions[0].state, SessionState::Working, "agents --json says busy, the turn is not over");
        std::fs::remove_dir_all(root).unwrap();
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
        assert_eq!(dir_of(&paths, "default", false), PathBuf::from("/d/kindric-accounts/default"));
        assert_eq!(dir_of(&paths, "louis", true), PathBuf::from("/d/kindric-accounts/louis"));
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
        assert!(parse_version("kindric-accounts 2.1.9").unwrap() < MIN_LAUNCHER_VERSION);
        assert!(parse_version("kindric-accounts 2.10.0").unwrap() > MIN_LAUNCHER_VERSION);
    }
}
