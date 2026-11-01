//! Small file helpers. On Unix, everything claude-accounts creates is private:
//! directories in 700, files in 600, like the `umask 077` of the 1.x script.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

pub fn create_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path)
}

/// Forces 700 on an existing directory (no-op outside Unix).
pub fn make_private(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(unix)]
fn private_file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.mode(0o600);
    options
}

#[cfg(not(unix))]
fn private_file_options() -> OpenOptions {
    OpenOptions::new()
}

/// Non-empty lines of `path`; a missing file has none.
pub fn read_lines(path: &Path) -> io::Result<Vec<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text.lines().filter(|line| !line.is_empty()).map(String::from).collect()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

pub fn append_line(path: &Path, line: &str) -> io::Result<()> {
    let mut file = private_file_options().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
}

/// Writes a sibling temporary file, then renames it: readers never see half a file.
pub fn write_atomic(path: &Path, content: &str) -> io::Result<()> {
    let name = path.file_name().ok_or_else(|| io::Error::other("path without file name"))?;
    let temporary = path.with_file_name(format!(".{}.{}.tmp", name.to_string_lossy(), std::process::id()));
    let written = private_file_options()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&temporary)
        .and_then(|mut file| file.write_all(content.as_bytes()))
        .and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// Removes every line equal to `line`; a missing file is left missing.
pub fn remove_line(path: &Path, line: &str) -> io::Result<()> {
    let lines = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let kept: String = lines.lines().filter(|kept| *kept != line).map(|kept| format!("{kept}\n")).collect();
    write_atomic(path, &kept)
}

#[cfg(test)]
pub mod test_dir {
    use std::path::PathBuf;

    /// A fresh empty directory for one test.
    pub fn fresh(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("claude-accounts-test-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_appended_read_and_removed_exactly() {
        let dir = test_dir::fresh("lines");
        let file = dir.join("list");
        assert!(read_lines(&file).unwrap().is_empty());
        for name in ["pro1", "pro10", "pro1"] {
            append_line(&file, name).unwrap();
        }
        remove_line(&file, "pro1").unwrap();
        assert_eq!(read_lines(&file).unwrap(), ["pro10"]);
        remove_line(&dir.join("missing"), "x").unwrap();
        assert!(!dir.join("missing").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn created_files_and_dirs_are_private() {
        let dir = test_dir::fresh("private");
        create_private_dir(&dir.join("a/b")).unwrap();
        append_line(&dir.join("a/list"), "x").unwrap();
        write_atomic(&dir.join("a/data.json"), "{}").unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.join("a/b")), 0o700);
        assert_eq!(mode(&dir.join("a/list")), 0o600);
        assert_eq!(mode(&dir.join("a/data.json")), 0o600);
        fs::remove_dir_all(dir).unwrap();
    }
}
