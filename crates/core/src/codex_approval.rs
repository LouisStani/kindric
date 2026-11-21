//! The approvals Codex sessions wait for, as requests of kindric's page (decision 431). Nothing is
//! written for them: each reading of the page's requests asks the daemon of the session's account
//! for the question still open, and an answer goes back to the daemon. Answered in Codex's TUI, a
//! question is gone at the next reading.

use kindric_accounts::{Paths, codex};

use kindric_accounts::agent::Agent;

use crate::accounts::{SessionInfo, SessionState};
use crate::overview::{Body, Request};

/// The start of the id of such a request: `codex:<account>:<session>:<item>`.
const PREFIX: &str = "codex:";

/// What `waiting_for` says of a session waiting for an approval (see `accounts::codex_session`).
const PERMISSION_PROMPT: &str = "permission prompt";

fn request_id(account: &str, thread: &str, item: &str) -> String {
    format!("{PREFIX}{account}:{thread}:{item}")
}

/// The account, session and item of a request id made by `request_id`; `None` for the requests of
/// Claude Code's sessions.
fn parse_id(id: &str) -> Option<(&str, &str, &str)> {
    let mut parts = id.strip_prefix(PREFIX)?.splitn(3, ':');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(account), Some(thread), Some(item)) if !account.is_empty() && !thread.is_empty() && !item.is_empty() => {
            Some((account, thread, item))
        },
        _ => None,
    }
}

/// Whether `id` names the approval of a Codex session.
pub fn is_codex_request(id: &str) -> bool {
    parse_id(id).is_some()
}

/// Whether `session` runs Codex and waits for an approval.
pub fn asks_approval(session: &SessionInfo) -> bool {
    session.agent == Agent::Codex && session.state == SessionState::Waiting && session.waiting_for.as_deref() == Some(PERMISSION_PROMPT)
}

/// The sessions of `sessions` that run Codex and wait for an approval.
fn waiting(sessions: &[SessionInfo]) -> impl Iterator<Item = &SessionInfo> {
    sessions.iter().filter(|session| asks_approval(session))
}

/// The request of the page for `approval`, asked by `session`: a command shows as Claude Code's
/// Bash does, a change of files with its reason.
fn request_of(session: &SessionInfo, approval: &codex::Approval, asked_at: i64) -> Request {
    let body = if approval.file_change {
        let reason = approval.reason.clone().unwrap_or_else(|| "change files".into());
        Body::Permission { tool: "Edit".into(), input: serde_json::json!({ "description": reason }) }
    } else {
        let mut input = serde_json::json!({ "command": approval.command.clone().unwrap_or_default() });
        if let Some(reason) = &approval.reason {
            input["description"] = serde_json::Value::from(reason.as_str());
        }
        Body::Permission { tool: "Bash".into(), input }
    };
    Request {
        id: request_id(&session.account, &approval.thread, &approval.item),
        session_id: session.session_id.clone(),
        cwd: session.cwd.clone(),
        asked_at,
        body,
    }
}

/// The approvals the Codex sessions of `sessions` wait for, read from their daemons now. A
/// daemon that cannot be asked leaves its session out: its question stays on its screen.
pub fn requests(sessions: &[SessionInfo], now: i64) -> Vec<Request> {
    let Ok(paths) = Paths::from_env() else {
        return Vec::new();
    };
    waiting(sessions)
        .filter_map(|session| {
            let dir = paths.account_dir(&session.account);
            let approval = codex::pending_approval(&dir, &session.session_id).ok()??;
            Some(request_of(session, &approval, now))
        })
        .collect()
}

/// Answers the approval of a Codex session named by `id`: `allow`, or decline it.
pub fn answer(id: &str, allow: bool) -> Result<(), String> {
    let (account, thread, item) = parse_id(id).ok_or_else(|| format!("{id} is not the approval of a Codex session"))?;
    let paths = Paths::from_env().map_err(|error| error.to_string())?;
    let answered = codex::answer_approval(&paths.account_dir(account), thread, item, allow)
        .map_err(|error| format!("cannot answer {thread}: {error}"))?;
    if answered { Ok(()) } else { Err("this question is no longer open: it was answered in the session".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(state: SessionState, waiting_for: Option<&str>, agent: Agent) -> SessionInfo {
        SessionInfo {
            account: "cx".into(),
            short_id: Some("01a1ff2c".into()),
            session_id: "01a1ff2c".into(),
            name: String::new(),
            cwd: "/home/u/api".into(),
            state,
            isolated: false,
            progress: None,
            tasks: None,
            waiting_for: waiting_for.map(String::from),
            agent,
        }
    }

    #[test]
    fn ids_name_the_account_the_session_and_the_item() {
        let id = request_id("cx", "01a1ff2c-2e25-70b0-b44f-c8f2c0faf0fb", "call_1");
        assert_eq!(parse_id(&id), Some(("cx", "01a1ff2c-2e25-70b0-b44f-c8f2c0faf0fb", "call_1")));
        assert!(is_codex_request(&id));
        assert!(!is_codex_request("1795284418968-4242"), "a request of Claude Code");
        assert_eq!(parse_id("codex:cx:only"), None);
        assert_eq!(parse_id("codex::t:i"), None);
    }

    #[test]
    fn only_codex_sessions_waiting_for_an_approval_are_asked() {
        let sessions = [
            session(SessionState::Waiting, Some("permission prompt"), Agent::Codex),
            session(SessionState::Waiting, Some("input needed"), Agent::Codex),
            session(SessionState::Waiting, Some("permission prompt"), Agent::ClaudeCode),
            session(SessionState::Working, None, Agent::Codex),
        ];
        assert_eq!(waiting(&sessions).count(), 1);
    }

    #[test]
    fn a_command_reads_as_bash_and_a_change_of_files_as_edit() {
        let asking = session(SessionState::Waiting, Some("permission prompt"), Agent::Codex);
        let command = codex::Approval {
            thread: "01a1ff2c".into(), item: "c1".into(), file_change: false,
            command: Some("rm -rf build".into()), reason: Some("clean".into()),
        };
        let request = request_of(&asking, &command, 1_000);
        assert_eq!(request.id, "codex:cx:01a1ff2c:c1");
        assert_eq!(request.session_id, "01a1ff2c");
        assert_eq!(request.body, Body::Permission { tool: "Bash".into(), input: serde_json::json!({ "command": "rm -rf build", "description": "clean" }) });
        let files = codex::Approval { file_change: true, command: None, reason: None, ..command };
        assert_eq!(request_of(&asking, &files, 1_000).body, Body::Permission { tool: "Edit".into(), input: serde_json::json!({ "description": "change files" }) });
    }
}
