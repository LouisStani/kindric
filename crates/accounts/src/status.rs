//! Login state, plan and running sessions of each account.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

use serde::Serialize;
use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::error::Result;
use crate::launch;
use crate::paths::Paths;
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
}

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

/// What `claude auth status --json` tells of an account.
struct Login {
    state: LoginState,
    plan: Option<String>,
    expires: Option<String>,
}

/// Exit code 0: logged in; 1: logged out; anything else: unknown. Claude Code reads
/// its own credentials: only the exit code, the plan name and the login's expiry are used here.
fn login(claude: &Path, dir: &Path) -> Login {
    let mut command = Command::new(claude);
    for variable in launch::STRIPPED_ENV {
        command.env_remove(variable);
    }
    let output = command
        .args(["auth", "status", "--json"])
        .env("CLAUDE_CONFIG_DIR", dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match output.map(|output| (output.status.code(), output.stdout)) {
        Ok((Some(0), stdout)) => {
            let text = String::from_utf8_lossy(&stdout);
            Login { state: LoginState::LoggedIn, plan: subscription_type(&text), expires: login_expires(&text) }
        },
        Ok((Some(1), _)) => Login { state: LoginState::LoggedOut, plan: None, expires: None },
        _ => Login { state: LoginState::Unknown, plan: None, expires: None },
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

/// Status of every account in `names`, queried in parallel, in the order given.
pub fn collect(paths: &Paths, names: &[String]) -> Result<Vec<AccountStatus>> {
    let claude = launch::require_claude()?;
    let isolated = registry::list_isolated(paths)?;
    let statuses = thread::scope(|scope| {
        let handles: Vec<_> = names
            .iter()
            .map(|name| {
                let dir = paths.account_dir(name);
                let claude = &claude;
                scope.spawn(move || {
                    let login = login(claude, &dir);
                    (login, active_sessions(&dir), dir)
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join()).collect::<Vec<_>>()
    });
    Ok(names
        .iter()
        .zip(statuses)
        .map(|(name, result)| {
            let unknown = Login { state: LoginState::Unknown, plan: None, expires: None };
            let (login, sessions, dir) = result.unwrap_or((unknown, 0, paths.account_dir(name)));
            AccountStatus {
                name: name.clone(),
                state: login.state,
                sessions,
                plan: login.plan,
                dir,
                isolated: isolated.contains(name),
                login_expires: login.expires,
            }
        })
        .collect())
}

/// Text lines of `status`, aligned like the 1.x script.
pub fn format_lines(statuses: &[AccountStatus]) -> Vec<String> {
    let name_width = statuses.iter().map(|status| status.name.len()).max().unwrap_or(0);
    let plan_width = statuses.iter().map(|status| status.plan.as_deref().unwrap_or("").len()).max().unwrap_or(0);
    statuses
        .iter()
        .map(|status| {
            let sessions = if status.sessions > 0 { format!("{} active", status.sessions) } else { String::new() };
            let line = format!(
                "{:<name_width$}  {:<10}  {:<plan_width$}  {sessions}",
                status.name,
                status.state.label(),
                status.plan.as_deref().unwrap_or("")
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
        assert_eq!(json, r#"{"name":"pro2","state":"logged-out","sessions":0,"plan":null,"dir":"/d/pro2","isolated":false,"login_expires":null}"#);
    }
}
