//! The last steps of a session, read from the end of its transcript: what the control center shows
//! of a session without opening it. The transcript is not documented: whatever it lacks or changes
//! gives fewer steps, never an error.

use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::Serialize;
use serde_json::Value;

use crate::images::{is_prompt, tail, transcript_of};

/// A step longer than this is cut.
const LONGEST: usize = 160;

/// The fields that say what a tool works on, first found first: the page names permissions with
/// the same list.
const SUBJECTS: [&str; 8] = ["command", "file_path", "path", "pattern", "url", "query", "prompt", "description"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// What the user typed.
    Prompt,
    /// What Claude wrote.
    Text,
    /// A tool Claude called, with what it works on.
    Tool,
    /// What a tool gave back.
    Result,
    /// A tool that failed.
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Step {
    pub kind: Kind,
    pub text: String,
}

/// The first line of `text` that is not blank, cut at `LONGEST` characters.
fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    if line.chars().count() <= LONGEST {
        return Some(line.to_string());
    }
    Some(format!("{}…", line.chars().take(LONGEST - 1).collect::<String>()))
}

fn flag(entry: &Value, name: &str) -> bool {
    entry.get(name).and_then(Value::as_bool).unwrap_or(false)
}

/// `Bash(cargo test)`: the tool and the first field that says what it works on.
fn tool_text(block: &Value) -> Option<String> {
    let name = block.get("name").and_then(Value::as_str)?;
    let input = block.get("input");
    let subject = SUBJECTS.iter().find_map(|field| input.and_then(|input| input.get(field)).and_then(Value::as_str)).and_then(first_line);
    first_line(&subject.map_or_else(|| name.to_string(), |subject| format!("{name}({subject})")))
}

/// A tool result's text: a string, or text blocks.
fn result_text(block: &Value) -> Option<String> {
    match block.get("content")? {
        Value::String(text) => first_line(text),
        Value::Array(parts) => parts.iter().filter_map(|part| part.get("text").and_then(Value::as_str)).find_map(first_line),
        _ => None,
    }
}

/// The steps of one entry of the transcript, in order.
fn steps_of(entry: &Value) -> Vec<Step> {
    if flag(entry, "isSidechain") || flag(entry, "isMeta") {
        return Vec::new();
    }
    let step = |kind, text: Option<String>| text.map(|text| Step { kind, text });
    match entry.get("type").and_then(Value::as_str) {
        Some("user") if is_prompt(entry) => {
            let text = match entry.pointer("/message/content") {
                Some(Value::String(text)) => first_line(text),
                Some(Value::Array(blocks)) => blocks.iter().filter_map(|block| block.get("text").and_then(Value::as_str)).find_map(first_line),
                _ => None,
            };
            step(Kind::Prompt, text).into_iter().collect()
        },
        Some("user") => {
            let Some(Value::Array(blocks)) = entry.pointer("/message/content") else {
                return Vec::new();
            };
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
                .filter_map(|block| step(if flag(block, "is_error") { Kind::Error } else { Kind::Result }, result_text(block)))
                .collect()
        },
        Some("assistant") => {
            let Some(Value::Array(blocks)) = entry.pointer("/message/content") else {
                return Vec::new();
            };
            blocks
                .iter()
                .filter_map(|block| match block.get("type").and_then(Value::as_str) {
                    Some("text") => step(Kind::Text, block.get("text").and_then(Value::as_str).and_then(first_line)),
                    Some("tool_use") => step(Kind::Tool, tool_text(block)),
                    _ => None,
                })
                .collect()
        },
        _ => Vec::new(),
    }
}

/// The last `most` steps of the main conversation in the end of a transcript, oldest first; a
/// broken line is skipped.
pub fn recent(transcript_tail: &str, most: usize) -> Vec<Step> {
    let all: Vec<Step> = transcript_tail.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).flat_map(|entry| steps_of(&entry)).collect();
    all[all.len().saturating_sub(most)..].to_vec()
}

/// The last steps of a session and when its transcript was last written (Unix seconds); none
/// when the transcript cannot be found or read.
pub fn of_session(account_dir: &Path, session_id: &str, most: usize) -> (Vec<Step>, Option<u64>) {
    let Some(transcript) = transcript_of(account_dir, session_id) else {
        return (Vec::new(), None);
    };
    let written = std::fs::metadata(&transcript)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_secs());
    (tail(&transcript).map(|text| recent(&text, most)).unwrap_or_default(), written)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn transcript() -> String {
        let long = "x".repeat(300);
        [
            json!({"type": "user", "message": {"content": "make the logo"}}),
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "\n  I will draw it.\nSecond line."}]}}),
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Bash", "input": {"command": "python3 draw.py", "description": "Draw"}}]}}),
            json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "3 pages, no errors\nmore", "is_error": false}]}}),
            json!({"type": "assistant", "isSidechain": true, "message": {"content": [{"type": "text", "text": "a subagent"}]}}),
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Write", "input": {"file_path": "/w/logo.svg", "content": "<svg/>"}}]}}),
            json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": [{"type": "text", "text": "permission denied"}], "is_error": true}]}}),
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "TodoWrite", "input": {"todos": []}}]}}),
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": long}]}}),
        ]
        .iter()
        .map(Value::to_string)
        .chain(std::iter::once("{broken".to_string()))
        .collect::<Vec<_>>()
        .join("\n")
    }

    #[test]
    fn the_steps_of_the_main_conversation_come_in_order() {
        let steps = recent(&transcript(), 20);
        let seen: Vec<(Kind, &str)> = steps.iter().map(|step| (step.kind, step.text.as_str())).collect();
        assert_eq!(seen[..7], [
            (Kind::Prompt, "make the logo"),
            (Kind::Text, "I will draw it."),
            (Kind::Tool, "Bash(python3 draw.py)"),
            (Kind::Result, "3 pages, no errors"),
            (Kind::Tool, "Write(/w/logo.svg)"),
            (Kind::Error, "permission denied"),
            (Kind::Tool, "TodoWrite"),
        ], "a subagent's steps and a broken line are left out");
        let last = &steps[7];
        assert_eq!(last.text.chars().count(), LONGEST, "a long step is cut");
        assert!(last.text.ends_with('…'));
    }

    #[test]
    fn only_the_last_steps_are_kept() {
        let steps = recent(&transcript(), 2);
        assert_eq!(steps.iter().map(|step| step.kind).collect::<Vec<_>>(), [Kind::Tool, Kind::Text]);
        assert!(recent("", 8).is_empty());
    }
}
