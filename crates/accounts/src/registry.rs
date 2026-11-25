//! The account list and the isolated-account list: one name per line.

use crate::error::{Result, fail};
use crate::files;
use crate::paths::Paths;

/// Claude Code's own configuration, `~/.claude`: kindric shows it as an account
/// of this name, so no account may take it.
pub const DEFAULT_ACCOUNT: &str = "default";

pub fn is_valid_name(name: &str) -> bool {
    (1..=32).contains(&name.len()) && name.bytes().all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
}

/// `is_valid_name` for an account to create, which no command may take for an option.
/// Accounts made before may start with a dash, so the other commands still accept them.
pub fn is_valid_new_name(name: &str) -> bool {
    is_valid_name(name) && !name.starts_with('-')
}

pub fn validate_new_name(name: &str) -> Result<()> {
    validate_name(name)?;
    if is_valid_new_name(name) {
        return Ok(());
    }
    fail(format!("invalid name '{name}': it cannot start with a dash"))
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
    fail(format!("no such account: {name} (see: kindric-accounts list)"))
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

    #[test]
    fn a_new_name_does_not_start_with_a_dash() {
        assert!(is_valid_new_name("x-y") && is_valid_name("--help"));
        assert!(!is_valid_new_name("--help") && !is_valid_new_name("Bad"));
        assert_eq!(validate_new_name("-a").unwrap_err().0, "invalid name '-a': it cannot start with a dash");
        assert!(validate_new_name("Bad").unwrap_err().0.contains("1 to 32 characters"));
    }
}
