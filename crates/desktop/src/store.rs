//! The small JSON files of the app, next to the mailbox: read whole, written whole.

use std::fs;
use std::io;
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// No file yet means the default value; a file that does not parse is an error naming it.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> io::Result<T> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| io::Error::other(format!("{}: {error}", path.display()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error),
    }
}

/// Writes to a hidden temporary file first, then renames: a reader never sees half a file.
pub fn save<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = path.with_file_name(format!(".{name}.tmp"));
    fs::write(&temporary, serde_json::to_vec_pretty(value).map_err(io::Error::other)?)?;
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-store-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn a_saved_value_loads_back_and_leaves_no_temporary_file() {
        let root = scratch("round-trip");
        let path = root.join("kindric/list.json");
        save(&path, &vec!["b".to_string(), "a".to_string()]).unwrap();
        assert_eq!(load::<Vec<String>>(&path).unwrap(), ["b", "a"]);
        assert!(!path.with_file_name(".list.json.tmp").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_file_means_the_default() {
        let root = scratch("missing");
        assert_eq!(load::<Vec<String>>(&root.join("list.json")).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn a_broken_file_is_an_error_naming_it() {
        let root = scratch("broken");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("list.json");
        fs::write(&path, b"[{").unwrap();
        let error = load::<Vec<String>>(&path).unwrap_err().to_string();
        assert!(error.contains("list.json"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }
}
