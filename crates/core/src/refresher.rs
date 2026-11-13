//! Keeps the session list current: refreshes when Claude Code or a mailbox writes
//! something, and every `fallback` in case a change was missed. Only the accounts written
//! to are queried again; the others keep what they gave last.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::accounts::{self, Found, SessionInfo, Snapshot};
use crate::mailbox::{self, Mailbox};
use crate::progress;
use crate::watcher::{self, ChangeWatcher, Wake};

/// Every account is queried again this often when some folder could not be watched.
const FALLBACK: Duration = Duration::from_secs(30);
/// And this often when everything is watched: each query runs `claude agents --json` per
/// account (about 0.55 s of processor each, all at once), so 30 s made a spike every 30 s.
/// What watching misses is a session killed outright, whose files stay as they were.
const CALM_FALLBACK: Duration = Duration::from_secs(300);
const SETTLE: Duration = Duration::from_millis(300);
const MAX_SETTLE: Duration = Duration::from_secs(1);
const WITHOUT_WATCHER: Duration = Duration::from_secs(10);

/// The session list with unread message counts by session id, or why it could not be read.
pub type Update = Result<(Snapshot, HashMap<String, usize>), String>;

/// What to read again: every account, or only those written to. A write in the mailbox
/// alone queries none: the unread counts are always counted again.
#[derive(Debug, PartialEq, Eq)]
enum Change {
    Full,
    Accounts {
        /// Accounts whose sessions may have changed: `claude agents --json` again.
        query: BTreeSet<String>,
        /// Accounts where only the files of some jobs were written, by short id: their progress
        /// lines are read again, without running claude (about 0.55 s of processor each time).
        jobs: BTreeMap<String, BTreeSet<String>>,
    },
}

/// The job a write in an account directory belongs to, when it is a file inside
/// `jobs/<id>/`: `jobs/<id>` itself is a job that appeared, which a query must list.
fn job_written(in_account: &Path) -> Option<String> {
    let mut parts = in_account.components();
    if parts.next()?.as_os_str() != "jobs" {
        return None;
    }
    let id = parts.next()?.as_os_str().to_string_lossy().into_owned();
    parts.next().map(|_| id)
}

/// The accounts the written `paths` belong to. A path of no account and not in the
/// mailbox, or no path at all, queries everything: better slow than stale.
fn classify(paths: &[PathBuf], account_dirs: &[(String, PathBuf)], mailbox_root: Option<&Path>) -> Change {
    if paths.is_empty() {
        return Change::Full;
    }
    let mut query = BTreeSet::new();
    let mut jobs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in paths {
        if mailbox_root.is_some_and(|root| path.starts_with(root)) {
            continue;
        }
        let Some((account, dir)) = account_dirs.iter().find(|(_, dir)| path.starts_with(dir)) else {
            return Change::Full;
        };
        match path.strip_prefix(dir).ok().and_then(job_written) {
            Some(job) => {
                jobs.entry(account.clone()).or_default().insert(job);
            },
            None => {
                query.insert(account.clone());
            },
        }
    }
    jobs.retain(|account, _| !query.contains(account));
    Change::Accounts { query, jobs }
}

/// The accounts to query, in registry order: those changed, and those never queried yet.
fn to_query(change: &Change, accounts: &[String], known: &BTreeMap<String, Found>) -> Vec<String> {
    accounts
        .iter()
        .filter(|account| match change {
            Change::Full => true,
            Change::Accounts { query, .. } => query.contains(*account) || !known.contains_key(*account),
        })
        .cloned()
        .collect()
}

/// Reads again the progress line of the jobs `ids` among the account's `sessions`, from the
/// account directory `dir`. False when one of them is not a known session: a query must list it.
fn reread_progress(sessions: &mut [SessionInfo], dir: &Path, ids: &BTreeSet<String>) -> bool {
    for id in ids {
        let Some(session) = sessions.iter_mut().find(|session| session.short_id.as_deref() == Some(id.as_str())) else {
            return false;
        };
        session.progress = progress::progress(dir, id);
    }
    true
}

struct Refresher {
    watcher: Result<ChangeWatcher, String>,
    mailbox: Result<Mailbox, String>,
    /// What each account gave when it was last queried.
    known: BTreeMap<String, Found>,
    account_dirs: Vec<(String, PathBuf)>,
    /// Checked again at each full refresh only: `claude auth status` takes about 0.2 s.
    default_logged_in: bool,
    /// Whether the sessions without a process are listed too.
    include_stopped: bool,
    /// Whether the last refresh watched every folder it should.
    watching_all: bool,
}

/// How long to wait for a change before querying everything again.
fn fallback(watching_all: bool) -> Duration {
    if watching_all { CALM_FALLBACK } else { FALLBACK }
}

impl Refresher {
    fn new(include_stopped: bool) -> Self {
        let mailbox = mailbox::default_root().map(Mailbox::new).and_then(|mailbox| {
            mailbox.create_root()?;
            Ok(mailbox)
        });
        Self {
            watcher: ChangeWatcher::new().map_err(|error| format!("file watching unavailable: {error}")),
            mailbox: mailbox.map_err(|error| format!("messages: {error}")),
            known: BTreeMap::new(),
            account_dirs: Vec::new(),
            default_logged_in: false,
            include_stopped,
            watching_all: false,
        }
    }

