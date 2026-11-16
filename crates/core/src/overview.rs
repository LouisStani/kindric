//! What sessions ask the user, answered from kindric's overview without opening the session.
//! A request is `overview/<session id>/<request id>.json` in kindric's data; its answer, written by
//! the page, is `<request id>.answer` next to it. One answer file per request: a hook left waiting
//! after the user answered in the terminal must never read the answer meant for the next request.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use kindric_accounts::files::{create_private_dir, write_atomic};
use serde::{Deserialize, Serialize};

use crate::away;
use crate::mailbox::{data_dir, is_valid_session_id};

const REQUEST: &str = "json";
const ANSWER: &str = "answer";

/// `overview` in the data directory.
pub fn default_root() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("overview"))
}

/// What is asked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    /// Claude Code asks whether a tool may run: the user sees the command alone.
    Permission { tool: String, input: serde_json::Value },
    /// The session asked with Claude Code's own question tool, AskUserQuestion: `questions` as
    /// the tool got them. The session waits for the answer.
    Question { questions: serde_json::Value },
    /// The session asked with `kindric ask` and keeps working on what does not depend on it.
    Ask { question: String, context: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub session_id: String,
    /// The folder of the session, as Claude Code gave it.
    pub cwd: String,
    /// When it was asked, in Unix seconds.
    pub asked_at: i64,
    #[serde(flatten)]
    pub body: Body,
}

/// What the user answered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Answer {
    Allow,
    Deny,
    /// For a question of AskUserQuestion: the text of each question, and the label chosen or
    /// the words typed.
    Choices { answers: BTreeMap<String, String> },
    /// For `kindric ask`.
    Text { text: String },
}

/// A request id: digits and dashes, as `new_id` makes them.
fn is_valid_request_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_digit() || c == '-')
}

/// An id no other request of the session has: the time in milliseconds, then the asking process.
pub fn new_id(now_millis: u128, pid: u32) -> String {
    format!("{now_millis}-{pid}")
}

fn session_dir(root: &Path, session: &str) -> io::Result<PathBuf> {
    if !is_valid_session_id(session) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid session id: {session:?}")));
    }
    Ok(root.join(session))
}

fn file(root: &Path, session: &str, id: &str, extension: &str) -> io::Result<PathBuf> {
    if !is_valid_request_id(id) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid request id: {id:?}")));
    }
    Ok(session_dir(root, session)?.join(format!("{id}.{extension}")))
}

/// Writes `request` for the page to show.
pub fn ask(root: &Path, request: &Request) -> io::Result<()> {
    let path = file(root, &request.session_id, &request.id, REQUEST)?;
    create_private_dir(root)?;
    create_private_dir(&session_dir(root, &request.session_id)?)?;
    write_atomic(&path, &serde_json::to_string(request).map_err(io::Error::other)?)
}

/// The requests waiting for the user, oldest first. A file that cannot be read is left out.
pub fn pending(root: &Path) -> Vec<Request> {
    let mut requests: Vec<Request> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|session| fs::read_dir(session.path()).into_iter().flatten().flatten())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == REQUEST))
        .filter_map(|path| serde_json::from_str(&fs::read_to_string(path).ok()?).ok())
        .collect();
    requests.sort_by(|a, b| (a.asked_at, &a.id).cmp(&(b.asked_at, &b.id)));
    requests
}

/// The user answers the request `id` of `session`. A request already settled (answered in the
/// terminal, or the session moved on) has no file any more: that is an error the page shows.
pub fn answer(root: &Path, session: &str, id: &str, answer: &Answer) -> io::Result<()> {
    if !file(root, session, id, REQUEST)?.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "this request is already settled"));
    }
    write_atomic(&file(root, session, id, ANSWER)?, &serde_json::to_string(answer).map_err(io::Error::other)?)
}

/// The answer to the request `id` of `session`, once the user gave one.
pub fn answer_of(root: &Path, session: &str, id: &str) -> Option<Answer> {
    serde_json::from_str(&fs::read_to_string(file(root, session, id, ANSWER).ok()?).ok()?).ok()
}

/// Removes the request and its answer, then the folder of the session once empty. Nothing to
/// remove is no error.
pub fn settle(root: &Path, session: &str, id: &str) -> io::Result<()> {
    for extension in [REQUEST, ANSWER] {
        match fs::remove_file(file(root, session, id, extension)?) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {},
        }
    }
    // Fails while another request of the session waits: that one keeps the folder.
    let _ = fs::remove_dir(session_dir(root, session)?);
    Ok(())
}

