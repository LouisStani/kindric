//! Login state, plan and running sessions of each account.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;

use serde::Serialize;
use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::agent::{self, Agent};
use crate::codex;
use crate::error::Result;
use crate::launch;
use crate::opencode;
use crate::paths::Paths;
use crate::provider;
use crate::registry;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoginState {
    LoggedIn,
    LoggedOut,
    Unknown,
}

impl LoginState {
    pub fn label(self) -> &'static str {
        match self {
            Self::LoggedIn => "logged in",
            Self::LoggedOut => "logged out",
            Self::Unknown => "unknown",
        }
    }
}

/// One entry of `status --json`. Field order and names are part of the 1.x contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AccountStatus {
    pub name: String,
    pub state: LoginState,
    pub sessions: usize,
    pub plan: Option<String>,
    pub dir: PathBuf,
    pub isolated: bool,
    /// When the login must be renewed, ISO UTC as Claude Code gives it (since 2.1.0).
    pub login_expires: Option<String>,
    /// Where the model comes from: `claude` (a Claude login), `api`, `openrouter`, `ollama` or
    /// `gateway` (since 2.2.0).
    pub provider: String,
    /// The model of a provider account, as the provider names it; `null` for Claude (since 2.2.0).
    pub model: Option<String>,
    /// The program the account runs: `claude-code`, `codex` (since 2.3.0) or `opencode` (since 2.4.0).
    pub agent: Agent,
}

/// The provider name of a Claude account in `status --json`.
pub const CLAUDE_PROVIDER: &str = "claude";

/// The provider name of a Codex account: its model comes from OpenAI, through the account's
/// ChatGPT login or API key.
pub const CODEX_PROVIDER: &str = "openai";

/// The provider name of an OpenCode account: OpenCode itself picks among its providers.
pub const OPENCODE_PROVIDER: &str = "opencode";

/// `subscriptionType` of `claude auth status --json`, if it is a plain lowercase word.
pub fn subscription_type(output: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(output).ok()?;
    let plan = value.get("subscriptionType")?.as_str()?;
    let plain = !plan.is_empty() && plan.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'_');
    plain.then(|| plan.to_string())
}

/// `loginExpiresAt` of `claude auth status --json`, when it is a date as Claude Code writes it,
/// `2026-11-13T15:45:31.701Z`.
pub fn login_expires(output: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(output).ok()?;
    let date = value.get("loginExpiresAt")?.as_str()?;
    let bytes = date.as_bytes();
    let shape = |at: usize, expected: u8| bytes.get(at) == Some(&expected);
    let digits = |from: usize, to: usize| bytes.get(from..to).is_some_and(|part| part.iter().all(u8::is_ascii_digit));
    let fraction = bytes.get(19..bytes.len().saturating_sub(1)).is_some_and(|part| {
        part.is_empty() || (part.len() > 1 && part[0] == b'.' && part[1..].iter().all(u8::is_ascii_digit))
    });
    let valid = digits(0, 4) && shape(4, b'-') && digits(5, 7) && shape(7, b'-') && digits(8, 10) && shape(10, b'T')
        && digits(11, 13) && shape(13, b':') && digits(14, 16) && shape(16, b':') && digits(17, 19)
        && fraction && date.ends_with('Z');
    valid.then(|| date.to_string())
}

/// What `claude auth status --json` tells of an account, or what its provider file and key say.
struct Login {
    state: LoginState,
    plan: Option<String>,
    expires: Option<String>,
    provider: String,
    model: Option<String>,
}

impl Login {
    fn claude(state: LoginState, plan: Option<String>, expires: Option<String>) -> Self {
        Self { state, plan, expires, provider: CLAUDE_PROVIDER.into(), model: None }
    }
}

/// A provider account is logged in once it has the key it needs. `claude auth status` cannot tell:
/// it does not see the key helper, which only the session's settings hold.
fn account_login(claude: &Path, dir: &Path) -> Login {
    match provider::read(dir) {
        Ok(None) => login(claude, dir),
        Ok(Some(provider)) => Login {
            state: if provider::is_ready(dir, &provider) { LoginState::LoggedIn } else { LoginState::LoggedOut },
            plan: None,
            expires: None,
            provider: provider.provider.name().into(),
            model: provider.model,
        },
        Err(_) => Login { state: LoginState::Unknown, plan: None, expires: None, provider: CLAUDE_PROVIDER.into(), model: None },
    }
}

