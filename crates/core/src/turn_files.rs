//! The files a session made to be looked at during its last turn (a page, an image, a PDF…) while
//! it kept none of them for the user with `keep_artifact`: what kindric's Stop hook asks it to
//! sort before it stops. Read from the end of the session's transcript, which is not documented:
//! whatever it lacks or changes gives no file, never an error.

use std::io;
use std::path::Path;

use serde_json::Value;

use crate::images::{is_prompt, tail};

/// What a file made to be looked at ends with.
const VISUAL: [&str; 13] =
    [".html", ".htm", ".svg", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".pdf", ".mp4", ".webm", ".mov", ".mkv"];
/// Words of a shell command that name the file written by what follows.
const OUTPUT_FLAGS: [&str; 6] = ["-o", "--output", "--out", "--screenshot", ">", ">>"];
/// At most this many files are named in the refusal.
const NAMED: usize = 10;
/// Beginnings of a shell word that name the file written in the same word.
const OUTPUT_PREFIXES: [&str; 5] = [">>", ">", "--output=", "--out=", "--screenshot="];

fn is_visual(path: &str) -> bool {
    let lower = path.to_lowercase();
    VISUAL.iter().any(|extension| lower.ends_with(extension) && lower.len() > extension.len())
}

/// A path the shell would still expand (`$OUT/a.png`, `shot-*.png`) cannot be named as written.
fn is_literal(path: &str) -> bool {
    !path.contains(['$', '*', '?', '{', '`'])
}

/// A shell word without its quotes and the punctuation that ends a command (`a.png;`).
fn unquoted(word: &str) -> &str {
    word.trim_end_matches([';', '&', '|', ')']).trim_matches(['"', '\''])
}

/// The visual files a shell command writes as it reads: after `-o`, `--output`, `>`… or in
/// `--output=x.png`. A file written by a program the command runs is not seen.
fn command_outputs(command: &str) -> Vec<String> {
    let words: Vec<&str> = command.split_whitespace().collect();
    let mut found = Vec::new();
    for (index, word) in words.iter().enumerate() {
        let attached = OUTPUT_PREFIXES.iter().find_map(|prefix| word.strip_prefix(prefix)).filter(|rest| !rest.is_empty());
        let after_flag = index > 0 && OUTPUT_FLAGS.contains(&words[index - 1]);
        let path = match attached {
            Some(rest) => unquoted(rest),
            None if after_flag => unquoted(word),
            None => continue,
        };
        if is_visual(path) && is_literal(path) {
            found.push(path.to_string());
        }
    }
    found
}

/// What a tool call of the main conversation does with files: the visual files it makes, and
/// whether it keeps a file for the user.
enum Call {
    Made(Vec<String>),
    KeptForUser,
}

fn call_of(block: &Value) -> Option<Call> {
    if block.get("type").and_then(Value::as_str) != Some("tool_use") {
        return None;
    }
    let name = block.get("name").and_then(Value::as_str)?;
    let input = block.get("input")?;
    let field = |key: &str| input.get(key).and_then(Value::as_str);
    if name == "keep_artifact" || name.ends_with("__keep_artifact") {
        return (field("for") == Some("user")).then_some(Call::KeptForUser);
    }
    match name {
        "Write" => field("file_path").filter(|path| is_visual(path)).map(|path| Call::Made(vec![path.to_string()])),
        "Bash" => field("command").map(|command| Call::Made(command_outputs(command))),
        _ => None,
    }
}

fn calls(entry: &Value) -> Vec<Call> {
    let main_conversation = entry.get("isSidechain").and_then(Value::as_bool) != Some(true);
    if entry.get("type").and_then(Value::as_str) != Some("assistant") || !main_conversation {
        return Vec::new();
    }
    let Some(Value::Array(blocks)) = entry.pointer("/message/content") else {
        return Vec::new();
    };
    blocks.iter().filter_map(call_of).collect()
}

