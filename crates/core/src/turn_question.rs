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

/// How much of the reply is kept as the question's context, in characters: the page shows it whole.
const CONTEXT: usize = 600;
/// And in lines: a long list of short ones would push the other sessions off the page.
const CONTEXT_LINES: usize = 12;

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

/// The end of `text` without the question it ends with, line by line as the page shows it: no code
/// block, and without the marks of Markdown that hide words the most (bold, code, titles, tables,
/// links). Whole lines, at most `CONTEXT` characters in all; only when they say little, the line
/// before them too, cut at a word.
pub(crate) fn context_of(text: &str, question: &str) -> String {
    let mut lines = readable_lines(text);
    if let Some(last) = lines.last_mut()
        && let Some(before) = last.strip_suffix(&plain(question))
    {
        let before = before.trim_end();
        *last = if is_list_mark(before) { String::new() } else { before.to_string() };
    }
    let mut kept: Vec<String> = Vec::new();
    let mut left = CONTEXT;
    for line in lines.into_iter().rev().filter(|line| !line.is_empty()) {
        if kept.len() == CONTEXT_LINES {
            kept.push("…".to_string());
            break;
        }
        let length = line.chars().count();
        if length <= left {
            // With the end of its line.
            left = left.saturating_sub(length + 1);
            kept.push(line);
            continue;
        }
        kept.push(if left > CONTEXT / 2 { end_of(&line, left) } else { "…".to_string() });
        break;
    }
    kept.reverse();
    kept.join("\n")
}

/// The last `most` characters of `line` at most, cut at a word.
fn end_of(line: &str, most: usize) -> String {
    let end: String = line.chars().skip(line.chars().count().saturating_sub(most)).collect();
    let from_word = end.split_once(' ').map_or(end.as_str(), |(_, rest)| rest);
    format!("…{from_word}")
}

/// The lines of a reply written in Markdown, without its code blocks, its empty lines, its rules
/// nor its marks; the cells of a table's row are separated by a dot.
fn readable_lines(text: &str) -> Vec<String> {
    let mut open: Option<(char, usize)> = None;
    let mut lines = Vec::new();
    for line in text.lines().map(str::trim) {
        match (open, fence(line)) {
            (None, Some((mark, length, _))) => open = Some((mark, length)),
            (Some((mark, length)), Some((closing, longer, true))) if closing == mark && longer >= length => open = None,
            (Some(_), _) => {},
            (None, None) if line.is_empty() || is_rule(line) => {},
            (None, None) => lines.push(plain(line)),
        }
    }
    lines
}

/// The mark of a line that opens or closes a code block, its length, and whether it is alone on
/// the line, as one that closes a block is.
fn fence(line: &str) -> Option<(char, usize, bool)> {
    let mark = line.chars().next().filter(|mark| matches!(mark, '`' | '~'))?;
    let length = line.chars().take_while(|character| *character == mark).count();
    let rest = &line[length..];
    (length >= 3 && !(mark == '`' && rest.contains('`'))).then_some((mark, length, rest.trim().is_empty()))
}

/// The mark of an item of a list without its text: `-`, `*`, `+`, `1.` or `1)`.
fn is_list_mark(text: &str) -> bool {
    let number = text.strip_suffix(['.', ')']).unwrap_or("");
    matches!(text, "-" | "*" | "+") || (!number.is_empty() && number.chars().all(|character| character.is_ascii_digit()))
}

/// A horizontal rule, or the line under the head of a table.
fn is_rule(line: &str) -> bool {
    line.chars().all(|character| "-*_|: ".contains(character)) && line.contains(['-', '*', '_'])
}

