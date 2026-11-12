//! The worktrees of a project: which are merged into the branch of the main checkout, and
//! removing those that can go without losing anything. Git is asked through its command.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::git::{failure, git, run, text, GitError};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Worktree {
    pub path: PathBuf,
    /// `None` for a detached head.
    pub branch: Option<String>,
    /// Unix seconds of the last commit of its head.
    pub last_commit: Option<i64>,
    /// A branch of the same name exists on `origin`: it is never touched.
    pub on_origin: bool,
    pub state: State,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum State {
    /// The checkout the others are compared with; never removed.
    Main,
    /// A live session works in it.
    Busy { session: String },
    /// Its folder is gone; `git worktree prune` forgets it.
    Missing,
    /// Locked with `git worktree lock`; Claude Code locks those of its live sessions.
    Locked { reason: String },
    Uncommitted,
    /// Every commit of its branch is in the main branch.
    Merged,
    /// Merging it would change nothing in the main branch: merged by squash or by hand.
    MergedSquash,
    NotMerged { ahead: u32 },
    /// A detached head, or a main checkout without a branch: nothing to compare.
    NoBranch,
    Failed { error: String },
}

impl State {
    /// Why it cannot be removed, or `None` when it can.
    fn refusal(&self) -> Option<String> {
        match self {
            State::Main => Some("the main checkout is never removed".to_string()),
            State::Busy { session } => Some(format!("the session {session} works in it")),
            State::Missing => Some("its folder is already gone: git worktree prune forgets it".to_string()),
            State::Locked { .. } => Some("it is locked: git worktree unlock first".to_string()),
            State::Uncommitted => Some("it has uncommitted changes".to_string()),
            State::Failed { error } => Some(error.clone()),
            State::Merged | State::MergedSquash | State::NotMerged { .. } | State::NoBranch => None,
        }
    }
}

/// One entry of `git worktree list --porcelain -z`.
#[derive(Debug, PartialEq, Eq)]
struct Listed {
    path: PathBuf,
    branch: Option<String>,
    bare: bool,
    /// The reason given, empty when none.
    locked: Option<String>,
    prunable: bool,
}

/// The worktrees of the repository `directory` is in, the main checkout first, then the most
/// recently committed. `sessions` are the folders of the live sessions with their names.
pub fn list(directory: &Path, sessions: &[(PathBuf, String)]) -> Result<Vec<Worktree>, String> {
    let listed = parse_list(&git(directory, &["worktree", "list", "--porcelain", "-z"]).map_err(|error| not_a_repository(directory, error))?);
    let Some(main) = listed.first().filter(|first| !first.bare) else {
        return Err(format!("{} is in a bare repository: no main checkout to compare with", directory.display()));
    };
    let remote_branches = remote_branches(&main.path)?;
    let mut worktrees: Vec<Worktree> = listed
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.bare)
        .map(|(index, entry)| {
            let state = if index == 0 { State::Main } else { state_of(entry, main, sessions) };
            Worktree {
                path: entry.path.clone(),
                branch: entry.branch.clone(),
                last_commit: if entry.prunable { None } else { last_commit(&entry.path) },
                on_origin: entry.branch.as_ref().is_some_and(|branch| remote_branches.iter().any(|remote| remote == branch)),
                state,
            }
        })
        .collect();
    worktrees[1..].sort_by_key(|worktree| std::cmp::Reverse(worktree.last_commit));
    Ok(worktrees)
}

fn not_a_repository(directory: &Path, error: GitError) -> String {
    match error {
        GitError::Missing(message) => message,
        GitError::Failed(_) => format!("{} is not in a git repository", directory.display()),
    }
}

