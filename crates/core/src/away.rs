//! The user is away: a session keeps working until a time instead of stopping to wait for an
//! approval. `away/<session id>` in kindric's data holds that time (Unix seconds); the Stop hook
//! `kindric away-hook` refuses the end of a turn before it. The session says it is done with
//! `kindric away done <session id>`, or that nothing is left without the user with
//! `kindric away blocked <session id> <what it needs>`: its need goes to the overview and
//! `away/<session id>.waiting` names that request; until it is answered, the session may stop.
//! A session relaunched by the hook that then runs no tool may stop too, its last words going to
//! the overview the same way: a session left idle must not be relaunched in a loop.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use kindric_accounts::files::{create_private_dir, write_atomic};
use serde_json::Value;

use crate::images::{last_turn_text, tail};
use crate::mailbox::{data_dir, is_valid_session_id};
use crate::overview::{self, Body, Request};
use crate::turn_question::context_of;

/// The extension of the file that names the request a session waits on.
const WAITING: &str = "waiting";

/// How the Stop hook's refusal starts: it marks the hook's relaunch in a transcript.
const RELAUNCH: &str = "The user is away for ";

/// The context of the request `kindric away blocked` puts in the overview.
const BLOCKED_CONTEXT: &str = "It stopped while you were away: nothing is left that it can do without you.";

/// The question put in the overview for a session relaunched by the hook that then did nothing.
const IDLE_QUESTION: &str = "Stopped while you were away: it ran nothing after being told to keep working. What should it do?";

/// What `kindric away blocked` tells the session.
const BLOCKED_SENT: &str = "What you need is in kindric's overview for the user. You may end your turn now: \
the hook no longer refuses it. Their answer will come as a message from them.";

/// `away` in the data directory.
pub fn default_root() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("away"))
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
}

fn file(root: &Path, session: &str) -> io::Result<PathBuf> {
    if !is_valid_session_id(session) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid session id: {session:?}")));
    }
    Ok(root.join(session))
}

/// The session `session` keeps working until `until`.
pub fn set(root: &Path, session: &str, until: i64) -> io::Result<()> {
    let file = file(root, session)?;
    create_private_dir(root)?;
    write_atomic(&file, &until.to_string())
}