/// `claude auth status --json` for the account in `dir`.
fn auth_status(claude: &Path, dir: &Path) -> io::Result<Output> {
    let mut command = Command::new(claude);
    for variable in launch::STRIPPED_ENV {
        command.env_remove(variable);
    }
    command
        .args(["auth", "status", "--json"])
        .env("CLAUDE_CONFIG_DIR", dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
}

/// Whether Anthropic bills the account in `dir` by use: logged in with a key of the Claude Console,
/// which `/login` made. `None` when it cannot be told. Takes about half a second.
pub fn billed_by_use(claude: &Path, dir: &Path) -> Option<bool> {
    let output = auth_status(claude, dir).ok().filter(|output| output.status.success())?;
    says_billed_by_use(&String::from_utf8_lossy(&output.stdout))
}

/// `authMethod` is `claude.ai` for a subscription, `api_key` for a key of the Console (seen on
/// 2.1.331); `apiProvider` is `firstParty` for Anthropic's own API.
fn says_billed_by_use(json: &str) -> Option<bool> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let text = |name: &str| value.get(name).and_then(serde_json::Value::as_str);
    Some(text("authMethod")? == "api_key" && text("apiProvider") == Some("firstParty"))
}

/// Exit code 0: logged in; 1: logged out; anything else: unknown. Claude Code reads
/// its own credentials: only the exit code, the plan name and the login's expiry are used here.
fn login(claude: &Path, dir: &Path) -> Login {
    match auth_status(claude, dir).map(|output| (output.status.code(), output.stdout)) {
        Ok((Some(0), stdout)) => {
            let text = String::from_utf8_lossy(&stdout);
            Login::claude(LoginState::LoggedIn, subscription_type(&text), login_expires(&text))
        },
        Ok((Some(1), _)) => Login::claude(LoginState::LoggedOut, None, None),
        _ => Login::claude(LoginState::Unknown, None, None),
    }
}

/// PIDs named in `<account>/sessions/<pid>.json`. The files stay after Claude Code
/// exits, so they are only candidates.
fn session_pids(dir: &Path) -> Vec<u32> {
    let Ok(entries) = fs::read_dir(dir.join("sessions")) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.strip_suffix(".json")?.parse().ok())
        .collect()
}

/// Sessions whose process is still running and is named `claude`.
fn active_sessions(dir: &Path) -> usize {
    let pids: Vec<Pid> = session_pids(dir).into_iter().map(Pid::from_u32).collect();
    if pids.is_empty() {
        return 0;
    }
    let Ok(mut system) = System::new() else {
        return 0;
    };
    system.refresh_processes(ProcessesToUpdate::Some(&pids), true);
    let is_claude = |pid: &Pid| {
        system.process(*pid).is_some_and(|process| {
            let name = process.name().to_string_lossy();
            name == "claude" || name == "claude.exe"
        })
    };
    pids.iter().filter(|pid| is_claude(pid)).count()
}

/// What `codex login status` says of a Codex account; unknown without `codex`.
fn codex_login(codex: Option<&Path>, dir: &Path) -> Login {
    let state = match codex.and_then(|codex| codex::logged_in(codex, dir)) {
        Some(true) => LoginState::LoggedIn,
        Some(false) => LoginState::LoggedOut,
        None => LoginState::Unknown,
    };
    Login { state, plan: None, expires: None, provider: CODEX_PROVIDER.into(), model: None }
}

/// Whether an OpenCode account has a credential, asked only while its service runs; unknown
/// otherwise, and without `opencode`.
fn opencode_login(opencode: Option<&Path>, dir: &Path) -> Login {
    let state = match opencode.and_then(|opencode| opencode::logged_in(opencode, dir)) {
        Some(true) => LoginState::LoggedIn,
        Some(false) => LoginState::LoggedOut,
        None => LoginState::Unknown,
    };
    Login { state, plan: None, expires: None, provider: OPENCODE_PROVIDER.into(), model: None }
}

/// The programs `collect` runs: `claude`, `codex` and `opencode`, when found.
#[derive(Clone, Copy, Default)]
struct Programs<'a> {
    claude: Option<&'a Path>,
    codex: Option<&'a Path>,
    opencode: Option<&'a Path>,
}

/// Login and running sessions of one account, after the program it runs.
fn account_status(agent: Result<Agent>, programs: Programs, dir: &Path) -> (Login, usize, Agent) {
    match (agent, programs.claude) {
        (Ok(Agent::Codex), _) => (codex_login(programs.codex, dir), codex::running_sessions(dir), Agent::Codex),
        (Ok(Agent::OpenCode), _) => {
            let sessions = programs.opencode.map_or(0, |opencode| opencode::running_sessions(opencode, dir));
            (opencode_login(programs.opencode, dir), sessions, Agent::OpenCode)
        },
        (Ok(Agent::ClaudeCode), Some(claude)) => (account_login(claude, dir), active_sessions(dir), Agent::ClaudeCode),
        (Ok(Agent::ClaudeCode), None) | (Err(_), _) => (Login::claude(LoginState::Unknown, None, None), 0, Agent::ClaudeCode),
    }
}

