//! A question a session wrote in its reply, put in kindric's overview by the Stop hook
//! `kindric question-check` (decision 366), so that the session does not have to use
//! `kindric ask`. Claude Code classes each end of turn of a background session as `blocked`
//! (it waits for the user) or `done` in `<job>/timeline.jsonl`, with a short `detail` (often the
//! question itself) and the reply's `text`; that line is written just before the Stop hooks run.
//! None of this is documented: whatever it lacks or changes gives no question, never an error.

use std::io;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::away;
use crate::mailbox::is_valid_session_id;
use crate::overview::{self, Body, Request};
use crate::progress::{one_line, timeline_tail};

/// The end of the reply kept as the question's context, in characters: the page shows it whole.
const CONTEXT: usize = 600;

#[derive(Deserialize)]
struct Entry {
    state: Option<String>,
    detail: Option<String>,
    text: Option<String>,
}

fn non_empty(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|text| !text.is_empty())
}

/// The question of the turn that just ended and its context: only when the last line of the
/// timeline is the end of that turn (it has the reply's text; the user's messages have none) and
/// Claude Code classed it `blocked`.
pub fn blocked_question(timeline_tail: &str) -> Option<(String, String)> {
    let last = timeline_tail.lines().rev().find(|line| !line.trim().is_empty())?;
    let entry: Entry = serde_json::from_str(last).ok()?;
    let text = non_empty(entry.text.as_deref())?;
    if entry.state.as_deref() != Some("blocked") {
        return None;
    }
    let question = non_empty(entry.detail.as_deref()).map_or_else(|| last_line(text), str::to_string);
    Some((one_line(&question), context_of(text, &question)))
}

fn last_line(text: &str) -> String {
    text.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or(text).trim().to_string()
}

/// The end of `text` without the question it ends with, on one line, at most `CONTEXT`
/// characters, cut at a word.
pub(crate) fn context_of(text: &str, question: &str) -> String {
    let text = text.trim();
    let before = text.strip_suffix(question.trim()).unwrap_or(text);
    let flat = before.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = flat.chars().count();
    if count <= CONTEXT {
        return flat;
    }
    let end: String = flat.chars().skip(count - CONTEXT).collect();
    let from_word = end.split_once(' ').map_or(end.as_str(), |(_, rest)| rest);
    format!("…{from_word}")
}

/// The request for the overview, for `input` (the JSON Claude Code gives a Stop hook) and the end
/// of the session's timeline. None when the turn asks nothing, when the session already has a
/// request in `pending` (a `kindric ask` it made itself, or the question of a stop before), or
/// while the user is away from it (marks in `away_root`): it decides alone then.
pub fn request_for(input: &str, timeline_tail: &str, pending: &[Request], away_root: &Path, now_millis: u128) -> Option<Request> {
    let input: Value = serde_json::from_str(input).ok()?;
    if input.get("hook_event_name").and_then(Value::as_str) != Some("Stop") {
        return None;
    }
    let session = input.get("session_id").and_then(Value::as_str).filter(|id| is_valid_session_id(id))?;
    if pending.iter().any(|request| request.session_id == session) {
        return None;
    }
    let now = i64::try_from(now_millis / 1000).unwrap_or(i64::MAX);
    if away::until(away_root, session).is_some_and(|until| until > now) {
        return None;
    }
    let (question, context) = blocked_question(timeline_tail)?;
    Some(Request {
        id: overview::new_id(now_millis, std::process::id()),
        session_id: session.to_string(),
        cwd: input.get("cwd").and_then(Value::as_str).unwrap_or_default().to_string(),
        asked_at: now,
        body: Body::Ask { question, context, from_reply: true },
    })
}

