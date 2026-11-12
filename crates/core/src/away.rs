//! The user is away: a session keeps working until a time instead of stopping to wait for an
//! approval. `away/<session id>` in kindric's data holds that time (Unix seconds); the Stop hook
//! `kindric away-hook` refuses the end of a turn before it. The session says it is done with
//! `kindric away done <session id>`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use kindric_accounts::files::{create_private_dir, write_atomic};

use crate::mailbox::{data_dir, is_valid_session_id};

/// `away` in the data directory.
pub fn default_root() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("away"))
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
}

fn file(root: &Path, session: &str) -> io::Result<PathBuf> {
    if !is_valid_session_id(session) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid session id: {session:?}")));
    }
    Ok(root.join(session))
}

/// The session `session` keeps working until `until`.
pub fn set(root: &Path, session: &str, until: i64) -> io::Result<()> {
    let file = file(root, session)?;
    create_private_dir(root)?;
    write_atomic(&file, &until.to_string())
}

/// The user is back, or the session is done: it may stop again. Nothing to clear is no error.
pub fn clear(root: &Path, session: &str) -> io::Result<()> {
    match fs::remove_file(file(root, session)?) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Until when `session` keeps working, if the user said they are away.
pub fn until(root: &Path, session: &str) -> Option<i64> {
    fs::read_to_string(file(root, session).ok()?).ok()?.trim().parse().ok()
}

/// What the session is told when it tries to stop while the user is away.
fn instruction(session: &str, minutes: i64) -> String {
    format!(
        "The user is away for {minutes} more minute{s} and asked you to keep working without waiting for their approval. \
         Continue the task: when a choice is needed, make it, write it down where the project keeps its decisions, and go on. \
         Allowed alone: edit, test, commit in your own worktree, write notes in memory. \
         Never while they are away: push, merge, delete what you did not create, install, use another account. \
         When the whole task is done and verified, run `kindric away done {session}`, then stop.",
        s = if minutes == 1 { "" } else { "s" },
    )
}

/// What `kindric away-hook` prints for `input`, the JSON Claude Code gives a Stop hook: a refusal
/// to stop while the user is away, else nothing. Once the time has passed, the file goes.
pub fn stop_hook_answer(root: &Path, input: &str, now: i64) -> Option<String> {
    let input: serde_json::Value = serde_json::from_str(input).ok()?;
    if input.get("hook_event_name")?.as_str()? != "Stop" {
        return None;
    }
    let session = input.get("session_id")?.as_str()?;
    let until = until(root, session)?;
    if until <= now {
        let _ = clear(root, session);
        return None;
    }
    // Rounded up: with 30 seconds left the session is told 1 minute, not 0.
    let minutes = (until - now + 59) / 60;
    Some(serde_json::json!({ "decision": "block", "reason": instruction(session, minutes) }).to_string())
}

/// `kindric away-hook`: reads Claude Code's input and prints the answer. Nothing that goes wrong
/// may disturb the session: it prints nothing then.
pub fn run_away_hook() {
    let mut input = String::new();
    if io::Read::read_to_string(&mut io::stdin(), &mut input).is_err() {
        return;
    }
    let Ok(root) = default_root() else {
        return;
    };
    if let Some(answer) = stop_hook_answer(&root, &input, now()) {
        println!("{answer}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-away-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn stop(session: &str) -> String {
        format!(r#"{{"session_id":"{session}","hook_event_name":"Stop","stop_hook_active":false}}"#)
    }

    #[test]
    fn stopping_is_refused_until_the_time() {
        let root = temp_root("refused");
        set(&root, "aaaa-1111", 1_000 + 42 * 60 - 30).unwrap();
        let answer = stop_hook_answer(&root, &stop("aaaa-1111"), 1_000).unwrap();
        let value: serde_json::Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(value["decision"], "block");
        let reason = value["reason"].as_str().unwrap();
        assert!(reason.starts_with("The user is away for 42 more minutes"), "{reason}");
        assert!(reason.contains("`kindric away done aaaa-1111`"), "{reason}");
        assert!(reason.contains("Never while they are away: push, merge"), "{reason}");
        assert_eq!(until(&root, "aaaa-1111"), Some(1_000 + 42 * 60 - 30), "still away");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn after_the_time_the_session_stops_and_the_file_goes() {
        let root = temp_root("over");
        set(&root, "aaaa-1111", 1_000).unwrap();
        assert_eq!(stop_hook_answer(&root, &stop("aaaa-1111"), 1_000), None);
        assert_eq!(until(&root, "aaaa-1111"), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn other_sessions_and_events_are_left_alone() {
        let root = temp_root("others");
        set(&root, "aaaa-1111", 9_999).unwrap();
        assert_eq!(stop_hook_answer(&root, &stop("bbbb-2222"), 1_000), None);
        let other_event = r#"{"session_id":"aaaa-1111","hook_event_name":"SubagentStop"}"#;
        assert_eq!(stop_hook_answer(&root, other_event, 1_000), None);
        assert_eq!(stop_hook_answer(&root, "not json", 1_000), None);
        assert_eq!(stop_hook_answer(&root, &stop("../aaaa-1111"), 1_000), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn done_lets_the_session_stop() {
        let root = temp_root("done");
        set(&root, "aaaa-1111", 9_999).unwrap();
        clear(&root, "aaaa-1111").unwrap();
        assert_eq!(stop_hook_answer(&root, &stop("aaaa-1111"), 1_000), None);
        clear(&root, "aaaa-1111").unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_malformed_id_is_refused() {
        let root = temp_root("malformed");
        assert!(set(&root, "../x", 9_999).is_err());
        assert!(clear(&root, "a/b").is_err());
        assert!(!root.exists());
    }

    #[test]
    fn one_minute_left_is_said_in_the_singular() {
        assert!(instruction("aaaa-1111", 1).starts_with("The user is away for 1 more minute and"));
    }
}