/// Status of every account in `names`, queried in parallel, in the order given. `claude` is
/// needed when one of them runs Claude Code; without `codex`, a Codex account's state is unknown.
pub fn collect(paths: &Paths, names: &[String]) -> Result<Vec<AccountStatus>> {
    let agents: Vec<Result<Agent>> = names.iter().map(|name| agent::read(&paths.account_dir(name))).collect();
    let claude = if agents.iter().any(|agent| matches!(agent, Ok(Agent::ClaudeCode))) { Some(launch::require_claude()?) } else { None };
    let codex = if agents.iter().any(|agent| matches!(agent, Ok(Agent::Codex))) { codex::require_codex().ok() } else { None };
    let opencode = if agents.iter().any(|agent| matches!(agent, Ok(Agent::OpenCode))) { opencode::require_opencode().ok() } else { None };
    let isolated = registry::list_isolated(paths)?;
    let statuses = thread::scope(|scope| {
        let handles: Vec<_> = names
            .iter()
            .zip(agents)
            .map(|(name, agent)| {
                let dir = paths.account_dir(name);
                let programs = Programs { claude: claude.as_deref(), codex: codex.as_deref(), opencode: opencode.as_deref() };
                scope.spawn(move || {
                    let (login, sessions, agent) = account_status(agent, programs, &dir);
                    (login, sessions, dir, agent)
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join()).collect::<Vec<_>>()
    });
    Ok(names
        .iter()
        .zip(statuses)
        .map(|(name, result)| {
            let unknown = Login::claude(LoginState::Unknown, None, None);
            let (login, sessions, dir, agent) = result.unwrap_or((unknown, 0, paths.account_dir(name), Agent::ClaudeCode));
            AccountStatus {
                name: name.clone(),
                state: login.state,
                sessions,
                plan: login.plan,
                dir,
                isolated: isolated.contains(name),
                login_expires: login.expires,
                provider: login.provider,
                model: login.model,
                agent,
            }
        })
        .collect())
}

/// The plan of a Claude account, `codex` or `opencode` for an account of those, the provider and
/// its model for the others.
pub fn plan_label(status: &AccountStatus) -> String {
    if status.agent != Agent::ClaudeCode {
        return status.agent.name().into();
    }
    if status.provider == CLAUDE_PROVIDER {
        return status.plan.clone().unwrap_or_default();
    }
    match &status.model {
        Some(model) => format!("{} {model}", status.provider),
        None => status.provider.clone(),
    }
}

/// Text lines of `status`, aligned like the 1.x script.
pub fn format_lines(statuses: &[AccountStatus]) -> Vec<String> {
    let name_width = statuses.iter().map(|status| status.name.len()).max().unwrap_or(0);
    let plan_width = statuses.iter().map(|status| plan_label(status).len()).max().unwrap_or(0);
    statuses
        .iter()
        .map(|status| {
            let sessions = if status.sessions > 0 { format!("{} active", status.sessions) } else { String::new() };
            let line = format!(
                "{:<name_width$}  {:<10}  {:<plan_width$}  {sessions}",
                status.name,
                status.state.label(),
                plan_label(status)
            );
            let mut line = line.trim_end().to_string();
            if status.isolated {
                line.push_str("  isolated");
            }
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(name: &str, state: LoginState, sessions: usize, plan: Option<&str>, isolated: bool) -> AccountStatus {
        AccountStatus {
            name: name.into(),
            state,
            sessions,
            plan: plan.map(Into::into),
            dir: PathBuf::from(format!("/d/{name}")),
            isolated,
            login_expires: None,
            provider: CLAUDE_PROVIDER.into(),
            model: None,
            agent: Agent::ClaudeCode,
        }
    }

    #[test]
    fn login_expiry_is_read_when_it_is_a_date() {
        let expires = |date: &str| login_expires(&format!(r#"{{"loggedIn": true, "loginExpiresAt": "{date}"}}"#));
        assert_eq!(expires("2026-11-13T15:45:31.701Z").as_deref(), Some("2026-11-13T15:45:31.701Z"));
        assert_eq!(expires("2026-11-13T15:45:31Z").as_deref(), Some("2026-11-13T15:45:31Z"));
        for bad in ["tomorrow", "2026-11-13", "2026-11-13T15:45:31.701", "2026-11-13T15:45:31.Z", "<b>2026-11-13T15:45:31Z"] {
            assert_eq!(expires(bad), None, "{bad}");
        }
        assert_eq!(login_expires(r#"{"loggedIn": true}"#), None);
    }

    #[test]
    fn plan_is_read_from_auth_status_json() {
        assert_eq!(subscription_type(r#"{"loggedIn": true, "subscriptionType": "max"}"#), Some("max".into()));
        assert_eq!(subscription_type(r#"{"loggedIn": true, "authMethod": "api_key"}"#), None);
        assert_eq!(subscription_type(r#"{"subscriptionType": "Pro Plus"}"#), None);
    }

    #[test]
    fn a_key_of_the_console_is_billed_by_use() {
        // As Claude Code 2.1.331 answered for a key made by /login, then for a subscription.
        let console = r#"{"loggedIn":true,"authMethod":"api_key","apiProvider":"firstParty","apiKeySource":"/login managed key"}"#;
        assert_eq!(says_billed_by_use(console), Some(true));
        assert_eq!(says_billed_by_use(r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","subscriptionType":"pro"}"#), Some(false));
        assert_eq!(says_billed_by_use(r#"{"loggedIn":true,"authMethod":"api_key","apiProvider":"gateway"}"#), Some(false));
        assert_eq!(says_billed_by_use("{}"), None);
        assert_eq!(says_billed_by_use("not json"), None);
        assert_eq!(subscription_type("not json"), None);
    }

    #[test]
    fn text_lines_are_aligned_like_1x() {
        let lines = format_lines(&[
            status("pro1", LoginState::LoggedIn, 1, Some("pro"), false),
            status("pro2", LoginState::LoggedOut, 0, None, false),
            status("work", LoginState::Unknown, 0, None, true),
        ]);
        assert_eq!(lines, ["pro1  logged in   pro  1 active", "pro2  logged out", "work  unknown  isolated"]);
    }

    #[test]
    fn json_keeps_the_1x_field_names_and_order() {
        let json = serde_json::to_string(&status("pro2", LoginState::LoggedOut, 0, None, false)).unwrap();
        assert_eq!(json, r#"{"name":"pro2","state":"logged-out","sessions":0,"plan":null,"dir":"/d/pro2","isolated":false,"login_expires":null,"provider":"claude","model":null,"agent":"claude-code"}"#);
    }

    #[test]
    fn a_provider_account_shows_its_provider_and_model_in_place_of_a_plan() {
        let mut router = status("router", LoginState::LoggedIn, 0, None, false);
        router.provider = "openrouter".into();
        router.model = Some("qwen/qwen3-coder".into());
        let mut local = status("local", LoginState::LoggedIn, 0, None, false);
        local.provider = "api".into();
        assert_eq!(plan_label(&router), "openrouter qwen/qwen3-coder");
        assert_eq!(plan_label(&local), "api");
        assert_eq!(plan_label(&status("pro", LoginState::LoggedIn, 0, Some("max"), false)), "max");
        let json = serde_json::to_string(&router).unwrap();
        assert!(json.ends_with(r#","provider":"openrouter","model":"qwen/qwen3-coder","agent":"claude-code"}"#), "{json}");
    }

    #[test]
    fn a_codex_account_says_so_in_place_of_a_plan() {
        let mut codex = status("cx", LoginState::LoggedOut, 0, None, false);
        codex.provider = CODEX_PROVIDER.into();
        codex.agent = Agent::Codex;
        assert_eq!(plan_label(&codex), "codex");
        assert_eq!(format_lines(&[codex.clone()]), ["cx  logged out  codex"]);
        let json = serde_json::to_string(&codex).unwrap();
        assert!(json.ends_with(r#","provider":"openai","model":null,"agent":"codex"}"#), "{json}");
    }

    #[test]
    fn an_account_runs_what_its_file_says() {
        let dir = Path::new("/nowhere");
        let (login, sessions, agent) = account_status(Ok(Agent::Codex), Programs::default(), dir);
        assert_eq!((login.state, login.provider.as_str(), sessions, agent), (LoginState::Unknown, CODEX_PROVIDER, 0, Agent::Codex));
        let (login, sessions, agent) = account_status(Ok(Agent::OpenCode), Programs::default(), dir);
        assert_eq!((login.state, login.provider.as_str(), sessions, agent), (LoginState::Unknown, OPENCODE_PROVIDER, 0, Agent::OpenCode));
        let (login, _, agent) = account_status(Err(crate::Error("broken".into())), Programs::default(), dir);
        assert_eq!((login.state, agent), (LoginState::Unknown, Agent::ClaudeCode));
    }

    #[test]
    fn an_opencode_account_says_so_in_place_of_a_plan() {
        let mut opencode = status("oc", LoginState::LoggedOut, 0, None, false);
        opencode.provider = OPENCODE_PROVIDER.into();
        opencode.agent = Agent::OpenCode;
        assert_eq!(plan_label(&opencode), "opencode");
        assert_eq!(format_lines(&[opencode.clone()]), ["oc  logged out  opencode"]);
        let json = serde_json::to_string(&opencode).unwrap();
        assert!(json.ends_with(r#","provider":"opencode","model":null,"agent":"opencode"}"#), "{json}");
    }
}
