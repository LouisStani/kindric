//! A Claude Code theme for the sessions a program starts, without changing the account's own:
//! kindric gives its sessions `custom:kindric`, a theme it rewrites to follow its own colours.
//! Claude Code keeps only the last `--settings`, so the theme goes into a copy of the file the
//! session would get, `.shared-settings/<name>.<theme>.json`. A launch without a theme never
//! writes that copy, so it cannot take the theme away from the sessions that have it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::files;
use crate::paths::Paths;
use crate::registry;

/// The file of the sessions of `name` with the theme `theme`. Names and themes hold no `.`, so it
/// is never another account's file.
pub fn themed_settings_file(paths: &Paths, name: &str, theme: &str) -> PathBuf {
    paths.shared_settings_file(name).with_extension(format!("{theme}.json"))
}

/// `settings` (the text of a settings file, or none) with Claude Code's theme set to
/// `custom:<theme>`; everything else is kept. Settings that are not an object are an error.
pub fn with_theme(settings: Option<&str>, theme: &str) -> Result<String, String> {
    let mut settings = match settings.map(serde_json::from_str::<Map<String, Value>>) {
        Some(Ok(settings)) => settings,
        Some(Err(error)) => return Err(error.to_string()),
        None => Map::new(),
    };
    settings.insert("theme".into(), Value::from(format!("custom:{theme}")));
    Ok(format!("{}\n", Value::Object(settings)))
}

/// Writes the settings of `source` (none: only the theme) with the theme `theme`, for the
/// sessions of `name`. Returns the file to give with `--settings`.
pub fn write_themed_settings(paths: &Paths, name: &str, source: Option<&Path>, theme: &str) -> Result<PathBuf, String> {
    if !registry::is_valid_name(theme) {
        return Err(format!("invalid theme name {theme:?}: use 1 to 32 characters among a-z 0-9 _ -"));
    }
    let text = match source {
        None => None,
        Some(path) => match fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
        },
    };
    let content = with_theme(text.as_deref(), theme).map_err(|error| match source {
        Some(path) => format!("{}: {error}", path.display()),
        None => error,
    })?;
    let file = themed_settings_file(paths, name, theme);
    file.parent()
        .map_or(Ok(()), files::create_private_dir)
        .and_then(|()| files::write_atomic(&file, &content))
        .map_err(|error| format!("cannot write {}: {error}", file.display()))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::test_dir;

    fn paths(label: &str) -> (PathBuf, Paths) {
        let root = test_dir::fresh(&format!("theme-{label}"));
        let paths = Paths::from_roots(root.join("config"), root.join("data"), root.join("home").join(".claude"));
        (root, paths)
    }

    fn read(file: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap()
    }

    #[test]
    fn the_theme_is_set_and_the_rest_kept() {
        let text = with_theme(Some(r#"{"theme": "dark-daltonized", "model": "opus", "hooks": {"Stop": []}}"#), "kindric").unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["theme"], "custom:kindric");
        assert_eq!(value["model"], "opus");
        assert_eq!(value["hooks"]["Stop"], Value::Array(Vec::new()));
        let alone: Value = serde_json::from_str(&with_theme(None, "kindric").unwrap()).unwrap();
        assert_eq!(alone, serde_json::json!({ "theme": "custom:kindric" }));
        assert!(with_theme(Some("[1]"), "kindric").is_err());
        assert!(with_theme(Some("not json"), "kindric").is_err());
    }

    #[test]
    fn the_themed_file_sits_beside_the_account_file_and_never_on_it() {
        let (root, paths) = paths("file");
        let file = themed_settings_file(&paths, "work", "kindric");
        assert_eq!(file, paths.data_root.join(".shared-settings").join("work.kindric.json"));
        assert_ne!(file, paths.shared_settings_file("work"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_copy_holds_the_source_with_the_theme() {
        let (root, paths) = paths("copy");
        let source = paths.shared_settings_file("work");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, r#"{"statusLine": {"type": "command", "command": "k statusline"}}"#).unwrap();
        let file = write_themed_settings(&paths, "work", Some(&source), "kindric").unwrap();
        let value = read(&file);
        assert_eq!(value["theme"], "custom:kindric");
        assert_eq!(value["statusLine"]["command"], "k statusline");
        assert!(fs::read_to_string(&source).unwrap().contains("statusLine"), "the source is left as it is");
        assert!(!fs::read_to_string(&source).unwrap().contains("theme"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn without_a_source_only_the_theme_is_given() {
        let (root, paths) = paths("alone");
        let file = write_themed_settings(&paths, "default", None, "kindric").unwrap();
        assert_eq!(read(&file), serde_json::json!({ "theme": "custom:kindric" }));
        let missing = paths.shared_root.join("settings.json");
        let file = write_themed_settings(&paths, "work", Some(&missing), "kindric").unwrap();
        assert_eq!(read(&file), serde_json::json!({ "theme": "custom:kindric" }), "a missing source is no settings");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_source_that_is_not_settings_or_a_bad_theme_name_writes_nothing() {
        let (root, paths) = paths("refused");
        fs::create_dir_all(&paths.shared_root).unwrap();
        let source = paths.shared_root.join("settings.json");
        fs::write(&source, "not json").unwrap();
        let error = write_themed_settings(&paths, "work", Some(&source), "kindric").unwrap_err();
        assert!(error.contains("settings.json"), "{error}");
        assert!(write_themed_settings(&paths, "work", None, "../x").is_err());
        assert!(write_themed_settings(&paths, "work", None, "a.b").is_err());
        assert!(!paths.data_root.join(".shared-settings").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
