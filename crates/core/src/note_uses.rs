//! Which sessions read or wrote a note of the memory lately (decision 337): the atlas shows it.
//! Read from Claude Code's transcripts, `<account>/projects/<project>/<session>.jsonl`, and those
//! of the agents a session started, `<session>/subagents/*.jsonl`. A note counts when a tool
//! names its file: `Read` reads it, `Write` and `Edit` write it, and a shell command that names
//! it reads it, or writes it after `>`, `tee` or `sed -i`. Only the lines that name the memory's
//! folder are parsed: transcripts are long.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use crate::changes::unix_seconds;

/// The tools of Claude Code that write the file named by their `file_path`.
const WRITERS: [&str; 4] = ["Write", "Edit", "MultiEdit", "NotebookEdit"];

/// What sessions did with one note, newest use first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteUse {
    /// From the memory's root, with `/` on every system, as the atlas names notes.
    pub path: String,
    pub account: String,
    pub session: String,
    /// Done by an agent the session started, not by the session itself.
    pub by_agent: bool,
    /// Unix seconds of the last read and the last write since the time asked.
    pub read_at: Option<i64>,
    pub written_at: Option<i64>,
}

/// The uses of notes under `memory` since `since` (Unix seconds) in the transcripts of
/// `accounts` (name, folder), newest first. Transcripts not written since are not opened.
pub fn since(memory: &Path, accounts: &[(String, PathBuf)], since: i64) -> Vec<NoteUse> {
    let needles = needles(memory);
    let mut uses: BTreeMap<(String, String, String, bool), NoteUse> = BTreeMap::new();
    for (account, dir) in accounts {
        for (file, session, by_agent) in transcripts(dir, since) {
            let Ok(opened) = fs::File::open(&file) else {
                continue;
            };
            // Line by line: a transcript can weigh hundreds of MB. A line that is not UTF-8 is read
            // as well as it can be.
            for bytes in io::BufReader::new(opened).split(b'\n').map_while(Result::ok) {
                let line = String::from_utf8_lossy(&bytes);
                if !needles.iter().any(|needle| line.contains(needle.as_str())) {
                    continue;
                }
                let Ok(entry) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let Some(at) = entry.get("timestamp").and_then(Value::as_str).and_then(unix_seconds).filter(|at| *at >= since) else {
                    continue;
                };
                for (path, wrote) in notes_used(&entry, &needles) {
                    let key = (path.clone(), account.clone(), session.clone(), by_agent);
                    let found = uses.entry(key).or_insert_with(|| NoteUse {
                        path,
                        account: account.clone(),
                        session: session.clone(),
                        by_agent,
                        read_at: None,
                        written_at: None,
                    });
                    let slot = if wrote { &mut found.written_at } else { &mut found.read_at };
                    *slot = Some(slot.map_or(at, |before| before.max(at)));
                }
            }
        }
    }
    let mut uses: Vec<NoteUse> = uses.into_values().collect();
    uses.sort_by_key(|found| std::cmp::Reverse(found.read_at.max(found.written_at)));
    uses
}

/// How a transcript names the memory's folder: its path, and `~/…` when it is under the home.
fn needles(memory: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut add = |path: &Path| {
        let text = format!("{}/", path.display()).replace('\\', "/");
        if !found.contains(&text) {
            found.push(text);
        }
    };
    add(memory);
    if let Ok(real) = fs::canonicalize(memory) {
        add(&real);
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from) {
        for path in found.clone() {
            let home = format!("{}/", home.display()).replace('\\', "/");
            if let Some(rest) = path.strip_prefix(&home) {
                found.push(format!("~/{rest}"));
            }
        }
    }
    found
}

/// The transcripts of the account in `dir` written since `since`: each with its session, and
/// whether it is an agent's.
fn transcripts(dir: &Path, since: i64) -> Vec<(PathBuf, String, bool)> {
    let since = UNIX_EPOCH + Duration::from_secs(u64::try_from(since).unwrap_or(0));
    let recent = |path: &Path| fs::metadata(path).and_then(|meta| meta.modified()).is_ok_and(|modified| modified >= since);
    let mut found = Vec::new();
    for project in listed(&dir.join("projects")) {
        for entry in listed(&project) {
            let Some(name) = entry.file_name().map(|name| name.to_string_lossy().into_owned()) else {
                continue;
            };
            if let Some(session) = name.strip_suffix(".jsonl") {
                if recent(&entry) {
                    found.push((entry.clone(), session.to_string(), false));
                }
                continue;
            }
            for agent in listed(&entry.join("subagents")) {
                if agent.extension().is_some_and(|extension| extension == "jsonl") && recent(&agent) {
                    found.push((agent, name.clone(), true));
                }
            }
        }
    }
    found
}

fn listed(dir: &Path) -> Vec<PathBuf> {
    match fs::read_dir(dir) {
        Ok(read) => read.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect(),
        // A folder that cannot be read holds no use the atlas can show.
        Err(_) => Vec::new(),
    }
}

/// The notes an entry's tool calls name, each with whether it is written.
fn notes_used(entry: &Value, needles: &[String]) -> Vec<(String, bool)> {
    if entry.get("type").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    let Some(blocks) = entry.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for block in blocks.iter().filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use")) {
        let name = block.get("name").and_then(Value::as_str).unwrap_or_default();
        let input = block.get("input");
        if let Some(file) = input.and_then(|input| input.get("file_path")).and_then(Value::as_str) {
            found.extend(note_path(file, needles).map(|path| (path, WRITERS.contains(&name))));
        } else if let Some(command) = input.and_then(|input| input.get("command")).and_then(Value::as_str) {
            found.extend(in_command(command, needles));
        }
    }
    found
}

