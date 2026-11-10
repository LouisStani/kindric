//! Keeps the session list current: refreshes when Claude Code or a mailbox writes
//! something, and every `FALLBACK` in case a change was missed. Only the accounts written
//! to are queried again; the others keep what they gave last.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::accounts::{self, Found, Snapshot};
use crate::mailbox::{self, Mailbox};
use crate::watcher::{self, ChangeWatcher, Wake};

const FALLBACK: Duration = Duration::from_secs(30);
const SETTLE: Duration = Duration::from_millis(300);
const MAX_SETTLE: Duration = Duration::from_secs(1);
const WITHOUT_WATCHER: Duration = Duration::from_secs(10);

/// The session list with unread message counts by session id, or why it could not be read.
pub type Update = Result<(Snapshot, HashMap<String, usize>), String>;

/// What to query again: every account, or only those written to. A write in the mailbox
/// alone queries none: the unread counts are always counted again.
#[derive(Debug, PartialEq, Eq)]
enum Change {
    Full,
    Accounts(BTreeSet<String>),
}

/// The accounts the written `paths` belong to. A path of no account and not in the
/// mailbox, or no path at all, queries everything: better slow than stale.
fn classify(paths: &[PathBuf], account_dirs: &[(String, PathBuf)], mailbox_root: Option<&Path>) -> Change {
    if paths.is_empty() {
        return Change::Full;
    }
    let mut accounts = BTreeSet::new();
    for path in paths {
        if mailbox_root.is_some_and(|root| path.starts_with(root)) {
            continue;
        }
        match account_dirs.iter().find(|(_, dir)| path.starts_with(dir)) {
            Some((account, _)) => {
                accounts.insert(account.clone());
            },
            None => return Change::Full,
        }
    }
    Change::Accounts(accounts)
}

/// The accounts to query, in registry order: those changed, and those never queried yet.
fn to_query(change: &Change, accounts: &[String], known: &BTreeMap<String, Found>) -> Vec<String> {
    accounts
        .iter()
        .filter(|account| match change {
            Change::Full => true,
            Change::Accounts(changed) => changed.contains(*account) || !known.contains_key(*account),
        })
        .cloned()
        .collect()
}

struct Refresher {
    watcher: Result<ChangeWatcher, String>,
    mailbox: Result<Mailbox, String>,
    watched_accounts: Vec<String>,
    /// What each account gave when it was last queried.
    known: BTreeMap<String, Found>,
    account_dirs: Vec<(String, PathBuf)>,
    /// Checked again at each full refresh only: `claude auth status` takes about 0.2 s.
    default_logged_in: bool,
}

impl Refresher {
    fn new() -> Self {
        let mailbox = mailbox::default_root().map(Mailbox::new).and_then(|mailbox| {
            mailbox.create_root()?;
            Ok(mailbox)
        });
        Self {
            watcher: ChangeWatcher::new().map_err(|error| format!("file watching unavailable: {error}")),
            mailbox: mailbox.map_err(|error| format!("messages: {error}")),
            watched_accounts: Vec::new(),
            known: BTreeMap::new(),
            account_dirs: Vec::new(),
            default_logged_in: false,
        }
    }

    /// New directories to watch appear only when the account list changes.
    fn watch_accounts(&mut self, snapshot: &mut Snapshot, registry_dir: &Path) {
        let (Ok(watcher), Ok(mailbox)) = (&mut self.watcher, &self.mailbox) else {
            return;
        };
        if snapshot.accounts == self.watched_accounts {
            return;
        }
        let dirs: Vec<PathBuf> = self.account_dirs.iter().map(|(_, dir)| dir.clone()).collect();
        snapshot.errors.extend(watcher.ensure(&watcher::targets(&dirs, mailbox.root(), registry_dir)));
        self.watched_accounts = snapshot.accounts.clone();
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
        let asked = to_query(change, &registered, &self.known);
        let queried = accounts::query_accounts(&paths, &asked, implicit_default);
        self.known.retain(|account, _| registered.contains(account));
        let mut timings = Vec::new();
        for query in queried {
            timings.push((query.account.clone(), query.took));
            self.known.insert(query.account, query.found);
        }
        self.account_dirs = registered
            .iter()
            .map(|account| (account.clone(), accounts::dir_of(&paths, account, implicit_default)))
            .collect();
        let mut snapshot = accounts::assemble(registered, isolated, &self.known, timings);
        snapshot.implicit_default = implicit_default;
        if let Err(error) = &self.watcher {
            snapshot.errors.push(error.clone());
        }
        self.watch_accounts(&mut snapshot, &paths.config_dir);
        let unread = self.unread_counts(&mut snapshot);
        Ok((snapshot, unread))
    }

    fn wait(&self) -> Change {
        let Ok(watcher) = &self.watcher else {
            thread::sleep(WITHOUT_WATCHER);
            return Change::Full;
        };
        match watcher.wait(FALLBACK, SETTLE, MAX_SETTLE) {
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

/// Hands every new list to `publish`; stops once `publish` returns false.
pub fn spawn(mut publish: impl FnMut(Update) -> bool + Send + 'static) {
    thread::spawn(move || {
        let mut refresher = Refresher::new();
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
        Change::Accounts(accounts.iter().map(|account| account.to_string()).collect())
    }

    fn known(accounts: &[&str]) -> BTreeMap<String, Found> {
        accounts.iter().map(|account| (account.to_string(), Ok(Vec::new()))).collect()
    }

    fn registry() -> Vec<String> {
        ["louis", "pro", "new"].map(String::from).to_vec()
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
        let paths = [PathBuf::from("/a/louis/jobs/abc/state.json")];
        assert_eq!(classify(&paths, &dirs(), None), changed(&["louis"]));
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