fn waiting_file(root: &Path, session: &str) -> io::Result<PathBuf> {
    Ok(file(root, session)?.with_extension(WAITING))
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// The user is back, or the session is done: it may stop again. Nothing to clear is no error.
pub fn clear(root: &Path, session: &str) -> io::Result<()> {
    remove(&file(root, session)?)?;
    remove(&waiting_file(root, session)?)
}

fn is_away(root: &Path, session: &str, now: i64) -> bool {
    until(root, session).is_some_and(|until| until > now)
}

/// Puts `request` in the overview and marks its session as waiting on it.
fn wait_on(root: &Path, overview_root: &Path, request: &Request) -> io::Result<()> {
    overview::ask(overview_root, request)?;
    create_private_dir(root)?;
    write_atomic(&waiting_file(root, &request.session_id)?, &request.id)
}

/// Whether `session` waits on a request of the overview that is not answered yet. Once it is, the
/// mark goes: the session works again on the answer and is kept working.
fn waits_on_the_user(root: &Path, overview_root: &Path, session: &str) -> bool {
    let Ok(path) = waiting_file(root, session) else {
        return false;
    };
    let Ok(id) = fs::read_to_string(&path) else {
        return false;
    };
    let id = id.trim();
    if overview::pending(overview_root).iter().any(|request| request.session_id == session && request.id == id) {
        return true;
    }
    let _ = remove(&path);
    false
}

fn ask_request(session: &str, cwd: &str, now_millis: u128, question: &str, context: &str) -> Request {
    Request {
        id: overview::new_id(now_millis, std::process::id()),
        session_id: session.to_string(),
        cwd: cwd.to_string(),
        asked_at: i64::try_from(now_millis / 1000).unwrap_or(i64::MAX),
        body: Body::Ask { question: question.to_string(), context: context.to_string(), from_reply: false },
    }
}

/// `kindric away blocked`, run by `session` once nothing is left that it can do without the user:
/// `need` goes to the overview and the session may stop until it is answered. Refused while the
/// user is not away: `kindric ask` is for that.
pub fn block(root: &Path, overview_root: &Path, session: &str, cwd: &Path, need: &str, now_millis: u128) -> Result<Request, String> {
    if !is_valid_session_id(session) {
        return Err(format!("invalid session id: {session:?}"));
    }
    let need = need.trim();
    if need.is_empty() {
        return Err("usage: kindric away blocked <session id> \"<what you need from the user>\"".to_string());
    }
    let now = i64::try_from(now_millis / 1000).unwrap_or(i64::MAX);
    if !is_away(root, session, now) {
        return Err("the user is not away from this session: ask them with `kindric ask \"<question>\" \"<context>\"`".to_string());
    }
    let request = ask_request(session, &cwd.display().to_string(), now_millis, need, BLOCKED_CONTEXT);
    wait_on(root, overview_root, &request).map_err(|error| format!("cannot put it in the overview ({}): {error}", overview_root.display()))?;
    Ok(request)
}

/// `kindric away blocked <session id> <need>`, from the folder the session runs in.
pub fn run_blocked(session: &str, need: &str) -> Result<(), String> {
    let root = default_root().map_err(|error| format!("cannot find kindric's data: {error}"))?;
    let overview_root = overview::default_root().map_err(|error| format!("cannot find kindric's data: {error}"))?;
    let cwd = std::env::current_dir().map_err(|error| format!("cannot read the current folder: {error}"))?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    block(&root, &overview_root, session, &cwd, need, now.as_millis())?;
    println!("{BLOCKED_SENT}");
    Ok(())
}

/// The text of a message, whether its content is a string or a list of blocks with text.
fn text_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks.iter().filter_map(|block| block.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Whether a transcript entry is the hook's relaunch: in Claude Code's transcript, the hook's
/// feedback as a user message Claude Code wrote itself (`isMeta`); in Codex's rollout, a user message holding a `<hook_prompt>`.
fn is_relaunch(entry: &Value) -> bool {
    let (text, mark) = if entry.get("type").and_then(Value::as_str) == Some("response_item") {
        let payload = entry.get("payload");
        let user = payload.and_then(|payload| payload.get("role")).and_then(Value::as_str) == Some("user");
        (if user { text_of(payload.and_then(|payload| payload.get("content"))) } else { String::new() }, "<hook_prompt")
    } else if entry.get("type").and_then(Value::as_str) == Some("user") && entry.get("isMeta").and_then(Value::as_bool) == Some(true) {
        (text_of(entry.pointer("/message/content")), "Stop hook feedback:")
    } else {
        return false;
    };
    text.contains(mark) && text.contains(RELAUNCH)
}

/// Whether a transcript entry runs a tool: a `tool_use` block of Claude Code, a `…_call` item of
/// Codex (`function_call`, `local_shell_call`…).
fn runs_a_tool(entry: &Value) -> bool {
    match entry.get("type").and_then(Value::as_str) {
        Some("assistant") => matches!(entry.pointer("/message/content"), Some(Value::Array(blocks))
            if blocks.iter().any(|block| block.get("type").and_then(Value::as_str).is_some_and(|kind| kind.ends_with("tool_use")))),
        Some("response_item") => entry.pointer("/payload/type").and_then(Value::as_str).is_some_and(|kind| kind.ends_with("_call")),
        _ => false,
    }
}

/// Whether, in a transcript's end, the session ran no tool since the hook last relaunched it.
/// False when no relaunch is in it: then nothing says the session is idle.
fn idle_since_relaunch(transcript_tail: &str) -> bool {
    let mut idle = false;
    for entry in transcript_tail.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
        if is_relaunch(&entry) {
            idle = true;
        } else if runs_a_tool(&entry) {
            idle = false;
        }
    }
    idle
}

/// The request for the overview when `input`, a Stop relaunched by a hook, ends a turn in which
/// the session ran no tool since this hook relaunched it; its last words are the context.
fn idle_request(input: &Value, session: &str, now: i64) -> Option<Request> {
    if input.get("stop_hook_active").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let transcript = tail(Path::new(input.get("transcript_path")?.as_str()?))?;
    if !idle_since_relaunch(&transcript) {
        return None;
    }
    let said = input
        .get("last_assistant_message")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map_or_else(|| last_turn_text(&transcript), str::to_string);
    let context = context_of(&said, "");
    let context = if context.is_empty() { "It wrote nothing.".to_string() } else { context };
    let cwd = input.get("cwd").and_then(Value::as_str).unwrap_or_default();
    let now_millis = u128::try_from(now).unwrap_or_default() * 1000;
    Some(ask_request(session, cwd, now_millis, IDLE_QUESTION, &context))
}

/// Until when `session` keeps working, if the user said they are away.
pub fn until(root: &Path, session: &str) -> Option<i64> {
    fs::read_to_string(file(root, session).ok()?).ok()?.trim().parse().ok()
}

/// What the session is told when it tries to stop while the user is away.
fn instruction(session: &str, minutes: i64) -> String {
    format!(
        "The user is away for {minutes} more minute{s} and asked you to keep working without waiting for their approval. \
         Continue the task: when a choice is needed, make it, write it down where the project keeps its decisions, and go on. \
         Allowed alone: edit, test, commit in your own worktree, write notes in memory. \
         Never while they are away: push, merge, delete what you did not create, install, use another account. \
         When the whole task is done and verified, run `kindric away done {session}`, then stop. \
         If what is left needs the user, first have a critical agent review your code and your choices: \
         it lists only what you can do without the user and what changes the result. Do that. \
         Once nothing is left without the user, run `kindric away blocked {session} \"<what you need from them>\"`, then stop.",
        s = if minutes == 1 { "" } else { "s" },
    )
}

/// What the app types into `session` in the user's name when they leave for `minutes`, or come
/// back with none. The session takes it from the user; it does not take the Stop hook's text over
/// what the user asked.
pub fn user_words(session: &str, minutes: Option<u32>) -> String {
    match minutes.filter(|minutes| *minutes > 0) {
        Some(minutes) => format!(
            "I am away for the next {minutes} minute{s}. Keep working on the task without waiting for my approval: \
             when a choice is needed, make it, write it down where the project keeps its decisions, and go on. \
             Allowed: edit, test, commit in your own worktree, write notes in memory. \
             Never: push, merge, delete what you did not create, install, use another account. \
             When the whole task is done and verified, run `kindric away done {session}`, then stop. \
             If what is left needs me, first have a critical agent review your code and your choices: \
             it lists only what you can do without me and what changes the result. Do that. \
             Once nothing is left without me, run `kindric away blocked {session} \"<what you need from me>\"`, then stop.",
            s = if minutes == 1 { "" } else { "s" },
        ),
        None => "I am back: the usual rules apply again, ask before acting.".to_owned(),
    }
}

/// A shell command cut into its simple commands (split on `;`, `&`, `|`, new lines, parentheses
/// and backquotes, so that `(…)`, `$(…)` and `` `…` `` count as commands of their own), each into
/// its words, quotes taken off. Quoted text is one word and never splits: `git commit -m "a; rm b"`
/// is one command. Not a shell: what `sh -c` and `eval` run is read again by `what_waits`.
fn simple_commands(line: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    fn end_word(word: &mut String, in_word: &mut bool, words: &mut Vec<String>) {
        if *in_word {
            words.push(std::mem::take(word));
            *in_word = false;
        }
    }
    for character in line.chars() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => word.push(character),
            (None, '\'' | '"') => {
                quote = Some(character);
                in_word = true;
            },
            (None, ';' | '&' | '|' | '\n' | '(' | ')' | '`') => {
                end_word(&mut word, &mut in_word, &mut words);
                if !words.is_empty() {
                    commands.push(std::mem::take(&mut words));
                }
            },
            (None, c) if c.is_whitespace() => end_word(&mut word, &mut in_word, &mut words),
            (None, c) => {
                word.push(c);
                in_word = true;
            },
        }
    }
    end_word(&mut word, &mut in_word, &mut words);
    if !words.is_empty() {
        commands.push(words);
    }
    commands
}