fn plain(line: &str) -> String {
    let line = line.trim();
    let line = match line.trim_start_matches('#').strip_prefix(' ') {
        Some(heading) if line.starts_with('#') => heading.trim(),
        _ => line.strip_prefix("> ").unwrap_or(line),
    };
    let cells = line.strip_prefix('|').and_then(|row| row.strip_suffix('|'));
    let line = match cells {
        Some(cells) => cells.split('|').map(str::trim).filter(|cell| !cell.is_empty()).collect::<Vec<_>>().join(" · "),
        None => line.to_string(),
    };
    let words = line.replace("**", "").replace('`', "");
    without_links(&words.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// `[text](target)` as its text, `![text](target)` too.
fn without_links(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some((before, after)) = rest.split_once('[') {
        let link = after.split_once("](").and_then(|(text, target)| Some((text, after_target(target)?)));
        match link {
            Some((text, after)) if !text.contains(['[', ']']) => {
                out.push_str(before.strip_suffix('!').unwrap_or(before));
                out.push_str(text);
                rest = after;
            },
            _ => {
                out.push_str(before);
                out.push('[');
                rest = after;
            },
        }
    }
    out.push_str(rest);
    out
}

/// What follows the parenthesis that closes the target of a link; the target may hold pairs.
fn after_target(target: &str) -> Option<&str> {
    let mut depth = 1;
    for (at, character) in target.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {},
        }
        if depth == 0 {
            return target.get(at + 1..);
        }
    }
    None
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
    fn the_context_keeps_the_lines_of_the_reply_without_its_marks_nor_its_code() {
        let reply = "## Le mail\n\nDeux mairies **n'ont rien reçu** :\n- `Nogent` : voir [la page](https://x.y/z) ;\n- tableau[0] [a] reste.\n\n```\ngit push\n```\n---\n| Ville | État |\n|---|:---:|\n| Nogent | **lu** |\n\n**Je l'envoie ?**";
        let (question, context) = blocked_question(&end("blocked", "Je l'envoie ?", reply)).unwrap();
        assert_eq!(question, "Je l'envoie ?");
        assert_eq!(context, "Le mail\nDeux mairies n'ont rien reçu :\n- Nogent : voir la page ;\n- tableau[0] [a] reste.\nVille · État\nNogent · lu");
    }

    #[test]
    fn a_long_reply_keeps_whole_lines_of_its_end() {
        let reply = format!("{}\n- {}\n- {}\nJe continue ?", "début ".repeat(60), "b".repeat(250), "c".repeat(250));
        let context = context_of(&reply, "Je continue ?");
        assert_eq!(context, format!("…\n- {}\n- {}", "b".repeat(250), "c".repeat(250)));
        let short_end = format!("{}\nVoilà.", "mot ".repeat(300));
        let context = context_of(&short_end, "");
        assert!(context.starts_with("…mot mot") && context.ends_with("mot\nVoilà."), "{context}");
        assert!(context.chars().count() <= CONTEXT + 2, "{}", context.chars().count());
    }

    #[test]
    fn a_question_that_is_an_item_of_a_list_leaves_no_mark_behind() {
        assert_eq!(context_of("Options:\n- A\n- Je continue ?", "Je continue ?"), "Options:\n- A");
        assert_eq!(context_of("Étapes\n1. Je continue ?", "Je continue ?"), "Étapes");
        assert_eq!(context_of("Reste 3. Je continue ?", "Je continue ?"), "Reste 3.");
    }

    #[test]
    fn a_code_block_is_closed_only_by_a_mark_as_long_as_its_own() {
        assert_eq!(context_of("a\n````md\n```\ncode\n```\n````\nb", ""), "a\nb");
        assert_eq!(context_of("a\n~~~\n```\ncode\n~~~\nb", ""), "a\nb");
        assert_eq!(context_of("a\n```\ncode\n```sh\nmore\n```\nb", ""), "a\nb");
        assert_eq!(context_of("a\n```one line```\nb", ""), "a\none line\nb");
    }

    #[test]
    fn images_and_targets_with_parentheses_leave_their_text() {
        assert_eq!(context_of("voir ![image](u.png) et [a](http://x/(b)) fin", ""), "voir image et a fin");
        assert_eq!(context_of("[a](http://x/ jamais fermé", ""), "[a](http://x/ jamais fermé");
    }

    #[test]
    fn a_long_list_keeps_its_last_lines_only() {
        let list: Vec<String> = (0..200).map(|number| format!("- {number}")).collect();
        let context = context_of(&list.join("\n"), "");
        assert_eq!(context.lines().count(), CONTEXT_LINES + 1);
        assert!(context.starts_with("…\n- 188") && context.ends_with("- 199"), "{context}");
        let lines: Vec<String> = (0..11).map(|number| format!("- {}{number}", "mot ".repeat(13))).collect();
        assert!(context_of(&lines.join("\n"), "").chars().count() <= CONTEXT + 2, "the ends of the lines count");
    }

    #[test]
    fn a_code_block_never_closed_and_a_reply_of_code_only_give_no_context() {
        assert_eq!(context_of("```\na\n~~~\nb\n", ""), "");
        assert_eq!(context_of("", ""), "");
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
