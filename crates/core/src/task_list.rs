//! The task list a session keeps with Claude Code's TaskCreate and TaskUpdate (decision 368): what
//! it does concretely, how far it is and the time left, for kindric's page; and
//! `kindric task-check`, the PreToolUse hook that has a long turn make one, since sessions do not
//! on their own. Claude Code writes each task in `<config>/tasks/<session id>/<id>.json`, the
//! file's date being its last change, and `.lock` when the list starts. None of this is
//! documented: whatever is missing or changes gives no progress and refuses nothing.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::images::{is_prompt, shell_command, tail};
use crate::mailbox::is_valid_session_id;

/// How far a session is in its task list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TaskProgress {
    /// The task in progress, in the words Claude Code shows while it runs ("Writing the tests").
    pub doing: Option<String>,
    /// What that task is about, as the session described it.
    pub about: Option<String>,
    pub done: usize,
    pub total: usize,
    /// Seconds left at the pace of the tasks done so far; none before one is done.
    pub left_seconds: Option<u64>,
    /// The whole list, in order.
    pub tasks: Vec<TaskItem>,
}

/// A task of the list, as the page lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TaskItem {
    pub subject: String,
    /// `pending`, `in_progress` or `completed`, as Claude Code writes it.
    pub status: String,
}

#[derive(Debug, Deserialize)]
struct Task {
    id: String,
    subject: Option<String>,
    description: Option<String>,
    #[serde(rename = "activeForm")]
    active_form: Option<String>,
    status: Option<String>,
}

impl Task {
    fn is(&self, status: &str) -> bool {
        self.status.as_deref() == Some(status)
    }

    fn is_open(&self) -> bool {
        self.is("pending") || self.is("in_progress")
    }

    fn number(&self) -> u64 {
        self.id.parse().unwrap_or(u64::MAX)
    }
}

/// A task marked done this soon after the list started was done before it: it does not set the
/// pace.
const DONE_BEFORE_SECONDS: u64 = 30;

