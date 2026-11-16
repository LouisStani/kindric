//! `kindric permission-hook`, the PermissionRequest hook of the sessions kindric-accounts starts:
//! a question Claude Code would ask on the screen of a session (may this tool run? the question of
//! AskUserQuestion) is also put in the overview, so that the user answers it from there.
//!
//! The question stays on the session's screen meanwhile; whichever answer comes first wins. One
//! given in the terminal changes `statusUpdatedAt` in Claude Code's file for the session, and the
//! hook then leaves without a word: Claude Code does not end a hook whose question was answered.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::away;
use crate::overview::{self, Answer, Body, Request};

/// The timeout kindric-accounts gives the hook, in seconds: a day, as for `wait-message`.
pub const TIMEOUT_SECONDS: u64 = 24 * 60 * 60;
/// The hook gives up a minute before Claude Code would end it, so that it removes its request.
const GIVE_UP: Duration = Duration::from_secs(TIMEOUT_SECONDS - 60);
const POLL: Duration = Duration::from_secs(1);

const REFUSED: &str = "The user refused this from kindric's overview.";

/// What Claude Code gives the hook, as far as kindric needs it.
#[derive(Debug, PartialEq, Eq)]
struct Asked {
    session_id: String,
    cwd: String,
    body: Body,
}

fn asked(input: &str) -> Option<Asked> {
    let input: Value = serde_json::from_str(input).ok()?;
    if input.get("hook_event_name")?.as_str()? != "PermissionRequest" {
        return None;
    }
    let tool = input.get("tool_name")?.as_str()?;
    let tool_input = input.get("tool_input").cloned().unwrap_or(Value::Null);
    let body = if tool == "AskUserQuestion" {
        Body::Question { questions: tool_input.get("questions")?.clone() }
    } else {
        Body::Permission { tool: tool.to_string(), input: tool_input }
    };
    Some(Asked {
        session_id: input.get("session_id")?.as_str()?.to_string(),
        cwd: input.get("cwd").and_then(Value::as_str).unwrap_or_default().to_string(),
        body,
    })
}

/// What the hook prints for `answer` to the request `body`; nothing for an answer that does not
/// fit it, and the question then stays on the session's screen.
fn decision(body: &Body, answer: &Answer) -> Option<String> {
    let decision = match (body, answer) {
        (Body::Permission { .. } | Body::Question { .. }, Answer::Deny) => json!({ "behavior": "deny", "message": REFUSED }),
        (Body::Permission { .. }, Answer::Allow) => json!({ "behavior": "allow" }),
        (Body::Question { questions }, Answer::Choices { answers }) => {
            json!({ "behavior": "allow", "updatedInput": { "questions": questions, "answers": answers } })
        },
        _ => return None,
    };
    Some(json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } }).to_string())
}

/// Claude Code's file for a running session, `<config>/sessions/<pid>.json`, as far as the hook
/// needs it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct SessionStatus {
    #[serde(rename = "sessionId", default)]
    session_id: String,
    status: Option<String>,
    #[serde(rename = "statusUpdatedAt")]
    updated_at: Option<i64>,
}

/// The file of `session` among the sessions of the account in `config_dir`, if one says so.
fn session_status(config_dir: &Path, session: &str) -> Option<SessionStatus> {
    fs::read_dir(config_dir.join("sessions"))
        .ok()?
        .flatten()
        .filter_map(|entry| serde_json::from_str::<SessionStatus>(&fs::read_to_string(entry.path()).ok()?).ok())
        .find(|status| status.session_id == session)
}

/// Tells, from successive looks at the session's file, when the question was settled elsewhere.
/// Claude Code marks the session `waiting` with a new `statusUpdatedAt` for each question; once
/// that mark changes, or the file goes (the session ended), this question is over.
#[derive(Debug, Default)]
struct Watch {
    /// `statusUpdatedAt` at the first look, once the file was found.
    first: Option<Option<i64>>,
    /// `statusUpdatedAt` when Claude Code marked this question.
    mark: Option<i64>,
}

impl Watch {
    fn settled(&mut self, now: Option<&SessionStatus>) -> bool {
        let Some(now) = now else {
            // Gone after it was found: the session ended. Never found (an older Claude Code):
            // only the answer ends the wait.
            return self.first.is_some();
        };
        let first = *self.first.get_or_insert(now.updated_at);
        let waiting = now.status.as_deref() == Some("waiting");
        match self.mark {
            Some(mark) => !waiting || now.updated_at != Some(mark),
            None if waiting => {
                self.mark = now.updated_at;
                false
            },
            // Answered in the terminal before the hook saw the question at all.
            None => now.updated_at != first,
        }
    }
}

