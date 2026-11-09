//! The images Claude proposed in the last turn of a session: the paths of image files its
//! messages cite, read from the session's transcript, `<account>/projects/*/<session id>.jsonl`.
//! That file is not documented: whatever it lacks or changes gives no image, never an error.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

const EXTENSIONS: [&str; 5] = [".png", ".jpg", ".jpeg", ".gif", ".webp"];
/// At most this many images per turn.
const MOST: usize = 6;
/// Only the end of the transcript is read: it grows with the session, and the last turn ends it.
const TRANSCRIPT_TAIL: u64 = 2 * 1024 * 1024;

/// The transcript of `session_id`, the latest written if several folders hold one: the folder
/// changes when the session moves to a worktree.
pub fn transcript_of(account_dir: &Path, session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty() || session_id.contains(['/', '\\', '.']) {
        return None;
    }
    let name = format!("{session_id}.jsonl");
    let mut latest: Option<(SystemTime, PathBuf)> = None;
    for folder in std::fs::read_dir(account_dir.join("projects")).ok()?.flatten() {
        let candidate = folder.path().join(&name);
        let Some(written) = std::fs::metadata(&candidate).ok().filter(|meta| meta.is_file()).and_then(|meta| meta.modified().ok()) else {
            continue;
        };
        if latest.as_ref().is_none_or(|(newest, _)| written > *newest) {
            latest = Some((written, candidate));
        }
    }
    latest.map(|(_, path)| path)
}

/// The end of `path`, whole lines only.
fn tail(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let start = file.metadata().ok()?.len().saturating_sub(TRANSCRIPT_TAIL);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if start == 0 {
        return Some(text);
    }
    Some(text.split_once('\n').map_or_else(String::new, |(_, rest)| rest.to_string()))
}

fn flag(entry: &Value, name: &str) -> bool {
    entry.get(name).and_then(Value::as_bool).unwrap_or(false)
}

/// What Louis typed: a user message of the main conversation that is not a tool result, a
/// notification or a shell command (those start with a tag, `<task-notification>`…).
fn is_prompt(entry: &Value) -> bool {
    if entry.get("type").and_then(Value::as_str) != Some("user") || flag(entry, "isMeta") || flag(entry, "isSidechain") {
        return false;
    }
    let text = match entry.pointer("/message/content") {
        Some(Value::String(text)) => Some(text.as_str()),
        Some(Value::Array(blocks)) if !blocks.iter().any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result")) => {
            blocks.iter().find_map(|block| block.get("text").and_then(Value::as_str))
        },
        _ => None,
    };
    text.is_some_and(|text| !text.trim_start().starts_with('<'))
}

/// The text blocks of an assistant message of the main conversation.
fn assistant_texts(entry: &Value) -> Vec<&str> {
    if entry.get("type").and_then(Value::as_str) != Some("assistant") || flag(entry, "isSidechain") {
        return Vec::new();
    }
    let Some(Value::Array(blocks)) = entry.pointer("/message/content") else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect()
}

/// What Claude wrote since the last prompt, in a transcript's end; a broken line is skipped.
pub fn last_turn_text(transcript_tail: &str) -> String {
    let mut texts: Vec<String> = Vec::new();
    for entry in transcript_tail.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
        if is_prompt(&entry) {
            texts.clear();
        }
        texts.extend(assistant_texts(&entry).into_iter().map(str::to_string));
    }
    texts.join("\n")
}

fn is_image_name(text: &str) -> bool {
    let lower = text.to_lowercase();
    EXTENSIONS.iter().any(|extension| lower.ends_with(extension) && lower.len() > extension.len())
}

/// A word as written in a message, without what surrounds a path: quotes, brackets, a
/// Markdown link's text, the punctuation that ends a sentence.
fn cleaned(word: &str) -> &str {
    let word = word.rsplit_once("](").map_or(word, |(_, target)| target);
    let word = word.trim_start_matches(['"', '\'', '`', '(', '[', '<', '*']);
    let word = word.trim_end_matches(['"', '\'', '`', ')', ']', '>', ',', '.', ';', ':', '!', '?', '*']);
    word.strip_prefix("file://").unwrap_or(word)
}

/// What may name an image in `text`, in order: a code span whole, since a path in one may hold
/// spaces; elsewhere, and in code blocks, each word.
fn candidates(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    for (index, part) in text.split('`').enumerate() {
        if index % 2 == 1 && !part.contains('\n') {
            found.push(part.trim());
        } else {
            found.extend(part.split_whitespace().map(cleaned));
        }
    }
    found.retain(|candidate| is_image_name(candidate));
    found
}

fn resolve(candidate: &str, cwd: &Path, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(rest) = candidate.strip_prefix("~/").or_else(|| candidate.strip_prefix("~\\")) {
        return home.map(|home| home.join(rest));
    }
    let path = Path::new(candidate);
    Some(if path.is_absolute() { path.to_path_buf() } else { cwd.join(path) })
}

/// The image files `text` cites that exist, each once, in the order cited, `MOST` at most. A
/// relative path is read from `cwd`, the folder of the session.
pub fn image_paths(text: &str, cwd: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for path in candidates(text).into_iter().filter_map(|candidate| resolve(candidate, cwd, home)) {
        if found.len() == MOST {
            break;
        }
        if path.is_file() && !found.contains(&path) {
            found.push(path);
        }
    }
    found
}

