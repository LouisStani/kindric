//! Isolated accounts (a work account, for example) give their MCP servers to no other
//! account and receive none from them.

use crate::error::Result;
use crate::files;
use crate::paths::Paths;
use crate::registry;

/// `default`, Claude Code's own configuration, can be isolated without being an account:
/// claude-sessions keeps it out of its messages.
fn require_isolatable(paths: &Paths, name: &str) -> Result<()> {
    if name == registry::DEFAULT_ACCOUNT {
        return Ok(());
    }
    registry::require_existing_account(paths, name)
}

/// Returns `false` when the account was already isolated.
pub fn isolate(paths: &Paths, name: &str) -> Result<bool> {
    require_isolatable(paths, name)?;
    if registry::is_isolated(paths, name)? {
        return Ok(false);
    }
    files::create_private_dir(&paths.config_dir)?;
    files::append_line(&paths.isolated_file(), name)?;
    Ok(true)
}

/// Returns `false` when the account was not isolated.
pub fn unisolate(paths: &Paths, name: &str) -> Result<bool> {
    require_isolatable(paths, name)?;
    if !registry::is_isolated(paths, name)? {
        return Ok(false);
    }
    files::remove_line(&paths.isolated_file(), name)?;
    Ok(true)
}
