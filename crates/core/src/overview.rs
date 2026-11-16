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
