//! What a session changed in the repositories it worked in: one block per working copy (the
//! checkout it started in, each worktree it went into), each compared with its state before the
//! session. The copies are the `cwd` of the messages of the session's conversation,
//! `<account>/projects/<directory>/<session id>.jsonl`, a file Claude Code does not document:
//! whatever it lacks gives fewer blocks, never an error. Git is asked through its command.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

/// Untracked files bigger than this are listed without counting their lines.
const LARGEST_COUNTED: u64 = 1024 * 1024;
/// Git calls a file binary when its first 8000 bytes hold a NUL; kindric does the same.
const BINARY_PROBE: usize = 8000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Relative to the copy, with `/` as git writes it.
    pub path: String,
    pub kind: ChangeKind,
    /// Lines added and removed; `None` for a binary file or an untracked one too big to count.
    pub added: Option<u32>,
    pub removed: Option<u32>,
}

/// Where the comparison of a copy starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// The state kindric wrote down when it launched the session.
    Recorded,
    /// The commit the copy's branch was created from, the session having created it.
    BranchCreated,
    /// The commit the copy was on when the session first worked in it, from git's log of HEAD:
    /// what was not committed then counts as changed by the session.
    Estimated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Changes { copy: PathBuf, branch: Option<String>, start: Start, files: Vec<FileChange> },
    /// A copy that no longer exists, such as a worktree removed since.
    Removed { copy: PathBuf },
    Failed { copy: PathBuf, error: String },
}

/// The state of the copy a session starts in, written down by kindric before it launches it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recorded {
    /// The top of the copy, as `git rev-parse --show-toplevel` prints it.
    pub copy: PathBuf,
    /// A commit holding the tracked files as they were, uncommitted changes included.
    pub commit: String,
    /// The untracked files then, which the session did not add.
    pub untracked: Vec<String>,
}

/// The state of the copy `directory` is in, taken just before a session is launched there.
/// `None` outside a repository, in one without any commit, or without git: the session's block
/// is then estimated, and the launch goes on.
pub fn record(directory: &Path) -> Result<Option<Recorded>, String> {
    let run = |dir: &Path, args: &[&str]| match git(dir, args) {
        Ok(output) => Ok(Some(text(&output).trim_end().to_string())),
        Err(GitError::Missing(_)) => Ok(None),
        Err(GitError::Failed(error)) => Err(error),
    };
    let Ok(Some(top)) = run(directory, &["rev-parse", "--show-toplevel"]) else {
        return Ok(None);
    };
    let copy = PathBuf::from(top);
    // A commit of the tracked files as they are, made without touching the branch, the stash
    // list or the files; nothing when nothing is uncommitted. It is never shared: an identity of
    // its own spares git asking for the user's, which may not be set.
    let identity = ["-c", "user.name=kindric", "-c", "user.email=kindric@localhost", "-c", "commit.gpgSign=false"];
    let Ok(Some(stash)) = run(&copy, &[&identity[..], &["stash", "create"][..]].concat()) else {
        return Ok(None);
    };
    let commit = match stash.trim() {
        "" => match run(&copy, &["rev-parse", "--verify", "-q", "HEAD"]) {
            Ok(Some(head)) => head,
            _ => return Ok(None),
        },
        sha => sha.to_string(),
    };
    let listed = git(&copy, &["--no-optional-locks", "ls-files", "-z", "--others", "--exclude-standard"])?;
    let untracked = nul_fields(&listed).filter(|path| !path.ends_with('/')).collect();
    Ok(Some(Recorded { copy, commit, untracked }))
}

/// `<data dir>/starts/<account>-<short id>.json`, the state written down at a session's launch.
fn recorded_file(data_dir: &Path, account: &str, short_id: &str) -> Option<PathBuf> {
    let plain = |name: &str| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    (plain(account) && plain(short_id)).then(|| data_dir.join("starts").join(format!("{account}-{short_id}.json")))
}

pub fn save_recorded(data_dir: &Path, account: &str, short_id: &str, recorded: &Recorded) -> Result<(), String> {
    let file = recorded_file(data_dir, account, short_id)
        .ok_or_else(|| format!("cannot write the starting state of session {account}/{short_id}: unexpected name"))?;
    let json = serde_json::to_string(recorded).map_err(|error| error.to_string())?;
    let write = |file: &Path| -> io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(file, json)
    };
    write(&file).map_err(|error| format!("cannot write {}: {error}", file.display()))
}

