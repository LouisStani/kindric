//! What kindric shows of a Codex session without opening it, read from the end of its rollout,
//! `<CODEX_HOME>/sessions/<year>/<month>/<day>/rollout-<time>-<id>.jsonl`: its last steps, what its
//! last turn wrote, how full its context is and what it does now. The rollout is not documented:
//! whatever it lacks or changes gives less, never an error. Read on Codex 0.172 (2026-11-25).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::images::tail_of;
use crate::steps::{Kind, SUBJECTS, Step, first_line};

/// The end of a rollout read for the detail and the images: a turn writes its tools there in
/// full, and the first lines of a session hold its whole instructions.
const DETAIL_TAIL: u64 = 2 * 1024 * 1024;

/// The end read at each refresh, for what the session does now and its context.
const LIVE_TAIL: u64 = 256 * 1024;

/// Lines Codex writes before the output of a command its code mode ran.
const OUTPUT_PREAMBLE: [&str; 3] = ["Script completed", "Wall time", "Output:"];

/// The rollout of `session_id`, the newest when several hold it; archived ones too.
pub fn rollout_of(codex_home: &Path, session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty() || session_id.contains(['/', '\\', '.']) {
        return None;
    }
    let suffix = format!("-{session_id}.jsonl");
    let mut found: Vec<PathBuf> = folders(&codex_home.join("archived_sessions"));
    for year in folders(&codex_home.join("sessions")) {
        for month in folders(&year) {
            for day in folders(&month) {
                found.extend(folders(&day));
            }
        }
    }
    found
        .into_iter()
        .filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("rollout-") && name.ends_with(&suffix)))
        .filter_map(|path| Some((fs::metadata(&path).ok()?.modified().ok()?, path)))
        .max_by_key(|(written, _)| *written)
        .map(|(_, path)| path)
}

fn folders(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir).map(|read| read.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect()).unwrap_or_default()
}

fn written_at(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

fn lines(tail: &str) -> impl Iterator<Item = Value> + '_ {
    tail.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok())
}

fn payload<'a>(entry: &'a Value, kind: &str) -> Option<&'a Value> {
    (entry.get("type").and_then(Value::as_str) == Some(kind)).then(|| entry.get("payload")).flatten()
}

/// The texts of a message's content, in order.
fn texts(message: &Value) -> Vec<&str> {
    let Some(Value::Array(parts)) = message.get("content") else {
        return Vec::new();
    };
    parts.iter().filter_map(|part| part.get("text").and_then(Value::as_str)).collect()
}

/// What the user typed: Codex puts its own context in user messages too, as tagged blocks
/// (`<environment_context>`) and the instructions of AGENTS.md.
fn typed(text: &str) -> bool {
    let text = text.trim_start();
    !text.is_empty() && !text.starts_with('<') && !text.starts_with("# AGENTS.md")
}

fn prompt_of(item: &Value) -> Option<String> {
    if item.get("type").and_then(Value::as_str) != Some("message") || item.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    texts(item).into_iter().filter(|text| typed(text)).find_map(first_line)
}

/// The string between the quotes after `key` in `code`: `cmd:"ls -la"` or `"cmd":"ls -la"`.
fn quoted_after(code: &str, key: &str) -> Option<String> {
    let start = [format!("{key}:\""), format!("\"{key}\":\"")].iter().filter_map(|pattern| code.find(pattern.as_str()).map(|at| at + pattern.len())).min()?;
    let mut value = String::new();
    let mut chars = code[start..].chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' => value.push(chars.next()?),
            '"' => return Some(value),
            other => value.push(other),
        }
    }
    None
}

/// `exec_command(ls -la)`: what a tool call works on. Code mode runs JavaScript that calls the
/// tools, so the command is read from the code.
fn tool_text(item: &Value) -> Option<String> {
    let name = item.get("name").and_then(Value::as_str)?;
    let subject = match item.get("type").and_then(Value::as_str) {
        Some("custom_tool_call") => {
            let code = item.get("input").and_then(Value::as_str).unwrap_or_default();
            if let Some(command) = quoted_after(code, "cmd") {
                return first_line(&format!("exec_command({command})"));
            }
            first_line(code)
        },
        _ => {
            let arguments: Value = item.get("arguments").and_then(Value::as_str).and_then(|text| serde_json::from_str(text).ok()).unwrap_or_default();
            ["cmd"].iter().chain(SUBJECTS.iter()).find_map(|field| arguments.get(field).and_then(Value::as_str)).and_then(first_line)
        },
    };
    first_line(&subject.map_or_else(|| name.to_string(), |subject| format!("{name}({subject})")))
}

