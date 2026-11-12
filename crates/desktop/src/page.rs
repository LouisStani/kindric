//! Checks on the page in `ui/`, which has no build step of its own.

use std::fs;
use std::path::{Path, PathBuf};

fn ui() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ui")
}

#[test]
fn every_vendored_script_is_loaded_and_has_its_license() {
    let vendor = ui().join("vendor");
    let page = include_str!("../ui/index.html");
    let mut scripts = Vec::new();
    for entry in fs::read_dir(&vendor).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        let Some(stem) = name.strip_suffix(".js") else {
            continue;
        };
        assert!(page.contains(&format!("<script src=\"vendor/{name}\"></script>")), "index.html does not load {name}");
        assert!(vendor.join(format!("LICENSE.{stem}")).is_file(), "{name} comes without LICENSE.{stem}");
        scripts.push(name);
    }
    scripts.sort();
    assert_eq!(scripts, ["addon-fit.js", "addon-unicode11.js", "addon-webgl.js", "xterm.js"]);
}

/// A command the page calls by name but Rust does not register fails only when it is used.
/// Checks the names written as `invoke("name"`; a name passed through a variable is not seen.
#[test]
fn every_command_the_page_invokes_is_registered() {
    let main = include_str!("main.rs");
    let start = main.find("generate_handler![").unwrap();
    let end = start + main[start..].find(']').unwrap();
    let registered: Vec<&str> = main[start + "generate_handler![".len()..end].split(',').map(str::trim).filter(|name| !name.is_empty()).collect();
    let mut invoked = 0;
    for entry in fs::read_dir(ui()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "js") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        for piece in text.split("invoke(\"").skip(1) {
            let name = &piece[..piece.find('"').unwrap()];
            invoked += 1;
            assert!(registered.contains(&name), "{}: invokes {name}, which main.rs does not register", path.display());
        }
    }
    assert!(invoked > 50, "only {invoked} calls found: the search no longer matches the page");
}

/// Claude Code counts an emoji two cells when it lines up a table; a terminal without the
/// Unicode 11 table counts one, and the borders come out of line.
#[test]
fn every_terminal_counts_emojis_as_claude_code_does() {
    let mut terminals = 0;
    for entry in fs::read_dir(ui()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "js") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains("new Terminal(") {
                continue;
            }
            terminals += 1;
            let next = lines.get(index + 1).copied().unwrap_or_default();
            assert!(next.contains("loadUnicode11("), "{}:{}: a terminal without loadUnicode11 on the next line", path.display(), index + 1);
        }
    }
    assert_eq!(terminals, 2, "the panes and the helper window");
}