/// What `kindric overview-start`, the SessionStart hook of the sessions kindric-accounts starts,
/// tells a session.
pub const START: &str = "kindric shows the user an overview of their sessions, where they answer \
questions without opening the session. When you need the user to decide something, prefer the shell \
command `kindric ask \"<question>\" \"<context>\"` to AskUserQuestion: it does not stop you. Write a \
single question that stands alone, and the context for someone who has not seen this session: what \
you are doing, what you found, why you need them. Their answer comes later as a message from them.";

/// What `kindric ask` tells the session once the question is sent.
pub const SENT: &str = "Question sent to the user through kindric's overview. Their answer will come \
later as a message from them. Until then, do only the work that does not depend on it; if none is \
left, end your turn now. Do not guess the answer.";

const HOW: &str = "usage: kindric ask \"<question>\" \"<context: what you are doing, what you found, \
why you need the user>\"";

/// `kindric ask`, run by the session `session` (from `CLAUDE_CODE_SESSION_ID`) in `cwd`: puts its
/// question in the overview. Refused without a question and its context, and while the user is
/// away from the session: it then decides alone, as it was told.
pub fn ask_from_session(
    root: &Path,
    away_root: &Path,
    session: Option<&str>,
    cwd: &Path,
    question: &str,
    context: &str,
    now_millis: u128,
) -> Result<Request, String> {
    let session = session.filter(|id| !id.is_empty()).ok_or_else(|| {
        "kindric ask is run by a Claude Code session: CLAUDE_CODE_SESSION_ID is not set".to_string()
    })?;
    let (question, context) = (question.trim(), context.trim());
    if question.is_empty() || context.is_empty() {
        return Err(format!("a question and its context are both needed. {HOW}"));
    }
    if away::until(away_root, session).is_some_and(|until| until > away::now()) {
        return Err("the user is away from this session: make the choice yourself, write it down \
where the project keeps its decisions, and go on"
            .to_string());
    }
    let request = Request {
        id: new_id(now_millis, std::process::id()),
        session_id: session.to_string(),
        cwd: cwd.display().to_string(),
        asked_at: i64::try_from(now_millis / 1000).unwrap_or(i64::MAX),
        body: Body::Ask { question: question.to_string(), context: context.to_string() },
    };
    ask(root, &request).map_err(|error| format!("cannot write the question in {}: {error}", root.display()))?;
    Ok(request)
}

