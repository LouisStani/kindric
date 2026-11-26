//! What kindric shows of an OpenCode session without opening it, read from its messages in the
//! account's service: its last steps, what its last turn wrote, how full its context is and what it
//! does now. The messages come newest first (`opencode::Service::messages`); whatever they lack
//! gives less, never an error. Read on OpenCode 2.4.1 (2026-11-25).

use std::path::Path;

use serde_json::Value;

use crate::opencode::Service;
use crate::steps::{Kind, SUBJECTS, Step, first_line};

fn kind(message: &Value) -> Option<&str> {
    message.get("type").and_then(Value::as_str)
}

/// `read(.)`: a tool and what it works on.
fn tool_text(part: &Value) -> Option<String> {
    let name = part.get("name").and_then(Value::as_str)?;
    let input = part.pointer("/state/input");
    let subject = SUBJECTS.iter().find_map(|field| input.and_then(|input| input.get(field)).and_then(Value::as_str)).and_then(first_line);
    first_line(&subject.map_or_else(|| name.to_string(), |subject| format!("{name}({subject})")))
}

/// What a tool gave back, as a step: its first line, or its error.
fn result_of(part: &Value) -> Option<Step> {
    let state = part.get("state")?;
    if state.get("status").and_then(Value::as_str) == Some("error") {
        let error = state.get("error").and_then(Value::as_str).and_then(first_line).unwrap_or_else(|| "failed".to_string());
        return Some(Step { kind: Kind::Error, text: error });
    }
    let text = match state.get("content").or_else(|| state.get("output")) {
        Some(Value::String(text)) => first_line(text),
        Some(Value::Array(parts)) => parts.iter().filter_map(|part| part.get("text").and_then(Value::as_str)).find_map(first_line),
        _ => None,
    }?;
    Some(Step { kind: Kind::Result, text })
}

/// The steps of one message, in order.
fn steps_of(message: &Value) -> Vec<Step> {
    match kind(message) {
        Some("user") => message.get("text").and_then(Value::as_str).and_then(first_line).map(|text| Step { kind: Kind::Prompt, text }).into_iter().collect(),
        Some("assistant") => {
            let Some(Value::Array(parts)) = message.get("content") else {
                return Vec::new();
            };
            parts
                .iter()
                .flat_map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("text") => part.get("text").and_then(Value::as_str).and_then(first_line).map(|text| Step { kind: Kind::Text, text }).into_iter().collect(),
                    Some("tool") => tool_text(part).map(|text| Step { kind: Kind::Tool, text }).into_iter().chain(result_of(part)).collect(),
                    _ => Vec::new(),
                })
                .collect()
        },
        _ => Vec::new(),
    }
}

/// The last `most` steps, oldest first, from messages newest first.
pub fn recent(messages: &[Value], most: usize) -> Vec<Step> {
    let all: Vec<Step> = messages.iter().rev().flat_map(steps_of).collect();
    all[all.len().saturating_sub(most)..].to_vec()
}

/// What the assistant wrote since the last prompt.
pub fn last_turn_text(messages: &[Value]) -> String {
    let turn = messages.iter().take_while(|message| kind(message) != Some("user"));
    let mut written: Vec<&str> = turn
        .filter(|message| kind(message) == Some("assistant"))
        .flat_map(|message| message.get("content").and_then(Value::as_array).into_iter().flatten())
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect();
    written.reverse();
    written.join("\n")
}

/// The model of the last answer and the tokens it held: what it read, cached or not, and wrote.
pub fn last_usage(messages: &[Value]) -> Option<(String, String, f64)> {
    let answer = messages.iter().find(|message| kind(message) == Some("assistant") && message.get("tokens").is_some())?;
    let tokens = answer.get("tokens")?;
    let count = |pointer: &str| tokens.pointer(pointer).and_then(Value::as_f64).unwrap_or(0.0);
    let used = count("/input") + count("/output") + count("/reasoning") + count("/cache/read") + count("/cache/write");
    let provider = answer.pointer("/model/providerID").and_then(Value::as_str)?.to_string();
    let model = answer.pointer("/model/id").and_then(Value::as_str)?.to_string();
    (used > 0.0).then_some((provider, model, used))
}

/// What the session does now: its last step.
pub fn doing(messages: &[Value]) -> Option<String> {
    recent(messages, 1).pop().map(|step| step.text)
}

/// When the session last wrote, in Unix seconds: the newest message.
fn written_at(messages: &[Value]) -> Option<u64> {
    let millis = messages.first()?.pointer("/time/created").and_then(Value::as_u64)?;
    Some(millis / 1000)
}

