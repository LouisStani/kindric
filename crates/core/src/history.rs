//! Search in what was said in a session: the words of a query in the messages of its transcript
//! (Claude Code, `<account>/projects/*/<session id>.jsonl`) or of its rollout (Codex), the user's
//! and the agent's, not the tools' output. Ctrl+Shift+F of the window shows, for each session, how
//! many messages hold every word and an excerpt of the last one. These files are not documented:
//! a line that cannot be read is skipped, never an error.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

use crate::attach::Agent;

/// Characters of an excerpt before the first word found.
const BEFORE: usize = 60;
/// Characters of an excerpt at most, without the marks of a cut.
const EXCERPT: usize = 180;

/// What marks a line of a tool's output, which is not looked at: in a transcript, a message's
/// text holds quotes escaped, so this can only be JSON of the file itself.
const TOOL_OUTPUT: &str = r#""type":"tool_result""#;

/// The messages of one session that hold every word of a query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// How many messages hold every word.
    pub matches: usize,
    /// The last of them around its first word, on one line.
    pub excerpt: String,
    /// When it was written, as the file gives it (ISO 8601 for both agents), if it does.
    pub at: Option<String>,
}

/// The words of `query`, lower case: a message must hold all of them, anywhere, in any case.
pub fn words_of(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// The messages of the session in `path`, a transcript of `agent`, that hold every word. `None`
/// when none does, when there is no word, or when the file cannot be opened. `stop` is asked
/// between lines: a search no longer wanted ends early, with nothing.
pub fn search(path: &Path, agent: Agent, words: &[String], stop: &dyn Fn() -> bool) -> Option<Found> {
    if words.is_empty() {
        return None;
    }
    let reader = BufReader::new(File::open(path).ok()?);
    let mut found: Option<Found> = None;
    for (number, line) in reader.split(b'\n').enumerate() {
        if number % 256 == 0 && stop() {
            return None;
        }
        let Ok(line) = line else { break };
        let Ok(line) = std::str::from_utf8(&line) else { continue };
        if line.contains(TOOL_OUTPUT) || !may_hold(line, words) {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        for text in said(&entry, agent) {
            let lower = text.to_lowercase();
            if !words.iter().all(|word| lower.contains(word.as_str())) {
                continue;
            }
            let at = entry.get("timestamp").and_then(Value::as_str).map(str::to_string);
            let matches = found.as_ref().map_or(0, |found| found.matches) + 1;
            found = Some(Found { matches, excerpt: excerpt(&text, words), at });
        }
    }
    found
}

/// Whether a raw line may hold every word, before it is read as JSON: most lines do not, and
/// reading them all would be slow. A word JSON escapes (a quote, a backslash) is not looked for.
fn may_hold(line: &str, words: &[String]) -> bool {
    let ascii = line.to_ascii_lowercase();
    let mut full: Option<String> = None;
    words.iter().filter(|word| !word.contains(['"', '\\'])).all(|word| {
        if word.is_ascii() {
            ascii.contains(word.as_str())
        } else {
            // Its longest part in ASCII first, cheap to look for: "cran" of "écran".
            ascii.contains(longest_ascii(word)) && full.get_or_insert_with(|| line.to_lowercase()).contains(word.as_str())
        }
    })
}

/// The longest run of ASCII characters in `word`, empty when it has none.
fn longest_ascii(word: &str) -> &str {
    word.split(|character: char| !character.is_ascii()).max_by_key(|part| part.len()).unwrap_or_default()
}

/// The texts the user and the agent wrote in a transcript entry; nothing for the rest (tools,
/// what the agent puts in user messages itself, subagents).
fn said(entry: &Value, agent: Agent) -> Vec<String> {
    match agent {
        Agent::ClaudeCode => claude_said(entry),
        Agent::Codex => codex_said(entry),
        Agent::OpenCode => Vec::new(),
    }
}

/// What a person typed or the agent answered, not a block the agent adds itself
/// (`<command-name>`, `<task-notification>`, Codex's `<environment_context>` and AGENTS.md).
fn typed(text: &str) -> bool {
    let text = text.trim_start();
    !text.is_empty() && !text.starts_with('<') && !text.starts_with("# AGENTS.md")
}

fn claude_said(entry: &Value) -> Vec<String> {
    let kind = entry.get("type").and_then(Value::as_str);
    if !matches!(kind, Some("user" | "assistant"))
        || entry.get("isSidechain").and_then(Value::as_bool) == Some(true)
        || entry.get("isMeta").and_then(Value::as_bool) == Some(true)
    {
        return Vec::new();
    }
    let texts: Vec<&str> = match entry.pointer("/message/content") {
        Some(Value::String(text)) => vec![text.as_str()],
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect(),
        _ => Vec::new(),
    };
    texts.into_iter().filter(|text| typed(text)).map(str::to_string).collect()
}

fn codex_said(entry: &Value) -> Vec<String> {
    if entry.get("type").and_then(Value::as_str) != Some("response_item") {
        return Vec::new();
    }
    let Some(item) = entry.get("payload") else {
        return Vec::new();
    };
    let role = item.get("role").and_then(Value::as_str);
    if item.get("type").and_then(Value::as_str) != Some("message") || !matches!(role, Some("user" | "assistant")) {
        return Vec::new();
    }
    let Some(Value::Array(parts)) = item.get("content") else {
        return Vec::new();
    };
    parts.iter().filter_map(|part| part.get("text").and_then(Value::as_str)).filter(|text| typed(text)).map(str::to_string).collect()
}

/// `text` on one line, around the first of `words` it holds: some characters before, a few more
/// after, an ellipsis where it was cut.
fn excerpt(text: &str, words: &[String]) -> String {
    let flat: Vec<char> = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect();
    let start = words.iter().filter_map(|word| position(&flat, word)).min().unwrap_or(0);
    let from = start.saturating_sub(BEFORE);
    let to = (from + EXCERPT).min(flat.len());
    let from = to.saturating_sub(EXCERPT).min(from);
    let mut cut: String = flat[from..to].iter().collect();
    if from > 0 {
        cut.insert(0, '…');
    }
    if to < flat.len() {
        cut.push('…');
    }
    cut
}

/// Where `word` (lower case) starts in `text`, in characters, in any case. A character whose lower
/// case is longer than one ("İ") is compared by its first.
fn position(text: &[char], word: &str) -> Option<usize> {
    let word: Vec<char> = word.chars().collect();
    let lower: Vec<char> = text.iter().map(|character| character.to_lowercase().next().unwrap_or(*character)).collect();
    if word.is_empty() || word.len() > lower.len() {
        return None;
    }
    (0..=lower.len() - word.len()).find(|&at| lower[at..at + word.len()] == word[..])
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;

    fn file(label: &str, entries: &[Value]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("kindric-history-{label}-{}.jsonl", std::process::id()));
        let mut text: String = entries.iter().map(|entry| format!("{entry}\n")).collect();
        text.push_str("{broken\n");
        std::fs::write(&path, text).unwrap();
        path
    }

    fn never() -> bool {
        false
    }

    fn claude() -> Vec<Value> {
        vec![
            json!({"type": "user", "timestamp": "2026-11-27T10:00:00.000Z", "message": {"content": "Draw the Logo of the shop"}}),
            json!({"type": "assistant", "timestamp": "2026-11-27T10:00:05.000Z", "message": {"content": [
                {"type": "text", "text": "I will draw the logo\n  in SVG."},
                {"type": "tool_use", "name": "Write", "input": {"content": "logo shop"}}]}}),
            json!({"type": "user", "timestamp": "2026-11-27T10:00:06.000Z", "message": {"content": [{"type": "tool_result", "content": "logo shop written"}]}}),
            json!({"type": "assistant", "isSidechain": true, "message": {"content": [{"type": "text", "text": "logo shop in a subagent"}]}}),
            json!({"type": "user", "isMeta": true, "message": {"content": "logo shop from a hook"}}),
            json!({"type": "user", "message": {"content": "<task-notification>logo shop</task-notification>"}}),
            json!({"type": "user", "timestamp": "2026-11-27T11:00:00.000Z", "message": {"content": [{"type": "text", "text": "the logo is fine, now the SHOP page"}]}}),
        ]
    }

    #[test]
    fn only_what_the_user_and_claude_wrote_counts() {
        let path = file("claude", &claude());
        let found = search(&path, Agent::ClaudeCode, &words_of("shop LOGO"), &never).unwrap();
        assert_eq!(found.matches, 2, "the prompt and the last one, not tools, subagents, hooks or notifications");
        assert_eq!(found.excerpt, "the logo is fine, now the SHOP page");
        assert_eq!(found.at.as_deref(), Some("2026-11-27T11:00:00.000Z"));
        assert_eq!(search(&path, Agent::ClaudeCode, &words_of("svg"), &never).unwrap().excerpt, "I will draw the logo in SVG.");
        assert_eq!(search(&path, Agent::ClaudeCode, &words_of("logo absent"), &never), None);
        assert_eq!(search(&path, Agent::ClaudeCode, &words_of("  "), &never), None);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn every_word_must_be_in_the_same_message() {
        let path = file("same", &claude());
        assert_eq!(search(&path, Agent::ClaudeCode, &words_of("draw page"), &never), None);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn codex_messages_are_read_from_the_rollout() {
        let message = |role: &str, text: &str| json!({"type": "response_item", "timestamp": "2026-11-25T15:00:00.000Z",
            "payload": {"type": "message", "role": role, "content": [{"type": "input_text", "text": text}]}});
        let path = file("codex", &[
            message("developer", &["You are Codex; list files"].concat()),
            message("user", "# AGENTS.md instructions\nlist files"),
            message("user", "<environment_context>list files</environment_context>"),
            message("user", "list the files in this folder"),
            json!({"type": "event_msg", "payload": {"type": "user_message", "message": "list the files in this folder"}}),
            json!({"type": "response_item", "payload": {"type": "function_call", "name": "exec_command", "arguments": "list files"}}),
            message("assistant", "There's one file in this folder."),
        ]);
        let found = search(&path, Agent::Codex, &words_of("files list"), &never).unwrap();
        assert_eq!(found.matches, 1, "the prompt once, not the instructions, the event or the tool");
        assert_eq!(found.excerpt, "list the files in this folder");
        assert_eq!(search(&path, Agent::Codex, &words_of("one file"), &never).unwrap().matches, 1);
        assert_eq!(search(&path, Agent::OpenCode, &words_of("files"), &never), None);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn case_and_accents_do_not_matter() {
        let path = file("accents", &[json!({"type": "user", "message": {"content": "Réglage de l'ÉCRAN, \"quoted\" word"}})]);
        assert_eq!(search(&path, Agent::ClaudeCode, &words_of("écran RÉGLAGE"), &never).unwrap().matches, 1);
        assert_eq!(search(&path, Agent::ClaudeCode, &words_of("\"quoted\""), &never).unwrap().matches, 1);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn the_ascii_part_of_a_word_is_looked_for_first() {
        assert_eq!(longest_ascii("écran"), "cran");
        assert_eq!(longest_ascii("été"), "t");
        assert_eq!(longest_ascii("日本"), "");
        assert!(may_hold("Le Grand ÉCRAN", &words_of("écran")));
        assert!(!may_hold("Le Grand ECRAN", &words_of("écran")));
    }

    #[test]
    fn a_search_no_longer_wanted_gives_nothing() {
        let path = file("stop", &claude());
        assert_eq!(search(&path, Agent::ClaudeCode, &words_of("logo"), &|| true), None);
        assert_eq!(search(Path::new("/no/such/file.jsonl"), Agent::ClaudeCode, &words_of("logo"), &never), None);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_long_message_is_cut_around_the_word() {
        let long = format!("{} needle {}", "a ".repeat(100), "b ".repeat(200));
        let cut = excerpt(&long, &words_of("NEEDLE"));
        assert!(cut.starts_with('…') && cut.ends_with('…'), "{cut}");
        assert_eq!(cut.chars().count(), EXCERPT + 2);
        assert_eq!(cut.chars().position(|character| character == 'n'), Some(BEFORE + 1));
        let end = format!("{} needle", "a ".repeat(200));
        let cut = excerpt(&end, &words_of("needle"));
        assert!(cut.starts_with('…') && cut.ends_with("needle"), "{cut}");
        assert_eq!(cut.chars().count(), EXCERPT + 1);
        assert_eq!(excerpt("short needle", &words_of("needle")), "short needle");
    }
}
