//! Wakes the session list up when Claude Code or a mailbox writes something, instead of
//! polling `claude agents --json` (about 0.55 s of CPU per account and call).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// What to watch in one account directory (`CLAUDE_CONFIG_DIR`):
/// `sessions/` gets a file per running session, rewritten when its state changes;
/// `daemon/` holds the roster of background sessions; `jobs/<id>/` their state;
/// `data/opencode/` the state of an OpenCode service (see `OPENCODE_DATA`).
/// The account directory itself is watched for these three folders, which a new account
/// does not have yet: what else it holds wakes nothing (`ChangeWatcher::wait`), as
/// `.claude.json`, which Claude Code rewrites all the time.
/// `registry_dir` holds the list of accounts: one added or removed shows at once.
/// `quiet_roots` are watched whole: the mailbox, and the requests of the overview, whose writes
/// query no account.
/// Reads `jobs/` to list the job directories present now.
pub fn targets(account_dirs: &[PathBuf], quiet_roots: &[&Path], registry_dir: &Path) -> Vec<(PathBuf, RecursiveMode)> {
    let mut targets = Vec::new();
    for dir in account_dirs {
        let jobs = dir.join("jobs");
        targets.push((dir.clone(), RecursiveMode::NonRecursive));
        targets.push((dir.join("sessions"), RecursiveMode::NonRecursive));
        targets.push((dir.join("daemon"), RecursiveMode::NonRecursive));
        targets.push((dir.join(OPENCODE_DATA[0]).join(OPENCODE_DATA[1]), RecursiveMode::NonRecursive));
        targets.extend(job_dirs(&jobs).into_iter().map(|job| (job, RecursiveMode::NonRecursive)));
        targets.push((jobs, RecursiveMode::NonRecursive));
    }
    targets.push((registry_dir.to_path_buf(), RecursiveMode::NonRecursive));
    targets.extend(quiet_roots.iter().map(|root| (root.to_path_buf(), RecursiveMode::Recursive)));
    targets
}

/// The directory of each background session, where only `state.json` is read. Not deeper:
/// `jobs/<id>/tmp/` holds links to whole trees (`~/.config`, `~/.local/share`), and watching
/// through them took tens of thousands of watches and failed on the first unreadable folder.
/// A link in `jobs/` is skipped for the same reason. `jobs/` itself is watched, so a new job
/// wakes the list up and the next call adds its directory.
fn job_dirs(jobs: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(jobs) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect()
}

/// The folders of an account directory that wake the list up when they appear.
const ACCOUNT_FOLDERS: [&str; 3] = ["sessions", "daemon", "jobs"];

/// The files right in the directory of a Codex account that Codex writes at each change of a
/// session: start and end of a turn, a permission asked and answered (measured on codex 0.171.0:
/// `state_5.sqlite-wal`, `thread_history_1.sqlite-wal`). Not a contract of Codex: if their names
/// change, the full refresh every few minutes still catches up.
const CODEX_STATE_FILES: [&str; 2] = ["state_", "thread_history_"];

/// Where the service of an OpenCode account keeps its data, under the account directory, and the
/// one file of it that changes with a session: OpenCode 2.3.2 writes nothing there at rest, and
/// only `opencode.db-wal` during a turn (measured 2026-11-22). Its `log/` is not watched.
const OPENCODE_DATA: [&str; 2] = ["data", "opencode"];
const OPENCODE_STATE_FILE: &str = "opencode.db-wal";

/// Whether `folder` is the data folder of the service of an OpenCode account.
fn is_opencode_data(folder: &Path, account_dirs: &[PathBuf]) -> bool {
    account_dirs.iter().any(|dir| folder == dir.join(OPENCODE_DATA[0]).join(OPENCODE_DATA[1]))
}

/// Whether a write at `path` may change the list: anything but a file or folder right in an
/// account directory other than `ACCOUNT_FOLDERS` and the state files of Codex, and in the data
/// folder of an OpenCode service, only its state file.
fn counts(path: &Path, account_dirs: &[PathBuf]) -> bool {
    let in_account = path.parent().is_some_and(|parent| account_dirs.iter().any(|dir| dir == parent));
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return !in_account;
    };
    if path.parent().is_some_and(|parent| is_opencode_data(parent, account_dirs)) {
        return name == OPENCODE_STATE_FILE;
    }
    !in_account
        || ACCOUNT_FOLDERS.contains(&name)
        || CODEX_STATE_FILES.iter().any(|prefix| name.starts_with(prefix) && name.ends_with(".sqlite-wal"))
}

