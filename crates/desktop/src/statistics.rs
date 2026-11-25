//! Which accounts the statistics count. Isolated accounts are kept out of messages and of the
//! statistics unless the user ticked them (`Settings::stats_isolated`).

use std::path::PathBuf;

use kindric_core::accounts::DEFAULT_ACCOUNT;
use kindric_core::stats::Source;

/// The accounts whose transcripts count: every ordinary account (the implicit `default` included
/// in `accounts`), and an isolated one only when ticked. `dir_of` tells where an account keeps
/// its files.
pub fn sources(accounts: &[String], isolated: &[String], shown_isolated: &[String], dir_of: impl Fn(&str) -> PathBuf) -> Vec<Source> {
    accounts
        .iter()
        .filter(|account| !isolated.contains(account) || shown_isolated.contains(account))
        .map(|account| Source::of(account.clone(), dir_of(account)))
        .collect()
}

/// The implicit `default` while it is not in `accounts`: a `claude auth status` that failed once
/// looks the same as a logout, and kindric has no way to remove it. Its figures stay even when the
/// user logged out for good.
pub fn unseen(accounts: &[String]) -> &'static [&'static str] {
    if accounts.iter().any(|account| account == DEFAULT_ACCOUNT) { &[] } else { &[DEFAULT_ACCOUNT] }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use kindric_core::stats::Cache;

    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    fn account_names(found: &[Source]) -> Vec<&str> {
        found.iter().map(|source| source.account.as_str()).collect()
    }

    fn under(root: &Path) -> impl Fn(&str) -> PathBuf + '_ {
        move |account| root.join(account)
    }

    #[test]
    fn an_isolated_account_not_ticked_is_not_a_source() {
        let found = sources(&names(&["default", "work", "pro"]), &names(&["pro"]), &[], under(Path::new("/data")));
        assert_eq!(account_names(&found), ["default", "work"]);
    }

    #[test]
    fn a_ticked_isolated_account_is_a_source_and_an_ordinary_one_always_is() {
        let found = sources(&names(&["default", "work", "pro", "joe"]), &names(&["pro", "joe"]), &names(&["pro", "work"]), under(Path::new("/data")));
        assert_eq!(account_names(&found), ["default", "work", "pro"]);
        assert_eq!(found[2].dir, Path::new("/data").join("pro"));
    }

    #[test]
    fn the_implicit_default_is_unseen_while_it_is_not_listed() {
        assert_eq!(unseen(&names(&["work"])), ["default"]);
        assert!(unseen(&names(&["default", "work"])).is_empty());
    }

    fn write_reply(root: &Path, account: &str) {
        let dir = root.join(account).join("projects").join("p");
        fs::create_dir_all(&dir).unwrap();
        let line = r#"{"type":"assistant","timestamp":"2026-11-18T15:52:20Z","cwd":"/w","message":{"id":"m","model":"m","usage":{"input_tokens":1,"output_tokens":2}}}"#;
        fs::write(dir.join("s1.jsonl"), format!("{line}\n")).unwrap();
    }

    #[test]
    fn unticking_removes_the_files_of_the_account_from_the_cache() {
        let root = std::env::temp_dir().join(format!("kindric-statistics-untick-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        write_reply(&root, "work");
        write_reply(&root, "pro");
        let accounts = names(&["work", "pro"]);
        let isolated = names(&["pro"]);
        let mut cache = Cache::default();

        let problems = cache.refresh(&sources(&accounts, &isolated, &names(&["pro"]), under(&root)), &[]);
        assert!(problems.is_empty(), "{problems:?}");
        let shown: Vec<String> = cache.summary(&std::collections::BTreeMap::new()).sessions.into_iter().map(|row| row.account).collect();
        assert_eq!(shown, ["pro", "work"]);

        cache.refresh(&sources(&accounts, &isolated, &[], under(&root)), &[]);
        let shown: Vec<String> = cache.summary(&std::collections::BTreeMap::new()).sessions.into_iter().map(|row| row.account).collect();
        assert_eq!(shown, ["work"]);
        assert!(cache.summary(&std::collections::BTreeMap::new()).hours.iter().all(|row| row.account == "work"));
        fs::remove_dir_all(root).unwrap();
    }
}
