//! The user is away: a session keeps working until a time instead of stopping to wait for an
//! approval. `away/<session id>` in kindric's data holds that time (Unix seconds); the Stop hook
//! `kindric away-hook` refuses the end of a turn before it. The session says it is done with
//! `kindric away done <session id>`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use kindric_accounts::files::{create_private_dir, write_atomic};

use crate::mailbox::{data_dir, is_valid_session_id};

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

/// The user is back, or the session is done: it may stop again. Nothing to clear is no error.
pub fn clear(root: &Path, session: &str) -> io::Result<()> {
    match fs::remove_file(file(root, session)?) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
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
         When the whole task is done and verified, run `kindric away done {session}`, then stop.",
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
             When the whole task is done and verified, run `kindric away done {session}`, then stop.",
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

/// What `kindric away-hook` prints for `input`, the JSON Claude Code gives a hook, while the user
/// is away from its session; nothing otherwise, so that Claude Code goes on as usual.
/// - Stop: a refusal to end the turn, with what to do instead.
/// - PreToolUse (shell commands, in every permission mode): a refusal of what waits for the user.
/// - PermissionRequest (a question Claude Code would ask): yes, unless it waits for the user.
pub fn hook_answer(root: &Path, input: &str, now: i64) -> Option<String> {
    let input: serde_json::Value = serde_json::from_str(input).ok()?;
    let event = input.get("hook_event_name")?.as_str()?;
    let (session, until) = away_session(root, &input, now)?;
    let command = || input.get("tool_input")?.get("command")?.as_str();
    let answer = match event {
        "Stop" => {
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
    let Ok(root) = default_root() else {
        return;
    };
    if let Some(answer) = hook_answer(&root, &input, now()) {
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
        let answer = hook_answer(&root, &stop("aaaa-1111"), 1_000).unwrap();
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
        assert_eq!(hook_answer(&root, &stop("aaaa-1111"), 1_000), None);
        assert_eq!(until(&root, "aaaa-1111"), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn other_sessions_and_events_are_left_alone() {
        let root = temp_root("others");
        set(&root, "aaaa-1111", 9_999).unwrap();
        assert_eq!(hook_answer(&root, &stop("bbbb-2222"), 1_000), None);
        let other_event = r#"{"session_id":"aaaa-1111","hook_event_name":"SubagentStop"}"#;
        assert_eq!(hook_answer(&root, other_event, 1_000), None);
        assert_eq!(hook_answer(&root, "not json", 1_000), None);
        assert_eq!(hook_answer(&root, &stop("../aaaa-1111"), 1_000), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn done_lets_the_session_stop() {
        let root = temp_root("done");
        set(&root, "aaaa-1111", 9_999).unwrap();
        clear(&root, "aaaa-1111").unwrap();
        assert_eq!(hook_answer(&root, &stop("aaaa-1111"), 1_000), None);
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
        hook_answer(root, input, 1_000).map(|text| serde_json::from_str(&text).unwrap())
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
            assert_eq!(hook_answer(&root, &tool_event(event, "bbbb-2222", "Bash", "git push"), 1_000), None, "{event}, another session");
            assert_eq!(hook_answer(&root, &tool_event(event, "aaaa-1111", "Bash", "git push"), 10_000), None, "{event}, time over");
        }
        assert_eq!(until(&root, "aaaa-1111"), None, "the time over, the file went");
        fs::remove_dir_all(root).unwrap();
    }
}
