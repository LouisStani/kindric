//! Creating and removing accounts.

use std::fs;
use std::path::PathBuf;

use crate::agent::{self, Agent};
use crate::codex;
use crate::error::{Error, Result, fail};
use crate::files;
use crate::paths::Paths;
use crate::provider::{self, Provider};
use crate::registry;
use crate::shared;

pub struct Created {
    pub dir: PathBuf,
    pub warnings: Vec<String>,
}

/// Creates the account directory (700), links the shared configuration and adds the
/// name to the list. An existing directory of the same name is reused, as in 1.x.
pub fn add(paths: &Paths, name: &str) -> Result<Created> {
    add_with(paths, name, None)
}

/// `add`, and with `provider` the account's requests go to that provider instead of a Claude
/// subscription. The provider is written before the name is listed: a listed account is complete.
pub fn add_with(paths: &Paths, name: &str, provider: Option<&Provider>) -> Result<Created> {
    check_new_name(paths, name)?;
    let dir = create_dir(paths, name)?;
    let warnings = shared::link_shared(&paths.shared_root, &dir)?;
    if let Some(provider) = provider {
        provider::write(&dir, provider)?;
    }
    files::append_line(&paths.registry_file(), name)?;
    Ok(Created { dir, warnings })
}

/// A Codex account (since 2.3.0): its directory is its `CODEX_HOME`, sharing `AGENTS.md`,
/// skills and prompts with `~/.codex`. Refused when the socket of its daemon would not fit the
/// system's limit, which would leave its sessions out of reach.
pub fn add_codex(paths: &Paths, name: &str) -> Result<Created> {
    check_new_name(paths, name)?;
    codex::check_socket_path(&paths.account_dir(name))?;
    let dir = create_dir(paths, name)?;
    let warnings = match paths.codex_root() {
        Some(codex_root) => shared::link_entries(&codex_root, &dir, &shared::CODEX_SHARED_ENTRIES)?,
        None => Vec::new(),
    };
    agent::write(&dir, Agent::Codex)?;
    files::append_line(&paths.registry_file(), name)?;
    Ok(Created { dir, warnings })
}

fn check_new_name(paths: &Paths, name: &str) -> Result<()> {
    registry::validate_name(name)?;
    if name == registry::DEFAULT_ACCOUNT {
        return fail(format!("'{name}' is reserved for Claude Code's own configuration (~/.claude)"));
    }
    if registry::account_exists(paths, name)? {
        return fail(format!("account already exists: {name}"));
    }
    Ok(())
}

/// The account directory, in 700. An existing directory of the same name is reused, as in 1.x.
fn create_dir(paths: &Paths, name: &str) -> Result<PathBuf> {
    files::create_private_dir(&paths.data_root)?;
    files::create_private_dir(&paths.config_dir)?;
    let dir = paths.account_dir(name);
    if let Err(error) = fs::create_dir(&dir)
        && !dir.is_dir()
    {
        return Err(Error(format!("cannot create {}: {error}", dir.display())));
    }
    files::make_private(&dir)?;
    Ok(dir)
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