fn seconds(path: &Path) -> Option<u64> {
    Some(fs::metadata(path).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

fn list_dir(config_dir: &Path, session_id: &str) -> Option<PathBuf> {
    is_valid_session_id(session_id).then(|| config_dir.join("tasks").join(session_id))
}

/// The tasks of the list in `dir`, each with when it last changed, in id order.
fn tasks_in(dir: &Path) -> Vec<(Task, u64)> {
    let mut tasks: Vec<(Task, u64)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .filter_map(|path| Some((serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?, seconds(&path)?)))
        .collect();
    tasks.sort_by_key(|(task, _)| task.number());
    tasks
}

/// The progress of `tasks` (each with when it last changed) on a list started at `started`, while
/// one is left to do; none once all are done.
fn progress_of(tasks: &[(Task, u64)], started: Option<u64>) -> Option<TaskProgress> {
    if !tasks.iter().any(|(task, _)| task.is_open()) {
        return None;
    }
    let done: Vec<u64> = tasks.iter().filter(|(task, _)| task.is("completed")).map(|(_, at)| *at).collect();
    let current = tasks.iter().map(|(task, _)| task).find(|task| task.is("in_progress"));
    let left_seconds = started.and_then(|started| {
        let paced: Vec<u64> = done.iter().copied().filter(|at| *at > started + DONE_BEFORE_SECONDS).collect();
        let last = *paced.iter().max()?;
        let per_task = (last - started) / paced.len() as u64;
        Some(per_task * (tasks.len() - done.len()) as u64)
    });
    let text = |value: &Option<String>| value.as_deref().map(str::trim).filter(|text| !text.is_empty()).map(str::to_string);
    Some(TaskProgress {
        doing: current.and_then(|task| text(&task.active_form).or_else(|| text(&task.subject))),
        about: current.and_then(|task| text(&task.description)),
        done: done.len(),
        total: tasks.len(),
        left_seconds,
        tasks: tasks
            .iter()
            .map(|(task, _)| TaskItem {
                subject: text(&task.subject).unwrap_or_else(|| format!("task {}", task.id)),
                status: task.status.clone().unwrap_or_default(),
            })
            .collect(),
    })
}

/// How far the session `session_id` of the account in `config_dir` is in its task list; none
/// without a list, or once all its tasks are done. A list none of whose tasks changed since
/// `turn_started` (Unix seconds), when the start of the turn is known, is an old one the session
/// left: none either (decision 499).
pub fn progress(config_dir: &Path, session_id: &str, turn_started: Option<i64>) -> Option<TaskProgress> {
    let dir = list_dir(config_dir, session_id)?;
    let tasks = tasks_in(&dir);
    let changed_in_turn = |start: i64| tasks.iter().any(|(_, at)| i64::try_from(*at).is_ok_and(|at| at >= start));
    if turn_started.is_some_and(|start| !changed_in_turn(start)) {
        return None;
    }
    progress_of(&tasks, seconds(&dir.join(".lock")))
}

/// From this many tools in one turn without a task list, the next one is refused.
pub const LONG_TURN: usize = 8;

/// What the session reads when a tool is refused.
pub const REASON: &str = "This turn is long: before going on, create your task list with TaskCreate (one task \
per step of this work, with a subject, a description and an activeForm), then keep each task's status up \
to date with TaskUpdate as you go. The user follows what you do, how far you are and the time left on \
kindric's page.";

/// The tools of the last turn of a transcript's end (since the user's last prompt, or the last
/// command typed with `!`, which Claude answers too), and whether one of them made a task.
fn turn_tools(transcript_tail: &str) -> (usize, bool) {
    let mut count = 0;
    let mut created = false;
    for entry in transcript_tail.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
        if is_prompt(&entry) || shell_command(&entry).is_some() {
            (count, created) = (0, false);
            continue;
        }
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let parts = entry.pointer("/message/content").and_then(Value::as_array).into_iter().flatten();
        for part in parts.filter(|part| part.get("type").and_then(Value::as_str) == Some("tool_use")) {
            count += 1;
            created |= part.get("name").and_then(Value::as_str) == Some("TaskCreate");
        }
    }
    (count, created)
}

/// What `kindric task-check` prints for `input`, the JSON Claude Code gives a PreToolUse hook: a
/// refusal once the turn ran `LONG_TURN` tools with no task made in it and none left to do.
/// ToolSearch, which loads TaskCreate, and the Task tools always pass, as do the tools of a
/// subagent.
pub fn hook_answer(input: &str, config_dir: &Path) -> Option<String> {
    let input: Value = serde_json::from_str(input).ok()?;
    if input.get("hook_event_name").and_then(Value::as_str) != Some("PreToolUse") || input.get("agent_id").is_some() {
        return None;
    }
    let tool = input.get("tool_name").and_then(Value::as_str)?;
    if tool == "ToolSearch" || tool.starts_with("Task") {
        return None;
    }
    let session = input.get("session_id").and_then(Value::as_str)?;
    if tasks_in(&list_dir(config_dir, session)?).iter().any(|(task, _)| task.is_open()) {
        return None;
    }
    let (count, created) = turn_tools(&tail(Path::new(input.get("transcript_path")?.as_str()?))?);
    if created || count < LONG_TURN {
        return None;
    }
    let refusal = serde_json::json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": REASON } });
    Some(refusal.to_string())
}

/// `kindric task-check`. Nothing that goes wrong may disturb the session: it prints nothing then,
/// and the tool runs.
pub fn run_hook() {
    let mut input = String::new();
    if io::Read::read_to_string(&mut io::stdin(), &mut input).is_err() {
        return;
    }
    let Some(config_dir) = crate::permission::config_dir() else {
        return;
    };
    if let Some(answer) = hook_answer(&input, &config_dir) {
        println!("{answer}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{Duration, SystemTime};

    const SESSION: &str = "aaaa-1111";

    fn task(id: &str, status: &str) -> Task {
        Task {
            id: id.into(),
            subject: Some(format!("subject {id}")),
            description: Some(format!("about {id}")),
            active_form: Some(format!("doing {id}")),
            status: Some(status.into()),
        }
    }

    #[test]
    fn the_task_in_progress_how_many_are_done_and_the_time_left_at_their_pace() {
        // Started at 1000; 1 was done before the list (at 1005), 2 took until 1100, 3 until 1200.
        let tasks = [(task("1", "completed"), 1005), (task("2", "completed"), 1100), (task("3", "completed"), 1200), (task("4", "in_progress"), 1201), (task("5", "pending"), 1000)];
        let progress = progress_of(&tasks, Some(1000)).unwrap();
        assert_eq!(progress.doing.as_deref(), Some("doing 4"));
        assert_eq!(progress.about.as_deref(), Some("about 4"));
        assert_eq!((progress.done, progress.total), (3, 5));
        assert_eq!(progress.left_seconds, Some(200), "100 s a task over 2 paced tasks, 2 left");
        let listed: Vec<(&str, &str)> = progress.tasks.iter().map(|item| (item.subject.as_str(), item.status.as_str())).collect();
        assert_eq!(listed[3], ("subject 4", "in_progress"));
        assert_eq!(listed.len(), 5);
    }

    #[test]
    fn no_time_left_before_a_task_is_done_nor_without_the_start() {
        let first = [(task("1", "completed"), 1010), (task("2", "in_progress"), 1011)];
        assert_eq!(progress_of(&first, Some(1000)).unwrap().left_seconds, None, "only one done before the list");
        let tasks = [(task("1", "completed"), 1100), (task("2", "pending"), 1000)];
        assert_eq!(progress_of(&tasks, None).unwrap().left_seconds, None);
        assert_eq!(progress_of(&tasks, None).unwrap().doing, None, "nothing in progress");
    }

    #[test]
    fn a_list_all_done_or_empty_shows_nothing() {
        assert_eq!(progress_of(&[(task("1", "completed"), 1100)], Some(1000)), None);
        assert_eq!(progress_of(&[], Some(1000)), None);
    }

    #[test]
    fn without_an_active_form_the_subject_says_what_it_does() {
        let mut current = task("1", "in_progress");
        current.active_form = Some("  ".into());
        assert_eq!(progress_of(&[(current, 1)], None).unwrap().doing.as_deref(), Some("subject 1"));
    }

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-tasks-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn write_task(dir: &Path, id: &str, status: &str, at: SystemTime) {
        let path = dir.join(format!("{id}.json"));
        let body = json!({"id": id, "subject": format!("subject {id}"), "description": "d", "activeForm": format!("doing {id}"), "status": status, "blocks": [], "blockedBy": []});
        fs::write(&path, body.to_string()).unwrap();
        fs::File::options().write(true).open(&path).unwrap().set_modified(at).unwrap();
    }

    /// The files of the trial of 2026-11-18 (Claude Code 2.1.327), shortened.
    #[test]
    fn the_progress_is_read_from_claude_codes_task_files() {
        let root = scratch("read");
        let dir = root.join("tasks").join(SESSION);
        fs::create_dir_all(&dir).unwrap();
        let start = SystemTime::now() - Duration::from_secs(600);
        fs::write(dir.join(".lock"), "").unwrap();
        fs::File::options().write(true).open(dir.join(".lock")).unwrap().set_modified(start).unwrap();
        fs::write(dir.join(".highwatermark"), "3").unwrap();
        write_task(&dir, "1", "completed", start + Duration::from_secs(120));
        write_task(&dir, "2", "in_progress", start + Duration::from_secs(121));
        write_task(&dir, "10", "pending", start);
        let progress = progress(&root, SESSION, None).unwrap();
        assert_eq!((progress.doing.as_deref(), progress.done, progress.total), (Some("doing 2"), 1, 3));
        let order: Vec<&str> = progress.tasks.iter().map(|item| item.subject.as_str()).collect();
        assert_eq!(order, ["subject 1", "subject 2", "subject 10"], "by number, not by name");
        assert_eq!(progress.left_seconds, Some(240));
        assert_eq!(super::progress(&root, "../x", None), None);
        assert_eq!(super::progress(&root, "bbbb-2222", None), None, "no list");
        fs::remove_dir_all(&root).unwrap();
    }

    // Decision 499: a list the turn did not touch is an old one, left behind.
    #[test]
    fn a_list_no_task_of_which_changed_in_the_turn_is_left_out() {
        let root = scratch("turn");
        let dir = root.join("tasks").join(SESSION);
        fs::create_dir_all(&dir).unwrap();
        let start = SystemTime::now() - Duration::from_secs(600);
        write_task(&dir, "1", "completed", start + Duration::from_secs(100));
        write_task(&dir, "2", "in_progress", start + Duration::from_secs(200));
        let at = |offset: u64| i64::try_from((start + Duration::from_secs(offset)).duration_since(UNIX_EPOCH).unwrap().as_secs()).unwrap();
        assert!(progress(&root, SESSION, Some(at(200))).is_some(), "the last change in the turn's first second counts");
        assert!(progress(&root, SESSION, Some(at(150))).is_some());
        assert_eq!(progress(&root, SESSION, Some(at(201))), None, "nothing changed since the prompt");
        fs::remove_dir_all(&root).unwrap();
    }

    fn prompt(text: &str) -> String {
        json!({"type": "user", "message": {"role": "user", "content": text}}).to_string()
    }

    fn tools(names: &[&str]) -> String {
        let parts: Vec<Value> = names.iter().map(|name| json!({"type": "tool_use", "name": name, "input": {}})).collect();
        json!({"type": "assistant", "message": {"content": parts}}).to_string()
    }

    fn result() -> String {
        json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "ok"}]}}).to_string()
    }

    #[test]
    fn the_tools_are_counted_since_the_last_prompt() {
        let tail = [prompt("first"), tools(&["Bash"; 9]), prompt("second"), tools(&["Read", "Bash"]), result(), tools(&["Edit"])].join("\n");
        assert_eq!(turn_tools(&tail), (3, false));
        let made = [prompt("go"), tools(&["Bash", "TaskCreate"]), result()].join("\n");
        assert_eq!(turn_tools(&made), (2, true));
        let notified = [prompt("go"), tools(&["Bash"]), prompt("<task-notification>done</task-notification>"), tools(&["Bash"])].join("\n");
        assert_eq!(turn_tools(&notified).0, 2, "a notification is not the user's prompt");
        // A command typed with `!` starts a turn too: Claude answers it.
        let shell = [prompt("go"), tools(&["Bash"; 9]), prompt("<bash-input>git push</bash-input>"), tools(&["Bash"])].join("\n");
        assert_eq!(turn_tools(&shell).0, 1, "a command typed with ! starts a turn");
    }

    fn pre_tool(tool: &str, transcript: &Path) -> String {
        json!({"session_id": SESSION, "hook_event_name": "PreToolUse", "tool_name": tool, "transcript_path": transcript, "tool_input": {}}).to_string()
    }

    #[test]
    fn a_long_turn_without_a_list_is_refused_until_it_makes_one() {
        let root = scratch("hook");
        fs::create_dir_all(&root).unwrap();
        let transcript = root.join("t.jsonl");
        fs::write(&transcript, [prompt("go"), tools(&["Bash"; LONG_TURN - 1])].join("\n")).unwrap();
        assert_eq!(hook_answer(&pre_tool("Write", &transcript), &root), None, "short so far");
        fs::write(&transcript, [prompt("go"), tools(&["Bash"; LONG_TURN])].join("\n")).unwrap();
        let refused: Value = serde_json::from_str(&hook_answer(&pre_tool("Write", &transcript), &root).unwrap()).unwrap();
        assert_eq!(refused["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(refused["hookSpecificOutput"]["permissionDecisionReason"], REASON);
        for passes in ["ToolSearch", "TaskCreate", "TaskUpdate"] {
            assert_eq!(hook_answer(&pre_tool(passes, &transcript), &root), None, "{passes}");
        }
        let mut subagent: Value = serde_json::from_str(&pre_tool("Write", &transcript)).unwrap();
        subagent["agent_id"] = json!("a1");
        assert_eq!(hook_answer(&subagent.to_string(), &root), None, "a subagent's tool");

        let dir = root.join("tasks").join(SESSION);
        fs::create_dir_all(&dir).unwrap();
        write_task(&dir, "1", "pending", SystemTime::now());
        assert_eq!(hook_answer(&pre_tool("Write", &transcript), &root), None, "a task left to do");
        write_task(&dir, "1", "completed", SystemTime::now());
        assert!(hook_answer(&pre_tool("Write", &transcript), &root).is_some(), "an old list, all done");
        fs::write(&transcript, [prompt("go"), tools(&["Bash"; LONG_TURN]), tools(&["TaskCreate"]), tools(&["Bash"; 3])].join("\n")).unwrap();
        assert_eq!(hook_answer(&pre_tool("Write", &transcript), &root), None, "a list made in this turn, now all done");
        assert_eq!(hook_answer("", &root), None, "the probe of kindric-accounts gives no input");
        fs::remove_dir_all(&root).unwrap();
    }
}
