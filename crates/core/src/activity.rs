//! When each session was last active: when its transcript, `<account>/projects/*/<session id>.jsonl`,
//! was last written. Claude Code writes it at every step, and it stays after the session stops,
//! so a paused session has one too. A Codex session has its own,
//! `<account>/sessions/<year>/<month>/<day>/rollout-<time>-<session id>.jsonl`. Neither file is
//! documented: a session whose transcript cannot be found has no time, never an error.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// For each session of `wanted` with a transcript in `account_dir`, when it was last written
/// (Unix seconds); the latest when several folders hold one (the folder changes when the session
/// moves to a worktree). Each folder is listed once, whatever the number of sessions: the list
/// asks for every session of the account at each refresh.
pub fn last_written(account_dir: &Path, wanted: &HashSet<&str>) -> HashMap<String, i64> {
    let mut latest = HashMap::new();
    for folder in subfolders(&account_dir.join("projects")) {
        note_files(&folder, wanted, claude_session, &mut latest);
    }
    let days = subfolders(&account_dir.join("sessions")).into_iter().flat_map(|year| subfolders(&year)).flat_map(|month| subfolders(&month));
    for day in days {
        note_files(&day, wanted, codex_session, &mut latest);
    }
    latest
}

fn subfolders(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries.flatten().filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir())).map(|entry| entry.path()).collect()
}

/// The session of a transcript of Claude Code: `<session id>.jsonl`.
fn claude_session(name: &str) -> Option<&str> {
    name.strip_suffix(".jsonl")
}

/// The session of a transcript of Codex: `rollout-<time>-<session id>.jsonl`, the id being a
/// UUID of 36 characters.
fn codex_session(name: &str) -> Option<&str> {
    let stem = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    stem.len().checked_sub(36).and_then(|start| stem.get(start..))
}

/// Notes, for each file of `folder` that `session_of` gives a wanted session, when it was written.
fn note_files(folder: &Path, wanted: &HashSet<&str>, session_of: fn(&str) -> Option<&str>, latest: &mut HashMap<String, i64>) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(session) = name.to_str().and_then(session_of).filter(|session| wanted.contains(session)) else {
            continue;
        };
        let Some(written) = entry.metadata().ok().and_then(|meta| meta.modified().ok()).and_then(|time| time.duration_since(UNIX_EPOCH).ok()) else {
            continue;
        };
        let seconds = i64::try_from(written.as_secs()).unwrap_or(i64::MAX);
        latest.entry(session.to_string()).and_modify(|known: &mut i64| *known = (*known).max(seconds)).or_insert(seconds);
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::time::{Duration, SystemTime};

    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-activity-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn transcript(account: &Path, folder: &str, name: &str, at: u64) {
        let dir = account.join("projects").join(folder);
        fs::create_dir_all(&dir).unwrap();
        let file = File::create(dir.join(name)).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(at)).unwrap();
    }

    #[test]
    fn each_session_wanted_gets_the_latest_write_of_its_transcripts() {
        let account = temp_root("latest");
        transcript(&account, "-home-u-app", "aaaa-1111.jsonl", 1_000);
        // The same session after it moved to a worktree: written later.
        transcript(&account, "-home-u-app--claude-worktrees-fix", "aaaa-1111.jsonl", 3_000);
        transcript(&account, "-home-u-app", "bbbb-2222.jsonl", 2_000);
        transcript(&account, "-home-u-app", "cccc-3333.jsonl", 5_000);
        transcript(&account, "-home-u-app", "dddd-4444.txt", 6_000);
        fs::create_dir_all(account.join("projects/-home-u-app/aaaa-1111/subagents")).unwrap();

        let wanted = HashSet::from(["aaaa-1111", "bbbb-2222", "dddd-4444", "eeee-5555"]);
        let latest = last_written(&account, &wanted);
        assert_eq!(latest, HashMap::from([("aaaa-1111".to_string(), 3_000), ("bbbb-2222".to_string(), 2_000)]));
        fs::remove_dir_all(&account).unwrap();
    }

    #[test]
    fn a_codex_session_gets_the_time_of_its_rollout() {
        let account = temp_root("codex");
        let id = "01a1ff2a-77e6-7382-a87e-6bb95a3c1b59";
        let day = account.join("sessions/2026/11/21");
        fs::create_dir_all(&day).unwrap();
        let file = File::create(day.join(format!("rollout-2026-11-21T18-32-15-{id}.jsonl"))).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(4_000)).unwrap();
        File::create(day.join("rollout-short.jsonl")).unwrap();
        let latest = last_written(&account, &HashSet::from([id, "aaaa-1111"]));
        assert_eq!(latest, HashMap::from([(id.to_string(), 4_000)]));
        assert_eq!(codex_session("rollout-x.jsonl"), None);
        fs::remove_dir_all(&account).unwrap();
    }

    #[test]
    fn an_account_without_transcripts_gives_no_time() {
        let account = temp_root("none");
        assert!(last_written(&account, &HashSet::from(["aaaa-1111"])).is_empty());
    }
}
