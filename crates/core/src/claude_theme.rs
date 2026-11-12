//! Claude Code draws for the background of kindric's theme. kindric writes a theme of its own,
//! `kindric.json`, among the themes every account shares (`~/.claude/themes`): light or dark after
//! the terminal colour of kindric's theme, in the variant the user chose for Claude Code
//! (daltonized, ANSI colours only). Claude Code reloads a theme file when it changes, so the
//! sessions that use `custom:kindric` follow at once.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use kindric_accounts::files::{create_private_dir, write_atomic};

/// The name of kindric's theme, in its file and in Claude Code's list.
pub const NAME: &str = "kindric";

/// The family of Claude Code's built-in themes, kept from light to dark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Plain,
    Daltonized,
    Ansi,
}

/// The variant of Claude Code's theme `theme` ("dark-daltonized", "light", …).
pub fn variant_of(theme: &str) -> Variant {
    if theme.ends_with("-daltonized") {
        Variant::Daltonized
    } else if theme.ends_with("-ansi") {
        Variant::Ansi
    } else {
        Variant::Plain
    }
}

/// The built-in theme of `variant` for a light or a dark background.
pub fn base(variant: Variant, light: bool) -> &'static str {
    match (variant, light) {
        (Variant::Plain, true) => "light",
        (Variant::Plain, false) => "dark",
        (Variant::Daltonized, true) => "light-daltonized",
        (Variant::Daltonized, false) => "dark-daltonized",
        (Variant::Ansi, true) => "light-ansi",
        (Variant::Ansi, false) => "dark-ansi",
    }
}

/// Red, green and blue of `css`, "#rrggbb", "#rgb", "#rrggbbaa" (the forms of `theme.json`, the
/// opacity left aside) or "rgb(r, g, b)".
fn rgb(css: &str) -> Option<[u8; 3]> {
    let css = css.trim();
    if let Some(hex) = css.strip_prefix('#') {
        let digits: Vec<u8> = hex.chars().map(|digit| digit.to_digit(16).and_then(|digit| u8::try_from(digit).ok())).collect::<Option<_>>()?;
        return match digits[..] {
            [r, g, b] => Some([r * 17, g * 17, b * 17]),
            [r1, r2, g1, g2, b1, b2] | [r1, r2, g1, g2, b1, b2, _, _] => Some([r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2]),
            _ => None,
        };
    }
    let inner = css.strip_prefix("rgb(")?.strip_suffix(')')?;
    let parts: Vec<u8> = inner.split(',').map(|part| part.trim().parse().ok()).collect::<Option<_>>()?;
    match parts[..] {
        [r, g, b] => Some([r, g, b]),
        _ => None,
    }
}

/// Whether the background `css` is light, the way Claude Code tells a terminal's: weighted
/// red, green and blue above one half.
pub fn is_light(css: &str) -> Option<bool> {
    let [r, g, b] = rgb(css)?;
    let luminance = 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b);
    Some(luminance / 255.0 > 0.5)
}

/// The themes every account shares, under `shared_root` (`~/.claude`).
pub fn themes_dir(shared_root: &Path) -> PathBuf {
    shared_root.join("themes")
}

