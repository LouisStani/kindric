//! Ctrl+Shift+F: the sessions whose conversation holds the words of a query (see
//! `kindric_core::history`), the last match first. Their files are read on a few threads; a newer
//! search makes an older one stop (`Searches`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use kindric_core::attach::Agent;
use kindric_core::history::{self, Found};
use kindric_core::opencode::Service;
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
    /// Unix seconds.
    pub at: Option<i64>,
}

/// The number of the last search the page asked for: an older one sees it changed and stops.
/// The page numbers its searches: two calls may run in another order than they were sent.
#[derive(Default)]
pub struct Searches(AtomicU64);

impl Searches {
    /// Search `number` starts: those with a lower number stop.
    pub fn start(&self, number: u64) {
        self.0.fetch_max(number, Ordering::SeqCst);
    }

    pub fn is_current(&self, number: u64) -> bool {
        self.0.load(Ordering::SeqCst) == number
    }
}

/// The OpenCode services that run, by account, for the OpenCode sessions of `sessions`: each is
/// asked once whether it runs, never started; `None` when the search is no longer wanted.
fn running_services(sessions: &[(SessionRow, PathBuf)], stop: &dyn Fn() -> bool) -> Option<HashMap<String, Service>> {
    let mut asked: HashMap<&str, Option<Service>> = HashMap::new();
    for (row, dir) in sessions.iter().filter(|(row, _)| row.info.agent == Agent::OpenCode) {
        if asked.contains_key(row.account.as_str()) {
            continue;
        }
        if stop() {
            return None;
        }
        asked.insert(&row.account, Service::running(dir).ok().flatten());
    }
    Some(asked.into_iter().filter_map(|(account, service)| Some((account.to_string(), service?))).collect())
}

/// What the session of `row`, of the account in `account_dir`, said that holds every word:
/// from its transcript, its rollout, or its account's OpenCode service in `services` if it runs.
fn found_in(account_dir: &Path, row: &SessionRow, services: &HashMap<String, Service>, words: &[String], stop: &(dyn Fn() -> bool + Sync)) -> Option<Found> {
    let id = &row.info.session_id;
    match row.info.agent {
        Agent::ClaudeCode => history::search(&images::transcript_of(account_dir, id)?, Agent::ClaudeCode, words, stop),
        Agent::Codex => history::search(&codex_history::rollout_of(account_dir, id)?, Agent::Codex, words, stop),
        Agent::OpenCode => history::search_messages(&services.get(&row.account)?.messages(id).ok()?, words),
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
    let sessions: Vec<(SessionRow, PathBuf)> = rows.into_iter().filter_map(|row| dir_of(&row.account).map(|dir| (row, dir))).collect();
    let Some(services) = running_services(&sessions, stop) else {
        return Vec::new();
    };
    let services = &services;
    let chunk = sessions.len().div_ceil(THREADS).max(1);
    let mut hits: Vec<Hit> = std::thread::scope(|scope| {
        let workers: Vec<_> = sessions
            .chunks(chunk)
            .map(|part| {
                let words = &words;
                scope.spawn(move || {
                    // Finding a Codex rollout walks the account's days, asking OpenCode goes through
                    // its service: in the threads too, and given up with the search.
                    part.iter()
                        .take_while(|_| !stop())
                        .filter_map(|(row, dir)| {
                            let Found { matches, excerpt, at } = found_in(dir, row, services, words, stop)?;
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

/// The last match first; a session whose agent gives no time last.
fn order(hits: &mut [Hit]) {
    hits.sort_by_key(|hit| std::cmp::Reverse(hit.at));
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
    fn a_match_without_a_time_comes_last() {
        let view = apply(&ListView::default(), Ok((Snapshot { sessions: vec![session("louis", "a", Agent::ClaudeCode)], ..Snapshot::default() }, HashMap::new())), None);
        let hit = |at: Option<i64>| Hit { row: view.sessions[0].clone(), matches: 1, excerpt: String::new(), at };
        let mut hits = vec![hit(Some(5)), hit(None), hit(Some(9))];
        order(&mut hits);
        assert_eq!(hits.iter().map(|hit| hit.at).collect::<Vec<_>>(), [Some(9), Some(5), None]);
    }

    #[test]
    fn a_newer_search_makes_the_older_one_stop() {
        let searches = Searches::default();
        searches.start(1);
        assert!(searches.is_current(1));
        searches.start(2);
        assert!(!searches.is_current(1));
        assert!(searches.is_current(2));
        searches.start(1);
        assert!(searches.is_current(2), "an older search that starts late does not stop the newer");
        assert!(!searches.is_current(1));
    }
}
