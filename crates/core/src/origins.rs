//! Which session kept each artifact: `artifacts/<project>/origins.json`, keyed by the path of the
//! file under the project's folder (`user/logos/logo.svg`). The moves kindric makes follow it; a
//! file moved by hand loses its origin. Two sessions keeping a file at the same instant may lose
//! one origin: the file itself is always kept.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

const FILE: &str = "origins.json";

/// The session that kept a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub session_id: String,
    pub account: String,
    /// Seconds since 1970.
    pub kept_at: u64,
}

pub type Origins = BTreeMap<String, Origin>;

/// `root/user/logos/logo.svg` → `user/logos/logo.svg`; `None` outside `root`.
fn key(root: &Path, path: &Path) -> Option<String> {
    let parts: Option<Vec<String>> = path
        .strip_prefix(root)
        .ok()?
        .components()
        .map(|part| match part {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    parts.filter(|parts| !parts.is_empty()).map(|parts| parts.join("/"))
}

/// The origins of a project; none when the file is missing or cannot be read.
pub fn read(root: &Path) -> Origins {
    fs::read_to_string(root.join(FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// How many files each session kept for the user in the projects of `roots`, by session id.
pub fn kept_for_user(roots: &[PathBuf]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for root in roots {
        for (key, origin) in read(root) {
            if key.starts_with("user/") {
                *counts.entry(origin.session_id).or_insert(0) += 1;
            }
        }
    }
    counts
}

/// Writes a hidden copy first, then renames it: a reader never sees half the file.
fn write(root: &Path, origins: &Origins) -> Result<(), String> {
    let text = serde_json::to_string_pretty(origins).map_err(|error| error.to_string())?;
    let temporary = root.join(format!(".{FILE}.{}.tmp", std::process::id()));
    fs::write(&temporary, text).map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, root.join(FILE)).map_err(|error| format!("cannot write {}: {error}", root.join(FILE).display()))
}

fn change(root: &Path, edit: impl FnOnce(&mut Origins) -> bool) -> Result<(), String> {
    let mut origins = read(root);
    if edit(&mut origins) { write(root, &origins) } else { Ok(()) }
}

/// Notes that `origin` kept the file at `path`, under `root`.
pub fn note(root: &Path, path: &Path, origin: Origin) -> Result<(), String> {
    let key = key(root, path).ok_or_else(|| format!("{} is not in {}", path.display(), root.display()))?;
    change(root, |origins| {
        origins.insert(key, origin);
        true
    })
}

/// The file at `from` is now at `to`.
pub fn moved(root: &Path, from: &Path, to: &Path) -> Result<(), String> {
    let (Some(from), Some(to)) = (key(root, from), key(root, to)) else {
        return Ok(());
    };
    change(root, |origins| match origins.remove(&from) {
        Some(origin) => {
            origins.insert(to, origin);
            true
        },
        None => false,
    })
}

/// The files of the folder `from` are now in `to`.
pub fn moved_folder(root: &Path, from: &Path, to: &Path) -> Result<(), String> {
    let (Some(from), Some(to)) = (key(root, from), key(root, to)) else {
        return Ok(());
    };
    let prefix = format!("{from}/");
    change(root, |origins| {
        let inside: Vec<String> = origins.keys().filter(|key| key.starts_with(&prefix)).cloned().collect();
        for old in &inside {
            if let Some(origin) = origins.remove(old) {
                origins.insert(format!("{to}/{}", &old[prefix.len()..]), origin);
            }
        }
        !inside.is_empty()
    })
}

/// The file at `path` is gone.
pub fn forget(root: &Path, path: &Path) -> Result<(), String> {
    let Some(key) = key(root, path) else {
        return Ok(());
    };
    change(root, |origins| origins.remove(&key).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-origins-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn origin(session: &str) -> Origin {
        Origin { session_id: session.into(), account: "louis".into(), kept_at: 7 }
    }

    #[test]
    fn an_origin_follows_its_file_and_its_folder() {
        let root = scratch("follow");
        note(&root, &root.join("user").join("a.png"), origin("s1")).unwrap();
        note(&root, &root.join("user").join("logos").join("l.svg"), origin("s2")).unwrap();
        moved(&root, &root.join("user").join("a.png"), &root.join("user").join("logos").join("a.png")).unwrap();
        moved_folder(&root, &root.join("user").join("logos"), &root.join("user").join("brand")).unwrap();
        let origins = read(&root);
        assert_eq!(origins.keys().collect::<Vec<_>>(), ["user/brand/a.png", "user/brand/l.svg"]);
        assert_eq!(origins["user/brand/a.png"].session_id, "s1");
        forget(&root, &root.join("user").join("brand").join("l.svg")).unwrap();
        assert_eq!(read(&root).len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_path_outside_the_project_is_refused_and_a_broken_file_reads_as_none() {
        let root = scratch("outside");
        assert!(note(&root, Path::new("/elsewhere/a.png"), origin("s1")).is_err());
        assert!(note(&root, &root.join("user").join("..").join("a.png"), origin("s1")).is_err());
        fs::write(root.join(FILE), "{broken").unwrap();
        assert!(read(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_files_kept_for_the_user_are_counted_by_session_across_projects() {
        let first = scratch("count-a");
        let second = scratch("count-b");
        note(&first, &first.join("user").join("a.png"), origin("s1")).unwrap();
        note(&first, &first.join("user").join("logos").join("l.svg"), origin("s1")).unwrap();
        note(&first, &first.join("claude").join("make.py"), origin("s1")).unwrap();
        note(&second, &second.join("user").join("b.png"), origin("s1")).unwrap();
        note(&second, &second.join("user").join("c.png"), origin("s2")).unwrap();
        let counts = kept_for_user(&[first.clone(), second.clone(), first.join("missing")]);
        assert_eq!(counts.into_iter().collect::<Vec<_>>(), [("s1".to_string(), 3), ("s2".to_string(), 1)]);
        fs::remove_dir_all(first).unwrap();
        fs::remove_dir_all(second).unwrap();
    }

    #[test]
    fn a_folder_with_a_longer_name_keeps_its_origins() {
        let root = scratch("prefix");
        note(&root, &root.join("user").join("logos2").join("x.png"), origin("s1")).unwrap();
        moved_folder(&root, &root.join("user").join("logos"), &root.join("user").join("brand")).unwrap();
        assert!(read(&root).contains_key("user/logos2/x.png"));
        fs::remove_dir_all(root).unwrap();
    }
}
