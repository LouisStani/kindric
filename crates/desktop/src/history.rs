//! Ctrl+Shift+F: the sessions whose conversation holds the words of a query (see
//! `kindric_core::history`), the last match first. Their files are read on a few threads; a newer
//! search makes an older one stop (`Searches`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use kindric_core::attach::Agent;
use kindric_core::history::{self, Found};
use kindric_core::{codex_history, images};
use serde::Serialize;

use crate::view::SessionRow;

/// Threads that read the files of one search.
const THREADS: usize = 4;

/// At most this many sessions are shown.
pub const MOST: usize = 50;

/// One session found, with what the page shows of it.
#[derive(Clone, Debug, Serialize)]
pub struct Hit {
    #[serde(flatten)]
    pub row: SessionRow,
    pub matches: usize,
    pub excerpt: String,
    pub at: Option<String>,
}

/// The number of the last search asked for: an older one sees it changed and stops.
#[derive(Default)]
pub struct Searches(AtomicU64);

impl Searches {
    /// A new search, which makes the one before stop.
    pub fn start(&self) -> u64 {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn is_current(&self, search: u64) -> bool {
        self.0.load(Ordering::SeqCst) == search
    }
}

/// The file that holds what was said in a session of `row`, in `account_dir`, if one does:
/// OpenCode keeps its sessions in its service, not in a file.
fn file_of(account_dir: &Path, row: &SessionRow) -> Option<PathBuf> {
    match row.info.agent {
        Agent::ClaudeCode => images::transcript_of(account_dir, &row.info.session_id),
        Agent::Codex => codex_history::rollout_of(account_dir, &row.info.session_id),
        Agent::OpenCode => None,
    }
}

/// The sessions of `rows` whose conversation holds every word of `query`, the last match first,
/// `MOST` at most; nothing when `stop` says the search is no longer wanted. `dir_of` gives the
/// directory of an account.
pub fn search(rows: Vec<SessionRow>, query: &str, dir_of: &dyn Fn(&str) -> Option<PathBuf>, stop: &(dyn Fn() -> bool + Sync)) -> Vec<Hit> {
    let words = history::words_of(query);
    if words.is_empty() {
        return Vec::new();
    }
    let files: Vec<(SessionRow, PathBuf)> = rows.into_iter().filter_map(|row| file_of(&dir_of(&row.account)?, &row).map(|file| (row, file))).collect();
    let chunk = files.len().div_ceil(THREADS).max(1);
    let mut hits: Vec<Hit> = std::thread::scope(|scope| {
        let workers: Vec<_> = files
            .chunks(chunk)
            .map(|part| {
                let words = &words;
                scope.spawn(move || {
                    part.iter()
                        .filter_map(|(row, file)| {
                            let Found { matches, excerpt, at } = history::search(file, row.info.agent, words, stop)?;
                            Some(Hit { row: row.clone(), matches, excerpt, at })
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers.into_iter().flat_map(|worker| worker.join().unwrap_or_default()).collect()
    });
    if stop() {
        return Vec::new();
    }
    order(&mut hits);
    hits.truncate(MOST);
    hits
}

/// The last match first; a session whose file gives no time last. Both agents write times in
/// ISO 8601 in UTC, which sort as text.
fn order(hits: &mut [Hit]) {
    hits.sort_by(|a, b| b.at.cmp(&a.at));
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use kindric_core::accounts::{SessionInfo, SessionState, Snapshot};
    use serde_json::json;

    use super::*;
    use crate::view::{ListView, apply};

    fn session(account: &str, id: &str, agent: Agent) -> SessionInfo {
        SessionInfo {
            account: account.into(),
            short_id: Some(id.into()),
            session_id: id.into(),
            name: id.into(),
            cwd: "/home/u/Dev".into(),
            state: SessionState::Waiting,
            isolated: false,
            progress: None,
            tasks: None,
            doing: None,
            waiting_for: None,
            agent,
        }
    }

    fn said(at: &str, text: &str) -> String {
        format!("{}\n", json!({"type": "user", "timestamp": at, "message": {"content": text}}))
    }

    #[test]
    fn the_sessions_are_found_the_last_match_first() {
        let root = std::env::temp_dir().join(format!("kindric-history-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folder = root.join("louis").join("projects").join("-home-u-Dev");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("old.jsonl"), said("2026-11-20T10:00:00.000Z", "the logo")).unwrap();
        std::fs::write(folder.join("new.jsonl"), said("2026-11-27T10:00:00.000Z", "a new logo")).unwrap();
        std::fs::write(folder.join("none.jsonl"), said("2026-11-28T10:00:00.000Z", "nothing here")).unwrap();
        let sessions = vec![
            session("louis", "old", Agent::ClaudeCode),
            session("louis", "new", Agent::ClaudeCode),
            session("louis", "none", Agent::ClaudeCode),
            session("louis", "missing", Agent::ClaudeCode),
            session("oc", "new", Agent::OpenCode),
            session("gone", "new", Agent::ClaudeCode),
        ];
        let view = apply(&ListView::default(), Ok((Snapshot { sessions, ..Snapshot::default() }, HashMap::new())), None);
        let dir_of = |account: &str| (account != "gone").then(|| root.join(account));
        let hits = search(view.sessions.clone(), "LOGO", &dir_of, &|| false);
        let found: Vec<(&str, &str)> = hits.iter().map(|hit| (hit.row.account.as_str(), hit.row.session_id.as_str())).collect();
        assert_eq!(found, [("louis", "new"), ("louis", "old")]);
        assert_eq!(hits[0].excerpt, "a new logo");
        assert!(search(view.sessions.clone(), "logo", &dir_of, &|| true).is_empty(), "a search no longer wanted");
        assert!(search(view.sessions, " ", &dir_of, &|| false).is_empty(), "no word");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_newer_search_makes_the_older_one_stop() {
        let searches = Searches::default();
        let first = searches.start();
        assert!(searches.is_current(first));
        let second = searches.start();
        assert!(!searches.is_current(first));
        assert!(searches.is_current(second));
    }
}