/// Waits for the user's answer to `request` and returns what to print, if anything. `look` reads
/// the session's file; `pause` waits between looks; `given_up` says when to stop waiting.
fn wait(
    root: &Path,
    request: &Request,
    mut look: impl FnMut() -> Option<SessionStatus>,
    mut pause: impl FnMut(),
    mut given_up: impl FnMut() -> bool,
) -> Option<String> {
    let mut watch = Watch::default();
    loop {
        if let Some(answer) = overview::answer_of(root, &request.session_id, &request.id) {
            return decision(&request.body, &answer);
        }
        if watch.settled(look().as_ref()) || given_up() {
            return None;
        }
        pause();
    }
}

/// What `kindric permission-hook` prints for `input`, waiting for the user as long as needed.
/// Nothing while the user is away from the session: the hook of absence answers then.
fn answer(input: &str, root: &Path, away_root: &Path, config_dir: &Path, started: Instant) -> Option<String> {
    let asked = asked(input)?;
    if away::until(away_root, &asked.session_id).is_some_and(|until| until > away::now()) {
        return None;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let request = Request {
        id: overview::new_id(now.as_millis(), std::process::id()),
        session_id: asked.session_id,
        cwd: asked.cwd,
        asked_at: i64::try_from(now.as_secs()).unwrap_or(i64::MAX),
        body: asked.body,
    };
    overview::ask(root, &request).ok()?;
    let printed = wait(
        root,
        &request,
        || session_status(config_dir, &request.session_id),
        || thread::sleep(POLL),
        || started.elapsed() >= GIVE_UP,
    );
    let _ = overview::settle(root, &request.session_id, &request.id);
    printed
}

/// The account's folder for the hook: the `CLAUDE_CONFIG_DIR` the session runs with, else
/// Claude Code's own default.
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).or_else(|| Some(std::env::home_dir()?.join(".claude")))
}

