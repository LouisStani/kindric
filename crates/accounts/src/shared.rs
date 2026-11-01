//! Links an account directory to the configuration every account shares (`~/.claude`).
//!
//! Unix: symbolic links. Windows: a symbolic link when allowed (developer mode),
//! otherwise a junction for directories and a copy for files. Copies are recorded with
//! a fingerprint, so a copy can be refreshed later and a file the user wrote, or a copy
//! edited inside the account, is never overwritten.

use std::fs;
use std::io;
use std::path::Path;

pub const SHARED_ENTRIES: [&str; 6] = ["CLAUDE.md", "skills", "agents", "commands", "plugins", "themes"];

/// What is found at the destination of a shared entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Existing {
    Missing,
    Symlink,
    Junction,
    /// A copy made by claude-accounts, unchanged since.
    CopyUnchanged,
    /// A copy made by claude-accounts, then edited inside the account.
    CopyEdited,
    /// Anything else: a file or directory that belongs to the account.
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Create,
    Recreate,
    Keep,
    WarnEdited,
    WarnNotShared,
}

pub fn decide(existing: Existing) -> Action {
    match existing {
        Existing::Missing => Action::Create,
        Existing::Symlink | Existing::CopyUnchanged => Action::Recreate,
        Existing::Junction => Action::Keep,
        Existing::CopyEdited => Action::WarnEdited,
        Existing::Other => Action::WarnNotShared,
    }
}

/// FNV-1a 64: stable across Rust versions, unlike the standard library hasher.
pub fn fingerprint(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

#[cfg(unix)]
mod platform {
    use super::*;

    pub fn existing(_account_dir: &Path, _entry: &str, destination: &Path) -> io::Result<Existing> {
        match fs::symlink_metadata(destination) {
            Ok(metadata) if metadata.file_type().is_symlink() => Ok(Existing::Symlink),
            Ok(_) => Ok(Existing::Other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Existing::Missing),
            Err(error) => Err(error),
        }
    }

    pub fn remove(destination: &Path) -> io::Result<()> {
        fs::remove_file(destination)
    }

    pub fn create(_account_dir: &Path, _entry: &str, source: &Path, destination: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(source, destination)
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::collections::BTreeMap;
    use std::os::windows::fs::{FileTypeExt, symlink_dir, symlink_file};

    const COPIES: &str = ".claude-accounts-copies.json";

    fn copies(account_dir: &Path) -> BTreeMap<String, String> {
        fs::read_to_string(account_dir.join(COPIES))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn record_copy(account_dir: &Path, entry: &str, content: &[u8]) -> io::Result<()> {
        let mut recorded = copies(account_dir);
        recorded.insert(entry.to_string(), fingerprint(content));
        let text = serde_json::to_string(&recorded).map_err(io::Error::other)?;
        crate::files::write_atomic(&account_dir.join(COPIES), &text)
    }

    pub fn existing(account_dir: &Path, entry: &str, destination: &Path) -> io::Result<Existing> {
        let metadata = match fs::symlink_metadata(destination) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Existing::Missing),
            Err(error) => return Err(error),
        };
        if metadata.file_type().is_symlink() {
            return Ok(Existing::Symlink);
        }
        if metadata.is_dir() && junction::exists(destination).unwrap_or(false) {
            return Ok(Existing::Junction);
        }
        if metadata.is_file()
            && let Some(recorded) = copies(account_dir).get(entry)
        {
            let unchanged = fingerprint(&fs::read(destination)?) == *recorded;
            return Ok(if unchanged { Existing::CopyUnchanged } else { Existing::CopyEdited });
        }
        Ok(Existing::Other)
    }

    /// A link to a directory is removed like a directory on Windows: `remove_file`
    /// fails on it with "Access is denied". `is_dir()` is false for such a link.
    pub fn remove(destination: &Path) -> io::Result<()> {
        let file_type = fs::symlink_metadata(destination)?.file_type();
        if file_type.is_symlink_dir() || file_type.is_dir() {
            fs::remove_dir(destination)
        } else {
            fs::remove_file(destination)
        }
    }

    pub fn create(account_dir: &Path, entry: &str, source: &Path, destination: &Path) -> io::Result<()> {
        let is_dir = source.is_dir();
        let linked = if is_dir { symlink_dir(source, destination) } else { symlink_file(source, destination) };
        if linked.is_ok() {
            return Ok(());
        }
        if is_dir {
            return junction::create(source, destination);
        }
        let content = fs::read(source)?;
        fs::write(destination, &content)?;
        record_copy(account_dir, entry, &content)
    }
}

fn with_path(path: &Path) -> impl FnOnce(io::Error) -> io::Error + '_ {
    move |error| io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}

