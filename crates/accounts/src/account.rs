//! Creating and removing accounts.

use std::fs;
use std::path::PathBuf;

use crate::error::{Error, Result, fail};
use crate::files;
use crate::paths::Paths;
use crate::registry;
use crate::shared;

pub struct Created {
    pub dir: PathBuf,
    pub warnings: Vec<String>,
}

/// Creates the account directory (700), links the shared configuration and adds the
/// name to the list. An existing directory of the same name is reused, as in 1.x.
pub fn add(paths: &Paths, name: &str) -> Result<Created> {
    registry::validate_name(name)?;
    if registry::account_exists(paths, name)? {
        return fail(format!("account already exists: {name}"));
    }
    files::create_private_dir(&paths.data_root)?;
    files::create_private_dir(&paths.config_dir)?;
    let dir = paths.account_dir(name);
    if let Err(error) = fs::create_dir(&dir)
        && !dir.is_dir()
    {
        return Err(Error(format!("cannot create {}: {error}", dir.display())));
    }
    files::make_private(&dir)?;
    let warnings = shared::link_shared(&paths.shared_root, &dir)?;
    files::append_line(&paths.registry_file(), name)?;
    Ok(Created { dir, warnings })
}

/// Removes the name from the account list and from the isolated list.
pub fn forget(paths: &Paths, name: &str) -> Result<()> {
    registry::require_existing_account(paths, name)?;
    files::remove_line(&paths.registry_file(), name)?;
    files::remove_line(&paths.isolated_file(), name)?;
    Ok(())
}

pub fn delete_dir(paths: &Paths, name: &str) -> Result<()> {
    Ok(fs::remove_dir_all(paths.account_dir(name))?)
}