/// The state written down at the launch of session `short_id`, if `session_id` is still its
/// first conversation: after a `/clear`, the session works in a new conversation, which that
/// state does not describe, and its block is estimated (decision 238). The first conversation
/// is the `sessionId` of `<account>/jobs/<short id>/state.json`, a file Claude Code does not
/// document: without it, no state.
pub fn load_recorded(data_dir: &Path, account_dir: &Path, account: &str, short_id: &str, session_id: &str) -> Option<Recorded> {
    #[derive(Deserialize)]
    struct State {
        #[serde(rename = "sessionId")]
        session_id: Option<String>,
    }
    let state = std::fs::read_to_string(account_dir.join("jobs").join(short_id).join("state.json")).ok()?;
    let first = serde_json::from_str::<State>(&state).ok()?.session_id?;
    if first != session_id {
        return None;
    }
    let file = recorded_file(data_dir, account, short_id)?;
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

/// A trace of git in the conversation, by which a commit on the copy the session started in
/// counts as the session's: `[main 138f002] …` printed by a commit, `Updating 527b7e3..932aa2c`
/// by a fast-forward merge.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GitMark {
    Commit(String),
    Range(String, String),
}

/// What kindric reads in a conversation.
#[derive(Debug, PartialEq, Eq)]
struct Conversation {
    /// The time of the first message, Unix seconds.
    started: i64,
    visits: Vec<Visit>,
    marks: Vec<GitMark>,
}

/// A directory the session worked in, with the time of its first message there (Unix seconds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visit {
    pub cwd: PathBuf,
    pub since: i64,
}

/// The blocks of the session `session_id` of the account in `account_dir`, in the order it first
/// worked in each copy. Fails only when git cannot be run at all.
pub fn session_changes(account_dir: &Path, session_id: &str, recorded: Option<&Recorded>) -> Result<Vec<Block>, String> {
    let Some(conversation) = conversation_of(account_dir, session_id) else {
        return Ok(Vec::new());
    };
    let Some(conversation) = read_conversation(&conversation) else {
        return Ok(Vec::new());
    };
    let mut blocks = Vec::new();
    for (index, copy) in copies(&conversation.visits)?.into_iter().enumerate() {
        // Other sessions commit on the copy this one started in too (a shared checkout): there,
        // only the session's own commits count (decision 236).
        let marks = (index == 0).then_some(conversation.marks.as_slice());
        blocks.push(match copy {
            Copy::Gone(copy) => Block::Removed { copy },
            Copy::Here { top, since } => block_of(top, since, conversation.started, recorded, marks),
        });
    }
    Ok(blocks)
}

/// `<account>/projects/*/<session_id>.jsonl`, if there is one.
fn conversation_of(account_dir: &Path, session_id: &str) -> Option<PathBuf> {
    let plain = !session_id.is_empty() && session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !plain {
        return None;
    }
    let file_name = format!("{session_id}.jsonl");
    std::fs::read_dir(account_dir.join("projects"))
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(&file_name))
        .find(|path| path.is_file())
}

/// `None` for a conversation without any dated message.
fn read_conversation(conversation: &Path) -> Option<Conversation> {
    let reader = BufReader::new(File::open(conversation).ok()?);
    conversation_from(reader.split(b'\n').map_while(Result::ok))
}

fn conversation_from(lines: impl Iterator<Item = Vec<u8>>) -> Option<Conversation> {
    let mut started = None;
    let mut visits: Vec<Visit> = Vec::new();
    let mut marks = Vec::new();
    for line in lines {
        for mark in git_marks(&line) {
            if !marks.contains(&mark) {
                marks.push(mark);
            }
        }
        let Some(since) = string_field(&line, "timestamp").and_then(|time| unix_seconds(&time)) else {
            continue;
        };
        started.get_or_insert(since);
        let Some(cwd) = string_field(&line, "cwd") else {
            continue;
        };
        if !visits.iter().any(|visit| visit.cwd.as_os_str() == cwd.as_str()) {
            visits.push(Visit { cwd: PathBuf::from(cwd), since });
        }
    }
    Some(Conversation { started: started?, visits, marks })
}

/// The git marks of a line: `[<branch> <sha>]` (also `[<branch> (root-commit) <sha>]`) and
/// `Updating <sha>..<sha>`. A mark that is not a commit of the copy is dropped later by git.
fn git_marks(line: &[u8]) -> Vec<GitMark> {
    let mut marks = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.iter().position(|&byte| byte == b'[') {
        rest = &rest[open + 1..];
        let Some(close) = rest.iter().take(200).position(|&byte| byte == b']') else {
            continue;
        };
        let inside = text(&rest[..close]);
        let words: Vec<&str> = inside.split(' ').collect();
        if let [branch, .., sha] = words.as_slice()
            && !branch.is_empty()
            && is_sha(sha)
        {
            marks.push(GitMark::Commit((*sha).to_string()));
        }
    }
    let updating = b"Updating ";
    let mut rest = line;
    while let Some(at) = find(rest, updating) {
        rest = &rest[at + updating.len()..];
        let from = hex_prefix(rest);
        let Some(after) = rest[from.len()..].strip_prefix(b"..") else {
            continue;
        };
        let to = hex_prefix(after);
        if is_sha(&from) && is_sha(&to) {
            marks.push(GitMark::Range(from, to));
        }
    }
    marks
}