/// `kindric ask <question> <context>` from the environment Claude Code gives a shell command.
pub fn run_ask(question: &str, context: &str) -> Result<(), String> {
    let root = default_root().map_err(|error| format!("cannot find kindric's data: {error}"))?;
    let away_root = away::default_root().map_err(|error| format!("cannot find kindric's data: {error}"))?;
    let cwd = std::env::current_dir().map_err(|error| format!("cannot read the current folder: {error}"))?;
    let session = std::env::var("CLAUDE_CODE_SESSION_ID").ok();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    ask_from_session(&root, &away_root, session.as_deref(), &cwd, question, context, now.as_millis())?;
    println!("{SENT}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-overview-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn request(session: &str, id: &str, asked_at: i64, body: Body) -> Request {
        Request { id: id.into(), session_id: session.into(), cwd: "/home/u/arcade".into(), asked_at, body }
    }

    fn permission() -> Body {
        Body::Permission { tool: "Bash".into(), input: serde_json::json!({ "command": "cargo test", "description": "Run tests" }) }
    }

    #[test]
    fn requests_are_listed_oldest_first_across_sessions() {
        let root = temp_root("list");
        let ask_body = Body::Ask { question: "UTC or local time?".into(), context: "Scores store UTC only.".into() };
        ask(&root, &request("aaaa-1111", "200-7", 200, ask_body.clone())).unwrap();
        ask(&root, &request("bbbb-2222", "100-8", 100, permission())).unwrap();
        let listed = pending(&root);
        assert_eq!(listed.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["100-8", "200-7"]);
        assert_eq!(listed[1].body, ask_body);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_answer_is_read_then_settled_with_its_request() {
        let root = temp_root("answer");
        ask(&root, &request("aaaa-1111", "100-7", 100, permission())).unwrap();
        assert_eq!(answer_of(&root, "aaaa-1111", "100-7"), None, "not answered yet");
        answer(&root, "aaaa-1111", "100-7", &Answer::Allow).unwrap();
        assert_eq!(answer_of(&root, "aaaa-1111", "100-7"), Some(Answer::Allow));
        settle(&root, "aaaa-1111", "100-7").unwrap();
        assert!(pending(&root).is_empty());
        assert!(!root.join("aaaa-1111").exists(), "the empty folder goes");
        settle(&root, "aaaa-1111", "100-7").unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_settled_request_takes_no_answer() {
        let root = temp_root("settled");
        let error = answer(&root, "aaaa-1111", "100-7", &Answer::Deny).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(!root.join("aaaa-1111").join("100-7.answer").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn each_request_has_its_own_answer() {
        let root = temp_root("own");
        ask(&root, &request("aaaa-1111", "100-7", 100, permission())).unwrap();
        ask(&root, &request("aaaa-1111", "101-8", 101, permission())).unwrap();
        answer(&root, "aaaa-1111", "101-8", &Answer::Deny).unwrap();
        assert_eq!(answer_of(&root, "aaaa-1111", "100-7"), None);
        settle(&root, "aaaa-1111", "100-7").unwrap();
        assert!(root.join("aaaa-1111").exists(), "kept for the other request");
        assert_eq!(answer_of(&root, "aaaa-1111", "101-8"), Some(Answer::Deny));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ids_cannot_leave_the_folder() {
        let root = temp_root("ids");
        assert!(ask(&root, &request("../aaaa", "100-7", 100, permission())).is_err());
        assert!(ask(&root, &request("aaaa-1111", "../x", 100, permission())).is_err());
        assert!(answer(&root, "aaaa-1111", "1/2", &Answer::Allow).is_err());
        assert_eq!(new_id(1_794_832_127_865, 4242), "1794832127865-4242");
        assert!(is_valid_request_id(&new_id(1, 2)));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_session_asks_with_its_context() {
        let (root, away_root) = (temp_root("ask"), temp_root("ask-away"));
        let cwd = Path::new("/home/u/arcade");
        let asked = ask_from_session(&root, &away_root, Some("aaaa-1111"), cwd, " UTC or local? ", "Scores store UTC.", 5_000).unwrap();
        assert_eq!(asked.body, Body::Ask { question: "UTC or local?".into(), context: "Scores store UTC.".into() });
        assert_eq!((asked.asked_at, asked.cwd.as_str()), (5, "/home/u/arcade"));
        assert_eq!(pending(&root), [asked]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_question_without_context_or_session_is_refused() {
        let (root, away_root) = (temp_root("refused"), temp_root("refused-away"));
        let cwd = Path::new("/w");
        let no_context = ask_from_session(&root, &away_root, Some("aaaa-1111"), cwd, "UTC?", "  ", 1).unwrap_err();
        assert!(no_context.contains("context are both needed") && no_context.contains("usage: kindric ask"), "{no_context}");
        assert!(ask_from_session(&root, &away_root, Some("aaaa-1111"), cwd, "", "c", 1).is_err());
        let outside = ask_from_session(&root, &away_root, None, cwd, "q", "c", 1).unwrap_err();
        assert!(outside.contains("CLAUDE_CODE_SESSION_ID"), "{outside}");
        away::set(&away_root, "aaaa-1111", away::now() + 3600).unwrap();
        let away = ask_from_session(&root, &away_root, Some("aaaa-1111"), cwd, "q", "c", 1).unwrap_err();
        assert!(away.contains("make the choice yourself"), "{away}");
        assert!(pending(&root).is_empty());
        let _ = fs::remove_dir_all(root);
        fs::remove_dir_all(away_root).unwrap();
    }

    /// The words of the trial of 2026-11-16 (e10), after which the session asked, did the work
    /// that did not depend on the answer, ended its turn, and finished once answered.
    #[test]
    fn the_session_is_told_to_go_on_without_guessing() {
        assert!(SENT.contains("do only the work that does not depend on it"));
        assert!(SENT.contains("end your turn now") && SENT.contains("Do not guess"));
        assert!(START.contains(r#"kindric ask "<question>" "<context>""#) && START.contains("AskUserQuestion"));
    }

    #[test]
    fn answers_keep_their_shape_on_disk() {
        let choices = Answer::Choices { answers: BTreeMap::from([("Red or blue?".to_string(), "Blue".to_string())]) };
        let json = serde_json::to_string(&choices).unwrap();
        assert_eq!(json, r#"{"kind":"choices","answers":{"Red or blue?":"Blue"}}"#);
        assert_eq!(serde_json::to_string(&Answer::Allow).unwrap(), r#"{"kind":"allow"}"#);
        let asked = request("aaaa-1111", "1-2", 3, Body::Ask { question: "q".into(), context: "c".into() });
        let json = serde_json::to_string(&asked).unwrap();
        assert!(json.contains(r#""kind":"ask""#), "{json}");
        assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), asked);
    }
}