/// The images the last turn of `session_id` proposed, as the page names them.
pub fn proposed(account_dir: &Path, session_id: &str, cwd: &Path, home: Option<&Path>) -> Vec<String> {
    let Some(text) = transcript_of(account_dir, session_id).and_then(|transcript| tail(&transcript)) else {
        return Vec::new();
    };
    image_paths(&last_turn_text(&text), cwd, home).into_iter().map(|path| path.to_string_lossy().into_owned()).collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-images-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn line(entry: serde_json::Value) -> String {
        entry.to_string()
    }

    fn prompt(text: &str) -> String {
        line(serde_json::json!({"type":"user","message":{"role":"user","content":text}}))
    }

    fn said(text: &str) -> String {
        line(serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":text}]}}))
    }

    #[test]
    fn the_last_turn_starts_at_the_last_prompt() {
        let tail = [
            prompt("draw the logo"),
            said("old.png is ready"),
            prompt("again, in blue"),
            said("I draw it."),
            line(serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{}}]}})),
            line(serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}})),
            prompt("<task-notification>done</task-notification>"),
            line(serde_json::json!({"type":"user","isMeta":true,"message":{"content":[{"type":"text","text":"caveat"}]}})),
            line(serde_json::json!({"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"agent.png"}]}})),
            said("Here it is: `blue.png`."),
            "{broken".to_string(),
        ]
        .join("\n");
        assert_eq!(last_turn_text(&tail), "I draw it.\nHere it is: `blue.png`.");
    }

    #[test]
    fn a_prompt_with_an_image_attached_starts_a_turn_too() {
        let attached = line(serde_json::json!({"type":"user","message":{"content":[{"type":"text","text":"look"},{"type":"image","source":{}}]}}));
        let tail = [said("before.png"), attached, said("after")].join("\n");
        assert_eq!(last_turn_text(&tail), "after");
    }

    #[test]
    fn paths_are_found_in_code_links_and_sentences() {
        let text = "Rendered `shots/with space.png` and [the logo](logo.PNG), see out/a.jpg.\n\
                    Also \"b.webp\", (c.gif) and not.png.bak nor .png alone.";
        assert_eq!(candidates(text), ["shots/with space.png", "logo.PNG", "out/a.jpg", "b.webp", "c.gif"]);
    }

    #[test]
    fn only_existing_files_count_each_once() {
        let root = scratch("exist");
        fs::create_dir_all(root.join("out")).unwrap();
        fs::write(root.join("out").join("a.png"), b"x").unwrap();
        fs::write(root.join("b.jpg"), b"x").unwrap();
        let absolute = root.join("b.jpg");
        let text = format!("`out/a.png` then out/a.png, {} and missing.png", absolute.display());
        assert_eq!(image_paths(&text, &root, None), [root.join("out").join("a.png"), absolute]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_tilde_path_is_read_from_home() {
        let root = scratch("home");
        fs::write(root.join("shot.png"), b"x").unwrap();
        assert_eq!(image_paths("see ~/shot.png", Path::new("/nowhere"), Some(&root)), [root.join("shot.png")]);
        assert!(image_paths("see ~/shot.png", Path::new("/nowhere"), None).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn at_most_six_images() {
        let root = scratch("most");
        let names: Vec<String> = (0..8).map(|i| format!("{i}.png")).collect();
        for name in &names {
            fs::write(root.join(name), b"x").unwrap();
        }
        assert_eq!(image_paths(&names.join(" "), &root, None).len(), MOST);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_latest_transcript_of_the_session_is_read() {
        let root = scratch("transcript");
        let session = "aae017d7-e825";
        for folder in ["-home-u-kindric", "-home-u-kindric--claude-worktrees-x"] {
            fs::create_dir_all(root.join("projects").join(folder)).unwrap();
        }
        let old = root.join("projects/-home-u-kindric").join(format!("{session}.jsonl"));
        let new = root.join("projects/-home-u-kindric--claude-worktrees-x").join(format!("{session}.jsonl"));
        fs::write(&old, [prompt("x"), said("old.png")].join("\n")).unwrap();
        fs::write(&new, [prompt("x"), said("new.png")].join("\n")).unwrap();
        let earlier = SystemTime::now() - std::time::Duration::from_secs(60);
        File::options().write(true).open(&old).unwrap().set_modified(earlier).unwrap();
        assert_eq!(transcript_of(&root, session), Some(new));
        assert_eq!(transcript_of(&root, "../escape"), None);
        assert_eq!(transcript_of(&root, "missing"), None);
        fs::write(root.join("new.png"), b"x").unwrap();
        assert_eq!(proposed(&root, session, &root, None), [root.join("new.png").to_string_lossy().into_owned()]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_cut_first_line_is_skipped() {
        let root = scratch("tail");
        let path = root.join("t.jsonl");
        let padding = said(&"x".repeat(usize::try_from(TRANSCRIPT_TAIL).unwrap()));
        fs::write(&path, [prompt("x"), padding, prompt("y"), said("kept")].join("\n")).unwrap();
        let read = tail(&path).unwrap();
        assert!(read.lines().all(|line| serde_json::from_str::<Value>(line).is_ok()), "no half line left");
        assert_eq!(last_turn_text(&read), "kept");
        fs::remove_dir_all(root).unwrap();
    }
}
