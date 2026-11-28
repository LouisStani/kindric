//! Background work a Claude Code session started and is still waiting for, read in the end of its
//! transcript: a command run in the background, an agent run in the background, a Monitor. Claude
//! Code wakes the session when such a task ends, so a session that ends its turn to wait for one
//! is not idle. A task ends with a notification that has a `<status>` (completed, failed,
//! killed) or with `TaskStop`. Notifications without a status do not end it: a Monitor's events,
//! Claude Code's check-ins on a quiet command.
//!
//! A task that showed no sign of life for `QUIET` seconds no longer counts: a server left running,
//! a command that hangs. Signs of life: its launch, a Monitor event, a write to its output file.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::changes::unix_seconds;

/// Thirty minutes, as Claude Code's own check-in on a command with no new output.
const QUIET: i64 = 30 * 60;

/// What a Bash result says before the path of the output file of a command run in the background.
const OUTPUT_SAID: &str = "Output is being written to: ";

const NOTICE_OPEN: &str = "<task-notification>";
const NOTICE_CLOSE: &str = "</task-notification>";

/// A task the session started: when it last showed a sign of life in the transcript, and its
/// output file, when the transcript names it.
struct Task {
    seen: i64,
    output: Option<PathBuf>,
}

/// What a notification says about a task: whether it ended, whether it brings an event.
struct Notice<'a> {
    id: &'a str,
    ended: bool,
    event: bool,
}

/// Whether, in `transcript_tail`, the session waits on background work it started that has not
/// ended and showed a sign of life in the last `QUIET` seconds before `now`.
pub(crate) fn waits_on_background(transcript_tail: &str, now: i64) -> bool {
    let mut tasks: HashMap<String, Task> = HashMap::new();
    for entry in transcript_tail.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
        let time = entry.get("timestamp").and_then(Value::as_str).and_then(unix_seconds);
        if let (Some((id, output)), Some(time)) = (launched(&entry), time) {
            tasks.insert(id, Task { seen: time, output });
            continue;
        }
        if let Some(id) = stopped(&entry) {
            tasks.remove(id);
        }
        for text in notice_texts(&entry) {
            for notice in notices(text) {
                if notice.ended {
                    tasks.remove(notice.id);
                } else if let (true, Some(time), Some(task)) = (notice.event, time, tasks.get_mut(notice.id)) {
                    task.seen = task.seen.max(time);
                }
            }
        }
    }
    tasks.values().any(|task| last_sign(task) > now - QUIET)
}

fn last_sign(task: &Task) -> i64 {
    let written = task.output.as_ref().and_then(|path| fs::metadata(path).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok());
    let written = written.and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok()).unwrap_or(i64::MIN);
    task.seen.max(written)
}

/// The task a tool result starts in the background, with its output file when it names one: a
/// Bash command (`backgroundTaskId`), an agent (`isAsync`, `agentId`), a Monitor (`taskId`).
fn launched(entry: &Value) -> Option<(String, Option<PathBuf>)> {
    if entry.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let result = entry.get("toolUseResult")?;
    let text = |key: &str| result.get(key).and_then(Value::as_str).filter(|value| !value.is_empty());
    if let Some(id) = text("backgroundTaskId") {
        return Some((id.to_string(), output_said(entry)));
    }
    if result.get("isAsync").and_then(Value::as_bool) == Some(true) {
        return Some((text("agentId")?.to_string(), text("outputFile").map(PathBuf::from)));
    }
    if result.get("timeoutMs").is_some() {
        return Some((text("taskId")?.to_string(), None));
    }
    None
}

/// The output file a Bash result names for a command run in the background.
fn output_said(entry: &Value) -> Option<PathBuf> {
    let blocks = entry.pointer("/message/content")?.as_array()?;
    let said = blocks.iter().filter_map(|block| block.get("content")?.as_str()).find(|said| said.contains(OUTPUT_SAID))?;
    let path = &said[said.find(OUTPUT_SAID)? + OUTPUT_SAID.len()..];
    let end = path.find(".output")? + ".output".len();
    Some(PathBuf::from(&path[..end]))
}

