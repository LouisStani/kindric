//! The account list and the isolated-account list: one name per line.

use crate::error::{Result, fail};
use crate::files;
use crate::paths::Paths;

pub fn is_valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len()) && name.bytes().all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
}

pub fn validate_name(name: &str) -> Result<()> {
    if is_valid_name(name) {
        return Ok(());
    }
    fail(format!("invalid name '{name}': use 1 to 32 characters among a-z 0-9 _ -"))
}

pub fn list_accounts(paths: &Paths) -> Result<Vec<String>> {
    Ok(files::read_lines(&paths.registry_file())?)
}

pub fn account_exists(paths: &Paths, name: &str) -> Result<bool> {
    Ok(list_accounts(paths)?.iter().any(|account| account == name))
}

pub fn require_existing_account(paths: &Paths, name: &str) -> Result<()> {
    validate_name(name)?;
    if account_exists(paths, name)? {
        return Ok(());
    }
    fail(format!("no such account: {name} (see: claude-accounts list)"))
}

pub fn list_isolated(paths: &Paths) -> Result<Vec<String>> {
    Ok(files::read_lines(&paths.isolated_file())?)
}

pub fn is_isolated(paths: &Paths, name: &str) -> Result<bool> {
    Ok(list_isolated(paths)?.iter().any(|account| account == name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_1x_rule() {
        for good in ["a", "pro1", "work_2", "x-y", &"a".repeat(32)] {
            assert!(is_valid_name(good), "{good}");
        }
        for bad in ["", "Pro", "../x", "a b", "é", &"a".repeat(33)] {
            assert!(!is_valid_name(bad), "{bad}");
        }
        assert_eq!(
            validate_name("Bad").unwrap_err().0,
            "invalid name 'Bad': use 1 to 32 characters among a-z 0-9 _ -"
        );
    }
}