/// A program that runs the rest of its words as a command: its options that take a value, and
/// how many words it takes before the command (`timeout 60 …`).
struct Wrapper {
    name: &'static str,
    with_value: &'static [&'static str],
    positional: usize,
}

const WRAPPERS: [Wrapper; 15] = [
    Wrapper { name: "sudo", with_value: &["-u", "-g", "-C", "-D", "-h", "-p", "-r", "-t", "-U", "-T"], positional: 0 },
    Wrapper { name: "doas", with_value: &["-u", "-C"], positional: 0 },
    Wrapper { name: "env", with_value: &["-u", "-C"], positional: 0 },
    Wrapper { name: "command", with_value: &[], positional: 0 },
    Wrapper { name: "exec", with_value: &["-a"], positional: 0 },
    Wrapper { name: "nohup", with_value: &[], positional: 0 },
    Wrapper { name: "time", with_value: &["-f", "-o"], positional: 0 },
    Wrapper { name: "nice", with_value: &["-n"], positional: 0 },
    Wrapper { name: "ionice", with_value: &["-c", "-n", "-p"], positional: 0 },
    Wrapper { name: "timeout", with_value: &["-k", "-s"], positional: 1 },
    Wrapper { name: "xargs", with_value: &["-n", "-P", "-I", "-d", "-L", "-s", "-E", "-a"], positional: 0 },
    Wrapper { name: "stdbuf", with_value: &["-i", "-o", "-e"], positional: 0 },
    Wrapper { name: "setsid", with_value: &[], positional: 0 },
    Wrapper { name: "watch", with_value: &["-n", "-d"], positional: 0 },
    Wrapper { name: "flock", with_value: &["-w", "-E"], positional: 1 },
];

/// Shell words that come before a command without being one: `{ …; }`, `! …`, `then …`, `do …`.
const SHELL_WORDS: [&str; 6] = ["{", "}", "!", "then", "do", "else"];

/// The words of a wrapper's command, past the wrapper's options, their values and the words it
/// takes first.
fn past_wrapper<'a>(wrapper: &Wrapper, args: &'a [String]) -> &'a [String] {
    let mut index = 0;
    let mut positional = wrapper.positional;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if arg.len() > 1 && arg.starts_with('-') {
            index += if wrapper.with_value.contains(&arg.as_str()) { 2 } else { 1 };
        } else if positional > 0 {
            positional -= 1;
            index += 1;
        } else {
            break;
        }
    }
    args.get(index..).unwrap_or(&[])
}

/// The program a simple command runs, past wrappers (`sudo`, `timeout 60`, `xargs -n 1`…),
/// shell words (`{`, `!`, `then`) and `VAR=value`, with its arguments.
fn program(words: &[String]) -> Option<(&str, &[String])> {
    let mut rest = words;
    while let Some((first, after)) = rest.split_first() {
        // `VAR=value` before the program: the environment it runs with, the account among it.
        let assignment = first.split_once('=').is_some_and(|(variable, _)| !variable.is_empty() && !variable.contains('/') && !first.starts_with('-'));
        if assignment {
            if first.starts_with("CLAUDE_CONFIG_DIR=") {
                return Some(("CLAUDE_CONFIG_DIR", after));
            }
            rest = after;
        } else if SHELL_WORDS.contains(&first.as_str()) {
            rest = after;
        } else {
            let name = first.rsplit('/').next().unwrap_or(first);
            match WRAPPERS.iter().find(|wrapper| wrapper.name == name) {
                Some(wrapper) => rest = past_wrapper(wrapper, after),
                None => return Some((name, after)),
            }
        }
    }
    None
}

/// git's subcommand and its arguments: the first word that is not an option, past `-C <dir>`
/// and `-c <setting>`.
fn git_subcommand(args: &[String]) -> Option<(&str, &[String])> {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "-C" | "-c" => index += 2,
            option if option.starts_with('-') => index += 1,
            sub => return Some((sub, args.get(index + 1..).unwrap_or(&[]))),
        }
    }
    None
}

