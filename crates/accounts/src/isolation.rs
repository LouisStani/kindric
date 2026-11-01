//! Isolated accounts (a work account, for example) give their MCP servers to no other
//! account and receive none from them.

use crate::error::Result;
use crate::files;
use crate::paths::Paths;
use crate::registry;

/// Returns `false` when the account was already isolated.
pub fn isolate(paths: &Paths, name: &str) -> Result<bool> {
    registry::require_existing_account(paths, name)?;
    if registry::is_isolated(paths, name)? {
        return Ok(false);
    }
    files::create_private_dir(&paths.config_dir)?;
    files::append_line(&paths.isolated_file(), name)?;
    Ok(true)
}

/// Returns `false` when the account was not isolated.
pub fn unisolate(paths: &Paths, name: &str) -> Result<bool> {
    registry::require_existing_account(paths, name)?;
    if !registry::is_isolated(paths, name)? {
        return Ok(false);
    }
    files::remove_line(&paths.isolated_file(), name)?;
    Ok(true)
}
