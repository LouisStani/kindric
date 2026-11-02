//! The progress line of a background session: the short `detail` Claude Code writes at each
//! change of state in `<account>/jobs/<short id>/state.json`, its history in `timeline.jsonl`.
//! These files are not documented: whatever they lack or change gives no line, never an error.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::Deserialize;

/// Longer lines are cut, in characters.
const LONGEST: usize = 160;
/// Only the end of the timeline is read: it grows with the session.
const TIMELINE_TAIL: u64 = 64 * 1024;

#[derive(Deserialize)]
struct Entry {
    state: Option<String>,
    detail: Option<String>,
}

/// The detail of an entry, unless it only repeats its state ("stopped", "done").
fn useful(entry: Entry) -> Option<String> {
    let detail = entry.detail?.trim().to_string();
    let repeats_state = entry.state.is_some_and(|state| state.eq_ignore_ascii_case(&detail));
    (!detail.is_empty() && !repeats_state).then_some(detail)
}

pub fn detail_of(state_json: &str) -> Option<String> {
    useful(serde_json::from_str(state_json).ok()?)
}

/// The last useful detail of the end of a timeline; a line cut by the read or broken is skipped.
pub fn last_detail(timeline_tail: &str) -> Option<String> {
    timeline_tail.lines().rev().find_map(|line| useful(serde_json::from_str(line).ok()?))
}

/// On one line, cut after `LONGEST` characters.
pub fn one_line(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= LONGEST {
        return flat;
    }
    let kept: String = flat.chars().take(LONGEST - 1).collect();
    format!("{}…", kept.trim_end())
}

fn timeline_tail(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(TIMELINE_TAIL))).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The progress line of the background session `short_id` of the account in `account_dir`.
pub fn progress(account_dir: &Path, short_id: &str) -> Option<String> {
    let job = account_dir.join("jobs").join(short_id);
    let current = std::fs::read_to_string(job.join("state.json")).ok().and_then(|text| detail_of(&text));
    let line = current.or_else(|| last_detail(&timeline_tail(&job.join("timeline.jsonl"))?))?;
    Some(one_line(&line))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-progress-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn the_detail_counts_unless_it_only_repeats_the_state() {
        assert_eq!(detail_of(r#"{"state":"blocked","detail":"awaiting a call on X"}"#).as_deref(), Some("awaiting a call on X"));
        assert_eq!(detail_of(r#"{"state":"stopped","detail":"Stopped"}"#), None);
        assert_eq!(detail_of(r#"{"state":"done","detail":"  "}"#), None);
        assert_eq!(detail_of(r#"{"state":"done"}"#), None);
        assert_eq!(detail_of("{broken"), None);
    }

    #[test]
    fn the_timeline_gives_its_last_useful_detail() {
        let tail = [
            r#"cut":"half a line"}"#,
            r#"{"at":"1","state":"working","detail":"writing the parser"}"#,
            r#"{"at":"2","state":"blocked","detail":"tests green, waiting for review"}"#,
            "{broken",
            r#"{"at":"3","state":"stopped","detail":"stopped"}"#,
        ]
        .join("\n");
        assert_eq!(last_detail(&tail).as_deref(), Some("tests green, waiting for review"));
        assert_eq!(last_detail(r#"{"state":"done","detail":"done"}"#), None);
    }

    #[test]
    fn a_line_is_flattened_and_cut_by_characters() {
        assert_eq!(one_line("tests green.\n\nwaiting  for\treview"), "tests green. waiting for review");
        let long = "é".repeat(200);
        let cut = one_line(&long);
        assert_eq!(cut.chars().count(), LONGEST);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn a_stopped_session_shows_its_last_line_and_no_files_show_none() {
        let root = scratch("files");
        let job = root.join("jobs/abc12345");
        fs::create_dir_all(&job).unwrap();
        fs::write(job.join("state.json"), r#"{"state":"stopped","detail":"stopped"}"#).unwrap();
        fs::write(job.join("timeline.jsonl"), "{\"state\":\"blocked\",\"detail\":\"spec paused,\\nawaiting a call\"}\n").unwrap();
        assert_eq!(progress(&root, "abc12345").as_deref(), Some("spec paused, awaiting a call"));
        assert_eq!(progress(&root, "missing0"), None);
        fs::remove_dir_all(root).unwrap();
    }
}