/// The first line of a tool's output that says something.
fn output_text(item: &Value) -> Option<String> {
    let texts: Vec<&str> = match item.get("output") {
        Some(Value::String(text)) => vec![text.as_str()],
        Some(Value::Array(parts)) => parts.iter().filter_map(|part| part.get("text").and_then(Value::as_str)).collect(),
        _ => Vec::new(),
    };
    texts
        .iter()
        .flat_map(|text| text.lines())
        .map(str::trim)
        .filter(|line| !OUTPUT_PREAMBLE.iter().any(|preamble| line.starts_with(preamble)))
        .find_map(first_line)
}

/// The steps of one line of a rollout.
fn steps_of(entry: &Value) -> Vec<Step> {
    let Some(item) = payload(entry, "response_item") else {
        return Vec::new();
    };
    let step = |kind, text: Option<String>| text.map(|text| Step { kind, text });
    match item.get("type").and_then(Value::as_str) {
        Some("message") => match item.get("role").and_then(Value::as_str) {
            Some("user") => step(Kind::Prompt, prompt_of(item)).into_iter().collect(),
            Some("assistant") => texts(item).into_iter().filter_map(|text| step(Kind::Text, first_line(text))).collect(),
            _ => Vec::new(),
        },
        Some("function_call" | "custom_tool_call") => step(Kind::Tool, tool_text(item)).into_iter().collect(),
        Some("function_call_output" | "custom_tool_call_output") => step(Kind::Result, output_text(item)).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// The last `most` steps in the end of a rollout, oldest first.
pub fn recent(tail: &str, most: usize) -> Vec<Step> {
    let all: Vec<Step> = lines(tail).flat_map(|entry| steps_of(&entry)).collect();
    all[all.len().saturating_sub(most)..].to_vec()
}

/// What the assistant wrote since the last prompt.
pub fn last_turn_text(tail: &str) -> String {
    let mut written: Vec<String> = Vec::new();
    for item in lines(tail).filter_map(|entry| payload(&entry, "response_item").cloned()) {
        if prompt_of(&item).is_some() {
            written.clear();
        }
        if item.get("role").and_then(Value::as_str) == Some("assistant") {
            written.extend(texts(&item).into_iter().map(str::to_string));
        }
    }
    written.join("\n")
}

/// How full the context window was at the last answer, in percent: the tokens of the last call
/// over the window Codex names.
pub fn context(tail: &str) -> Option<f64> {
    lines(tail)
        .filter_map(|entry| {
            let info = payload(&entry, "event_msg").filter(|event| event.get("type").and_then(Value::as_str) == Some("token_count"))?.get("info")?;
            let used = info.pointer("/last_token_usage/total_tokens").and_then(Value::as_f64)?;
            let window = info.get("model_context_window").and_then(Value::as_f64).filter(|window| *window > 0.0)?;
            Some((used / window * 100.0).min(100.0))
        })
        .last()
}

/// What the session does now: its last step.
pub fn doing(tail: &str) -> Option<String> {
    recent(tail, 1).pop().map(|step| step.text)
}

/// The last steps of a session and when its rollout was last written.
pub fn of_session(codex_home: &Path, session_id: &str, most: usize) -> (Vec<Step>, Option<u64>) {
    let Some(rollout) = rollout_of(codex_home, session_id) else {
        return (Vec::new(), None);
    };
    let steps = tail_of(&rollout, DETAIL_TAIL).map(|tail| recent(&tail, most)).unwrap_or_default();
    (steps, written_at(&rollout))
}

/// The end of the rollout of `session_id` read at each refresh.
pub fn live_tail(codex_home: &Path, session_id: &str) -> Option<String> {
    tail_of(&rollout_of(codex_home, session_id)?, LIVE_TAIL)
}

/// What the last turn of `session_id` wrote.
pub fn last_turn_of(codex_home: &Path, session_id: &str) -> String {
    rollout_of(codex_home, session_id).and_then(|rollout| tail_of(&rollout, DETAIL_TAIL)).map(|tail| last_turn_text(&tail)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn rollout(entries: &[Value]) -> String {
        entries.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")
    }

    fn item(payload: Value) -> Value {
        json!({"type": "response_item", "payload": payload})
    }

    fn message(role: &str, texts: &[&str]) -> Value {
        let kind = if role == "assistant" { "output_text" } else { "input_text" };
        item(json!({"type": "message", "role": role, "content": texts.iter().map(|text| json!({"type": kind, "text": text})).collect::<Vec<_>>()}))
    }

    /// The lines of a real turn of Codex 0.172 (session essai, 2026-11-25), shortened.
    fn turn() -> String {
        rollout(&[
            message("developer", &["You are Codex, an agent based on GPT-6."]),
            message("user", &["# AGENTS.md instructions\n\n<INSTRUCTIONS>", "<environment_context>\n  <cwd>/tmp/essai</cwd>"]),
            message("user", &["list the files in this folder"]),
            message("assistant", &["I'll list the current folder's entries."]),
            item(json!({"type": "custom_tool_call", "name": "exec", "input": "const r = await tools.exec_command({cmd:\"ls -la\",\"workdir\":\"/tmp/essai\"}); text(r.output);\n"})),
            item(json!({"type": "custom_tool_call_output", "output": [{"type": "input_text", "text": "Script completed\nWall time 0.1 seconds\nOutput:\n"}, {"type": "input_text", "text": "total 4\n-rw-r--r-- 1 u u 16 lisez-moi.txt\n"}]})),
            item(json!({"type": "function_call", "name": "wait", "arguments": "{\"cell_id\":\"g2:1\",\"yield_time_ms\":1000}"})),
            item(json!({"type": "reasoning", "summary": []})),
            json!({"type": "event_msg", "payload": {"type": "token_count", "info": {"last_token_usage": {"total_tokens": 25840}, "model_context_window": 258400}}}),
            message("assistant", &["There's one file in this folder: [lisez-moi.txt](/tmp/essai/lisez-moi.txt)."]),
        ])
    }

    fn texts_of(steps: &[Step]) -> Vec<(Kind, &str)> {
        steps.iter().map(|step| (step.kind, step.text.as_str())).collect()
    }

    #[test]
    fn the_steps_of_a_turn_leave_out_what_codex_adds_itself() {
        assert_eq!(
            texts_of(&recent(&turn(), 10)),
            [
                (Kind::Prompt, "list the files in this folder"),
                (Kind::Text, "I'll list the current folder's entries."),
                (Kind::Tool, "exec_command(ls -la)"),
                (Kind::Result, "total 4"),
                (Kind::Tool, "wait"),
                (Kind::Text, "There's one file in this folder: [lisez-moi.txt](/tmp/essai/lisez-moi.txt)."),
            ]
        );
        assert_eq!(recent(&turn(), 2).len(), 2, "the last ones only");
    }

    #[test]
    fn a_function_call_names_what_it_works_on() {
        let call = item(json!({"type": "function_call", "name": "shell", "arguments": "{\"command\":\"cargo test\"}"}));
        assert_eq!(texts_of(&recent(&rollout(&[call]), 5)), [(Kind::Tool, "shell(cargo test)")]);
        let escaped = item(json!({"type": "custom_tool_call", "name": "exec", "input": "tools.exec_command({cmd:\"echo \\\"hi\\\"\"})"}));
        assert_eq!(texts_of(&recent(&rollout(&[escaped]), 5)), [(Kind::Tool, "exec_command(echo \"hi\")")]);
    }

    #[test]
    fn the_last_turn_is_what_the_assistant_wrote_since_the_prompt() {
        let text = last_turn_text(&format!("{}\n{}", rollout(&[message("user", &["before"]), message("assistant", &["old"])]), turn()));
        assert_eq!(text, "I'll list the current folder's entries.\nThere's one file in this folder: [lisez-moi.txt](/tmp/essai/lisez-moi.txt).");
    }

    #[test]
    fn the_context_is_the_last_call_over_the_window() {
        assert_eq!(context(&turn()), Some(10.0));
        assert_eq!(context(&rollout(&[message("user", &["hi"])])), None);
    }

    #[test]
    fn what_it_does_now_is_its_last_step_and_a_broken_line_is_skipped() {
        assert_eq!(doing(&format!("{}\n{{broken", turn())).as_deref(), Some("There's one file in this folder: [lisez-moi.txt](/tmp/essai/lisez-moi.txt)."));
        assert_eq!(doing(""), None);
    }

    #[test]
    fn the_rollout_is_found_by_its_id_under_its_day() {
        let home = std::env::temp_dir().join(format!("kindric-codex-history-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let day = home.join("sessions/2026/11/25");
        fs::create_dir_all(&day).unwrap();
        let id = "01a21321-d20b-7b33-9624-9d25bb39acc8";
        let file = day.join(format!("rollout-2026-11-25T15-35-14-{id}.jsonl"));
        fs::write(&file, turn()).unwrap();
        assert_eq!(rollout_of(&home, id), Some(file));
        assert_eq!(rollout_of(&home, "../etc"), None);
        assert_eq!(rollout_of(&home, "01a21321"), None, "the whole id, not a part");
        let (steps, written) = of_session(&home, id, 3);
        assert_eq!(steps.len(), 3);
        assert!(written.is_some());
        fs::remove_dir_all(home).unwrap();
    }
}