fn parse_list(output: &[u8]) -> Vec<Listed> {
    let mut entries = Vec::new();
    for field in output.split(|&byte| byte == 0).map(text) {
        if let Some(path) = field.strip_prefix("worktree ") {
            entries.push(Listed { path: PathBuf::from(path), branch: None, bare: false, locked: None, prunable: false });
            continue;
        }
        let Some(entry) = entries.last_mut() else {
            continue;
        };
        let word = field.split(' ').next().unwrap_or_default();
        match word {
            "branch" => entry.branch = field.strip_prefix("branch refs/heads/").map(str::to_string),
            "bare" => entry.bare = true,
            "locked" => entry.locked = Some(field.strip_prefix("locked").unwrap_or_default().trim().to_string()),
            "prunable" => entry.prunable = true,
            _ => {},
        }
    }
    entries
}

/// The branch names on `origin`, without the `origin/` prefix.
fn remote_branches(top: &Path) -> Result<Vec<String>, String> {
    let output = git(top, &["for-each-ref", "--format=%(refname)", "refs/remotes/origin/"])?;
    Ok(text(&output).lines().filter_map(|name| name.strip_prefix("refs/remotes/origin/")).filter(|name| *name != "HEAD").map(str::to_string).collect())
}

fn state_of(entry: &Listed, main: &Listed, sessions: &[(PathBuf, String)]) -> State {
    if let Some((_, session)) = sessions.iter().find(|(directory, _)| directory.starts_with(&entry.path)) {
        return State::Busy { session: session.clone() };
    }
    if entry.prunable || !entry.path.is_dir() {
        return State::Missing;
    }
    if let Some(reason) = &entry.locked {
        return State::Locked { reason: reason.clone() };
    }
    match uncommitted(&entry.path) {
        Ok(true) => return State::Uncommitted,
        Ok(false) => {},
        Err(error) => return State::Failed { error },
    }
    let (Some(branch), Some(target)) = (&entry.branch, &main.branch) else {
        return State::NoBranch;
    };
    merged(&main.path, branch, target).unwrap_or_else(|error| State::Failed { error })
}

fn uncommitted(path: &Path) -> Result<bool, String> {
    Ok(!git(path, &["status", "--porcelain"])?.is_empty())
}

fn last_commit(path: &Path) -> Option<i64> {
    let output = git(path, &["log", "-1", "--format=%ct", "HEAD"]).ok()?;
    text(&output).trim().parse().ok()
}

/// `branch` against `target`: an ancestor is merged; otherwise, a merge that would leave the
/// tree of `target` as it is means its content is already there (a squash).
fn merged(top: &Path, branch: &str, target: &str) -> Result<State, String> {
    let ancestor = ["merge-base", "--is-ancestor", branch, target];
    let output = run(top, &ancestor)?;
    match output.status.code() {
        Some(0) => return Ok(State::Merged),
        Some(1) => {},
        _ => return Err(failure(top, &ancestor, &output).into()),
    }
    // Exit code 1 means conflicts: the tree printed is then not the target's, as wanted.
    let merge = run(top, &["merge-tree", "--write-tree", target, branch])?;
    let merged_tree = text(&merge.stdout).lines().next().unwrap_or_default().to_string();
    let target_tree = git(top, &["rev-parse", &format!("{target}^{{tree}}")])?;
    if merge.status.success() && merged_tree == text(&target_tree).trim() {
        return Ok(State::MergedSquash);
    }
    let ahead = git(top, &["rev-list", "--count", &format!("{target}..{branch}")])?;
    Ok(State::NotMerged { ahead: text(&ahead).trim().parse().unwrap_or(0) })
}

/// What `remove` did with the branch of the worktree removed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum BranchOutcome {
    Deleted,
    /// Git refused `branch -d`: the branch keeps the commits the main branch does not have.
    Kept { reason: String },
    NoBranch,
}