/// The paths of a report that count, or None if none does. A report without paths counts:
/// something changed, where is not known.
fn counted(paths: Vec<PathBuf>, account_dirs: &[PathBuf]) -> Option<Vec<PathBuf>> {
    if paths.is_empty() {
        return Some(paths);
    }
    let kept: Vec<PathBuf> = paths.into_iter().filter(|path| counts(path, account_dirs)).collect();
    (!kept.is_empty()).then_some(kept)
}

pub struct ChangeWatcher {
    watcher: RecommendedWatcher,
    watched: HashSet<PathBuf>,
    changes: Receiver<Vec<PathBuf>>,
}

pub enum Wake {
    /// The paths written during the burst, so only what they belong to is read again.
    Changed(Vec<PathBuf>),
    TimedOut,
    Closed,
}

impl ChangeWatcher {
    pub fn new() -> notify::Result<Self> {
        let (sender, changes) = mpsc::channel();
        let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            // Reads (open/close events) are ignored: `claude agents --json` reads these files.
            let Ok(event) = result else {
                return;
            };
            // The receiver only disappears when the refresher stops.
            if event.need_rescan() {
                // Events were lost (a full inotify queue): no path, so everything is read again.
                let _ = sender.send(Vec::new());
            } else if event.kind.is_create() || event.kind.is_modify() || event.kind.is_remove() {
                let _ = sender.send(event.paths);
            }
        })?;
        Ok(Self { watcher, watched: HashSet::new(), changes })
    }

    /// Starts watching the targets not watched yet. Missing directories are skipped
    /// silently (an account that never ran a background session has no `jobs/`);
    /// other failures are returned. A watched directory that disappeared (a finished job)
    /// is forgotten: its watch went with it.
    pub fn ensure(&mut self, targets: &[(PathBuf, RecursiveMode)]) -> Vec<String> {
        self.watched.retain(|path| path.is_dir());
        let mut errors = Vec::new();
        for (path, mode) in targets {
            if self.watched.contains(path) || !path.is_dir() {
                continue;
            }
            match self.watcher.watch(path, *mode) {
                Ok(()) => {
                    self.watched.insert(path.clone());
                },
                Err(error) => errors.push(format!("cannot watch {}: {error}", path.display())),
            }
        }
        errors
    }

    /// Waits for a change, then lets a burst of writes settle so it counts as one: until
    /// `settle` passes without a write, but no longer than `max_settle`, or a session that
    /// keeps writing would hold the list back. Writes in `account_dirs` that do not count
    /// (see `counts`) are let go by without moving the `fallback` deadline.
    pub fn wait(&self, fallback: Duration, settle: Duration, max_settle: Duration, account_dirs: &[PathBuf]) -> Wake {
        let fallback_at = Instant::now() + fallback;
        let mut paths = loop {
            let left = fallback_at.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Wake::TimedOut;
            }
            match self.changes.recv_timeout(left) {
                Ok(paths) => {
                    if let Some(kept) = counted(paths, account_dirs) {
                        break kept;
                    }
                },
                Err(RecvTimeoutError::Timeout) => return Wake::TimedOut,
                Err(RecvTimeoutError::Disconnected) => return Wake::Closed,
            }
        };
        let deadline = Instant::now() + max_settle;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match self.changes.recv_timeout(settle.min(left)) {
                Ok(more) => paths.extend(more.into_iter().filter(|path| counts(path, account_dirs))),
                Err(_) => break,
            }
        }
        Wake::Changed(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;

    #[test]
    fn targets_cover_each_account_the_list_of_accounts_and_the_quiet_roots() {
        let dirs = [PathBuf::from("/a/louis"), PathBuf::from("/a/pro")];
        let targets = targets(&dirs, &[Path::new("/m"), Path::new("/o")], Path::new("/c"));
        assert_eq!(targets.len(), 13);
        assert!(targets.contains(&(PathBuf::from("/c"), RecursiveMode::NonRecursive)));
        assert!(targets.contains(&(PathBuf::from("/a/louis"), RecursiveMode::NonRecursive)));
        assert!(targets.contains(&(PathBuf::from("/a/pro/jobs"), RecursiveMode::NonRecursive)));
        assert!(targets.contains(&(PathBuf::from("/a/louis/sessions"), RecursiveMode::NonRecursive)));
        assert!(targets.contains(&(PathBuf::from("/a/pro/data/opencode"), RecursiveMode::NonRecursive)));
        assert!(targets.contains(&(PathBuf::from("/m"), RecursiveMode::Recursive)));
        assert_eq!(targets.last(), Some(&(PathBuf::from("/o"), RecursiveMode::Recursive)));
        assert!(!targets.iter().any(|(path, _)| path.ends_with(".claude.json")));
    }

    /// Waits until a wake-up reports a file named `name`, letting other reports go by.
    fn woken_by(watcher: &ChangeWatcher, name: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Wake::Changed(paths) = watcher.wait(Duration::from_millis(500), Duration::from_millis(50), Duration::from_secs(1), &[])
                && paths.iter().any(|path| path.file_name().is_some_and(|file| file == name))
            {
                return true;
            }
        }
        false
    }

    #[test]
    fn each_job_directory_is_watched_but_not_what_it_holds() {
        let root = std::env::temp_dir().join(format!("kindric-watch-targets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("jobs/abc/tmp/deep")).unwrap();
        fs::create_dir_all(root.join("jobs/def")).unwrap();
        fs::write(root.join("jobs/notes.txt"), b"").unwrap();
        let mut found = targets(std::slice::from_ref(&root), &[Path::new("/m")], Path::new("/c"));
        found.retain(|(path, _)| path.starts_with(root.join("jobs")));
        found.sort();
        let jobs = root.join("jobs");
        let expected = [jobs.clone(), jobs.join("abc"), jobs.join("def")].map(|path| (path, RecursiveMode::NonRecursive));
        assert_eq!(found, expected);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_new_job_wakes_up_the_list_and_its_state_is_watched_next() {
        let root = std::env::temp_dir().join(format!("kindric-watch-new-job-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("jobs")).unwrap();
        let account = [root.clone()];
        let mut watcher = ChangeWatcher::new().unwrap();
        assert!(watcher.ensure(&targets(&account, &[root.join("mailbox").as_path()], &root.join("registry"))).is_empty());
        fs::create_dir(root.join("jobs/abc")).unwrap();
        assert!(woken_by(&watcher, "abc"), "the new job did not wake the list up");
        assert!(watcher.ensure(&targets(&account, &[root.join("mailbox").as_path()], &root.join("registry"))).is_empty());
        fs::write(root.join("jobs/abc/state.json"), b"{}").unwrap();
        assert!(woken_by(&watcher, "state.json"), "the state of the new job did not wake the list up");
        fs::remove_dir_all(root).unwrap();
    }

    /// A new account has none of `sessions/`, `daemon/`, `jobs/`: the first one Claude Code
    /// creates wakes the list up, which then watches it.
    #[test]
    fn the_first_folder_of_a_new_account_wakes_the_list_up() {
        let root = std::env::temp_dir().join(format!("kindric-watch-new-account-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let account = [root.clone()];
        let mut watcher = ChangeWatcher::new().unwrap();
        assert!(watcher.ensure(&targets(&account, &[root.join("mailbox").as_path()], &root.join("registry"))).is_empty());
        fs::create_dir(root.join("sessions")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut woken = false;
        while !woken && Instant::now() < deadline {
            woken = matches!(watcher.wait(Duration::from_millis(500), Duration::from_millis(50), Duration::from_secs(1), &account),
                Wake::Changed(paths) if paths.iter().any(|path| path.ends_with("sessions")));
        }
        assert!(woken, "the new sessions/ did not wake the list up");
        assert!(watcher.ensure(&targets(&account, &[root.join("mailbox").as_path()], &root.join("registry"))).is_empty());
        assert!(watcher.watched.contains(&root.join("sessions")));
        fs::remove_dir_all(root).unwrap();
    }

    /// Claude Code rewrites `.claude.json` all the time, in the account directory now watched.
    #[test]
    fn the_state_files_of_codex_wake_the_list_up_and_its_logs_do_not() {
        let account = PathBuf::from("/d/kindric-accounts/cx");
        let dirs = [account.clone()];
        assert!(counts(&account.join("state_5.sqlite-wal"), &dirs));
        assert!(counts(&account.join("thread_history_1.sqlite-wal"), &dirs));
        assert!(!counts(&account.join("logs_2.sqlite-wal"), &dirs), "Codex logs all the time");
        assert!(!counts(&account.join("state_5.sqlite"), &dirs));
        assert!(!counts(&account.join("history.jsonl"), &dirs));
        assert!(counts(&account.join("sessions"), &dirs));
        assert!(counts(&account.join("sessions").join("x.json"), &dirs), "inside a folder, as before");
    }

    #[test]
    fn the_state_file_of_an_opencode_service_wakes_the_list_up_and_the_rest_of_its_data_does_not() {
        let account = PathBuf::from("/d/kindric-accounts/oc");
        let dirs = [account.clone()];
        let data = account.join("data").join("opencode");
        assert!(counts(&data.join("opencode.db-wal"), &dirs));
        assert!(!counts(&data.join("opencode.db-shm"), &dirs));
        assert!(!counts(&data.join("opencode.db"), &dirs));
        assert!(!counts(&data.join("log"), &dirs));
        assert!(!counts(&account.join("data"), &dirs));
        assert!(targets(&dirs, &[], Path::new("/d/registry")).contains(&(data, RecursiveMode::NonRecursive)));
    }

    #[test]
    fn a_write_beside_the_account_folders_does_not_wake_the_list_up() {
        let made = std::env::temp_dir().join(format!("kindric-watch-beside-{}", std::process::id()));
        let _ = fs::remove_dir_all(&made);
        fs::create_dir_all(&made).unwrap();
        // The path as events name it (/private/var on macOS for /var).
        let root = fs::canonicalize(&made).unwrap();
        let account = [root.clone()];
        let mut watcher = ChangeWatcher::new().unwrap();
        assert!(watcher.ensure(&targets(&account, &[root.join("mailbox").as_path()], &root.join("registry"))).is_empty());
        // macOS can report the creation of the folder itself after watching has started (CI,
        // 2026-11-14): those reports go first, so that only the writes below are tested.
        let quiet_by = Instant::now() + Duration::from_secs(5);
        while Instant::now() < quiet_by
            && matches!(watcher.wait(Duration::from_millis(300), Duration::from_millis(50), Duration::from_secs(1), &account), Wake::Changed(_))
        {}
        let started = Instant::now();
        for index in 0..5 {
            fs::write(root.join(".claude.json"), index.to_string()).unwrap();
            fs::write(root.join("history.jsonl"), index.to_string()).unwrap();
        }
        match watcher.wait(Duration::from_millis(800), Duration::from_millis(50), Duration::from_secs(1), &account) {
            Wake::TimedOut => {},
            Wake::Changed(paths) => panic!("woken by a change of {paths:?}"),
            Wake::Closed => panic!("the watcher stopped"),
        }
        assert!(started.elapsed() < Duration::from_secs(2), "the writes let go by moved the fallback deadline");
        fs::remove_dir_all(root).unwrap();
    }

    /// Claude Code links `jobs/<id>/tmp/` to whole trees; one unreadable folder in them made
    /// the whole `jobs/` unwatched, after setting tens of thousands of watches.
    #[cfg(unix)]
    #[test]
    fn a_link_to_an_unreadable_folder_in_a_job_is_neither_an_error_nor_watched() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = std::env::temp_dir().join(format!("kindric-watch-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (account, outside) = (root.join("account"), root.join("outside"));
        fs::create_dir_all(account.join("jobs/abc/tmp")).unwrap();
        fs::create_dir_all(outside.join("locked")).unwrap();
        fs::set_permissions(outside.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        symlink(&outside, account.join("jobs/abc/tmp/config")).unwrap();
        symlink(&outside, account.join("jobs/linked")).unwrap();
        let mut watcher = ChangeWatcher::new().unwrap();
        let errors = watcher.ensure(&targets(std::slice::from_ref(&account), &[root.join("mailbox").as_path()], &root.join("registry")));
        fs::set_permissions(outside.join("locked"), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!watcher.watched.iter().any(|path| path.starts_with(account.join("jobs/abc/tmp")) || path.ends_with("linked")));
        fs::write(outside.join("written"), b"").unwrap();
        assert!(!woken_by(&watcher, "written"), "a write behind a link woke the list up");
        fs::write(account.join("jobs/abc/state.json"), b"{}").unwrap();
        assert!(woken_by(&watcher, "state.json"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_write_wakes_up_and_a_burst_counts_once() {
        let root = std::env::temp_dir().join(format!("kindric-watch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("jobs/abc")).unwrap();
        let mut watcher = ChangeWatcher::new().unwrap();
        let errors = watcher.ensure(&[
            (root.join("jobs"), RecursiveMode::Recursive),
            (root.join("missing"), RecursiveMode::NonRecursive),
        ]);
        assert!(errors.is_empty(), "{errors:?}");
        for index in 0..5 {
            fs::write(root.join(format!("jobs/abc/state-{index}.json")), b"{}").unwrap();
        }
        let started = Instant::now();
        let Wake::Changed(paths) = watcher.wait(Duration::from_secs(5), Duration::from_millis(200), Duration::from_secs(5), &[]) else {
            panic!("no change seen");
        };
        assert!(started.elapsed() < Duration::from_secs(2));
        // Some systems name the directory differently (/private/var on macOS): the file names count.
        for index in 0..5 {
            let name = format!("state-{index}.json");
            assert!(paths.iter().any(|path| path.file_name().is_some_and(|file| file == name.as_str())), "{name} not in {paths:?}");
        }
        let quiet = watcher.wait(Duration::from_millis(300), Duration::from_millis(50), Duration::from_secs(5), &[]);
        assert!(matches!(quiet, Wake::TimedOut));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_session_that_keeps_writing_does_not_hold_the_list_back() {
        let root = std::env::temp_dir().join(format!("kindric-watch-busy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut watcher = ChangeWatcher::new().unwrap();
        assert!(watcher.ensure(&[(root.clone(), RecursiveMode::Recursive)]).is_empty());
        let writing = root.clone();
        let writer = std::thread::spawn(move || {
            for index in 0..20 {
                fs::write(writing.join("state.json"), index.to_string()).unwrap();
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let started = Instant::now();
        let woke = watcher.wait(Duration::from_secs(5), Duration::from_millis(300), Duration::from_secs(1), &[]);
        let waited = started.elapsed();
        writer.join().unwrap();
        assert!(matches!(woke, Wake::Changed(_)));
        assert!(waited < Duration::from_millis(1500), "waited {waited:?} while writes went on for 2 s");
        fs::remove_dir_all(root).unwrap();
    }

    /// `claude agents --json` reads the watched files: reading must not wake us up,
    /// or every refresh triggers the next one.
    #[test]
    fn reading_a_watched_file_does_not_wake_up() {
        let root = std::env::temp_dir().join(format!("kindric-watch-read-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("roster.json"), b"{}").unwrap();
        let mut watcher = ChangeWatcher::new().unwrap();
        assert!(watcher.ensure(&[(root.clone(), RecursiveMode::Recursive)]).is_empty());
        // macOS can report the folder and the file written above after watching has started:
        // those reports go first, so that only the reads below are tested.
        let quiet_by = Instant::now() + Duration::from_secs(5);
        while Instant::now() < quiet_by
            && matches!(watcher.wait(Duration::from_millis(300), Duration::from_millis(50), Duration::from_secs(1), &[]), Wake::Changed(_))
        {}
        fs::read(root.join("roster.json")).unwrap();
        fs::read_dir(&root).unwrap().for_each(drop);
        match watcher.wait(Duration::from_millis(500), Duration::from_millis(50), Duration::from_secs(1), &[]) {
            Wake::TimedOut => {},
            Wake::Changed(paths) => panic!("woken by a change of {paths:?}"),
            Wake::Closed => panic!("the watcher stopped"),
        }
        fs::remove_dir_all(root).unwrap();
    }
}
