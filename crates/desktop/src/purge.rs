//! Purging the sessions idle for long (decisions 357, 358, 361, 362), when the user asks it from
//! the settings: each is removed with `claude rm`, which stops a session still running, and keeps
//! its conversation for `claude --resume`.

use serde::Serialize;

use crate::view::SessionRow;

/// A session to purge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Target {
    pub account: String,
    pub short_id: String,
    /// What the bar calls it.
    pub label: String,
}

impl Target {
    /// `row` as a target; `None` for a session that is not a background one.
    pub fn of(row: &SessionRow) -> Option<Self> {
        Some(Self { account: row.account.clone(), short_id: row.short_id.clone()?, label: row.label.clone() })
    }
}

/// How many sessions of an account a purge would remove.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Count {
    pub account: String,
    pub sessions: usize,
}

/// The targets by account, in the order the accounts first come.
pub fn counts(targets: &[Target]) -> Vec<Count> {
    let mut counts: Vec<Count> = Vec::new();
    for target in targets {
        match counts.iter_mut().find(|count| count.account == target.account) {
            Some(count) => count.sessions += 1,
            None => counts.push(Count { account: target.account.clone(), sessions: 1 }),
        }
    }
    counts
}

/// A session Claude Code would not remove, with its words.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Refused {
    pub target: Target,
    pub reason: String,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub removed: Vec<Target>,
    pub refused: Vec<Refused>,
}

/// Removes each target with `remove(target)`. A target whose removal fails stays as it was, with
/// the reason; the others go on.
pub fn purge(targets: Vec<Target>, mut remove: impl FnMut(&Target) -> Result<(), String>) -> Report {
    let mut report = Report::default();
    for target in targets {
        match remove(&target) {
            Ok(()) => report.removed.push(target),
            Err(reason) => report.refused.push(Refused { target, reason }),
        }
    }
    report
}

/// What Claude Code said when a command failed: its error output, else its output, else the
/// exit code.
pub fn failure_words(stderr: &[u8], stdout: &[u8], code: Option<i32>) -> String {
    [stderr, stdout]
        .iter()
        .map(|bytes| String::from_utf8_lossy(bytes).trim().to_string())
        .find(|words| !words.is_empty())
        .unwrap_or_else(|| code.map_or_else(|| "stopped by a signal".to_string(), |code| format!("exit code {code}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(account: &str, short_id: &str) -> Target {
        Target { account: account.into(), short_id: short_id.into(), label: format!("{short_id}-label") }
    }

    #[test]
    fn each_session_is_removed_once() {
        let mut calls = Vec::new();
        let report = purge(vec![target("louis", "aa"), target("pro", "bb")], |target| {
            calls.push(format!("{}/{}", target.account, target.short_id));
            Ok(())
        });
        assert_eq!(calls, ["louis/aa", "pro/bb"]);
        assert_eq!(report.removed, [target("louis", "aa"), target("pro", "bb")]);
        assert!(report.refused.is_empty());
    }

    #[test]
    fn a_refusal_keeps_the_session_and_the_others_go_on() {
        let targets = vec![target("louis", "aa"), target("louis", "bb"), target("louis", "cc")];
        let report = purge(targets, |target| match target.short_id.as_str() {
            "bb" => Err("worktree has unpushed commits".into()),
            _ => Ok(()),
        });
        assert_eq!(report.removed, [target("louis", "aa"), target("louis", "cc")]);
        let refused: Vec<(&str, &str)> = report.refused.iter().map(|refused| (refused.target.short_id.as_str(), refused.reason.as_str())).collect();
        assert_eq!(refused, [("bb", "worktree has unpushed commits")]);
    }

    #[test]
    fn counts_go_by_account_in_order() {
        let targets = [target("pro", "a"), target("louis", "b"), target("pro", "c")];
        let counts = counts(&targets);
        assert_eq!(counts, [Count { account: "pro".into(), sessions: 2 }, Count { account: "louis".into(), sessions: 1 }]);
    }

    #[test]
    fn a_failure_says_what_claude_code_said() {
        assert_eq!(failure_words(b"  refused: unpushed\n", b"out", Some(1)), "refused: unpushed");
        assert_eq!(failure_words(b" \n", b"Nothing to remove\n", Some(1)), "Nothing to remove");
        assert_eq!(failure_words(b"", b"", Some(3)), "exit code 3");
        assert_eq!(failure_words(b"", b"", None), "stopped by a signal");
    }
}