/// `kindric permission-hook`. Nothing that goes wrong may disturb the session: it prints nothing
/// then, and Claude Code asks on the screen as usual.
pub fn run_hook() {
    let started = Instant::now();
    let mut input = String::new();
    if io::Read::read_to_string(&mut io::stdin(), &mut input).is_err() {
        return;
    }
    let (Ok(root), Ok(away_root), Some(config_dir)) = (overview::default_root(), away::default_root(), config_dir()) else {
        return;
    };
    if let Some(printed) = answer(&input, &root, &away_root, &config_dir, started) {
        println!("{printed}");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-permission-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    /// The input of the trial of 2026-11-16 (Claude Code 2.1.325), shortened.
    const BASH: &str = r#"{"session_id":"55f56114-d91e","transcript_path":"/t.jsonl","cwd":"/home/u/e2",
        "permission_mode":"default","hook_event_name":"PermissionRequest","tool_name":"Bash",
        "tool_input":{"command":"touch e2-made.txt","description":"Create empty file e2-made.txt"},
        "permission_suggestions":[]}"#;
    const QUESTION: &str = r#"{"session_id":"96e9bca6-aaaa","cwd":"/home/u/e7","hook_event_name":"PermissionRequest",
        "tool_name":"AskUserQuestion","tool_input":{"questions":[{"question":"Red or blue?","header":"Color",
        "options":[{"label":"Red","description":"Answer red"},{"label":"Blue","description":"Answer blue"}],
        "multiSelect":false}]}}"#;

    fn status(state: &str, updated_at: i64) -> SessionStatus {
        SessionStatus { session_id: "s".into(), status: Some(state.into()), updated_at: Some(updated_at) }
    }

    #[test]
    fn a_command_and_a_question_become_requests() {
        let bash = asked(BASH).unwrap();
        assert_eq!(bash.session_id, "55f56114-d91e");
        assert_eq!(bash.cwd, "/home/u/e2");
        let Body::Permission { tool, input } = bash.body else { panic!("{:?}", bash.body) };
        assert_eq!((tool.as_str(), input["command"].as_str()), ("Bash", Some("touch e2-made.txt")));
        let Body::Question { questions } = asked(QUESTION).unwrap().body else { panic!() };
        assert_eq!(questions[0]["question"], "Red or blue?");
        assert_eq!(asked(r#"{"hook_event_name":"Stop","session_id":"x"}"#), None);
        assert_eq!(asked(""), None, "kindric-accounts probes the hook with no input");
    }

    /// The decision that made the session of the trial e7 go on with « Answer: Blue ».
    #[test]
    fn answers_become_the_decisions_claude_code_takes() {
        let Body::Question { questions } = asked(QUESTION).unwrap().body else { panic!() };
        let question = Body::Question { questions };
        let choices = Answer::Choices { answers: BTreeMap::from([("Red or blue?".into(), "Blue".into())]) };
        let printed: Value = serde_json::from_str(&decision(&question, &choices).unwrap()).unwrap();
        let decided = &printed["hookSpecificOutput"]["decision"];
        assert_eq!(printed["hookSpecificOutput"]["hookEventName"], "PermissionRequest");
        assert_eq!(decided["behavior"], "allow");
        assert_eq!(decided["updatedInput"]["answers"]["Red or blue?"], "Blue");
        assert_eq!(decided["updatedInput"]["questions"][0]["header"], "Color");

        let bash = asked(BASH).unwrap().body;
        let allow: Value = serde_json::from_str(&decision(&bash, &Answer::Allow).unwrap()).unwrap();
        assert_eq!(allow["hookSpecificOutput"]["decision"], json!({ "behavior": "allow" }));
        let deny: Value = serde_json::from_str(&decision(&bash, &Answer::Deny).unwrap()).unwrap();
        assert_eq!(deny["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(deny["hookSpecificOutput"]["decision"]["message"], REFUSED);
        assert_eq!(decision(&bash, &Answer::Text { text: "yes".into() }), None, "does not fit: stays on screen");
        assert_eq!(decision(&question, &Answer::Allow), None, "a question needs its answers");
    }

    /// What the trial e9 saw: waiting at the question, then busy and a new mark once answered.
    #[test]
    fn a_question_answered_in_the_terminal_is_settled() {
        let mut watch = Watch::default();
        assert!(!watch.settled(Some(&status("busy", 100))), "before Claude Code marks the question");
        assert!(!watch.settled(Some(&status("waiting", 101))));
        assert!(!watch.settled(Some(&status("waiting", 101))));
        assert!(watch.settled(Some(&status("busy", 102))), "answered in the terminal");

        let mut next = Watch::default();
        assert!(!next.settled(Some(&status("waiting", 101))));
        assert!(next.settled(Some(&status("waiting", 103))), "already the next question");

        let mut quick = Watch::default();
        assert!(!quick.settled(Some(&status("busy", 100))));
        assert!(quick.settled(Some(&status("busy", 102))), "answered before the hook saw it");

        let mut ended = Watch::default();
        assert!(!ended.settled(Some(&status("waiting", 101))));
        assert!(ended.settled(None), "the session ended");

        let mut no_file = Watch::default();
        assert!(!no_file.settled(None));
        assert!(!no_file.settled(None), "no file at all: only the answer ends the wait");
    }

    #[test]
    fn the_hook_waits_for_the_answer_of_its_own_request() {
        let root = temp_root("wait");
        let asked = asked(BASH).unwrap();
        let request = Request { id: "1-2".into(), session_id: asked.session_id, cwd: asked.cwd, asked_at: 1, body: asked.body };
        overview::ask(&root, &request).unwrap();
        let mut looks = 0;
        let printed = wait(
            &root,
            &request,
            || Some(status("waiting", 101)),
            || {
                looks += 1;
                if looks == 3 {
                    overview::answer(&root, "55f56114-d91e", "1-2", &Answer::Allow).unwrap();
                }
            },
            || false,
        );
        assert_eq!(looks, 3);
        assert!(printed.unwrap().contains(r#""behavior":"allow""#));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_hook_stops_without_a_word_once_settled_or_given_up() {
        let root = temp_root("stop");
        let asked = asked(BASH).unwrap();
        let request = Request { id: "1-2".into(), session_id: asked.session_id, cwd: asked.cwd, asked_at: 1, body: asked.body };
        overview::ask(&root, &request).unwrap();
        let mut states = [status("waiting", 101), status("busy", 102)].into_iter();
        assert_eq!(wait(&root, &request, || states.next(), || {}, || false), None);
        assert_eq!(wait(&root, &request, || Some(status("waiting", 101)), || {}, || true), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_session_file_is_found_by_its_session_id() {
        let root = temp_root("file");
        fs::create_dir_all(root.join("sessions")).unwrap();
        let file = r#"{"pid":595771,"sessionId":"849c6b69-aaaa","status":"waiting","waitingFor":"permission prompt","statusUpdatedAt":1794832127865}"#;
        fs::write(root.join("sessions").join("595771.json"), file).unwrap();
        fs::write(root.join("sessions").join("1.json"), "not json").unwrap();
        let found = session_status(&root, "849c6b69-aaaa").unwrap();
        assert_eq!((found.status.as_deref(), found.updated_at), (Some("waiting"), Some(1794832127865)));
        assert_eq!(session_status(&root, "other"), None);
        fs::remove_dir_all(root).unwrap();
    }

    /// While the user is away, the hook of absence answers: this one leaves no request behind.
    #[test]
    fn nothing_is_asked_while_the_user_is_away() {
        let (root, away_root) = (temp_root("away-requests"), temp_root("away-marks"));
        away::set(&away_root, "55f56114-d91e", away::now() + 3600).unwrap();
        assert_eq!(answer(BASH, &root, &away_root, &root, Instant::now()), None);
        assert!(overview::pending(&root).is_empty());
        let _ = fs::remove_dir_all(root);
        fs::remove_dir_all(away_root).unwrap();
    }
}