/// The visual files made since the last prompt, each once, in the order made; none when the turn
/// kept a file for the user, since the session has then sorted them. A broken line is skipped.
pub fn unkept(transcript_tail: &str) -> Vec<String> {
    let mut made: Vec<String> = Vec::new();
    let mut kept = false;
    for entry in transcript_tail.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
        if is_prompt(&entry) {
            made.clear();
            kept = false;
        }
        for call in calls(&entry) {
            match call {
                Call::KeptForUser => kept = true,
                Call::Made(paths) => {
                    for path in paths {
                        if !made.contains(&path) {
                            made.push(path);
                        }
                    }
                },
            }
        }
    }
    if kept { Vec::new() } else { made }
}

fn refusal(files: &[String]) -> String {
    let mut names = files.iter().take(NAMED).map(String::as_str).collect::<Vec<_>>().join(", ");
    if files.len() > NAMED {
        names.push_str(&format!(" and {} more", files.len() - NAMED));
    }
    format!(
        "This turn made files to look at and kept none for the user: {names}. \
         Keep each one made for the user to see with keep_artifact (for=\"user\"); \
         one that only checked your own work needs nothing. Then stop: this is asked once per turn."
    )
}

/// What `kindric artifact-check` prints for `input`, the JSON Claude Code gives a Stop hook: a
/// refusal to stop while the turn made visual files and kept none for the user. Only at the first
/// stop of a turn (`stop_hook_active` false), so that the session can answer that none is for the
/// user; nothing when Claude Code does not say.
pub fn hook_answer(input: &str) -> Option<String> {
    let input: Value = serde_json::from_str(input).ok()?;
    let first_stop = input.get("stop_hook_active").and_then(Value::as_bool) == Some(false);
    if input.get("hook_event_name").and_then(Value::as_str) != Some("Stop") || !first_stop {
        return None;
    }
    let transcript = input.get("transcript_path")?.as_str()?;
    let files = unkept(&tail(Path::new(transcript))?);
    if files.is_empty() {
        return None;
    }
    Some(serde_json::json!({ "decision": "block", "reason": refusal(&files) }).to_string())
}

