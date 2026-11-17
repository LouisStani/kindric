//! When each session was last active: when its transcript, `<account>/projects/*/<session id>.jsonl`,
//! was last written. Claude Code writes it at every step, and it stays after the session stops,
//! so a paused session has one too. That file is not documented: a session whose transcript
//! cannot be found has no time, never an error.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// For each session of `wanted` with a transcript in `account_dir`, when it was last written
/// (Unix seconds); the latest when several folders hold one (the folder changes when the session
/// moves to a worktree). Each folder is listed once, whatever the number of sessions: the list
/// asks for every session of the account at each refresh.
pub fn last_written(account_dir: &Path, wanted: &HashSet<&str>) -> HashMap<String, i64> {
    let mut latest = HashMap::new();
    let Ok(folders) = fs::read_dir(account_dir.join("projects")) else {
        return latest;
    };
    for folder in folders.flatten() {
        let Ok(entries) = fs::read_dir(folder.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(session) = name.to_str().and_then(|name| name.strip_suffix(".jsonl")).filter(|session| wanted.contains(session)) else {
                continue;
            };
            let Some(written) = entry.metadata().ok().and_then(|meta| meta.modified().ok()).and_then(|time| time.duration_since(UNIX_EPOCH).ok()) else {
                continue;
            };
            let seconds = i64::try_from(written.as_secs()).unwrap_or(i64::MAX);
            latest.entry(session.to_string()).and_modify(|known: &mut i64| *known = (*known).max(seconds)).or_insert(seconds);
        }
    }
    latest
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::path::PathBuf;
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
    fn an_account_without_transcripts_gives_no_time() {
        let account = temp_root("none");
        assert!(last_written(&account, &HashSet::from(["aaaa-1111"])).is_empty());
    }
}
