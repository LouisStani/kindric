//! Running git, for what a session changed (`changes`) and the worktrees of a project
//! (`worktrees`).

use std::io;
use std::path::Path;
use std::process::{Command, Output, Stdio};

pub(crate) enum GitError {
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

/// `git args` run in `dir`, whatever its exit code.
pub(crate) fn run(dir: &Path, args: &[&str]) -> Result<Output, GitError> {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).args(args).stdin(Stdio::null());
    hide_console(&mut command);
    command.output().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => GitError::Missing("git not found: install git".to_string()),
        _ => GitError::Missing(format!("git could not be run: {error}")),
    })
}

/// The output of `git args` run in `dir`.
pub(crate) fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
    let output = run(dir, args)?;
    if !output.status.success() {
        return Err(failure(dir, args, &output));
    }
    Ok(output.stdout)
}

pub(crate) fn failure(dir: &Path, args: &[&str], output: &Output) -> GitError {
    let stderr = text(&output.stderr);
    GitError::Failed(format!("git {} in {}: {}", args.join(" "), dir.display(), stderr.trim()))
}

pub(crate) fn text(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(bytes)
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

/// Git run for setting up a test repository, away from the user's configuration.
#[cfg(test)]
pub(crate) fn setup_git(dir: &Path, args: &[&str]) -> String {
    let empty = std::env::temp_dir().join(format!("kindric-test-gitconfig-{}", std::process::id()));
    std::fs::write(&empty, "").unwrap();
    let output = std::process::Command::new("git")
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