/// What a git command does that waits for the user, if anything. `pull` merges what it fetched;
/// `reset --hard`, `stash drop`, `restore` throw away work not committed.
fn git_waits(sub: &str, args: &[String]) -> Option<&'static str> {
    let has = |word: &str| args.iter().any(|arg| arg == word);
    let first = args.iter().find(|arg| !arg.starts_with('-')).map(String::as_str);
    match sub {
        "push" => Some("pushing"),
        "merge" | "pull" => Some("merging"),
        "rm" | "clean" => Some("deleting"),
        "branch" if has("-d") || has("-D") || has("--delete") => Some("deleting"),
        "worktree" if first == Some("remove") => Some("deleting"),
        "reset" if has("--hard") => Some("deleting"),
        "stash" if matches!(first, Some("drop" | "clear")) => Some("deleting"),
        // Only the index is put back with --staged alone: the files stay as they are.
        "restore" if !((has("--staged") || has("-S")) && !has("--worktree") && !has("-W")) => Some("deleting"),
        _ => None,
    }
}

/// The program `find -exec`, `-execdir` or `-ok` runs.
fn find_runs(args: &[String]) -> impl Iterator<Item = &str> {
    args.windows(2)
        .filter(|pair| matches!(pair[0].as_str(), "-exec" | "-execdir" | "-ok" | "-okdir"))
        .map(|pair| pair[1].rsplit('/').next().unwrap_or(&pair[1]))
}

/// Shells read again what they are given to run, this deep at most.
const SHELL_DEPTH: usize = 3;

/// What `sh -c '…'` (or bash, zsh, dash) runs: the first word after an option made of letters
/// that holds `c` (`-c`, `-lc`, `-ec`).
fn shell_script(args: &[String]) -> Option<&str> {
    let at = args.iter().position(|arg| arg.starts_with('-') && !arg.starts_with("--") && arg[1..].chars().all(|c| c.is_ascii_alphabetic()) && arg.contains('c'))?;
    args.get(at + 1..)?.iter().find(|arg| !arg.starts_with('-')).map(String::as_str)
}

/// What waits for the user's return in a simple command, if anything: pushing, merging, deleting,
/// installing, using another account (decision 249). `depth` counts the shells already read into.
fn waits_for_the_user(words: &[String], depth: usize) -> Option<&'static str> {
    let (name, args) = program(words)?;
    let has = |word: &str| args.iter().any(|arg| arg == word);
    let first = args.iter().find(|arg| !arg.starts_with('-')).map(String::as_str);
    match name {
        "CLAUDE_CONFIG_DIR" | "kindric-accounts" | "claude-accounts" => Some("using another account"),
        "export" | "declare" | "typeset" | "readonly" | "local" if args.iter().any(|arg| arg.starts_with("CLAUDE_CONFIG_DIR=")) => {
            Some("using another account")
        },
        "sh" | "bash" | "zsh" | "dash" | "ksh" if depth < SHELL_DEPTH => what_waits(shell_script(args)?, depth + 1),
        "eval" if depth < SHELL_DEPTH => what_waits(&args.join(" "), depth + 1),
        "git" => {
            let (sub, rest) = git_subcommand(args)?;
            git_waits(sub, rest)
        },
        "gh" if has("merge") => Some("merging"),
        "rm" | "rmdir" | "unlink" | "shred" | "truncate" => Some("deleting"),
        "find" if has("-delete") || find_runs(args).any(|run| matches!(run, "rm" | "rmdir" | "unlink" | "shred")) => Some("deleting"),
        "rsync" if args.iter().any(|arg| arg.starts_with("--delete") || arg.starts_with("--remove-source-files")) => Some("deleting"),
        "cargo" if matches!(first, Some("install" | "add")) => Some("installing"),
        "pipx" | "gem" if first == Some("install") => Some("installing"),
        "go" if matches!(first, Some("install" | "get")) => Some("installing"),
        "npm" | "pnpm" if matches!(first, Some("install" | "i" | "add" | "ci")) => Some("installing"),
        "yarn" if matches!(first, Some("add" | "global")) => Some("installing"),
        "bun" | "deno" if matches!(first, Some("add" | "install" | "i")) => Some("installing"),
        "pip" | "pip3" | "apt" | "apt-get" | "dnf" | "brew" | "mise" if matches!(first, Some("install" | "use")) => Some("installing"),
        python if python.starts_with("python") && args.windows(2).any(|pair| pair[0] == "-m" && pair[1].starts_with("pip")) && has("install") => {
            Some("installing")
        },
        "uv" if args.iter().any(|arg| arg == "install" || arg == "add") => Some("installing"),
        "pacman" | "yay" | "paru" if args.iter().any(|arg| arg.starts_with("-S")) => Some("installing"),
        "omarchy" if has("pkg") && (has("add") || has("install")) => Some("installing"),
        "rustup" if has("add") || has("install") => Some("installing"),
        _ => None,
    }
}

/// What waits for the user's return in a shell command, from the first of its simple commands that does.
fn what_waits(command: &str, depth: usize) -> Option<&'static str> {
    simple_commands(command).iter().find_map(|words| waits_for_the_user(words, depth))
}

/// Why `command` waits for the user's return, if one of its simple commands does.
fn refusal(command: &str) -> Option<String> {
    let what = what_waits(command, 0)?;
    Some(format!(
        "Refused while the user is away: {what} waits for their return. Write down what you would have done, then go on with the rest of the task."
    ))
}

/// The session `input` speaks for, while the user is away from it; once the time has passed, the
/// file goes and the session is no longer away.
fn away_session<'a>(root: &Path, input: &'a serde_json::Value, now: i64) -> Option<(&'a str, i64)> {
    let session = input.get("session_id")?.as_str()?;
    let until = until(root, session)?;
    if until <= now {
        let _ = clear(root, session);
        return None;
    }
    Some((session, until))
}