/// Removes the worktree at `path` of the repository `directory` is in, then its branch if git
/// finds it merged. Never forces: git refuses itself a worktree with changes or a branch with
/// commits of its own. The branch on `origin` is never touched.
pub fn remove(directory: &Path, path: &Path, sessions: &[(PathBuf, String)]) -> Result<BranchOutcome, String> {
    let worktrees = list(directory, sessions)?;
    let Some(worktree) = worktrees.iter().find(|worktree| worktree.path == path) else {
        return Err(format!("{} is not a worktree of this project", path.display()));
    };
    if let Some(refusal) = worktree.state.refusal() {
        return Err(refusal);
    }
    let top = &worktrees[0].path;
    let target = path.to_string_lossy();
    git(top, &["worktree", "remove", target.as_ref()])?;
    let Some(branch) = &worktree.branch else {
        return Ok(BranchOutcome::NoBranch);
    };
    let delete = ["branch", "-d", branch.as_str()];
    let output = run(top, &delete)?;
    if output.status.success() {
        return Ok(BranchOutcome::Deleted);
    }
    let reason = text(&output.stderr).lines().next().unwrap_or("git refused").trim().to_string();
    Ok(BranchOutcome::Kept { reason })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::git::setup_git;

    /// A repository with one commit on main, in a fresh folder named after the test.
    fn repository(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-worktrees-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let top = root.join("repo");
        fs::create_dir_all(&top).unwrap();
        setup_git(&top, &["init", "-q"]);
        fs::write(top.join("a.txt"), "one\n").unwrap();
        setup_git(&top, &["add", "-A"]);
        setup_git(&top, &["commit", "-q", "-m", "first"]);
        // git prints the folder resolved (/private/var on macOS): compare with what it prints.
        PathBuf::from(setup_git(&top, &["rev-parse", "--show-toplevel"]))
    }

    fn add_worktree(top: &Path, name: &str) -> PathBuf {
        let path = top.join(".claude").join("worktrees").join(name);
        setup_git(top, &["worktree", "add", "-q", "-b", &format!("worktree-{name}"), &path.to_string_lossy()]);
        path
    }

    fn commit(dir: &Path, file: &str, content: &str) {
        fs::write(dir.join(file), content).unwrap();
        setup_git(dir, &["add", "-A"]);
        setup_git(dir, &["commit", "-q", "-m", file]);
    }

    fn state(worktrees: &[Worktree], name: &str) -> State {
        worktrees.iter().find(|worktree| worktree.path.ends_with(name)).map(|worktree| worktree.state.clone()).unwrap()
    }

    fn clean(top: &Path) {
        fs::remove_dir_all(top.parent().unwrap()).unwrap();
    }

    #[test]
    fn porcelain_entries_read_with_their_marks() {
        let output = b"worktree /r\0HEAD 1111\0branch refs/heads/main\0\0worktree /r/w\0HEAD 2222\0detached\0locked claude agent 1234\0\0worktree /gone\0HEAD 3333\0branch refs/heads/x\0prunable gitdir file points to non-existent location\0\0";
        let entries = parse_list(output);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], Listed { path: PathBuf::from("/r"), branch: Some("main".into()), bare: false, locked: None, prunable: false });
        assert_eq!(entries[1], Listed { path: PathBuf::from("/r/w"), branch: None, bare: false, locked: Some("claude agent 1234".into()), prunable: false });
        assert!(entries[2].prunable);
    }

    #[test]
    fn each_worktree_gets_its_state() {
        let top = repository("states");
        let merged = add_worktree(&top, "merged");
        commit(&merged, "m.txt", "m\n");
        setup_git(&top, &["merge", "-q", "--no-ff", "-m", "merge", "worktree-merged"]);
        let squashed = add_worktree(&top, "squashed");
        commit(&squashed, "s.txt", "s\n");
        setup_git(&top, &["merge", "-q", "--squash", "worktree-squashed"]);
        setup_git(&top, &["commit", "-q", "-m", "squash"]);
        let ahead = add_worktree(&top, "ahead");
        commit(&ahead, "x.txt", "x\n");
        commit(&ahead, "y.txt", "y\n");
        let dirty = add_worktree(&top, "dirty");
        fs::write(dirty.join("new.txt"), "untracked\n").unwrap();
        let busy = add_worktree(&top, "busy");
        let sessions = [(busy.join("src"), "fix".to_string())];

        let worktrees = list(&top, &sessions).unwrap();
        assert_eq!(worktrees[0].path, top);
        assert_eq!(worktrees[0].state, State::Main);
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
        assert_eq!(state(&worktrees, "merged"), State::Merged);
        assert_eq!(state(&worktrees, "squashed"), State::MergedSquash);
        assert_eq!(state(&worktrees, "ahead"), State::NotMerged { ahead: 2 });
        assert_eq!(state(&worktrees, "dirty"), State::Uncommitted);
        assert_eq!(state(&worktrees, "busy"), State::Busy { session: "fix".into() });
        assert!(worktrees.iter().all(|worktree| worktree.last_commit.is_some() && !worktree.on_origin));
        // Asked from inside a worktree, the list is the same, main checkout first.
        assert_eq!(list(&ahead, &sessions).unwrap(), worktrees);
        clean(&top);
    }

    #[test]
    fn a_branch_also_on_origin_is_marked() {
        let top = repository("origin");
        let remote = top.parent().unwrap().join("remote.git");
        setup_git(top.parent().unwrap(), &["init", "-q", "--bare", &remote.to_string_lossy()]);
        setup_git(&top, &["remote", "add", "origin", &remote.to_string_lossy()]);
        let pushed = add_worktree(&top, "pushed");
        commit(&pushed, "p.txt", "p\n");
        setup_git(&top, &["push", "-q", "origin", "worktree-pushed"]);
        add_worktree(&top, "local");
        let worktrees = list(&top, &[]).unwrap();
        let on_origin = |name: &str| worktrees.iter().find(|worktree| worktree.path.ends_with(name)).unwrap().on_origin;
        assert!(on_origin("pushed"));
        assert!(!on_origin("local"));
        clean(&top);
    }

    #[test]
    fn outside_a_repository_says_so() {
        let folder = std::env::temp_dir().join(format!("kindric-worktrees-none-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let error = list(&folder, &[]).unwrap_err();
        assert!(error.contains("not in a git repository"), "{error}");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_merged_worktree_goes_with_its_branch() {
        let top = repository("remove-merged");
        let merged = add_worktree(&top, "merged");
        commit(&merged, "m.txt", "m\n");
        setup_git(&top, &["merge", "-q", "--no-ff", "-m", "merge", "worktree-merged"]);
        assert_eq!(remove(&top, &merged, &[]), Ok(BranchOutcome::Deleted));
        assert!(!merged.exists());
        assert_eq!(setup_git(&top, &["branch", "--list", "worktree-merged"]), "");
        clean(&top);
    }

    #[test]
    fn a_branch_with_its_own_commits_is_kept() {
        let top = repository("remove-ahead");
        let ahead = add_worktree(&top, "ahead");
        commit(&ahead, "x.txt", "x\n");
        let outcome = remove(&top, &ahead, &[]).unwrap();
        assert!(matches!(outcome, BranchOutcome::Kept { .. }), "{outcome:?}");
        assert!(!ahead.exists());
        assert_eq!(setup_git(&top, &["branch", "--list", "worktree-ahead", "--format=%(refname:short)"]), "worktree-ahead");
        clean(&top);
    }

    #[test]
    fn refused_before_git_is_asked() {
        let top = repository("refused");
        let dirty = add_worktree(&top, "dirty");
        fs::write(dirty.join("a.txt"), "changed\n").unwrap();
        let busy = add_worktree(&top, "busy");
        let sessions = [(busy.clone(), "fix".to_string())];
        assert_eq!(remove(&top, &dirty, &sessions), Err("it has uncommitted changes".to_string()));
        assert_eq!(remove(&top, &busy, &sessions), Err("the session fix works in it".to_string()));
        assert_eq!(remove(&top, &top, &sessions), Err("the main checkout is never removed".to_string()));
        let elsewhere = top.join("nowhere");
        assert!(remove(&top, &elsewhere, &sessions).unwrap_err().contains("is not a worktree"));
        assert!(dirty.exists() && busy.exists());
        clean(&top);
    }
}