    /// Watches what appeared since the last call: an account, or a job directory, whose
    /// creation wakes the list up through its account's `jobs/`. Called before the accounts
    /// are queried, so that a state written meanwhile is not missed.
    fn watch(&mut self, registry_dir: &Path) -> Vec<String> {
        let dirs = self.dirs();
        let (Ok(watcher), Ok(mailbox)) = (&mut self.watcher, &self.mailbox) else {
            return Vec::new();
        };
        watcher.ensure(&watcher::targets(&dirs, mailbox.root(), registry_dir))
    }

    /// The progress lines of `ids` in `account`, read again from its files; false if the account
    /// has no list yet or one of them is not in it.
    fn reread_jobs(&mut self, account: &str, ids: &BTreeSet<String>) -> bool {
        let Some((_, dir)) = self.account_dirs.iter().find(|(name, _)| name == account) else {
            return false;
        };
        match self.known.get_mut(account) {
            Some(Ok(sessions)) => reread_progress(sessions, dir, ids),
            _ => false,
        }
    }

    fn dirs(&self) -> Vec<PathBuf> {
        self.account_dirs.iter().map(|(_, dir)| dir.clone()).collect()
    }

    fn unread_counts(&self, snapshot: &mut Snapshot) -> HashMap<String, usize> {
        let mailbox = match &self.mailbox {
            Ok(mailbox) => mailbox,
            Err(error) => {
                snapshot.errors.push(error.clone());
                return HashMap::new();
            },
        };
        snapshot
            .sessions
            .iter()
            .filter(|session| !session.isolated)
            .filter_map(|session| {
                let count = mailbox.unread_count(&session.session_id).ok()?;
                (count > 0).then(|| (session.session_id.clone(), count))
            })
            .collect()
    }

    fn refresh(&mut self, change: &Change) -> Update {
        let paths = accounts::paths().map_err(|error| error.to_string())?;
        if *change == Change::Full {
            self.default_logged_in = accounts::default_logged_in();
        }
        let (registry, isolated) = accounts::registered(&paths).map_err(|error| error.to_string())?;
        let (registered, implicit_default) = accounts::with_default(registry, self.default_logged_in);
        self.account_dirs = registered
            .iter()
            .map(|account| (account.clone(), accounts::dir_of(&paths, account, implicit_default)))
            .collect();
        let watch_errors = self.watch(&paths.config_dir);
        self.watching_all = self.watcher.is_ok() && watch_errors.is_empty();
        let mut asked = to_query(change, &registered, &self.known);
        if let Change::Accounts { jobs, .. } = change {
            for (account, ids) in jobs {
                if !asked.contains(account) && !self.reread_jobs(account, ids) {
                    asked.push(account.clone());
                }
            }
        }
        let queried = accounts::query_accounts(&paths, &asked, implicit_default, self.include_stopped);
        self.known.retain(|account, _| registered.contains(account));
        let mut timings = Vec::new();
        for query in queried {
            timings.push((query.account.clone(), query.took));
            self.known.insert(query.account, query.found);
        }
        let mut snapshot = accounts::assemble(registered, isolated, &self.known, timings);
        snapshot.implicit_default = implicit_default;
        if let Err(error) = &self.watcher {
            snapshot.errors.push(error.clone());
        }
        snapshot.errors.extend(watch_errors);
        let unread = self.unread_counts(&mut snapshot);
        Ok((snapshot, unread))
    }

    fn wait(&self) -> Change {
        let Ok(watcher) = &self.watcher else {
            thread::sleep(WITHOUT_WATCHER);
            return Change::Full;
        };
        match watcher.wait(fallback(self.watching_all), SETTLE, MAX_SETTLE, &self.dirs()) {
            Wake::Changed(paths) => {
                let mailbox_root = self.mailbox.as_ref().ok().map(Mailbox::root);
                classify(&paths, &self.account_dirs, mailbox_root)
            },
            Wake::TimedOut => Change::Full,
            Wake::Closed => {
                thread::sleep(WITHOUT_WATCHER);
                Change::Full
            },
        }
    }
}