/// What `kindric away-hook` prints for `input`, the JSON Claude Code (or Codex) gives a hook,
/// while the user is away from its session; nothing otherwise, so that the session goes on as
/// usual. `overview_root` holds the requests for the user.
/// - Stop: a refusal to end the turn, with what to do instead. Not while the session waits on a
///   request of `kindric away blocked`, nor when it ran no tool since the last refusal: that turn
///   goes to the overview and the session stops.
/// - PreToolUse (shell commands, in every permission mode): a refusal of what waits for the user.
/// - PermissionRequest (a question Claude Code would ask): yes, unless it waits for the user.
pub fn hook_answer(root: &Path, overview_root: &Path, input: &str, now: i64) -> Option<String> {
    let input: serde_json::Value = serde_json::from_str(input).ok()?;
    let event = input.get("hook_event_name")?.as_str()?;
    let (session, until) = away_session(root, &input, now)?;
    let command = || input.get("tool_input")?.get("command")?.as_str();
    let answer = match event {
        "Stop" => {
            if waits_on_the_user(root, overview_root, session) {
                return None;
            }
            if let Some(request) = idle_request(&input, session, now) {
                match wait_on(root, overview_root, &request) {
                    Ok(()) => return None,
                    Err(error) => eprintln!("kindric away-hook: the idle turn was not put in the overview ({}): {error}", overview_root.display()),
                }
            }
            // Rounded up: with 30 seconds left the session is told 1 minute, not 0.
            let minutes = (until - now + 59) / 60;
            serde_json::json!({ "decision": "block", "reason": instruction(session, minutes) })
        },
        "PreToolUse" => {
            let reason = refusal(command()?)?;
            serde_json::json!({ "hookSpecificOutput": {
                "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason } })
        },
        "PermissionRequest" => {
            let decision = match command().and_then(refusal) {
                Some(reason) => serde_json::json!({ "behavior": "deny", "message": reason }),
                None => serde_json::json!({ "behavior": "allow" }),
            };
            serde_json::json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } })
        },
        _ => return None,
    };
    Some(answer.to_string())
}