/// `kindric question-check`. Claude Code gives the job's folder to its hooks as `CLAUDE_JOB_DIR`;
/// a session that is not in the background has none, and nothing is asked. Nothing that goes wrong
/// may stop the session: a question that cannot be written is said on stderr.
pub fn run_hook() {
    let mut input = String::new();
    if io::Read::read_to_string(&mut io::stdin(), &mut input).is_err() || input.trim().is_empty() {
        return;
    }
    let Some(job) = std::env::var_os("CLAUDE_JOB_DIR") else {
        return;
    };
    let Some(tail) = timeline_tail(&Path::new(&job).join("timeline.jsonl")) else {
        return;
    };
    let (Ok(root), Ok(away_root)) = (overview::default_root(), away::default_root()) else {
        return;
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let Some(request) = request_for(&input, &tail, &overview::pending(&root), &away_root, now.as_millis()) else {
        return;
    };
    if let Err(error) = overview::ask(&root, &request) {
        eprintln!("kindric question-check: the question was not put in the overview ({}): {error}", root.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SESSION: &str = "aaaa-1111";

    fn end(state: &str, detail: &str, text: &str) -> String {
        json!({"at": "2026-11-18T09:54:44.172Z", "state": state, "detail": detail, "text": text}).to_string()
    }

    fn user(detail: &str) -> String {
        json!({"at": "2026-11-18T09:55:00.000Z", "state": "blocked", "detail": detail, "text": ""}).to_string()
    }

    fn stop() -> String {
        json!({"session_id": SESSION, "hook_event_name": "Stop", "cwd": "/home/u/arcade", "stop_hook_active": false}).to_string()
    }

    fn no_marks() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("kindric-question-none-{}", std::process::id()))
    }

    fn asked(session: &str) -> Request {
        Request { id: "1-2".into(), session_id: session.into(), cwd: "/home/u/arcade".into(), asked_at: 1, body: Body::Ask { question: "q".into(), context: "c".into(), from_reply: false } }
    }

    #[test]
    fn a_turn_classed_blocked_gives_its_question_and_the_reply_before_it() {
        let tail = [end("working", "reading", ""), end("blocked", "Quelle couleur ?", "J'ai fini le menu.\n\nQuelle couleur ?")].join("\n");
        assert_eq!(blocked_question(&tail), Some(("Quelle couleur ?".into(), "J'ai fini le menu.".into())));
    }

    #[test]
    fn a_turn_done_or_a_line_that_is_not_its_end_asks_nothing() {
        assert_eq!(blocked_question(&end("done", "menu finished", "J'ai fini le menu.")), None);
        assert_eq!(blocked_question(&end("working", "still on it", "Je continue.")), None);
        let answered = [end("blocked", "Quelle couleur ?", "Quelle couleur ?"), user("bleu")].join("\n");
        assert_eq!(blocked_question(&answered), None, "the question of a turn before, already answered");
        assert_eq!(blocked_question(&end("blocked", "Quelle couleur ?", "  ")), None, "no reply text: not an end of turn");
        assert_eq!(blocked_question("{broken"), None);
        assert_eq!(blocked_question(""), None);
    }

    #[test]
    fn without_a_detail_the_last_line_of_the_reply_is_the_question() {
        let (question, context) = blocked_question(&end("blocked", " ", "Le menu est prêt.\nTu le veux en bleu ?\n")).unwrap();
        assert_eq!(question, "Tu le veux en bleu ?");
        assert_eq!(context, "Le menu est prêt.");
    }

    #[test]
    fn a_long_reply_keeps_its_end_cut_at_a_word() {
        let long = format!("{} fin.", "mot ".repeat(400));
        let (_, context) = blocked_question(&end("blocked", "Une question ?", &long)).unwrap();
        assert!(context.starts_with('…') && context.ends_with("mot fin."), "{context}");
        assert!(context.chars().count() <= CONTEXT + 1, "{}", context.chars().count());
        assert!(context[3..].starts_with("mot"), "cut at a word: {context}");
    }

    #[test]
    fn the_stop_of_a_session_with_a_blocked_turn_gives_a_kindric_ask() {
        let tail = end("blocked", "Quelle couleur ?", "Le menu est prêt. Quelle couleur ?");
        let request = request_for(&stop(), &tail, &[], &no_marks(), 1_800_000_000_123).unwrap();
        assert_eq!((request.session_id.as_str(), request.cwd.as_str(), request.asked_at), (SESSION, "/home/u/arcade", 1_800_000_000));
        assert_eq!(request.body, Body::Ask { question: "Quelle couleur ?".into(), context: "Le menu est prêt.".into(), from_reply: true });
        assert!(request.id.starts_with("1800000000123-"));
        let written = serde_json::to_string(&request).unwrap();
        assert!(written.contains(r#""kind":"ask""#) && written.contains(r#""from_reply":true"#), "{written}");
        let own = Body::Ask { question: "q".into(), context: "c".into(), from_reply: false };
        assert!(!serde_json::to_string(&own).unwrap().contains("from_reply"), "a kindric ask is written as before");
    }

    #[test]
    fn nothing_is_asked_twice_nor_for_another_event_or_a_bad_session() {
        let tail = end("blocked", "Quelle couleur ?", "Quelle couleur ?");
        assert_eq!(request_for(&stop(), &tail, &[asked(SESSION)], &no_marks(), 1), None, "the session already asks");
        assert!(request_for(&stop(), &tail, &[asked("bbbb-2222")], &no_marks(), 1).is_some(), "another session's request does not count");
        let start = json!({"session_id": SESSION, "hook_event_name": "SessionStart"}).to_string();
        assert_eq!(request_for(&start, &tail, &[], &no_marks(), 1), None);
        let bad = json!({"session_id": "../x", "hook_event_name": "Stop"}).to_string();
        assert_eq!(request_for(&bad, &tail, &[], &no_marks(), 1), None);
        assert_eq!(request_for("", &tail, &[], &no_marks(), 1), None, "the probe of kindric-accounts gives no input");
    }

    #[test]
    fn nothing_is_asked_while_the_user_is_away_from_the_session() {
        let root = std::env::temp_dir().join(format!("kindric-question-away-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let tail = end("blocked", "Quelle couleur ?", "Quelle couleur ?");
        away::set(&root, SESSION, 2_000).unwrap();
        assert_eq!(request_for(&stop(), &tail, &[], &root, 1_999_000), None, "away until 2000, now 1999");
        assert!(request_for(&stop(), &tail, &[], &root, 2_000_000).is_some(), "the absence is over");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