/// The note `file` is, from the memory's root, if it is a `.md` under it.
fn note_path(file: &str, needles: &[String]) -> Option<String> {
    let file = file.replace('\\', "/");
    let rest = needles.iter().find_map(|needle| file.strip_prefix(needle.as_str()))?;
    let plain = Path::new(rest).components().all(|part| matches!(part, Component::Normal(_)));
    // `knowledge/*.md` is a pattern of the shell, not a note.
    let pattern = rest.contains(['*', '?', '[', '{']);
    (plain && !pattern && rest.ends_with(".md")).then(|| rest.to_string())
}

/// The notes a shell command names, each written when it comes after `>`, or in a command with
/// `tee` or `sed -i`.
fn in_command(command: &str, needles: &[String]) -> Vec<(String, bool)> {
    // Paths of Windows are written with `\`; the needles have `/`.
    let command = if cfg!(windows) { command.replace('\\', "/") } else { command.to_string() };
    let command = command.as_str();
    let mut found = Vec::new();
    for needle in needles {
        for (start, _) in command.match_indices(needle.as_str()) {
            let rest = &command[start..];
            let end = rest.find(|character: char| character.is_whitespace() || "'\"`;|&()<>,".contains(character)).unwrap_or(rest.len());
            let Some(path) = note_path(rest[..end].trim_end_matches([':', '.']), needles) else {
                continue;
            };
            let simple = &command[command[..start].rfind([';', '|', '&', '\n']).map_or(0, |at| at + 1)..start];
            let wrote = simple.trim_end().ends_with('>') || simple.contains("tee ") || (simple.contains("sed ") && simple.contains(" -i"));
            found.push((path, wrote));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::away::now;
    use crate::changes::iso;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-note-uses-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn tool(time: i64, name: &str, input: Value) -> String {
        serde_json::json!({ "type": "assistant", "timestamp": iso(time),
            "message": { "role": "assistant", "content": [{ "type": "tool_use", "id": "t1", "name": name, "input": input }] } })
        .to_string()
    }

    fn write(path: &Path, lines: &[String]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn reads_and_writes_of_notes_are_found_per_session_and_agent() {
        let root = scratch("found");
        let memory = root.join("memory");
        let account = root.join("louis");
        let note = format!("{}/projets/app/status/app.md", memory.display());
        let now = now();
        write(&account.join("projects/-app/s1.jsonl"), &[
            tool(now - 100, "Read", serde_json::json!({ "file_path": note })),
            tool(now - 50, "Edit", serde_json::json!({ "file_path": note, "old_string": "a", "new_string": "b" })),
            tool(now - 40, "Read", serde_json::json!({ "file_path": "/elsewhere/x.md" })),
            tool(now - 30, "Read", serde_json::json!({ "file_path": format!("{}/projets/app/notes.txt", memory.display()) })),
        ]);
        write(&account.join("projects/-app/s1/subagents/agent-a1.jsonl"), &[
            tool(now - 20, "Bash", serde_json::json!({ "command": format!("cat {} | head", note) })),
        ]);
        let uses = since(&memory, &[("louis".to_string(), account)], now - 3_600);
        let session = |by_agent: bool| uses.iter().find(|found| found.by_agent == by_agent).cloned();
        assert_eq!(uses.len(), 2, "{uses:?}");
        assert_eq!(session(false), Some(NoteUse {
            path: "projets/app/status/app.md".into(), account: "louis".into(), session: "s1".into(), by_agent: false,
            read_at: Some(now - 100), written_at: Some(now - 50) }));
        let agent = session(true).unwrap();
        assert_eq!((agent.session.as_str(), agent.read_at, agent.written_at), ("s1", Some(now - 20), None));
        assert!(uses[0].by_agent, "newest first");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn what_is_older_than_asked_does_not_count() {
        let root = scratch("old");
        let memory = root.join("memory");
        let account = root.join("louis");
        let note = format!("{}/global/rules.md", memory.display());
        let now = now();
        write(&account.join("projects/-app/s1.jsonl"), &[tool(now - 7_200, "Read", serde_json::json!({ "file_path": note }))]);
        assert!(since(&memory, &[("louis".to_string(), account.clone())], now - 3_600).is_empty());
        assert_eq!(since(&memory, &[("louis".to_string(), account)], now - 8_000).len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_shell_command_reads_a_note_or_writes_it_after_a_redirection() {
        let needles = vec!["/home/u/.kindric/memory/".to_string(), "~/.kindric/memory/".to_string()];
        let used = |command: &str| in_command(command, &needles);
        assert_eq!(used("cat ~/.kindric/memory/projets/a/INDEX.md"), [("projets/a/INDEX.md".to_string(), false)]);
        assert_eq!(used("printf '%s\\n' x >> ~/.kindric/memory/projets/a/INDEX.md; ls"), [("projets/a/INDEX.md".to_string(), true)]);
        assert_eq!(used("echo x | tee -a /home/u/.kindric/memory/global/r.md"), [("global/r.md".to_string(), true)]);
        assert_eq!(used("sed -i 's/a/b/' \"/home/u/.kindric/memory/global/r.md\""), [("global/r.md".to_string(), true)]);
        assert_eq!(used("ls ~/.kindric/memory/projets/ && cat ~/.kindric/memory/x.md:"), [("x.md".to_string(), false)], "a folder is no note");
        assert_eq!(used("cat ~/.kindric/memory/../secret.md"), []);
        assert_eq!(used("head ~/.kindric/memory/projets/a/knowledge/*.md"), [], "a pattern");
    }
}