/// `kindric away-hook`: reads Claude Code's input and prints the answer. Nothing that goes wrong
/// may disturb the session: it prints nothing then.
pub fn run_away_hook() {
    let mut input = String::new();
    if io::Read::read_to_string(&mut io::stdin(), &mut input).is_err() {
        return;
    }
    let (Ok(root), Ok(overview_root)) = (default_root(), overview::default_root()) else {
        return;
    };
    if let Some(answer) = hook_answer(&root, &overview_root, &input, now()) {
        println!("{answer}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-away-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn stop(session: &str) -> String {
        format!(r#"{{"session_id":"{session}","hook_event_name":"Stop","stop_hook_active":false}}"#)
    }

    #[test]
    fn stopping_is_refused_until_the_time() {
        let root = temp_root("refused");
        set(&root, "aaaa-1111", 1_000 + 42 * 60 - 30).unwrap();
        let answer = hook_answer(&root, &root.join("overview"), &stop("aaaa-1111"), 1_000).unwrap();
        let value: serde_json::Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(value["decision"], "block");
        let reason = value["reason"].as_str().unwrap();
        assert!(reason.starts_with("The user is away for 42 more minutes"), "{reason}");
        assert!(reason.contains("`kindric away done aaaa-1111`"), "{reason}");
        assert!(reason.contains("Never while they are away: push, merge"), "{reason}");
        assert_eq!(until(&root, "aaaa-1111"), Some(1_000 + 42 * 60 - 30), "still away");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn after_the_time_the_session_stops_and_the_file_goes() {
        let root = temp_root("over");
        set(&root, "aaaa-1111", 1_000).unwrap();
        assert_eq!(hook_answer(&root, &root.join("overview"), &stop("aaaa-1111"), 1_000), None);
        assert_eq!(until(&root, "aaaa-1111"), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn other_sessions_and_events_are_left_alone() {
        let root = temp_root("others");
        set(&root, "aaaa-1111", 9_999).unwrap();
        assert_eq!(hook_answer(&root, &root.join("overview"), &stop("bbbb-2222"), 1_000), None);
        let other_event = r#"{"session_id":"aaaa-1111","hook_event_name":"SubagentStop"}"#;
        assert_eq!(hook_answer(&root, &root.join("overview"), other_event, 1_000), None);
        assert_eq!(hook_answer(&root, &root.join("overview"), "not json", 1_000), None);
        assert_eq!(hook_answer(&root, &root.join("overview"), &stop("../aaaa-1111"), 1_000), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn done_lets_the_session_stop() {
        let root = temp_root("done");
        set(&root, "aaaa-1111", 9_999).unwrap();
        clear(&root, "aaaa-1111").unwrap();
        assert_eq!(hook_answer(&root, &root.join("overview"), &stop("aaaa-1111"), 1_000), None);
        clear(&root, "aaaa-1111").unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_malformed_id_is_refused() {
        let root = temp_root("malformed");
        assert!(set(&root, "../x", 9_999).is_err());
        assert!(clear(&root, "a/b").is_err());
        assert!(!root.exists());
    }

    #[test]
    fn one_minute_left_is_said_in_the_singular() {
        assert!(instruction("aaaa-1111", 1).starts_with("The user is away for 1 more minute and"));
    }

    #[test]
    fn leaving_tells_the_session_how_long_what_it_may_do_and_how_to_finish() {
        let words = user_words("aaaa-1111", Some(60));
        assert!(words.starts_with("I am away for the next 60 minutes."), "{words}");
        assert!(words.contains("without waiting for my approval"), "{words}");
        assert!(words.contains("Allowed: edit, test, commit in your own worktree"), "{words}");
        assert!(words.contains("Never: push, merge, delete what you did not create, install"), "{words}");
        assert!(words.contains("`kindric away done aaaa-1111`"), "{words}");
        assert!(!words.contains('\n'), "one line, sent with one Enter");
        assert!(user_words("aaaa-1111", Some(1)).starts_with("I am away for the next 1 minute."));
    }

    #[test]
    fn coming_back_brings_the_usual_rules_back() {
        let back = "I am back: the usual rules apply again, ask before acting.";
        assert_eq!(user_words("aaaa-1111", None), back);
        assert_eq!(user_words("aaaa-1111", Some(0)), back, "0 minutes is back");
    }

    fn tool_event(event: &str, session: &str, tool: &str, command: &str) -> String {
        serde_json::json!({ "session_id": session, "hook_event_name": event, "tool_name": tool, "tool_input": { "command": command } })
            .to_string()
    }

    fn answer_of(root: &Path, input: &str) -> Option<serde_json::Value> {
        hook_answer(root, &root.join("overview"), input, 1_000).map(|text| serde_json::from_str(&text).unwrap())
    }

    #[test]
    fn what_waits_for_the_user_is_found_in_a_command() {
        let cases = [
            ("git push", "pushing"),
            ("git -C ../work push origin main", "pushing"),
            ("cd crates && cargo test && git push", "pushing"),
            ("git merge main", "merging"),
            ("gh pr merge 12 --squash", "merging"),
            ("rm -rf target", "deleting"),
            ("sudo rm /etc/x", "deleting"),
            ("/usr/bin/rm notes.md", "deleting"),
            ("find . -name '*.o' -delete", "deleting"),
            ("git branch -D old", "deleting"),
            ("git clean -fdx", "deleting"),
            ("cargo install ripgrep", "installing"),
            ("npm i -g typescript", "installing"),
            ("pip install requests", "installing"),
            ("uv pip install requests", "installing"),
            ("sudo pacman -S jq", "installing"),
            ("omarchy pkg add jq", "installing"),
            ("kindric-accounts run pro claude -p hello", "using another account"),
            ("CLAUDE_CONFIG_DIR=/home/me/pro claude agents", "using another account"),
            ("FOO=1 git push", "pushing"),
        ];
        for (command, what) in cases {
            let reason = refusal(command).unwrap_or_else(|| panic!("{command} let through"));
            assert!(reason.contains(&format!("{what} waits for their return")), "{command}: {reason}");
        }
    }

    /// Ways an agent writes these commands without meaning to get round anything (audit of
    /// 2026-11-14, decision 319).
    #[test]
    fn what_waits_is_found_inside_groups_wrappers_and_shells() {
        let cases = [
            ("(git push)", "pushing"),
            ("{ git push; }", "pushing"),
            ("! git push", "pushing"),
            ("nice -n 10 git push", "pushing"),
            ("timeout 60 git push", "pushing"),
            ("timeout -k 5 60s git push origin", "pushing"),
            ("sudo -u louis rm notes.md", "deleting"),
            ("doas rm notes.md", "deleting"),
            ("ls | xargs rm", "deleting"),
            ("ls | xargs -n 1 rm -f", "deleting"),
            ("bash -c \"git push\"", "pushing"),
            ("sh -c 'cd x && rm -rf y'", "deleting"),
            ("eval git push", "pushing"),
            ("export CLAUDE_CONFIG_DIR=/home/me/pro; claude -p hi", "using another account"),
            ("python -m pip install requests", "installing"),
            ("python3 -m pip install --user requests", "installing"),
            ("find . -name '*.o' -exec rm {} +", "deleting"),
            ("rsync -a --delete src/ dst/", "deleting"),
            ("git pull", "merging"),
            ("git reset --hard origin/main", "deleting"),
            ("git stash drop", "deleting"),
            ("git stash clear", "deleting"),
            ("git restore src/lib.rs", "deleting"),
            ("cargo add serde", "installing"),
            ("npm ci", "installing"),
            ("go get example.com/x", "installing"),
            ("bun add zod", "installing"),
        ];
        for (command, what) in cases {
            let reason = refusal(command).unwrap_or_else(|| panic!("{command} let through"));
            assert!(reason.contains(&format!("{what} waits for their return")), "{command}: {reason}");
        }
    }

    #[test]
    fn ordinary_work_goes_through_wrappers_and_shells_too() {
        let cases = [
            "timeout 60 cargo test",
            "nice -n 10 cargo build --release",
            "bash -c \"cargo test --workspace\"",
            "(cd crates/core && cargo test)",
            "python -m pytest -q",
            "export PATH=/opt/bin:$PATH",
            "find . -name '*.rs' -exec grep -n unwrap {} +",
            "git stash push -m wip",
            "git restore --staged src/lib.rs",
            "git reset HEAD~1",
            "ls | xargs wc -l",
        ];
        for command in cases {
            assert_eq!(refusal(command), None, "{command}");
        }
    }

    #[test]
    fn ordinary_work_goes_through() {
        let cases = [
            "cargo test --workspace",
            "cargo build --release",
            "git status",
            "git commit -m \"remove the rm; git push later\"",
            "git log --grep push",
            "git diff main",
            "echo git push",
            "npm test",
            "npm run build",
            "pip list",
            "ls -la && grep -rn merge src",
            "node --test ui/atlas.test.js",
        ];
        for command in cases {
            assert_eq!(refusal(command), None, "{command}");
        }
    }

    #[test]
    fn while_away_a_command_that_waits_is_refused_before_it_runs() {
        let root = temp_root("pre-tool");
        set(&root, "aaaa-1111", 9_999).unwrap();
        let refused = answer_of(&root, &tool_event("PreToolUse", "aaaa-1111", "Bash", "git push")).unwrap();
        assert_eq!(refused["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(refused["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(refused["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().contains("pushing waits"));
        assert_eq!(answer_of(&root, &tool_event("PreToolUse", "aaaa-1111", "Bash", "cargo test")), None, "the usual way");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn while_away_a_question_is_answered_yes_unless_the_command_waits() {
        let root = temp_root("permission");
        set(&root, "aaaa-1111", 9_999).unwrap();
        let yes = answer_of(&root, &tool_event("PermissionRequest", "aaaa-1111", "Write", "")).unwrap();
        assert_eq!(yes["hookSpecificOutput"]["hookEventName"], "PermissionRequest");
        assert_eq!(yes["hookSpecificOutput"]["decision"]["behavior"], "allow");
        let shell = answer_of(&root, &tool_event("PermissionRequest", "aaaa-1111", "Bash", "touch b.txt")).unwrap();
        assert_eq!(shell["hookSpecificOutput"]["decision"]["behavior"], "allow");
        let no = answer_of(&root, &tool_event("PermissionRequest", "aaaa-1111", "Bash", "cargo install x")).unwrap();
        assert_eq!(no["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert!(no["hookSpecificOutput"]["decision"]["message"].as_str().unwrap().contains("installing waits"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn when_the_user_is_there_claude_code_decides_and_asks_as_usual() {
        let root = temp_root("present");
        set(&root, "aaaa-1111", 9_999).unwrap();
        for event in ["PreToolUse", "PermissionRequest"] {
            assert_eq!(hook_answer(&root, &root.join("overview"), &tool_event(event, "bbbb-2222", "Bash", "git push"), 1_000), None, "{event}, another session");
            assert_eq!(hook_answer(&root, &root.join("overview"), &tool_event(event, "aaaa-1111", "Bash", "git push"), 10_000), None, "{event}, time over");
        }
        assert_eq!(until(&root, "aaaa-1111"), None, "the time over, the file went");
        fs::remove_dir_all(root).unwrap();
    }

    // Entries shaped as in real files: Claude Code's transcript of session f78a25d0 and the
    // rollout of codex 0.171.0 on the bench (2026-11-21).
    const CLAUDE_RELAUNCH: &str = r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"Stop hook feedback:\nThe user is away for 12 more minutes and asked you to keep working."}}"#;
    const CLAUDE_TEXT: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Nothing new: I still need the user."}]}}"#;
    const CLAUDE_TOOL: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}"#;
    const CODEX_RELAUNCH: &str = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<hook_prompt hook_run_id=\"stop:5:/x/hooks.json\">The user is away for 12 more minutes and asked you to keep working.</hook_prompt>"}]}}"#;
    const CODEX_CALL: &str = r#"{"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{}","call_id":"c1"}}"#;
    const CODEX_TEXT: &str = r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}}"#;

    /// A Stop relaunched by a hook, for a transcript of `lines` written in `root`; `extra` adds
    /// or replaces fields.
    fn relaunched_stop(root: &Path, session: &str, lines: &[&str], extra: serde_json::Value) -> String {
        create_private_dir(root).unwrap();
        let transcript = root.join(format!("{session}.jsonl"));
        fs::write(&transcript, lines.join("\n") + "\n").unwrap();
        let mut input = serde_json::json!({ "session_id": session, "hook_event_name": "Stop", "stop_hook_active": true,
            "transcript_path": transcript, "cwd": "/work" });
        if let (Some(input), serde_json::Value::Object(extra)) = (input.as_object_mut(), extra) {
            input.extend(extra);
        }
        input.to_string()
    }

    fn blocks(root: &Path, input: &str) -> bool {
        answer_of(root, input).is_some_and(|value| value["decision"] == "block")
    }

    #[test]
    fn a_relaunched_session_that_runs_no_tool_stops_and_its_last_words_go_to_the_overview() {
        let root = temp_root("idle");
        let overview_root = root.join("overview");
        set(&root, "aaaa-1111", 9_999).unwrap();
        let input = relaunched_stop(&root, "aaaa-1111", &[CLAUDE_TOOL, CLAUDE_RELAUNCH, CLAUDE_TEXT], serde_json::json!({}));
        assert_eq!(hook_answer(&root, &overview_root, &input, 1_000), None, "it may stop");
        let pending = overview::pending(&overview_root);
        assert_eq!(pending.len(), 1, "{pending:?}");
        assert_eq!(pending[0].session_id, "aaaa-1111");
        assert_eq!(pending[0].cwd, "/work");
        let context = "Nothing new: I still need the user.".to_string();
        assert_eq!(pending[0].body, Body::Ask { question: IDLE_QUESTION.into(), context, from_reply: false });
        assert_eq!(hook_answer(&root, &overview_root, &input, 1_001), None, "still waiting on it");
        assert_eq!(overview::pending(&overview_root).len(), 1, "asked once");
        assert_eq!(until(&root, "aaaa-1111"), Some(9_999), "still away: its commands are still checked");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_relaunched_session_that_ran_a_tool_is_kept_working() {
        let root = temp_root("busy");
        set(&root, "aaaa-1111", 9_999).unwrap();
        assert!(blocks(&root, &relaunched_stop(&root, "aaaa-1111", &[CLAUDE_RELAUNCH, CLAUDE_TOOL, CLAUDE_TEXT], serde_json::json!({}))));
        assert!(blocks(&root, &relaunched_stop(&root, "aaaa-1111", &[CODEX_RELAUNCH, CODEX_CALL, CODEX_TEXT], serde_json::json!({}))));
        assert!(overview::pending(&root.join("overview")).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn without_a_relaunch_to_see_the_session_is_kept_working() {
        let root = temp_root("unseen");
        set(&root, "aaaa-1111", 9_999).unwrap();
        let first = relaunched_stop(&root, "aaaa-1111", &[CLAUDE_RELAUNCH, CLAUDE_TEXT], serde_json::json!({ "stop_hook_active": false }));
        assert!(blocks(&root, &first), "the first stop of a turn");
        assert!(blocks(&root, &relaunched_stop(&root, "aaaa-1111", &[CLAUDE_TEXT], serde_json::json!({}))), "no relaunch in it");
        let gone = relaunched_stop(&root, "aaaa-1111", &[], serde_json::json!({ "transcript_path": root.join("none.jsonl") }));
        assert!(blocks(&root, &gone), "no transcript");
        assert!(blocks(&root, &relaunched_stop(&root, "aaaa-1111", &[], serde_json::json!({ "transcript_path": null }))));
        // A message of the user that quotes the hook is not its relaunch.
        let quoted = r#"{"type":"user","message":{"role":"user","content":"Stop hook feedback:\nThe user is away for 3 more minutes"}}"#;
        assert!(blocks(&root, &relaunched_stop(&root, "aaaa-1111", &[quoted, CLAUDE_TEXT], serde_json::json!({}))));
        assert!(overview::pending(&root.join("overview")).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_idle_codex_session_stops_with_the_last_message_codex_gives() {
        let root = temp_root("idle-codex");
        set(&root, "01a1ff2b-df33", 9_999).unwrap();
        let said = serde_json::json!({ "last_assistant_message": "I need the token." });
        let input = relaunched_stop(&root, "01a1ff2b-df33", &[CODEX_CALL, CODEX_RELAUNCH, CODEX_TEXT], said);
        assert_eq!(hook_answer(&root, &root.join("overview"), &input, 1_000), None);
        let pending = overview::pending(&root.join("overview"));
        assert!(matches!(&pending[0].body, Body::Ask { context, .. } if context == "I need the token."), "{pending:?}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn blocked_lets_the_session_stop_until_the_user_answers() {
        let root = temp_root("blocked");
        let overview_root = root.join("overview");
        set(&root, "aaaa-1111", 9_999).unwrap();
        let request = block(&root, &overview_root, "aaaa-1111", Path::new("/work"), " the API key of the bench ", 1_000_000).unwrap();
        let body = Body::Ask { question: "the API key of the bench".into(), context: BLOCKED_CONTEXT.into(), from_reply: false };
        assert_eq!(request.body, body);
        assert_eq!(overview::pending(&overview_root), vec![request.clone()]);
        assert_eq!(hook_answer(&root, &overview_root, &stop("aaaa-1111"), 1_000), None, "it may stop");
        assert!(answer_of(&root, &tool_event("PreToolUse", "aaaa-1111", "Bash", "git push")).is_some(), "still away");
        overview::settle(&overview_root, "aaaa-1111", &request.id).unwrap();
        assert!(blocks(&root, &stop("aaaa-1111")), "answered: it works on the answer");
        assert!(!waiting_file(&root, "aaaa-1111").unwrap().exists(), "the mark went");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn blocked_is_refused_without_absence_or_need() {
        let root = temp_root("blocked-refused");
        let overview_root = root.join("overview");
        let present = block(&root, &overview_root, "aaaa-1111", Path::new("/w"), "a key", 1_000_000).unwrap_err();
        assert!(present.contains("kindric ask"), "{present}");
        set(&root, "aaaa-1111", 9_999).unwrap();
        assert!(block(&root, &overview_root, "aaaa-1111", Path::new("/w"), "  ", 1_000_000).unwrap_err().starts_with("usage:"));
        assert!(block(&root, &overview_root, "a/b", Path::new("/w"), "a key", 1_000_000).is_err());
        assert!(overview::pending(&overview_root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn coming_back_clears_the_mark_of_a_blocked_session() {
        let root = temp_root("blocked-back");
        set(&root, "aaaa-1111", 9_999).unwrap();
        block(&root, &root.join("overview"), "aaaa-1111", Path::new("/w"), "a key", 1_000_000).unwrap();
        clear(&root, "aaaa-1111").unwrap();
        assert!(!waiting_file(&root, "aaaa-1111").unwrap().exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn both_texts_say_how_to_stop_when_only_the_user_can_unblock() {
        for text in [instruction("aaaa-1111", 5), user_words("aaaa-1111", Some(5))] {
            assert!(text.contains("critical agent review your code and your choices"), "{text}");
            assert!(text.contains(r#"`kindric away blocked aaaa-1111 "<what you need from"#), "{text}");
            assert!(!text.contains('\n'), "one line");
        }
        assert!(instruction("aaaa-1111", 5).starts_with(RELAUNCH), "the hook finds its relaunch by it");
    }
}