/// Shares every entry of `shared_root` that exists, and returns warnings for entries
/// left alone.
pub fn link_shared(shared_root: &Path, account_dir: &Path) -> io::Result<Vec<String>> {
    let mut warnings = Vec::new();
    for entry in SHARED_ENTRIES {
        let source = shared_root.join(entry);
        if fs::metadata(&source).is_err() {
            continue;
        }
        let destination = account_dir.join(entry);
        let existing = platform::existing(account_dir, entry, &destination).map_err(with_path(&destination))?;
        match decide(existing) {
            Action::Create => {
                platform::create(account_dir, entry, &source, &destination).map_err(with_path(&destination))?;
            },
            Action::Recreate => {
                platform::remove(&destination).map_err(with_path(&destination))?;
                platform::create(account_dir, entry, &source, &destination).map_err(with_path(&destination))?;
            },
            Action::Keep => {},
            Action::WarnEdited => warnings.push(format!(
                "{} was edited in this account and is no longer refreshed from {}: copy your changes there",
                destination.display(),
                source.display()
            )),
            Action::WarnNotShared => {
                warnings.push(format!("{} exists and is not a link, not shared", destination.display()));
            },
        }
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_files_and_edited_copies_are_never_replaced() {
        assert_eq!(decide(Existing::Missing), Action::Create);
        assert_eq!(decide(Existing::Symlink), Action::Recreate);
        assert_eq!(decide(Existing::CopyUnchanged), Action::Recreate);
        assert_eq!(decide(Existing::Junction), Action::Keep);
        assert_eq!(decide(Existing::CopyEdited), Action::WarnEdited);
        assert_eq!(decide(Existing::Other), Action::WarnNotShared);
    }

    #[test]
    fn fingerprint_is_stable_and_sensitive() {
        assert_eq!(fingerprint(b""), "cbf29ce484222325");
        assert_eq!(fingerprint(b"a"), "af63dc4c8601ec8c");
        assert_ne!(fingerprint(b"# shared"), fingerprint(b"# shared "));
    }

    #[cfg(unix)]
    #[test]
    fn links_existing_entries_and_never_overwrites_real_files() {
        use crate::files::test_dir;
        let root = test_dir::fresh("shared");
        let shared = root.join(".claude");
        let account = root.join("account");
        fs::create_dir_all(shared.join("skills")).unwrap();
        fs::write(shared.join("CLAUDE.md"), "# shared").unwrap();
        fs::create_dir_all(&account).unwrap();
        fs::write(account.join("CLAUDE.md"), "# mine").unwrap();
        let warnings = link_shared(&shared, &account).unwrap();
        assert_eq!(fs::read_link(account.join("skills")).unwrap(), shared.join("skills"));
        assert_eq!(fs::read_to_string(account.join("CLAUDE.md")).unwrap(), "# mine");
        assert_eq!(warnings.len(), 1);
        assert!(!account.join("agents").exists());
        fs::create_dir_all(shared.join("agents")).unwrap();
        link_shared(&shared, &account).unwrap();
        assert!(fs::symlink_metadata(account.join("agents")).unwrap().file_type().is_symlink());
        fs::remove_dir_all(root).unwrap();
    }
}