/// The messages of `session_id` from the running service of the account in `dir`.
pub fn messages_of(dir: &Path, session_id: &str) -> Option<Vec<Value>> {
    Service::running(dir).ok().flatten()?.messages(session_id).ok()
}

/// The last steps of a session and when it last wrote; none while its service does not run.
pub fn of_session(dir: &Path, session_id: &str, most: usize) -> (Vec<Step>, Option<u64>) {
    let Some(messages) = messages_of(dir, session_id) else {
        return (Vec::new(), None);
    };
    (recent(&messages, most), written_at(&messages))
}

/// How full the context window of `session_id` was at its last answer, in percent.
pub fn context_of(dir: &Path, session_id: &str) -> Option<f64> {
    let service = Service::running(dir).ok().flatten()?;
    let (provider, model, used) = last_usage(&service.messages(session_id).ok()?)?;
    let window = service.context_window(&provider, &model).ok().flatten().filter(|window| *window > 0.0)?;
    Some((used / window * 100.0).min(100.0))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A real turn of OpenCode 2.4.1 (session « Folder file listing », 2026-11-25), newest first.
    fn turn() -> Vec<Value> {
        vec![
            json!({"type": "idle", "time": {"created": 1795618866600_u64}, "outcome": "succeeded"}),
            json!({"type": "assistant", "time": {"created": 1795618862990_u64}, "model": {"id": "qwen3.6-35b-a3b:q4", "providerID": "ollama"},
                "content": [{"type": "reasoning", "text": "The user asked to list files."}, {"type": "text", "text": "There's one file in this folder:\n\n| Name |"}],
                "tokens": {"input": 60, "output": 47, "reasoning": 0, "cache": {"read": 13050, "write": 0}}}),
            json!({"type": "assistant", "time": {"created": 1795618858400_u64}, "model": {"id": "qwen3.6-35b-a3b:q4", "providerID": "ollama"},
                "content": [{"type": "tool", "name": "read", "state": {"status": "completed", "input": {"path": "."}, "content": [{"type": "text", "text": "Read directory ., entries 1-1\nlisez-moi.txt"}]}}],
                "tokens": {"input": 13054, "output": 48, "reasoning": 0, "cache": {"read": 0, "write": 0}}}),
            json!({"type": "user", "time": {"created": 1795618858000_u64}, "text": "list the files in this folder"}),
            json!({"type": "model-switched", "model": {"id": "qwen3.6-35b-a3b:q4", "providerID": "ollama"}}),
        ]
    }

    fn texts_of(steps: &[Step]) -> Vec<(Kind, &str)> {
        steps.iter().map(|step| (step.kind, step.text.as_str())).collect()
    }

    #[test]
    fn the_steps_come_oldest_first_with_tools_and_their_results() {
        assert_eq!(
            texts_of(&recent(&turn(), 10)),
            [
                (Kind::Prompt, "list the files in this folder"),
                (Kind::Tool, "read(.)"),
                (Kind::Result, "Read directory ., entries 1-1"),
                (Kind::Text, "There's one file in this folder:"),
            ]
        );
        assert_eq!(texts_of(&recent(&turn(), 1)), [(Kind::Text, "There's one file in this folder:")]);
    }

    #[test]
    fn a_failed_tool_is_an_error() {
        let failed = json!({"type": "assistant", "content": [{"type": "tool", "name": "shell", "state": {"status": "error", "input": {"command": "false"}, "error": "exit code 1"}}]});
        assert_eq!(texts_of(&recent(&[failed], 5)), [(Kind::Tool, "shell(false)"), (Kind::Error, "exit code 1")]);
    }

    #[test]
    fn the_last_turn_is_what_the_assistant_wrote_since_the_prompt() {
        let mut messages = turn();
        messages.push(json!({"type": "assistant", "content": [{"type": "text", "text": "before"}]}));
        assert_eq!(last_turn_text(&messages), "There's one file in this folder:\n\n| Name |");
    }

    #[test]
    fn the_usage_is_the_last_answer_with_its_cache() {
        assert_eq!(last_usage(&turn()), Some(("ollama".to_string(), "qwen3.6-35b-a3b:q4".to_string(), 13157.0)));
        assert_eq!(last_usage(&[json!({"type": "user", "text": "hi"})]), None);
    }

    #[test]
    fn what_it_does_now_and_when_it_wrote() {
        assert_eq!(doing(&turn()).as_deref(), Some("There's one file in this folder:"));
        assert_eq!(written_at(&turn()), Some(1795618866));
        assert_eq!(doing(&[]), None);
    }
}