/// The task a `TaskStop` result stopped.
fn stopped(entry: &Value) -> Option<&str> {
    let result = entry.get("toolUseResult")?;
    let message = result.get("message")?.as_str()?;
    message.starts_with("Successfully stopped task").then(|| result.get("task_id")?.as_str()).flatten()
}

/// The texts of an entry where Claude Code delivers notifications: the queue it puts them in,
/// the queued command it hands the session, a user message that is one. Not a tool's result: a
/// command that prints a notification does not end a task.
fn notice_texts(entry: &Value) -> Vec<&str> {
    let mut texts = Vec::new();
    match entry.get("type").and_then(Value::as_str) {
        Some("queue-operation") if entry.get("operation").and_then(Value::as_str) == Some("enqueue") => {
            texts.extend(entry.get("content").and_then(Value::as_str));
        },
        Some("attachment") => texts.extend(entry.pointer("/attachment/prompt").and_then(Value::as_str)),
        Some("user") => texts.extend(entry.pointer("/message/content").and_then(Value::as_str)),
        _ => {},
    }
    texts.retain(|text| text.trim_start().starts_with(NOTICE_OPEN));
    texts
}

/// The notifications in a text, each one `<task-notification>…</task-notification>`.
fn notices(text: &str) -> Vec<Notice<'_>> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(NOTICE_OPEN) {
        let after = &rest[start + NOTICE_OPEN.len()..];
        let (body, next) = after.split_once(NOTICE_CLOSE).unwrap_or((after, ""));
        if let Some(id) = between(body, "<task-id>", "</task-id>") {
            found.push(Notice { id, ended: body.contains("<status>"), event: body.contains("<event>") });
        }
        rest = next;
    }
    found
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let length = text[start..].find(close)?;
    Some(text[start..start + length].trim()).filter(|id| !id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-27T00:00:00Z: times in the tests are seconds after it.
    const T0: i64 = 1_795_737_600;

    fn at(seconds: i64) -> String {
        crate::changes::iso(T0 + seconds)
    }

    // Entries shaped as in this session's own transcript (Claude Code 2.1.332, 2026-11-28).
    fn bash_launch(id: &str, output: &str, time: i64) -> String {
        let said = format!("Command running in background with ID: {id}. {OUTPUT_SAID}{output}. You will be notified when it completes.");
        serde_json::json!({ "type": "user", "timestamp": at(time),
            "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": said }] },
            "toolUseResult": { "stdout": "", "stderr": "", "interrupted": false, "backgroundTaskId": id } })
        .to_string()
    }

    fn agent_launch(id: &str, output: &str, time: i64) -> String {
        serde_json::json!({ "type": "user", "timestamp": at(time),
            "toolUseResult": { "isAsync": true, "status": "async_launched", "agentId": id, "outputFile": output } })
        .to_string()
    }

    fn monitor_launch(id: &str, time: i64) -> String {
        serde_json::json!({ "type": "user", "timestamp": at(time), "toolUseResult": { "taskId": id, "timeoutMs": 1_800_000, "persistent": false } })
            .to_string()
    }

    fn queued(notice: &str, time: i64) -> String {
        serde_json::json!({ "type": "queue-operation", "operation": "enqueue", "timestamp": at(time), "content": notice }).to_string()
    }

    fn ended(id: &str, status: &str) -> String {
        format!("{NOTICE_OPEN}\n<task-id>{id}</task-id>\n<status>{status}</status>\n<summary>Background command \"x\" completed</summary>\n{NOTICE_CLOSE}")
    }

    fn check_in(id: &str) -> String {
        format!("{NOTICE_OPEN}\n<task-id>{id}</task-id>\n<summary>Background command \"x\" is still running after 30m</summary>\n{NOTICE_CLOSE}\nThis is a check-in, not a completion.")
    }

    fn event(id: &str) -> String {
        format!("{NOTICE_OPEN}\n<task-id>{id}</task-id>\n<summary>Monitor event: \"e2e\"</summary>\n<event>Ran 52 tests</event>\n{NOTICE_CLOSE}")
    }

    fn waits(lines: &[String], now: i64) -> bool {
        waits_on_background(&lines.join("\n"), now)
    }

    #[test]
    fn a_command_running_in_the_background_is_waited_on_until_it_ends() {
        let launch = bash_launch("b1", "/nowhere/b1.output", 0);
        assert!(waits(std::slice::from_ref(&launch), T0 + 60));
        for status in ["completed", "failed", "killed"] {
            assert!(!waits(&[launch.clone(), queued(&ended("b1", status), 30)], T0 + 60), "{status}");
        }
        assert!(waits(&[launch.clone(), queued(&ended("b2", "completed"), 30)], T0 + 60), "another task ended");
        assert!(waits(&[launch, queued(&check_in("b1"), 30)], T0 + 60), "a check-in is no end");
    }

    #[test]
    fn an_agent_and_a_monitor_are_waited_on_too() {
        assert!(waits(&[agent_launch("a1", "/nowhere/a1.output", 0)], T0 + 60));
        let attachment = serde_json::json!({ "type": "attachment", "timestamp": at(10),
            "attachment": { "type": "queued_command", "prompt": ended("a1", "completed") } })
        .to_string();
        assert!(!waits(&[agent_launch("a1", "/nowhere/a1.output", 0), attachment], T0 + 60));
        assert!(waits(&[monitor_launch("m1", 0)], T0 + 60));
        let stop = serde_json::json!({ "type": "user", "timestamp": at(10),
            "toolUseResult": { "message": "Successfully stopped task: m1 (tail -f x)", "task_id": "m1", "task_type": "local_bash" } })
        .to_string();
        assert!(!waits(&[monitor_launch("m1", 0), stop], T0 + 60), "TaskStop ends it");
    }

    #[test]
    fn a_task_quiet_for_half_an_hour_no_longer_counts() {
        let launch = bash_launch("b1", "/nowhere/b1.output", 0);
        assert!(waits(std::slice::from_ref(&launch), T0 + QUIET - 1));
        assert!(!waits(std::slice::from_ref(&launch), T0 + QUIET), "a server left running");
        assert!(!waits(&[launch, queued(&check_in("b1"), QUIET - 5)], T0 + QUIET), "a check-in is no sign of life");
        let monitor = [monitor_launch("m1", 0), queued(&event("m1"), 1_000)];
        assert!(waits(&monitor, T0 + QUIET + 500), "its last event was 1_300 s ago");
        assert!(!waits(&monitor, T0 + 1_000 + QUIET));
    }

    #[test]
    fn a_write_to_the_output_file_is_a_sign_of_life() {
        let folder = std::env::temp_dir().join(format!("kindric-background-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let output = folder.join("b1.output");
        fs::write(&output, "step 1\n").unwrap();
        let now = crate::away::now();
        assert!(waits(&[bash_launch("b1", output.to_str().unwrap(), 0)], now), "launched long ago, written just now");
        assert!(!waits(&[bash_launch("b1", "/nowhere/b1.output", 0)], now), "no such file");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_notification_quoted_by_a_tool_or_without_a_launch_changes_nothing() {
        let quoted = serde_json::json!({ "type": "user", "timestamp": at(10),
            "message": { "role": "user", "content": [{ "type": "tool_result", "content": ended("b1", "completed") }] } })
        .to_string();
        assert!(waits(&[bash_launch("b1", "/nowhere/b1.output", 0), quoted], T0 + 60), "a grep that prints one");
        assert!(!waits(&[queued(&ended("b1", "completed"), 0)], T0 + 60));
        assert!(!waits(&[], T0));
        let foreground = serde_json::json!({ "type": "user", "timestamp": at(0), "toolUseResult": { "stdout": "ok", "backgroundTaskId": null } }).to_string();
        assert!(!waits(&[foreground], T0 + 60), "a command run in the foreground");
    }

    #[test]
    fn the_output_file_is_read_in_what_bash_says() {
        let launch: Value = serde_json::from_str(&bash_launch("b1", "/tmp/claude-1000/p/s/tasks/b1.output", 0)).unwrap();
        assert_eq!(output_said(&launch), Some(PathBuf::from("/tmp/claude-1000/p/s/tasks/b1.output")));
    }
}
