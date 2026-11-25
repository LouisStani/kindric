//! The recap image of the stats window, drawn by the page: kindric writes it in a folder of the user
//! and sends it nowhere.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const STEM: &str = "kindric-recap";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
/// Larger than any recap.
const LARGEST: usize = 16 * 1024 * 1024;
const MOST_NAMES: u32 = 999;

/// Writes `png` in `dir` under a name no file has: `kindric-recap.png`, then `kindric-recap-2.png`.
pub fn save(dir: &Path, png: &[u8]) -> Result<PathBuf, String> {
    if !png.starts_with(PNG) || png.len() > LARGEST {
        return Err("the recap is not a PNG image kindric made".into());
    }
    let failed = |path: &Path, error: &io::Error| format!("cannot write {}: {error}", path.display());
    for number in 1..=MOST_NAMES {
        let path = dir.join(if number == 1 { format!("{STEM}.png") } else { format!("{STEM}-{number}.png") });
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(failed(&path, &error)),
        };
        if let Err(error) = file.write_all(png) {
            drop(file);
            // Half an image, just created here.
            let _ = fs::remove_file(&path);
            return Err(failed(&path, &error));
        }
        return Ok(path);
    }
    Err(format!("{} already has {MOST_NAMES} recaps: move some away", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kindric-recap-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn image(last: u8) -> Vec<u8> {
        [PNG, &[last]].concat()
    }

    #[test]
    fn a_recap_never_writes_over_a_file() {
        let dir = folder("names");
        fs::write(dir.join("kindric-recap-2.png"), "of the user").unwrap();
        assert_eq!(save(&dir, &image(1)).unwrap(), dir.join("kindric-recap.png"));
        assert_eq!(save(&dir, &image(2)).unwrap(), dir.join("kindric-recap-3.png"));
        assert_eq!(fs::read(dir.join("kindric-recap.png")).unwrap(), image(1));
        assert_eq!(fs::read(dir.join("kindric-recap-2.png")).unwrap(), b"of the user");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_an_image_is_written() {
        let dir = folder("image");
        assert!(save(&dir, b"#!/bin/sh").is_err());
        assert!(save(&dir, &[PNG.to_vec(), vec![0; LARGEST]].concat()).is_err());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_folder_that_is_not_there_is_named() {
        let dir = std::env::temp_dir().join("kindric-recap-nowhere");
        let error = save(&dir, &image(1)).unwrap_err();
        assert!(error.contains("kindric-recap-nowhere"), "{error}");
    }
}