/// The variant of the theme the user chose for Claude Code in `shared_root/settings.json`; a
/// custom theme gives that of its base. Plain when nothing says otherwise.
pub fn user_variant(shared_root: &Path) -> Variant {
    let theme = read_json(&shared_root.join("settings.json"))
        .and_then(|settings| settings.get("theme")?.as_str().map(str::to_string));
    let Some(theme) = theme else {
        return Variant::Plain;
    };
    match theme.strip_prefix("custom:") {
        None => variant_of(&theme),
        Some(slug) => read_json(&themes_dir(shared_root).join(format!("{slug}.json")))
            .and_then(|custom| custom.get("base")?.as_str().map(variant_of))
            .unwrap_or(Variant::Plain),
    }
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// The theme of the sessions of the project in `directory`: `kindric-` and 8 hex digits of its
/// folder, the same as long as the project stays in that folder. Each project has its own, so
/// its sessions follow its colours.
pub fn project_theme(directory: &Path) -> String {
    let fingerprint = kindric_accounts::shared::fingerprint(directory.to_string_lossy().as_bytes());
    format!("{NAME}-{}", &fingerprint[..8])
}

fn content(name: &str, base: &str) -> String {
    let theme = serde_json::json!({ "name": name, "base": base, "overrides": {} });
    format!("{}\n", serde_json::to_string_pretty(&theme).unwrap_or_default())
}

/// Writes `themes/<name>.json` on `base`, unless it already says so: rewriting it would make
/// every session using it redraw. Returns whether it was written. A file kindric did not write
/// (another name inside) is left as it is.
pub fn write(themes: &Path, name: &str, base: &str) -> io::Result<bool> {
    let file = themes.join(format!("{name}.json"));
    let wanted = content(name, base);
    match fs::read_to_string(&file) {
        Ok(existing) if existing == wanted => return Ok(false),
        Ok(existing) => {
            let ours = serde_json::from_str::<serde_json::Value>(&existing)
                .is_ok_and(|theme| theme.get("name").and_then(serde_json::Value::as_str) == Some(name));
            if !ours {
                return Err(io::Error::other(format!("{} was not written by kindric: left as it is", file.display())));
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_private_dir(themes)?,
        Err(error) => return Err(error),
    }
    write_atomic(&file, &wanted)?;
    Ok(true)
}

/// A project and the terminal colour its sessions are shown on: its own theme's, or kindric's.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct ProjectBackground {
    pub directory: PathBuf,
    pub background: String,
}

fn follow_one(themes: &Path, variant: Variant, name: &str, background: &str) -> Result<bool, String> {
    let light = is_light(background).ok_or_else(|| format!("{name}: cannot tell whether the terminal colour {background:?} is light: use #rrggbb"))?;
    write(themes, name, base(variant, light)).map_err(|error| error.to_string())
}

/// kindric shows its terminals on `background`, those of each project on its own: Claude Code's
/// theme `kindric`, and that of each project, follow, in the user's variant. A theme that cannot
/// be written does not stop the others; all that went wrong is said together.
pub fn follow(shared_root: &Path, background: &str, projects: &[ProjectBackground]) -> Result<(), String> {
    let variant = user_variant(shared_root);
    let themes = themes_dir(shared_root);
    let wanted = std::iter::once((NAME.to_string(), background))
        .chain(projects.iter().map(|project| (project_theme(&project.directory), project.background.as_str())));
    let errors: Vec<String> = wanted.filter_map(|(name, background)| follow_one(&themes, variant, &name, background).err()).collect();
    if errors.is_empty() {
        return Ok(());
    }
    Err(format!("Claude Code's theme not changed: {}", errors.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-claude-theme-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn base_of(root: &Path, name: &str) -> String {
        let theme = read_json(&themes_dir(root).join(format!("{name}.json"))).unwrap();
        assert_eq!(theme["name"], name);
        theme["base"].as_str().unwrap().to_string()
    }

    fn written_base(root: &Path) -> String {
        base_of(root, NAME)
    }

    fn project(directory: &str, background: &str) -> ProjectBackground {
        ProjectBackground { directory: PathBuf::from(directory), background: background.into() }
    }

    #[test]
    fn the_variant_is_read_from_the_theme_name() {
        assert_eq!(variant_of("dark-daltonized"), Variant::Daltonized);
        assert_eq!(variant_of("light-daltonized"), Variant::Daltonized);
        assert_eq!(variant_of("dark-ansi"), Variant::Ansi);
        assert_eq!(variant_of("light"), Variant::Plain);
        assert_eq!(variant_of("auto"), Variant::Plain);
    }

    #[test]
    fn each_variant_has_a_light_and_a_dark_base() {
        assert_eq!(base(Variant::Daltonized, true), "light-daltonized");
        assert_eq!(base(Variant::Daltonized, false), "dark-daltonized");
        assert_eq!(base(Variant::Ansi, true), "light-ansi");
        assert_eq!(base(Variant::Ansi, false), "dark-ansi");
        assert_eq!(base(Variant::Plain, true), "light");
        assert_eq!(base(Variant::Plain, false), "dark");
    }

    #[test]
    fn colours_are_light_or_dark_as_claude_code_tells_them() {
        assert_eq!(is_light("#ffffff"), Some(true));
        assert_eq!(is_light("#000"), Some(false));
        assert_eq!(is_light("#FFF"), Some(true));
        assert_eq!(is_light("rgb(251, 246, 238)"), Some(true));
        assert_eq!(is_light(" #1f120a "), Some(false));
        assert_eq!(is_light("#fbf6ee00"), Some(true), "opacity left aside");
        assert_eq!(is_light("#1f120aff"), Some(false));
        // Just under and over one half: 0.2126 + 0.7152 + 0.0722 = 1.
        assert_eq!(is_light("#7f7f7f"), Some(false));
        assert_eq!(is_light("#818181"), Some(true));
        assert_eq!(is_light("tomato"), None);
        assert_eq!(is_light("#12345"), None);
        assert_eq!(is_light("rgb(1, 2)"), None);
        assert_eq!(is_light("#gggggg"), None);
    }

    #[test]
    fn the_terminals_of_the_page_themes_are_told_apart() {
        let themes = include_str!("../../desktop/ui/themes.js");
        for (name, light) in [("ember", false), ("paper", true), ("graphite", false)] {
            let block = &themes[themes.find(&format!("  {name}: {{")).unwrap_or_else(|| panic!("no theme {name}"))..];
            let block = &block[..block.find("  },").unwrap()];
            let start = block.find("    terminal: \"").unwrap() + "    terminal: \"".len();
            let colour = &block[start..start + block[start..].find('"').unwrap()];
            assert_eq!(is_light(colour), Some(light), "{name}: {colour}");
        }
    }

    #[test]
    fn the_users_variant_comes_from_their_settings() {
        let root = temp_root("variant");
        assert_eq!(user_variant(&root), Variant::Plain, "no settings");
        fs::write(root.join("settings.json"), r#"{"theme": "dark-daltonized"}"#).unwrap();
        assert_eq!(user_variant(&root), Variant::Daltonized);
        fs::write(root.join("settings.json"), r#"{"model": "opus"}"#).unwrap();
        assert_eq!(user_variant(&root), Variant::Plain, "no theme");
        fs::write(root.join("settings.json"), "not json").unwrap();
        assert_eq!(user_variant(&root), Variant::Plain, "unreadable");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_custom_theme_gives_the_variant_of_its_base() {
        let root = temp_root("custom");
        fs::create_dir_all(themes_dir(&root)).unwrap();
        fs::write(root.join("settings.json"), r#"{"theme": "custom:mine"}"#).unwrap();
        assert_eq!(user_variant(&root), Variant::Plain, "file missing");
        fs::write(themes_dir(&root).join("mine.json"), r#"{"name": "Mine", "base": "light-ansi"}"#).unwrap();
        assert_eq!(user_variant(&root), Variant::Ansi);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn following_changes_the_base_with_the_background() {
        let root = temp_root("follow");
        fs::write(root.join("settings.json"), r#"{"theme": "dark-daltonized"}"#).unwrap();
        assert_eq!(follow(&root, "#fbf6ee", &[]), Ok(()));
        assert_eq!(written_base(&root), "light-daltonized");
        assert_eq!(follow(&root, "#1f120a", &[]), Ok(()));
        assert_eq!(written_base(&root), "dark-daltonized");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_theme_already_so_is_not_rewritten() {
        let root = temp_root("same");
        let themes = themes_dir(&root);
        assert!(write(&themes, NAME, "dark").unwrap());
        assert!(!write(&themes, NAME, "dark").unwrap(), "already so");
        assert!(write(&themes, NAME, "light").unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn each_project_gets_a_theme_of_its_own() {
        let root = temp_root("projects");
        let projects = [project("/home/u/api", "#fbf6ee"), project("/home/u/blog", "#161719")];
        assert_eq!(follow(&root, "#1f120a", &projects), Ok(()));
        assert_eq!(written_base(&root), "dark");
        assert_eq!(base_of(&root, &project_theme(Path::new("/home/u/api"))), "light");
        assert_eq!(base_of(&root, &project_theme(Path::new("/home/u/blog"))), "dark");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_project_theme_is_named_after_its_folder_only() {
        let api = project_theme(Path::new("/home/u/api"));
        assert_eq!(api, project_theme(Path::new("/home/u/api")), "the same each time");
        assert_ne!(api, project_theme(Path::new("/home/u/blog")));
        assert!(api.starts_with("kindric-") && api.len() == "kindric-".len() + 8, "{api}");
        // Claude Code's settings take it, and kindric-accounts too (`theme`).
        assert!(kindric_accounts::registry::is_valid_name(&api), "{api}");
    }

    #[test]
    fn a_kindric_json_written_by_someone_else_is_left_alone() {
        let root = temp_root("foreign");
        fs::create_dir_all(themes_dir(&root)).unwrap();
        let file = themes_dir(&root).join("kindric.json");
        fs::write(&file, r#"{"name": "My kindric", "base": "dark"}"#).unwrap();
        let error = follow(&root, "#ffffff", &[project("/home/u/api", "#ffffff")]).unwrap_err();
        assert!(error.contains("was not written by kindric"), "{error}");
        assert_eq!(fs::read_to_string(&file).unwrap(), r#"{"name": "My kindric", "base": "dark"}"#);
        assert_eq!(base_of(&root, &project_theme(Path::new("/home/u/api"))), "light", "the others are still written");
        fs::write(&file, "not json").unwrap();
        assert!(follow(&root, "#ffffff", &[]).is_err(), "unreadable: not ours either");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_colour_kindric_cannot_read_changes_nothing() {
        let root = temp_root("unreadable");
        let error = follow(&root, "var(--terminal)", &[]).unwrap_err();
        assert!(error.contains("var(--terminal)"), "{error}");
        assert!(!themes_dir(&root).exists());
        fs::remove_dir_all(root).unwrap();
    }
}
