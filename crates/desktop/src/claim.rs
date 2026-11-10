//! One kindric window per session: a window holds a system lock on
//! `<account>-<short_id>.lock` for each session it shows. The lock goes when the window
//! detaches the session or ends, even by a crash. The files stay: removing one while another
//! window opens it would let both windows hold a lock.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::Path;

use crate::layout::SessionKey;

/// Held while a pane shows the session; dropping it lets another window show it.
pub struct Claim {
    _file: File,
}

/// Takes the lock of `key` in `directory`, or says why it cannot.
pub fn claim(directory: &Path, key: &SessionKey) -> Result<Claim, String> {
    fs::create_dir_all(directory).map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let path = directory.join(file_name(key));
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Claim { _file: file }),
        Err(TryLockError::WouldBlock) => {
            Err(format!("{} is open in another kindric window: close it there to show it here", key.short_id))
        },
        Err(TryLockError::Error(error)) => Err(format!("cannot lock {}: {error}", path.display())),
    }
}

/// Characters other than letters, digits, `-` and `_` become `_`: the name stays a plain file
/// name on every system.
fn file_name(key: &SessionKey) -> String {
    let plain = |text: &str| -> String {
        text.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
    };
    format!("{}-{}.lock", plain(&key.account), plain(&key.short_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-claim-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn key(account: &str, short_id: &str) -> SessionKey {
        SessionKey { account: account.into(), short_id: short_id.into() }
    }

    #[test]
    fn a_session_held_elsewhere_is_refused_until_released() {
        let directory = scratch("held");
        let first = claim(&directory, &key("lystriq", "378ce649")).expect("first claim");
        let refused = claim(&directory, &key("lystriq", "378ce649")).err().expect("second claim refused");
        assert!(refused.contains("another kindric window"), "{refused}");
        drop(first);
        claim(&directory, &key("lystriq", "378ce649")).expect("claim after release");
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn other_sessions_are_not_held() {
        let directory = scratch("others");
        let _first = claim(&directory, &key("lystriq", "378ce649")).expect("first claim");
        claim(&directory, &key("lystriq", "9f0bd287")).expect("other session");
        claim(&directory, &key("louis", "378ce649")).expect("same id, other account");
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn odd_characters_stay_inside_the_directory() {
        assert_eq!(file_name(&key("../work", "378ce649")), "___work-378ce649.lock");
    }
}