/// `kindric artifact-check`: reads Claude Code's input and prints the answer. Nothing that goes
/// wrong may disturb the session: it prints nothing then.
pub fn run_hook() {
    let mut input = String::new();
    if io::Read::read_to_string(&mut io::stdin(), &mut input).is_err() {
        return;
    }
    if let Some(answer) = hook_answer(&input) {
        println!("{answer}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(entry: Value) -> String {
        entry.to_string()
    }

    fn prompt(text: &str) -> String {
        line(json!({"type":"user","message":{"role":"user","content":text}}))
    }

    fn tool(name: &str, input: Value) -> String {
        line(json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":name,"input":input}]}}))
    }

    fn write(path: &str) -> String {
        tool("Write", json!({"file_path": path, "content": "x"}))
    }

    fn bash(command: &str) -> String {
        tool("Bash", json!({"command": command}))
    }

    fn keep(path: &str, who: &str) -> String {
        tool("mcp__kindric__keep_artifact", json!({"path": path, "for": who}))
    }

    #[test]
    fn the_visual_files_written_in_the_last_turn_are_listed_once() {
        let tail = [
            prompt("draw the old logo"),
            write("/tmp/old.png"),
            prompt("draw the logo"),
            write("/work/logo.svg"),
            write("/work/notes.md"),
            write("/work/logo.svg"),
            write("/work/page.HTML"),
            "{broken".to_string(),
        ]
        .join("\n");
        assert_eq!(unkept(&tail), ["/work/logo.svg", "/work/page.HTML"]);
    }

    #[test]
    fn a_shell_command_names_its_outputs() {
        let tail = [
            prompt("render it"),
            bash("chromium --headless --screenshot=shots/a.png page.html"),
            bash("curl -o 'b.jpg' https://x && pandoc r.md -o r.pdf > log.txt"),
            bash("cat x >out.html; convert in.png --out c.webp"),
            bash("ffmpeg -i in.mp4 -o \"$OUT/clip.mp4\" && magick a.png -o shot-*.png"),
        ]
        .join("\n");
        assert_eq!(unkept(&tail), ["shots/a.png", "b.jpg", "r.pdf", "out.html", "c.webp"]);
    }

    #[test]
    fn a_file_kept_for_the_user_settles_the_turn() {
        let tail = [prompt("make the report"), write("/work/report.pdf"), write("/work/shot.png"), keep("/work/report.pdf", "user")].join("\n");
        assert!(unkept(&tail).is_empty());
    }

    #[test]
    fn a_file_kept_for_claude_does_not() {
        let tail = [prompt("make the report"), write("/work/report.pdf"), keep("/work/build.py", "claude")].join("\n");
        assert_eq!(unkept(&tail), ["/work/report.pdf"]);
    }

    #[test]
    fn a_keep_of_an_earlier_turn_does_not_count() {
        let tail = [prompt("first"), keep("/work/a.png", "user"), prompt("second"), write("/work/b.png")].join("\n");
        assert_eq!(unkept(&tail), ["/work/b.png"]);
    }

    #[test]
    fn tool_results_notifications_and_subagents_neither_start_a_turn_nor_make_files() {
        let tail = [
            prompt("draw"),
            write("/work/a.png"),
            line(json!({"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}})),
            prompt("<task-notification>done</task-notification>"),
            line(json!({"type":"assistant","isSidechain":true,"message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/agent/b.png"}}]}})),
        ]
        .join("\n");
        assert_eq!(unkept(&tail), ["/work/a.png"]);
    }

    fn hook_input(transcript: &Path, first_stop: bool) -> String {
        json!({"hook_event_name":"Stop","stop_hook_active":!first_stop,"transcript_path":transcript}).to_string()
    }

    fn transcript(label: &str, lines: &[String]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("kindric-turn-files-{label}-{}.jsonl", std::process::id()));
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    #[test]
    fn the_first_stop_of_a_turn_with_unkept_files_is_refused_with_their_names() {
        let path = transcript("refused", &[prompt("draw"), write("/work/logo.png"), bash("pandoc r.md -o r.pdf")]);
        let answer: Value = serde_json::from_str(&hook_answer(&hook_input(&path, true)).unwrap()).unwrap();
        assert_eq!(answer["decision"], "block");
        let reason = answer["reason"].as_str().unwrap();
        assert!(reason.contains("/work/logo.png, r.pdf."), "{reason}");
        assert!(reason.contains("keep_artifact (for=\"user\")"), "{reason}");
        assert_eq!(hook_answer(&hook_input(&path, false)), None, "asked once per turn");
        let no_flag = json!({"hook_event_name":"Stop","transcript_path":path}).to_string();
        assert_eq!(hook_answer(&no_flag), None, "Claude Code does not say whether it refused already");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn nothing_is_asked_without_files_a_transcript_or_a_stop() {
        let path = transcript("calm", &[prompt("fix"), write("/work/main.rs")]);
        assert_eq!(hook_answer(&hook_input(&path, true)), None);
        assert_eq!(hook_answer(&hook_input(&path.with_extension("gone"), true)), None);
        let other = json!({"hook_event_name":"SubagentStop","stop_hook_active":false,"transcript_path":path}).to_string();
        assert_eq!(hook_answer(&other), None);
        assert_eq!(hook_answer("{broken"), None);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_long_list_is_cut() {
        let files: Vec<String> = (0..13).map(|n| format!("shot{n}.png")).collect();
        let reason = refusal(&files);
        assert!(reason.contains("shot9.png and 3 more."), "{reason}");
        assert!(!reason.contains("shot10.png"), "{reason}");
    }

    #[test]
    fn a_turn_without_visual_files_gives_none() {
        let tail = [prompt("fix the bug"), write("/work/main.rs"), bash("cargo test > out.txt")].join("\n");
        assert!(unkept(&tail).is_empty());
    }
}
