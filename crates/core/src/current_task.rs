//! What a session of Claude Code is doing now, for the line of its row (decisions 498, 499): the
//! last sentence Claude wrote in the turn, as it says what it does before its tools ("I add a
//! test that does not depend on timing."), else the prompt that started the turn. Read from the
//! end of the transcript: a prompt further back leaves the start of the turn unknown.

use serde_json::Value;

use crate::changes::unix_seconds;
use crate::images::{assistant_texts, prompt_text};
use crate::progress::one_line;

/// The turn at the end of a transcript.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Turn {
    /// When its prompt was typed, in Unix seconds; none when the prompt is out of the end read.
    pub started: Option<i64>,
    /// What the session does, on one line: the last sentence Claude wrote in the turn, else the
    /// prompt.
    pub doing: Option<String>,
}

/// A sentence this short ("Done.", "Je relance.") says little alone: the one before it comes too.
const SHORT: usize = 30;

/// The turn at the end of `transcript_tail`; a broken line is skipped.
pub fn turn_of(transcript_tail: &str) -> Turn {
    let mut turn = Turn::default();
    let mut said: Option<String> = None;
    let mut asked: Option<String> = None;
    for entry in transcript_tail.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
        if let Some(prompt) = prompt_text(&entry) {
            turn.started = entry.get("timestamp").and_then(Value::as_str).and_then(unix_seconds);
            asked = Some(prompt.to_string());
            said = None;
            continue;
        }
        for text in assistant_texts(&entry) {
            if let Some(sentence) = last_sentence(text) {
                said = Some(sentence);
            }
        }
    }
    turn.doing = said.or(asked).map(|text| one_line(&text)).filter(|text| !text.is_empty());
    turn
}

/// The last sentence of the last paragraph of prose of `text`: code blocks, tables, headings,
/// quotes and list items are left out. With the one before it when it is short.
pub fn last_sentence(text: &str) -> Option<String> {
    let paragraph = last_prose(text)?;
    let sentences = sentences(&paragraph);
    let (last, before) = sentences.split_last()?;
    match before.last() {
        Some(previous) if last.chars().count() < SHORT => Some(format!("{previous} {last}")),
        _ => Some(last.clone()),
    }
}

/// The last paragraph of prose of `text`, its lines joined.
fn last_prose(text: &str) -> Option<String> {
    let mut paragraphs: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut in_code = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if line.is_empty() || !is_prose(line) {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(line);
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    paragraphs.pop().map(|lines| lines.join(" "))
}

/// A line of prose, not of a table, a heading, a quote or a list.
fn is_prose(line: &str) -> bool {
    if line.starts_with(['|', '#', '>']) {
        return false;
    }
    let marker = line.trim_start_matches(|character: char| character.is_ascii_digit());
    let numbered = marker.len() < line.len() && marker.starts_with(". ");
    !(numbered || line.starts_with("- ") || line.starts_with("* "))
}

/// The sentences of `paragraph`: cut after `.`, `!`, `?` or `…` followed by a space.
fn sentences(paragraph: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut current = String::new();
    let mut characters = paragraph.chars().peekable();
    while let Some(character) = characters.next() {
        current.push(character);
        let ends = matches!(character, '.' | '!' | '?' | '…') && characters.peek().is_some_and(|next| next.is_whitespace());
        if ends {
            found.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        found.push(current.trim().to_string());
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn prompt(text: &str, at: &str) -> String {
        json!({"type": "user", "timestamp": at, "message": {"role": "user", "content": text}}).to_string()
    }

    fn said(text: &str) -> String {
        json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}}).to_string()
    }

    fn tool() -> String {
        json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "tool_use", "name": "Bash", "input": {}}]}}).to_string()
    }

    fn result() -> String {
        json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "content": "ok"}]}}).to_string()
    }

    fn of(lines: &[String]) -> Turn {
        turn_of(&lines.join("\n"))
    }

    #[test]
    fn the_prompt_until_claude_says_what_it_does() {
        let turn = of(&[prompt("fix the macOS test", "2026-11-26T10:00:00.000Z"), tool(), result()]);
        assert_eq!(turn.doing.as_deref(), Some("fix the macOS test"));
        assert_eq!(turn.started, Some(1_795_687_200));
    }

    #[test]
    fn then_the_last_sentence_it_wrote_in_the_turn() {
        let turn = of(&[
            prompt("go", "2026-11-26T10:00:00.000Z"),
            said("The 40 unit tests pass. I run the whole suite in the background."),
            tool(),
            result(),
            said("Two journeys fail. I read their screenshots before touching the code."),
            tool(),
        ]);
        assert_eq!(turn.doing.as_deref(), Some("I read their screenshots before touching the code."));
    }

    #[test]
    fn what_claude_wrote_before_the_prompt_does_not_count() {
        let turn = of(&[said("I wrote the notes."), prompt("now the docs", "2026-11-26T10:00:00.000Z"), tool()]);
        assert_eq!(turn.doing.as_deref(), Some("now the docs"));
    }

    #[test]
    fn a_tool_result_or_a_notification_does_not_start_a_turn() {
        let notification = prompt("<task-notification>done</task-notification>", "2026-11-26T11:00:00.000Z");
        let turn = of(&[prompt("go", "2026-11-26T10:00:00.000Z"), said("I build the image."), result(), notification]);
        assert_eq!(turn.doing.as_deref(), Some("I build the image."));
        assert_eq!(turn.started, Some(1_795_687_200));
    }

    #[test]
    fn without_the_prompt_in_the_end_read_the_start_is_unknown() {
        let turn = of(&[said("I watch the CI."), tool()]);
        assert_eq!(turn, Turn { started: None, doing: Some("I watch the CI.".into()) });
        assert_eq!(of(&[]), Turn::default());
    }

    #[test]
    fn a_short_sentence_comes_with_the_one_before() {
        assert_eq!(last_sentence("On gagne 51 liens au réglage. J'écris la décision.").as_deref(), Some("On gagne 51 liens au réglage. J'écris la décision."));
        assert_eq!(last_sentence("Done.").as_deref(), Some("Done."));
    }

    #[test]
    fn tables_code_headings_and_lists_are_not_prose() {
        let text = "I sum up the state.\n\n## Before the v1\n\n| N | Task |\n|---|---|\n| 1 | cards |\n\n```sh\nmake test.\n```\n- one item.\n2. another.";
        assert_eq!(last_sentence(text).as_deref(), Some("I sum up the state."));
        assert_eq!(last_sentence("| a | b |"), None);
    }

    #[test]
    fn the_lines_of_a_paragraph_make_one_sentence() {
        assert_eq!(last_sentence("I move it into `request_for`\nso that it can be tested.").as_deref(), Some("I move it into `request_for` so that it can be tested."));
        assert_eq!(last_sentence("Version 2.1 is out. Je relance la CI e.g. maintenant…").as_deref(), Some("Je relance la CI e.g. maintenant…"));
    }

    #[test]
    fn a_long_sentence_is_cut_on_one_line() {
        let long = format!("I {}.", "check every file ".repeat(20));
        let turn = of(&[prompt("go", "2026-11-26T10:00:00.000Z"), said(&long)]);
        let doing = turn.doing.unwrap_or_default();
        assert!(doing.ends_with('…') && doing.chars().count() == 160, "{doing}");
    }
}