/// Hands every new list to `publish`; stops once `publish` returns false. `include_stopped`
/// lists the sessions without a process too, so that they can be found and opened again.
pub fn spawn(include_stopped: bool, mut publish: impl FnMut(Update) -> bool + Send + 'static) {
    thread::spawn(move || {
        let mut refresher = Refresher::new(include_stopped);
        let mut change = Change::Full;
        loop {
            if !publish(refresher.refresh(&change)) {
                return;
            }
            change = refresher.wait();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> Vec<(String, PathBuf)> {
        ["lou", "louis", "pro"].iter().map(|name| (name.to_string(), PathBuf::from(format!("/a/{name}")))).collect()
    }

    fn changed(accounts: &[&str]) -> Change {
        Change::Accounts { query: accounts.iter().map(|account| account.to_string()).collect(), jobs: BTreeMap::new() }
    }

    fn jobs_written(account: &str, ids: &[&str]) -> Change {
        let ids = ids.iter().map(|id| id.to_string()).collect();
        Change::Accounts { query: BTreeSet::new(), jobs: BTreeMap::from([(account.to_string(), ids)]) }
    }

    fn known(accounts: &[&str]) -> BTreeMap<String, Found> {
        accounts.iter().map(|account| (account.to_string(), Ok(Vec::new()))).collect()
    }

    fn registry() -> Vec<String> {
        ["louis", "pro", "new"].map(String::from).to_vec()
    }

    #[test]
    fn everything_watched_waits_longer_before_querying_everything_again() {
        assert_eq!(fallback(true), CALM_FALLBACK);
        assert_eq!(fallback(false), FALLBACK);
        assert!(CALM_FALLBACK > FALLBACK);
    }

    #[test]
    fn a_message_queries_no_account() {
        let paths = [PathBuf::from("/m/inbox/abc/1.json")];
        assert_eq!(classify(&paths, &dirs(), Some(Path::new("/m"))), changed(&[]));
    }

    #[test]
    fn a_write_queries_only_its_account() {
        let paths = [PathBuf::from("/a/pro/jobs/abc/state.json"), PathBuf::from("/a/pro/sessions/1.json")];
        assert_eq!(classify(&paths, &dirs(), Some(Path::new("/m"))), changed(&["pro"]));
        let paths = [PathBuf::from("/a/louis/daemon/roster.json"), PathBuf::from("/m/x"), PathBuf::from("/a/lou/jobs/y")];
        assert_eq!(classify(&paths, &dirs(), Some(Path::new("/m"))), changed(&["lou", "louis"]));
    }

    #[test]
    fn an_account_is_not_mistaken_for_one_whose_name_it_starts() {
        let paths = [PathBuf::from("/a/louis/sessions/1.json")];
        assert_eq!(classify(&paths, &dirs(), None), changed(&["louis"]));
    }

    #[test]
    fn a_write_inside_jobs_only_reads_their_progress_again() {
        let paths = [
            PathBuf::from("/a/pro/jobs/abc/state.json"),
            PathBuf::from("/a/pro/jobs/abc/state.json.tmp.1"),
            PathBuf::from("/a/pro/jobs/def/timeline.jsonl"),
        ];
        assert_eq!(classify(&paths, &dirs(), None), jobs_written("pro", &["abc", "def"]));
        // A job that appeared, or a write elsewhere in the account, needs the query.
        assert_eq!(classify(&[PathBuf::from("/a/pro/jobs/abc")], &dirs(), None), changed(&["pro"]));
        let paths = [PathBuf::from("/a/pro/jobs/abc/state.json"), PathBuf::from("/a/pro/daemon/roster.json")];
        assert_eq!(classify(&paths, &dirs(), None), changed(&["pro"]));
        assert_eq!(to_query(&jobs_written("pro", &["abc"]), &registry(), &known(&["louis", "pro"])), ["new"]);
    }

    #[test]
    fn the_progress_of_a_known_job_is_read_again_and_an_unknown_job_asks_for_the_query() {
        let dir = std::env::temp_dir().join(format!("kindric-reread-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("jobs/abc")).unwrap();
        std::fs::write(dir.join("jobs/abc/state.json"), r#"{"state":"working","detail":"reading progress.rs"}"#).unwrap();
        let mut sessions = vec![SessionInfo {
            account: "pro".to_string(),
            state: accounts::SessionState::Working,
            short_id: Some("abc".to_string()),
            session_id: String::new(),
            name: String::new(),
            cwd: String::new(),
            isolated: false,
            progress: None,
        }];
        assert!(reread_progress(&mut sessions, &dir, &BTreeSet::from(["abc".to_string()])));
        assert_eq!(sessions[0].progress.as_deref(), Some("reading progress.rs"));
        assert!(!reread_progress(&mut sessions, &dir, &BTreeSet::from(["zzz".to_string()])));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unknown_path_or_none_queries_everything() {
        let paths = [PathBuf::from("/a/pro/jobs/x"), PathBuf::from("/elsewhere/file")];
        assert_eq!(classify(&paths, &dirs(), Some(Path::new("/m"))), Change::Full);
        assert_eq!(classify(&[], &dirs(), Some(Path::new("/m"))), Change::Full);
        // Without a mailbox, its paths belong to nothing known.
        assert_eq!(classify(&[PathBuf::from("/m/inbox/1.json")], &dirs(), None), Change::Full);
    }

    #[test]
    fn only_changed_and_new_accounts_are_queried() {
        let known = known(&["louis", "pro"]);
        assert_eq!(to_query(&changed(&[]), &registry(), &known), ["new"]);
        assert_eq!(to_query(&changed(&["pro"]), &registry(), &known), ["pro", "new"]);
        assert_eq!(to_query(&Change::Full, &registry(), &known), registry());
        let everything = self::known(&["louis", "pro", "new"]);
        assert!(to_query(&changed(&[]), &registry(), &everything).is_empty());
    }
}