fn hex_prefix(bytes: &[u8]) -> String {
    bytes.iter().take_while(|byte| byte.is_ascii_hexdigit()).map(|&byte| char::from(byte)).collect()
}

/// An abbreviated or full commit id as git prints it.
fn is_sha(word: &str) -> bool {
    (7..=40).contains(&word.len()) && word.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The string value of the first `"name":"…"` of a JSON line, found without decoding the whole
/// line (a conversation can weigh megabytes). Inside a string, quotes are escaped, so a match
/// is a real key.
fn string_field(line: &[u8], name: &str) -> Option<String> {
    let key = format!("\"{name}\":\"");
    let start = find(line, key.as_bytes())? + key.len() - 1;
    let mut escaped = false;
    let length = line[start + 1..].iter().position(|&byte| {
        let closes = byte == b'"' && !escaped;
        escaped = byte == b'\\' && !escaped;
        closes
    })?;
    serde_json::from_slice(&line[start..start + length + 2]).ok()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// `2026-11-10T10:40:12.822Z` (the form Claude Code writes, UTC) in Unix seconds.
fn unix_seconds(iso: &str) -> Option<i64> {
    let bytes = iso.as_bytes();
    let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    if bytes.len() < 19 || separators.iter().any(|&(at, separator)| bytes[at] != separator) {
        return None;
    }
    let number = |from: usize, to: usize| iso.get(from..to)?.parse::<i64>().ok();
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Days since 1970-01-01 of a date of the proleptic Gregorian calendar (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

enum Copy {
    Here { top: PathBuf, since: i64 },
    Gone(PathBuf),
}

/// The copies the visits went through, each once, in the order of their first visit. A
/// directory outside any repository gives nothing; one that no longer exists gives the highest
/// of its missing parents, the copy that was removed.
fn copies(visits: &[Visit]) -> Result<Vec<Copy>, String> {
    let mut found: Vec<Copy> = Vec::new();
    for visit in visits {
        let copy = if visit.cwd.is_dir() {
            match git(&visit.cwd, &["rev-parse", "--show-toplevel"]) {
                Ok(top) => Copy::Here { top: PathBuf::from(text(&top).trim_end()), since: visit.since },
                Err(GitError::Missing(error)) => return Err(error),
                Err(GitError::Failed(_)) => continue,
            }
        } else {
            Copy::Gone(highest_missing(&visit.cwd))
        };
        let known = found.iter().any(|seen| match (seen, &copy) {
            (Copy::Here { top: seen, .. }, Copy::Here { top, .. }) => seen == top,
            (Copy::Gone(seen), Copy::Gone(gone)) => seen == gone,
            _ => false,
        });
        if !known {
            found.push(copy);
        }
    }
    Ok(found)
}

fn highest_missing(path: &Path) -> PathBuf {
    path.ancestors()
        .take_while(|ancestor| !ancestor.as_os_str().is_empty() && !ancestor.exists())
        .last()
        .unwrap_or(path)
        .to_path_buf()
}

fn block_of(top: PathBuf, since: i64, started: i64, recorded: Option<&Recorded>, marks: Option<&[GitMark]>) -> Block {
    match changes_of(&top, since, started, recorded, marks) {
        Ok((branch, start, files)) => Block::Changes { copy: top, branch, start, files },
        Err(error) => Block::Failed { copy: top, error },
    }
}

type CopyChanges = (Option<String>, Start, Vec<FileChange>);

/// `marks`: for the copy the session started in, the git marks of its conversation; a tracked
/// file then counts only when it is not committed or one of these commits touched it.
fn changes_of(top: &Path, since: i64, started: i64, recorded: Option<&Recorded>, marks: Option<&[GitMark]>) -> Result<CopyChanges, String> {
    let branch = git(top, &["symbolic-ref", "--short", "-q", "HEAD"]).ok().map(|name| text(&name).trim().to_string());
    let (start, base, known_untracked) = match recorded.filter(|recorded| recorded.copy == top) {
        Some(recorded) => (Start::Recorded, recorded.commit.clone(), recorded.untracked.clone()),
        None => {
            let created = match &branch {
                Some(name) => creation(&git(top, &["log", "-g", "--date=unix", "--format=%H%x00%gd%x00%gs", &format!("refs/heads/{name}"), "--"])?),
                None => None,
            };
            match created.filter(|(_, at)| *at >= started) {
                Some((commit, _)) => (Start::BranchCreated, commit, Vec::new()),
                None => (Start::Estimated, head_at(&git(top, &["log", "-g", "--date=unix", "--format=%H%x00%gd", "HEAD", "--"])?, since), Vec::new()),
            }
        }
    };
    let diff = ["--no-optional-locks", "diff", "--no-color", "--no-ext-diff", "-z", "-M"];
    let statuses = git(top, &[&diff[..], &["--name-status", base.as_str(), "--"][..]].concat())?;
    let counts = git(top, &[&diff[..], &["--numstat", base.as_str(), "--"][..]].concat())?;
    let mut files = tracked_changes(&statuses, &counts);
    if let Some(marks) = marks {
        let own = own_paths(top, marks)?;
        files.retain(|file| own.contains(&file.path) || matches!(&file.kind, ChangeKind::Renamed { from } if own.contains(from)));
    }
    let untracked = git(top, &["--no-optional-locks", "ls-files", "-z", "--others", "--exclude-standard"])?;
    // A path ending in `/` is a repository or worktree nested in the copy: not a file change, and
    // a worktree gets its own block.
    for path in nul_fields(&untracked).filter(|path| !path.ends_with('/') && !known_untracked.contains(path)) {
        let (added, removed) = match count_lines(&top.join(&path)) {
            Some(lines) => (Some(lines), Some(0)),
            None => (None, None),
        };
        files.push(FileChange { path, kind: ChangeKind::Added, added, removed });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((branch, start, files))
}

/// The tracked paths of the copy that are not committed, or that a commit or merge of `marks`
/// touched. Marks git does not know in this copy are skipped.
fn own_paths(top: &Path, marks: &[GitMark]) -> Result<Vec<String>, String> {
    let names = ["--no-optional-locks", "diff", "--no-color", "--no-ext-diff", "-z", "--name-only"];
    let mut paths: Vec<String> = nul_fields(&git(top, &[&names[..], &["HEAD", "--"][..]].concat())?).collect();
    for mark in marks {
        let touched = match mark {
            GitMark::Commit(sha) => git(top, &["diff-tree", "-r", "--root", "--no-commit-id", "--name-only", "-z", sha]),
            GitMark::Range(from, to) => git(top, &[&names[..], &[from.as_str(), to.as_str(), "--"][..]].concat()),
        };
        match touched {
            Ok(output) => paths.extend(nul_fields(&output)),
            Err(GitError::Missing(error)) => return Err(error),
            Err(GitError::Failed(_)) => {}
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// The commit and time of the creation of a branch, the oldest line of its reflog
/// (`%H NUL %gd NUL %gs`, newest first), when that line says so.
fn creation(reflog: &[u8]) -> Option<(String, i64)> {
    let oldest = text(reflog).lines().rev().find(|line| !line.is_empty())?.to_string();
    let mut fields = oldest.split('\0');
    let (commit, selector, subject) = (fields.next()?, fields.next()?, fields.next()?);
    if !subject.starts_with("branch: Created from") {
        return None;
    }
    Some((commit.to_string(), selector_time(selector)?))
}

/// The commit HEAD was on at `time`, from its reflog (`%H NUL %gd`, newest first): the newest
/// entry not after `time`, else the oldest one, else HEAD itself.
fn head_at(reflog: &[u8], time: i64) -> String {
    let entries: Vec<(String, i64)> = text(reflog)
        .lines()
        .filter_map(|line| {
            let (commit, selector) = line.split_once('\0')?;
            Some((commit.to_string(), selector_time(selector)?))
        })
        .collect();
    entries
        .iter()
        .find(|(_, at)| *at <= time)
        .or(entries.last())
        .map_or_else(|| "HEAD".to_string(), |(commit, _)| commit.clone())
}

/// The time of `HEAD@{1794307212}`, as `--date=unix` prints a reflog selector.
fn selector_time(selector: &str) -> Option<i64> {
    selector.rsplit_once("@{")?.1.strip_suffix('}')?.parse().ok()
}

/// The files of `git diff -z --name-status` with their counts from `git diff -z --numstat`.
fn tracked_changes(statuses: &[u8], counts: &[u8]) -> Vec<FileChange> {
    let counted = numstat(counts);
    let mut fields = nul_fields(statuses);
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        let kind = match status.chars().next() {
            Some('A') => ChangeKind::Added,
            Some('D') => ChangeKind::Deleted,
            Some('R') => match fields.next() {
                Some(from) => ChangeKind::Renamed { from },
                None => break,
            },
            // C (copied) keeps a source path too; it is shown as the new file it is.
            Some('C') => match fields.next() {
                Some(_) => ChangeKind::Added,
                None => break,
            },
            _ => ChangeKind::Modified,
        };
        let Some(path) = fields.next() else { break };
        let (added, removed) = counted.get(&path).copied().unwrap_or((None, None));
        files.push(FileChange { path, kind, added, removed });
    }
    files
}

/// `added TAB removed TAB path NUL`, or for a rename `added TAB removed TAB NUL from NUL to NUL`;
/// `-` counts for a binary file. Keyed by the path after the change.
fn numstat(output: &[u8]) -> HashMap<String, (Option<u32>, Option<u32>)> {
    let mut counted = HashMap::new();
    let mut fields = nul_fields(output);
    while let Some(field) = fields.next() {
        let mut parts = field.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let path = if path.is_empty() {
            let _from = fields.next();
            match fields.next() {
                Some(to) => to,
                None => break,
            }
        } else {
            path.to_string()
        };
        counted.insert(path, (added.parse().ok(), removed.parse().ok()));
    }
    counted
}

fn nul_fields(output: &[u8]) -> impl Iterator<Item = String> + '_ {
    output.split(|&byte| byte == 0).filter(|field| !field.is_empty()).map(|field| text(field).into_owned())
}

fn text(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

/// The lines of a text file; `None` for a binary one or one too big to read each time.
fn count_lines(path: &Path) -> Option<u32> {
    let file = File::open(path).ok()?;
    if file.metadata().ok()?.len() > LARGEST_COUNTED {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(LARGEST_COUNTED).read_to_end(&mut bytes).ok()?;
    if bytes[..bytes.len().min(BINARY_PROBE)].contains(&0) {
        return None;
    }
    let newlines = bytes.iter().filter(|&&byte| byte == b'\n').count();
    let unterminated = bytes.last().is_some_and(|&byte| byte != b'\n');
    u32::try_from(newlines + usize::from(unterminated)).ok()
}

enum GitError {
    /// Git cannot be run: nothing can be shown for any copy.
    Missing(String),
    Failed(String),
}

impl From<GitError> for String {
    fn from(error: GitError) -> Self {
        match error {
            GitError::Missing(message) | GitError::Failed(message) => message,
        }
    }
}

/// The output of `git args` run in `dir`.
fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).args(args).stdin(Stdio::null());
    hide_console(&mut command);
    let output = command.output().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => GitError::Missing("git not found: install git to see what a session changed".to_string()),
        _ => GitError::Missing(format!("git could not be run: {error}")),
    })?;
    if !output.status.success() {
        let stderr = text(&output.stderr);
        return Err(GitError::Failed(format!("git {} in {}: {}", args.join(" "), dir.display(), stderr.trim())));
    }
    Ok(output.stdout)
}

/// A program started by a windowed application opens a console window of its own on Windows
/// unless told not to; git runs at each change of state of a session.
#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn lines(text: &str) -> impl Iterator<Item = Vec<u8>> + '_ {
        text.lines().map(|line| line.as_bytes().to_vec())
    }

    #[test]
    fn iso_times_read_as_unix_seconds() {
        // References: `date -u -d '<time>' +%s`.
        assert_eq!(unix_seconds("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(unix_seconds("2026-11-10T10:40:12.822Z"), Some(1_794_307_212));
        assert_eq!(unix_seconds("2000-02-29T23:59:59Z"), Some(951_868_799));
        assert_eq!(unix_seconds("2026-13-10T10:40:12Z"), None);
        assert_eq!(unix_seconds("yesterday"), None);
    }

    #[test]
    fn a_string_field_is_found_and_unescaped() {
        let line = br#"{"type":"user","cwd":"C:\\Users\\l \"x\"","timestamp":"2026-11-10T10:40:12.822Z"}"#;
        assert_eq!(string_field(line, "cwd").as_deref(), Some(r#"C:\Users\l "x""#));
        assert_eq!(string_field(line, "timestamp").as_deref(), Some("2026-11-10T10:40:12.822Z"));
        assert_eq!(string_field(line, "missing"), None);
        // A key quoted inside another string is escaped, so it is not taken for a key.
        assert_eq!(string_field(br#"{"text":"say \"cwd\":\"/etc\""}"#, "cwd"), None);
    }

    #[test]
    fn visits_keep_each_directory_once_with_its_first_time() {
        let conversation = r#"{"type":"mode"}
{"type":"user","cwd":"/r","timestamp":"2026-11-10T10:00:00.000Z"}
{"type":"assistant","cwd":"/r/.claude/worktrees/w","timestamp":"2026-11-10T10:05:00.000Z"}
{"type":"user","cwd":"/r","timestamp":"2026-11-10T10:09:00.000Z"}
{"type":"user","cwd":"/r/.claude/worktrees/w/src","timest
{"type":"user","cwd":"/r/.claude/worktrees/w/src","timestamp":"2026-11-10T10:10:00.000Z"}"#;
        let read = conversation_from(lines(conversation)).unwrap();
        let base = unix_seconds("2026-11-10T10:00:00Z").unwrap();
        assert_eq!(read.started, base);
        assert_eq!(
            read.visits,
            vec![
                Visit { cwd: PathBuf::from("/r"), since: base },
                Visit { cwd: PathBuf::from("/r/.claude/worktrees/w"), since: base + 300 },
                Visit { cwd: PathBuf::from("/r/.claude/worktrees/w/src"), since: base + 600 },
            ]
        );
        assert_eq!(conversation_from(lines(r#"{"type":"mode"}"#)), None);
    }

    #[test]
    fn git_marks_are_commit_and_merge_outputs() {
        let line = br#"{"content":"[worktree-changes 138f002] feat: x\n 2 files changed","more":"[main (root-commit) abc1234] first"}"#;
        assert_eq!(git_marks(line), vec![GitMark::Commit("138f002".into()), GitMark::Commit("abc1234".into())]);
        let merge = br#"{"stdout":"Already up to date.\nUpdating 527b7e3..932aa2c\nFast-forward"}"#;
        assert_eq!(git_marks(merge), vec![GitMark::Range("527b7e3".into(), "932aa2c".into())]);
        for line in [&br#"[link](http://x)"#[..], br#"[a word]"#, br#"[deadbeef]"#, br#"Updating index"#] {
            assert_eq!(git_marks(line), vec![], "{}", String::from_utf8_lossy(line));
        }
    }

    #[test]
    fn diff_outputs_give_kinds_and_counts() {
        let statuses = b"M\0src/a.rs\0A\0new.txt\0D\0old.txt\0R087\0from.rs\0to.rs\0M\0logo.png\0";
        let counts = b"3\t1\tsrc/a.rs\x002\t0\tnew.txt\x000\t5\told.txt\x001\t1\t\x00from.rs\x00to.rs\x00-\t-\tlogo.png\x00";
        let files = tracked_changes(statuses, counts);
        let change = |path: &str, kind, added, removed| FileChange { path: path.into(), kind, added, removed };
        assert_eq!(
            files,
            vec![
                change("src/a.rs", ChangeKind::Modified, Some(3), Some(1)),
                change("new.txt", ChangeKind::Added, Some(2), Some(0)),
                change("old.txt", ChangeKind::Deleted, Some(0), Some(5)),
                change("to.rs", ChangeKind::Renamed { from: "from.rs".into() }, Some(1), Some(1)),
                change("logo.png", ChangeKind::Modified, None, None),
            ]
        );
    }

    #[test]
    fn a_branch_creation_is_its_oldest_reflog_line() {
        let reflog = "bbb\0w@{1794307300}\0commit: second\naaa\0w@{1794307200}\0branch: Created from aaa^{commit}\n";
        assert_eq!(creation(reflog.as_bytes()), Some(("aaa".to_string(), 1_794_307_200)));
        assert_eq!(creation(b"aaa\0w@{1794307200}\0clone: from somewhere\n"), None);
        assert_eq!(creation(b""), None);
    }

    #[test]
    fn head_at_a_time_is_the_newest_entry_not_after_it() {
        let reflog = b"ccc\0HEAD@{300}\nbbb\0HEAD@{200}\naaa\0HEAD@{100}\n";
        assert_eq!(head_at(reflog, 250), "bbb");
        assert_eq!(head_at(reflog, 200), "bbb");
        assert_eq!(head_at(reflog, 50), "aaa", "before the reflog: its oldest entry");
        assert_eq!(head_at(b"", 50), "HEAD");
    }

    #[test]
    fn a_removed_copy_is_its_highest_missing_parent() {
        let root = std::env::temp_dir().join(format!("kindric-changes-gone-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let gone = root.join("worktrees").join("escape");
        assert_eq!(highest_missing(&gone.join("crates").join("desktop")), root.join("worktrees"));
        fs::create_dir_all(root.join("worktrees")).unwrap();
        assert_eq!(highest_missing(&gone.join("crates")), gone);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn untracked_text_files_count_their_lines() {
        let root = std::env::temp_dir().join(format!("kindric-changes-count-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "one\ntwo\nthree").unwrap();
        fs::write(root.join("b.bin"), b"PNG\0\x01").unwrap();
        fs::write(root.join("c.txt"), "").unwrap();
        assert_eq!(count_lines(&root.join("a.txt")), Some(3));
        assert_eq!(count_lines(&root.join("b.bin")), None);
        assert_eq!(count_lines(&root.join("c.txt")), Some(0));
        fs::remove_dir_all(&root).unwrap();
    }

    /// Git run for setting up a test repository, away from the user's configuration.
    fn setup_git(dir: &Path, args: &[&str]) -> String {
        let empty = std::env::temp_dir().join(format!("kindric-changes-gitconfig-{}", std::process::id()));
        fs::write(&empty, "").unwrap();
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=test", "-c", "user.email=test@example.com", "-c", "init.defaultBranch=main"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", &empty)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn conversation_line(cwd: &Path, time: i64) -> String {
        let cwd = serde_json::to_string(&cwd.to_string_lossy()).unwrap();
        format!(r#"{{"type":"user","cwd":{cwd},"timestamp":"{}"}}"#, iso(time))
    }

    fn iso(time: i64) -> String {
        let (days, seconds) = (time.div_euclid(86_400), time.rem_euclid(86_400));
        // Inverse of days_from_civil, enough for the tests (Howard Hinnant's civil_from_days).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let day_of_era = z - era * 146_097;
        let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let mp = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z", seconds / 3600, seconds / 60 % 60, seconds % 60)
    }

    fn now() -> i64 {
        let elapsed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
        i64::try_from(elapsed.as_secs()).unwrap()
    }

    fn top_of(dir: &Path) -> PathBuf {
        PathBuf::from(setup_git(dir, &["rev-parse", "--show-toplevel"]))
    }

    #[test]
    fn iso_is_the_inverse_of_unix_seconds() {
        for time in [0, 951_868_799, 1_794_307_212] {
            assert_eq!(unix_seconds(&iso(time)), Some(time));
        }
    }

    /// A repository with a committed file, a session that starts in it, leaves an uncommitted
    /// change there, then creates a worktree and changes files in it, plus a worktree removed.
    #[test]
    fn a_session_gives_one_block_per_copy() {
        let root = std::env::temp_dir().join(format!("kindric-changes-repo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        setup_git(&repo, &["init", "-q"]);
        fs::write(repo.join("kept.txt"), "a\nb\n").unwrap();
        fs::write(repo.join("gone.txt"), "x\n").unwrap();
        fs::write(repo.join("notes.txt"), "n\n").unwrap();
        setup_git(&repo, &["add", "-A"]);
        setup_git(&repo, &["commit", "-q", "-m", "first"]);
        // Before the session: an uncommitted change and an untracked file, then the state kindric
        // writes down at launch.
        fs::write(repo.join("notes.txt"), "n2\n").unwrap();
        fs::write(repo.join("before.txt"), "b\n").unwrap();
        let snapshot = record(&repo).unwrap().unwrap();

        // The session starts after this commit: git's reflog times are in seconds.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let started = now();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(repo.join("kept.txt"), "a\nb\nc\n").unwrap();
        // On the shared checkout: a commit of another session, then one of this session, whose
        // output is in its conversation.
        fs::write(repo.join("other.txt"), "o\n").unwrap();
        setup_git(&repo, &["add", "other.txt"]);
        setup_git(&repo, &["commit", "-q", "-m", "another session"]);
        fs::write(repo.join("mine.txt"), "m\n").unwrap();
        setup_git(&repo, &["add", "mine.txt"]);
        setup_git(&repo, &["commit", "-q", "-m", "this session"]);
        let mine = setup_git(&repo, &["rev-parse", "--short", "HEAD"]);

        let worktree = repo.join(".claude").join("worktrees").join("w");
        setup_git(&repo, &["worktree", "add", "-q", "-b", "worktree-w", worktree.to_str().unwrap()]);
        fs::create_dir_all(worktree.join("src")).unwrap();
        fs::write(worktree.join("new.txt"), "1\n2\n").unwrap();
        fs::remove_file(worktree.join("gone.txt")).unwrap();
        setup_git(&worktree, &["add", "-A"]);
        setup_git(&worktree, &["commit", "-q", "-m", "session work"]);
        fs::write(worktree.join("untracked.txt"), "u\n").unwrap();

        let removed = repo.join(".claude").join("worktrees").join("old");
        let account = root.join("account");
        let project = account.join("projects").join("-repo");
        fs::create_dir_all(&project).unwrap();
        let output = serde_json::json!({
            "type": "user",
            "message": { "content": format!("[main {mine}] this session\n 1 file changed") },
        });
        let conversation = [
            conversation_line(&repo, started),
            output.to_string(),
            conversation_line(&worktree.join("src"), started + 1),
            conversation_line(&worktree, started + 2),
            conversation_line(&removed.join("crates"), started + 3),
        ]
        .join("\n");
        fs::write(project.join("abc-123.jsonl"), conversation).unwrap();

        let blocks = session_changes(&account, "abc-123", None).unwrap();
        let file = |path: &str, kind, added, removed| FileChange { path: path.into(), kind, added: Some(added), removed: Some(removed) };
        assert_eq!(
            blocks,
            vec![
                Block::Changes {
                    copy: top_of(&repo),
                    branch: Some("main".into()),
                    start: Start::Estimated,
                    // Estimated: what was not committed before the session counts as its own.
                    files: vec![
                        file("before.txt", ChangeKind::Added, 1, 0),
                        file("kept.txt", ChangeKind::Modified, 1, 0),
                        file("mine.txt", ChangeKind::Added, 1, 0),
                        file("notes.txt", ChangeKind::Modified, 1, 1),
                    ],
                },
                Block::Changes {
                    copy: top_of(&worktree),
                    branch: Some("worktree-w".into()),
                    start: Start::BranchCreated,
                    files: vec![
                        file("gone.txt", ChangeKind::Deleted, 0, 1),
                        file("new.txt", ChangeKind::Added, 2, 0),
                        file("untracked.txt", ChangeKind::Added, 1, 0),
                    ],
                },
                Block::Removed { copy: removed },
            ]
        );

        // With the state kindric wrote down at launch, what was not committed before the session
        // is no longer counted as the session's.
        let blocks = session_changes(&account, "abc-123", Some(&snapshot)).unwrap();
        let Block::Changes { start, files, .. } = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(start, &Start::Recorded);
        assert_eq!(files, &vec![file("kept.txt", ChangeKind::Modified, 1, 0), file("mine.txt", ChangeKind::Added, 1, 0)]);

        fs::remove_dir_all(&root).unwrap();
    }

    /// The state is a commit of the uncommitted change plus the untracked files, taken without
    /// moving HEAD or adding to the stash list.
    #[test]
    fn recording_a_copy_leaves_it_as_it_was() {
        let root = std::env::temp_dir().join(format!("kindric-changes-record-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let repo = root.join("repo");
        fs::create_dir_all(repo.join("sub")).unwrap();
        setup_git(&repo, &["init", "-q"]);
        assert_eq!(record(&repo), Ok(None), "no commit yet");
        fs::write(repo.join("a.txt"), "1\n").unwrap();
        setup_git(&repo, &["add", "-A"]);
        setup_git(&repo, &["commit", "-q", "-m", "first"]);
        let head = setup_git(&repo, &["rev-parse", "HEAD"]);

        let clean = record(&repo.join("sub")).unwrap().unwrap();
        assert_eq!((clean.copy, clean.commit.as_str(), clean.untracked.len()), (top_of(&repo), head.as_str(), 0));

        fs::write(repo.join("a.txt"), "2\n").unwrap();
        fs::write(repo.join("sub").join("u.txt"), "u\n").unwrap();
        let dirty = record(&repo).unwrap().unwrap();
        assert_ne!(dirty.commit, head);
        assert_eq!(setup_git(&repo, &["show", &format!("{}:a.txt", dirty.commit)]), "2");
        assert_eq!(dirty.untracked, vec!["sub/u.txt".to_string()]);
        assert_eq!(setup_git(&repo, &["rev-parse", "HEAD"]), head);
        assert_eq!(setup_git(&repo, &["stash", "list"]), "");
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "2\n");

        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        assert_eq!(record(&outside), Ok(None));
        fs::remove_dir_all(&root).unwrap();
    }

    /// The state comes back for the session's first conversation only.
    #[test]
    fn a_recorded_state_is_for_the_first_conversation() {
        let root = std::env::temp_dir().join(format!("kindric-changes-saved-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (data, account) = (root.join("data"), root.join("account"));
        let recorded = Recorded { copy: PathBuf::from("/r"), commit: "abc1234".into(), untracked: vec!["x".into()] };
        save_recorded(&data, "lystriq", "06af535d", &recorded).unwrap();
        assert_eq!(load_recorded(&data, &account, "lystriq", "06af535d", "first"), None, "no state.json");

        let job = account.join("jobs").join("06af535d");
        fs::create_dir_all(&job).unwrap();
        fs::write(job.join("state.json"), r#"{"state":"idle","sessionId":"first","resumeSessionId":"second"}"#).unwrap();
        assert_eq!(load_recorded(&data, &account, "lystriq", "06af535d", "first"), Some(recorded.clone()));
        assert_eq!(load_recorded(&data, &account, "lystriq", "06af535d", "second"), None, "after a /clear");
        assert_eq!(load_recorded(&data, &account, "other", "06af535d", "first"), None, "another account");
        assert!(save_recorded(&data, "../x", "06af535d", &recorded).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn no_conversation_gives_no_block() {
        let account = std::env::temp_dir().join(format!("kindric-changes-none-{}", std::process::id()));
        assert_eq!(session_changes(&account, "abc-123", None), Ok(Vec::new()));
        assert_eq!(session_changes(&account, "../escape", None), Ok(Vec::new()));
    }

    /// The blocks of a real session, printed: `KINDRIC_CHANGES_ACCOUNT=<account dir>
    /// KINDRIC_CHANGES_SESSION=<session id> cargo test -p kindric-core real_session -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_session() {
        let account = std::env::var("KINDRIC_CHANGES_ACCOUNT").expect("KINDRIC_CHANGES_ACCOUNT");
        let session = std::env::var("KINDRIC_CHANGES_SESSION").expect("KINDRIC_CHANGES_SESSION");
        let begun = std::time::Instant::now();
        let blocks = session_changes(Path::new(&account), &session, None).unwrap();
        println!("{} blocks in {:?}", blocks.len(), begun.elapsed());
        for block in blocks {
            match block {
                Block::Changes { copy, branch, start, files } => {
                    println!("{} ({}, {start:?}): {} files", copy.display(), branch.unwrap_or_default(), files.len());
                    for file in files {
                        println!("  {:?} {} +{:?} -{:?}", file.kind, file.path, file.added, file.removed);
                    }
                }
                other => println!("{other:?}"),
            }
        }
    }
}
