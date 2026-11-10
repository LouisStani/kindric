//! What the app keeps about itself, in `settings.json` (see `store`): the theme chosen with
//! Ctrl+Shift+E and the shortcuts changed in the settings. And the theme of `theme.json`, colors
//! only.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The colors a theme may set: the color variables of `style.css`, without `--`.
pub const COLORS: [&str; 19] = [
    "background",
    "surface",
    "raised",
    "line",
    "line-strong",
    "text",
    "text-strong",
    "muted",
    "faint",
    "terminal",
    "terminal-text",
    "selection",
    "active",
    "on-active",
    "waiting",
    "working",
    "done",
    "error",
    "on-accent",
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Shortcut id → its keys, for the shortcuts changed only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, String>,
}

const MODIFIERS: [&str; 3] = ["ctrl", "shift", "alt"];

/// The id of a shortcut: lowercase words joined by `-`.
pub fn is_shortcut_id(id: &str) -> bool {
    (1..=32).contains(&id.len()) && id.split('-').all(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase()))
}

/// Keys as the page names them: `ctrl`, `shift`, `alt` at most once each and in that order, then
/// one key of letters and digits (`d`, `f5`, `comma`). Nothing else is saved.
pub fn is_key_name(keys: &str) -> bool {
    let mut parts: Vec<&str> = keys.split('+').collect();
    let Some(key) = parts.pop() else {
        return false;
    };
    let key_ok = (1..=16).contains(&key.len()) && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let mut rest = MODIFIERS.iter();
    key_ok && parts.iter().all(|part| rest.by_ref().any(|modifier| modifier == part))
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa`: nothing else reaches the page's CSS.
fn is_color(value: &str) -> bool {
    value.strip_prefix('#').is_some_and(|hex| matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit()))
}

/// The colors of `theme.json`, checked: a name `COLORS` lacks or a value that is not a color
/// refuses the whole theme.
pub fn custom_theme(json: &str) -> Result<BTreeMap<String, String>, String> {
    let colors: BTreeMap<String, String> = serde_json::from_str(json).map_err(|error| format!("theme.json: {error}"))?;
    for (name, value) in &colors {
        if !COLORS.contains(&name.as_str()) {
            return Err(format!("theme.json: unknown color {name:?} (known: {})", COLORS.join(", ")));
        }
        if !is_color(value) {
            return Err(format!("theme.json: {name} must be #rgb, #rrggbb or #rrggbbaa, not {value:?}"));
        }
    }
    Ok(colors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_of_known_colors_is_taken() {
        let colors = custom_theme(r##"{"background": "#101010", "text": "#EEE", "active": "#c04020cc"}"##).unwrap();
        assert_eq!(colors.len(), 3);
        assert_eq!(colors["text"], "#EEE");
    }

    #[test]
    fn an_unknown_color_is_refused_by_name() {
        let error = custom_theme(r##"{"backgroud": "#101010"}"##).unwrap_err();
        assert!(error.contains("\"backgroud\""), "{error}");
    }

    #[test]
    fn only_hex_colors_reach_the_page() {
        for value in ["red; background: url(x)", "#12345", "#ggg", "red", "", "#1010101"] {
            let json = serde_json::json!({ "text": value }).to_string();
            assert!(custom_theme(&json).is_err(), "{value:?} was taken");
        }
        assert!(custom_theme("{broken").is_err());
        assert!(custom_theme(r##"{"text": 3}"##).is_err());
    }

    #[test]
    fn the_colors_are_those_of_the_style_sheet() {
        let css = include_str!("../ui/style.css");
        let root = &css[css.find(":root {").unwrap()..];
        let root = &root[..root.find('}').unwrap()];
        let mut found: Vec<&str> = root
            .lines()
            .filter_map(|line| line.trim().strip_prefix("--"))
            .filter(|line| line.contains(": #"))
            .map(|line| line.split(':').next().unwrap())
            .collect();
        let mut known = COLORS.to_vec();
        found.sort_unstable();
        known.sort_unstable();
        assert_eq!(found, known);
    }

    #[test]
    fn only_a_chosen_theme_is_written() {
        assert_eq!(serde_json::to_string(&Settings::default()).unwrap(), "{}");
        let chosen = Settings { theme: Some("paper".into()), ..Settings::default() };
        assert_eq!(serde_json::to_string(&chosen).unwrap(), r#"{"theme":"paper"}"#);
    }

    #[test]
    fn a_settings_file_without_keys_still_reads_and_changed_keys_are_kept() {
        let old: Settings = serde_json::from_str(r#"{"theme":"paper"}"#).unwrap();
        assert!(old.keys.is_empty());
        let mut changed = old.clone();
        changed.keys.insert("split".into(), "ctrl+alt+d".into());
        let json = serde_json::to_string(&changed).unwrap();
        assert_eq!(json, r#"{"theme":"paper","keys":{"split":"ctrl+alt+d"}}"#);
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), changed);
    }

    #[test]
    fn only_key_names_of_the_page_are_taken() {
        for keys in ["ctrl+alt+d", "f5", "ctrl+comma", "ctrl+shift+alt+bracketleft", "alt+3"] {
            assert!(is_key_name(keys), "{keys:?} refused");
        }
        for keys in ["", "ctrl+", "+d", "<script>", "shift+ctrl+d", "ctrl+ctrl+d", "meta+d", "ctrl+D", "ctrl+averyveryverylongkey"] {
            assert!(!is_key_name(keys), "{keys:?} taken");
        }
    }

    #[test]
    fn shortcut_ids_are_plain_words() {
        assert!(is_shortcut_id("new-session"));
        assert!(is_shortcut_id("split"));
        for id in ["", "New", "a--b", "-a", "a b", "../x"] {
            assert!(!is_shortcut_id(id), "{id:?} taken");
        }
    }
}
