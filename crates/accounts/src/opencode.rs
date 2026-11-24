//! OpenCode accounts (since 2.4.0): finding `opencode`, the directories it runs with, its
//! service and its login.
//!
//! OpenCode keeps everything under the four XDG base directories, so an account's directory holds
//! `config`, `data`, `state` and `cache`, and `opencode` runs with `XDG_*_HOME` pointing at them;
//! `HOME` stays the user's. The commands of its sessions would inherit those variables too (git
//! would lose `~/.config/git/config`): the account's `opencode.json` names `kindric-shell` as the
//! shell, which gives them the user's own back. Its sessions live in a background service, one per
//! account, on 127.0.0.1 and a port of its own: every service takes 49374 unless told otherwise.
//! Read on OpenCode 2.3.2 (2026-11-22).

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::agent::{self, Agent};
use crate::error::{Error, Result, fail};
use crate::files;
use crate::launch;
use crate::paths::Paths;
use crate::registry;
use crate::shared;
use crate::usage;

/// Removed before starting OpenCode. A key exported in the shell must never override the login
/// of the account the user picked, nor bill another one. The variables of a Claude Code or Codex
/// session that runs kindric-accounts must not reach the service, which keeps the environment it
/// starts with and hands it to every command of every session: `kindric ask` would then speak for
/// the wrong session.
pub const STRIPPED_ENV: [&str; 7] =
    ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "OPENCODE_API_KEY", "CLAUDE_CONFIG_DIR", "CLAUDE_JOB_DIR", "CODEX_HOME", "CODEX_THREAD_ID"];

/// Every variable of Claude Code's own (`CLAUDE_CODE_SESSION_ID`…) goes too.
const STRIPPED_PREFIX: &str = "CLAUDE_CODE_";

/// The XDG variables OpenCode reads, and the folder of the account each points at.
pub const XDG: [(&str, &str); 4] =
    [("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"), ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache")];

/// Where the user's own value of an XDG variable travels to `kindric-shell`.
const USER_PREFIX: &str = "KINDRIC_USER_";

/// The shell OpenCode runs the commands of the account's sessions with.
pub const SHELL_FILE: &str = "kindric-shell";

/// The first line after the shebang of `kindric-shell`: a file without it is not ours.
const SHELL_MARK: &str = "# Written by kindric-accounts";

/// The port of OpenCode's own service is 49374; accounts take the next ones.
pub const FIRST_PORT: u16 = 49375;

/// What an OpenCode account shares with the user's own OpenCode configuration: its instructions,
/// agents and commands, under both names OpenCode reads (checked on 2.3.2). Not `opencode.json`,
/// which holds the account's shell, nor skills, which OpenCode does not read from there.
pub const SHARED_ENTRIES: [&str; 5] = ["AGENTS.md", "agents", "agent", "commands", "command"];

#[cfg(windows)]
const OPENCODE_NAMES: [&str; 3] = ["opencode.exe", "opencode.cmd", "opencode"];
#[cfg(not(windows))]
const OPENCODE_NAMES: [&str; 1] = ["opencode"];

pub fn find_opencode_in(path: &OsString) -> Option<PathBuf> {
    launch::find_in(path, &OPENCODE_NAMES)
}

pub fn require_opencode() -> Result<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    match find_opencode_in(&path) {
        Some(opencode) => Ok(opencode),
        None => fail("opencode not found in PATH: install OpenCode, then check that `opencode --version` works here"),
    }
}

/// The shell that gives commands the user's XDG directories back is a POSIX shell script, and
/// where OpenCode keeps its files on Windows is not checked yet.
pub fn check_supported() -> Result<()> {
    if cfg!(windows) {
        return fail("OpenCode accounts are not available on Windows yet");
    }
    Ok(())
}

/// `opencode` for the account in `dir`, from the environment of this process.
pub fn command(opencode: &Path, dir: &Path) -> Command {
    command_from(opencode, dir, std::env::vars_os())
}

/// `opencode` for the account in `dir`, from the environment `env`: the account's XDG
/// directories, the user's own kept for `kindric-shell`, the keys and session variables removed.
fn command_from(opencode: &Path, dir: &Path, env: impl Iterator<Item = (OsString, OsString)>) -> Command {
    let mut command = Command::new(opencode);
    for (name, value) in env {
        let Some(text) = name.to_str() else {
            continue;
        };
        if STRIPPED_ENV.contains(&text) || text.starts_with(STRIPPED_PREFIX) || text.starts_with(USER_PREFIX) {
            command.env_remove(&name);
        }
        if XDG.iter().any(|(variable, _)| *variable == text) {
            command.env(format!("{USER_PREFIX}{text}"), &value);
        }
    }
    for (variable, folder) in XDG {
        command.env(variable, dir.join(folder));
    }
    command
}

/// The text of `kindric-shell`: each XDG variable back to the user's value, or unset when they had
/// none, then the user's shell with what OpenCode gave (`-c <command>`).
fn shell_script() -> String {
    let mut script = format!(
        "#!/bin/sh\n{SHELL_MARK}: OpenCode runs the commands of this account's sessions with it.\n\
         # It gives them the user's own XDG directories back, then runs the user's shell.\n"
    );
    for (variable, _) in XDG {
        script.push_str(&format!(
            "if [ -n \"${{{USER_PREFIX}{variable}+set}}\" ]; then export {variable}=\"${USER_PREFIX}{variable}\"; else unset {variable}; fi\n\
             unset {USER_PREFIX}{variable}\n"
        ));
    }
    script.push_str("exec \"${SHELL:-/bin/sh}\" \"$@\"\n");
    script
}

pub fn shell_file(dir: &Path) -> PathBuf {
    dir.join(SHELL_FILE)
}

/// Writes `kindric-shell`, executable by the user alone. One that is not kindric-accounts' own is
/// left as it is, with a warning.
fn write_shell(dir: &Path) -> Result<Option<String>> {
    let file = shell_file(dir);
    let script = shell_script();
    match fs::read_to_string(&file) {
        Ok(existing) if existing == script => return Ok(None),
        Ok(existing) if !existing.lines().nth(1).is_some_and(|line| line.starts_with(SHELL_MARK)) => {
            return Ok(Some(format!("{} was not written by kindric-accounts and is left as it is", file.display())));
        },
        Ok(_) => {},
        Err(error) if error.kind() == io::ErrorKind::NotFound => {},
        Err(error) => return Err(Error(format!("cannot read {}: {error}", file.display()))),
    }
    files::write_atomic(&file, &script).map_err(|error| Error(format!("cannot write {}: {error}", file.display())))?;
    make_executable(&file)?;
    Ok(None)
}

#[cfg(unix)]
fn make_executable(file: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(file, fs::Permissions::from_mode(0o700)).map_err(|error| Error(format!("cannot make {} executable: {error}", file.display())))
}

#[cfg(not(unix))]
fn make_executable(_file: &Path) -> Result<()> {
    Ok(())
}

/// OpenCode's configuration folder of the account.
pub fn config_dir(dir: &Path) -> PathBuf {
    dir.join("config").join("opencode")
}

/// The name of kindric's MCP server in the account's configuration.
const MESSAGES_SERVER: &str = "kindric";

/// `text` as one word of `sh`, fish or zsh, between single quotes.
fn shell_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// kindric's MCP server for the account `name` in `dir`. OpenCode runs one for all the sessions of
/// its service, which names the session in each call, and gives it its own environment, whose XDG
/// directories are the account's: `kindric-shell` gives the user's back first, so that kindric
/// finds its data.
fn mcp_server(dir: &Path, kindric: &Path, name: &str) -> Value {
    let run = format!("exec {} mcp --account {}", shell_word(&kindric.to_string_lossy()), shell_word(name));
    serde_json::json!({ "type": "local", "command": [shell_file(dir), "-c", run], "enabled": true })
}

/// The configuration kindric-accounts writes: `kindric-shell`, and kindric's MCP server when given.
fn kindric_config(dir: &Path, server: Option<&Value>) -> Value {
    let mut config = serde_json::json!({ "$schema": "https://opencode.ai/config.json", "shell": shell_file(dir) });
    if let Some(server) = server {
        config["mcp"] = serde_json::json!({ MESSAGES_SERVER: server });
    }
    config
}

/// A configuration kindric-accounts wrote, with or without its MCP server, whichever one: nothing
/// in it comes from the user.
fn is_kindrics(config: &Value, dir: &Path) -> bool {
    let Some(entries) = config.as_object() else {
        return false;
    };
    let ours = |key: &str| match key {
        "$schema" => true,
        "shell" => config.get("shell").and_then(Value::as_str).is_some_and(|shell| Path::new(shell) == shell_file(dir)),
        "mcp" => config.get("mcp").and_then(Value::as_object).is_some_and(|servers| servers.keys().all(|server| server == MESSAGES_SERVER)),
        _ => false,
    };
    entries.keys().all(|key| ours(key))
}

/// Writes the account's `opencode.json`: `kindric-shell`, and kindric's MCP `server` when given.
/// It is written again only while it is kindric-accounts' own; one the user changed is left alone,
/// and a warning says what it lacks.
fn write_config(dir: &Path, server: Option<&Value>) -> Result<Option<String>> {
    let folder = config_dir(dir);
    let (json, jsonc) = (folder.join("opencode.json"), folder.join("opencode.jsonc"));
    let shell = shell_file(dir);
    let wanted = kindric_config(dir, server);
    let file = if jsonc.exists() && !json.exists() { jsonc } else { json };
    let existing = fs::read_to_string(&file).ok().map(|text| serde_json::from_str::<Value>(&text).ok());
    match existing {
        Some(Some(config)) if config == wanted => return Ok(None),
        Some(Some(config)) if is_kindrics(&config, dir) => {},
        Some(config) => {
            let config = config.unwrap_or(Value::Null);
            let mut lacks = Vec::new();
            if config.get("shell").and_then(Value::as_str).map(PathBuf::from) != Some(shell.clone()) {
                lacks.push(format!("does not name {} as its shell: the commands of this account's sessions see its own XDG directories, not yours", shell.display()));
            }
            if let Some(server) = server
                && config.pointer(&format!("/mcp/{MESSAGES_SERVER}")).is_none()
            {
                lacks.push(format!("has no messages server: its sessions get no messages until you add \"mcp\": {{\"{MESSAGES_SERVER}\": {server}}}"));
            }
            return Ok((!lacks.is_empty()).then(|| format!("{} {}", file.display(), lacks.join("; it "))));
        },
        None => {},
    }
    let text = serde_json::to_string_pretty(&wanted).map_err(|error| Error(error.to_string()))?;
    files::write_atomic(&file, &format!("{text}\n")).map_err(|error| Error(format!("cannot write {}: {error}", file.display())))?;
    Ok(None)
}

/// kindric's MCP server for the account `name` in `dir`: none for an isolated account, whose
/// sessions take no part in messages, nor without a `kindric` in `PATH` that runs it.
fn messages_server(paths: &Paths, dir: &Path, name: &str) -> Result<Option<Value>> {
    if registry::is_isolated(paths, name)? {
        return Ok(None);
    }
    Ok(usage::kindric_program().filter(|kindric| usage::runs_mcp_for(kindric, name)).map(|kindric| mcp_server(dir, &kindric, name)))
}

/// Makes the account in `dir` ready to run: its XDG folders (700), `kindric-shell`, its
/// `opencode.json` with kindric's MCP server, and links to what it shares with the user's OpenCode
/// configuration. Returns the warnings.
pub fn prepare(paths: &Paths, dir: &Path) -> Result<Vec<String>> {
    for (_, folder) in XDG {
        files::create_private_dir(&dir.join(folder))?;
    }
    files::create_private_dir(&config_dir(dir))?;
    let name = dir.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let server = messages_server(paths, dir, &name)?;
    let mut warnings: Vec<String> = [write_shell(dir)?, write_config(dir, server.as_ref())?].into_iter().flatten().collect();
    if let Some(root) = paths.opencode_root() {
        warnings.extend(shared::link_entries(&root, &config_dir(dir), &SHARED_ENTRIES)?);
    }
    Ok(warnings)
}

/// The port the service of the account in `dir` was given, as OpenCode wrote it.
fn port_of(dir: &Path) -> Option<u16> {
    let text = fs::read_to_string(config_dir(dir).join("service.json")).ok()?;
    let port = serde_json::from_str::<Value>(&text).ok()?.get("port")?.as_u64()?;
    u16::try_from(port).ok()
}

/// The first port from `FIRST_PORT` that none of `taken` uses.
fn first_free(taken: &BTreeSet<u16>) -> Result<u16> {
    (FIRST_PORT..=u16::MAX).find(|port| !taken.contains(port)).ok_or_else(|| Error("no port left for another OpenCode account".into()))
}

/// A port no other OpenCode account of the list uses.
pub fn free_port(paths: &Paths) -> Result<u16> {
    let taken: BTreeSet<u16> = registry::list_accounts(paths)?
        .iter()
        .map(|name| paths.account_dir(name))
        .filter(|dir| agent::read(dir).is_ok_and(|agent| agent == Agent::OpenCode))
        .filter_map(|dir| port_of(&dir))
        .collect();
    first_free(&taken)
}

fn quiet(mut command: Command) -> Command {
    command.stdin(Stdio::null()).stderr(Stdio::null());
    command
}

/// Gives the service of the account in `dir` its port: OpenCode writes it in its own file.
pub fn set_port(opencode: &Path, dir: &Path, port: u16) -> Result<()> {
    let mut command = quiet(command(opencode, dir));
    let status = command
        .args(["service", "set", "port", &port.to_string()])
        .stdout(Stdio::null())
        .status()
        .map_err(|error| Error(format!("cannot run opencode: {error}")))?;
    if !status.success() {
        return fail(format!("opencode could not set the port {port} of the account in {}", dir.display()));
    }
    Ok(())
}

/// The standard output of `opencode <args>` for the account in `dir`, when it succeeds.
fn output(opencode: &Path, dir: &Path, args: &[&str]) -> Option<String> {
    let mut command = quiet(command(opencode, dir));
    let done = command.args(args).output().ok()?;
    done.status.success().then(|| String::from_utf8_lossy(&done.stdout).into_owned())
}

/// Whether `service status` says the service runs: it prints its address then, `stopped` when it
/// does not. It never starts the service, unlike the commands that talk to it.
fn runs(status: &str) -> Option<bool> {
    match status.trim() {
        "stopped" => Some(false),
        text if text.starts_with("http://") || text.starts_with("https://") => Some(true),
        _ => None,
    }
}

fn service_runs(opencode: &Path, dir: &Path) -> Option<bool> {
    runs(&output(opencode, dir, &["service", "status"])?)
}

/// Whether the account has a credential, from `auth list --format json`: a list, or an object
/// with one under `data`.
fn has_credentials(listed: &str) -> Option<bool> {
    let value: Value = serde_json::from_str(listed.trim()).ok()?;
    let list = value.as_array().or_else(|| value.get("data").and_then(Value::as_array))?;
    Some(!list.is_empty())
}

/// Whether the account is signed in. Asked only while its service runs, since asking would start
/// it; `None` otherwise, or when OpenCode does not say.
pub fn logged_in(opencode: &Path, dir: &Path) -> Option<bool> {
    if !service_runs(opencode, dir)? {
        return None;
    }
    has_credentials(&output(opencode, dir, &["auth", "list", "--format", "json"])?)
}

/// The sessions running in `GET /api/session/active`: `{"data": {"<id>": {"type": "running"}}}`.
fn active_count(answer: &str) -> usize {
    serde_json::from_str::<Value>(answer.trim())
        .ok()
        .and_then(|value| value.get("data").and_then(Value::as_object).map(serde_json::Map::len))
        .unwrap_or(0)
}

/// How many sessions of the account run a turn: none while its service is stopped, which is not
/// started to ask.
pub fn running_sessions(opencode: &Path, dir: &Path) -> usize {
    if service_runs(opencode, dir) != Some(true) {
        return 0;
    }
    output(opencode, dir, &["api", "GET", "/api/session/active"]).map_or(0, |answer| active_count(&answer))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;
    use crate::files::test_dir;

    /// The value `command` gives `name`: `Some(None)` when it removes it, `None` when it leaves it.
    fn env_of<'a>(command: &'a Command, name: &str) -> Option<Option<&'a OsStr>> {
        command.get_envs().find(|(key, _)| *key == OsStr::new(name)).map(|(_, value)| value)
    }

    fn env(pairs: &[(&str, &str)]) -> impl Iterator<Item = (OsString, OsString)> {
        pairs.iter().map(|(name, value)| (OsString::from(name), OsString::from(value))).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn opencode_runs_with_the_account_folders_and_without_the_keys_and_sessions_of_the_shell() {
        let dir = Path::new("/data/kindric-accounts/oc");
        let shell = env(&[
            ("XDG_CONFIG_HOME", "/home/u/.config"),
            ("PATH", "/usr/bin"),
            ("CLAUDE_CODE_SESSION_ID", "f78a"),
            ("CLAUDE_CODE_ENTRYPOINT", "cli"),
            ("ANTHROPIC_API_KEY", "sk"),
            ("CODEX_THREAD_ID", "01a1"),
            ("KINDRIC_USER_XDG_DATA_HOME", "/stale"),
        ]);
        let command = command_from(Path::new("/usr/bin/opencode"), dir, shell);
        assert_eq!(env_of(&command, "XDG_CONFIG_HOME"), Some(Some(dir.join("config").as_os_str())));
        assert_eq!(env_of(&command, "XDG_CACHE_HOME"), Some(Some(dir.join("cache").as_os_str())));
        assert_eq!(env_of(&command, "KINDRIC_USER_XDG_CONFIG_HOME"), Some(Some(OsStr::new("/home/u/.config"))));
        assert_eq!(env_of(&command, "KINDRIC_USER_XDG_DATA_HOME"), Some(None), "the user had none: none is passed on");
        for gone in ["CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_ENTRYPOINT", "ANTHROPIC_API_KEY", "CODEX_THREAD_ID"] {
            assert_eq!(env_of(&command, gone), Some(None), "{gone}");
        }
        assert_eq!(env_of(&command, "PATH"), None, "the rest is left as it is");
    }

    #[test]
    fn the_shell_gives_the_user_their_folders_back_and_runs_their_shell() {
        let script = shell_script();
        assert!(script.starts_with("#!/bin/sh\n# Written by kindric-accounts"), "{script}");
        assert!(script.contains(
            "if [ -n \"${KINDRIC_USER_XDG_CONFIG_HOME+set}\" ]; then export XDG_CONFIG_HOME=\"$KINDRIC_USER_XDG_CONFIG_HOME\"; else unset XDG_CONFIG_HOME; fi\nunset KINDRIC_USER_XDG_CONFIG_HOME\n"
        ), "{script}");
        assert!(script.ends_with("exec \"${SHELL:-/bin/sh}\" \"$@\"\n"), "{script}");
        assert_eq!(XDG.iter().filter(|(variable, _)| script.contains(&format!("unset {variable};"))).count(), 4);
    }

    #[cfg(unix)]
    #[test]
    fn the_shell_restores_the_folders_in_a_real_shell() {
        let dir = test_dir::fresh("opencode-shell");
        write_shell(&dir).unwrap();
        let out = Command::new(shell_file(&dir))
            .args(["-c", "echo \"${XDG_CONFIG_HOME-none}|${XDG_DATA_HOME-none}|${KINDRIC_USER_XDG_CONFIG_HOME-gone}\""])
            .env("XDG_CONFIG_HOME", "/account/config")
            .env("XDG_DATA_HOME", "/account/data")
            .env("KINDRIC_USER_XDG_CONFIG_HOME", "/home/u/.config")
            .env_remove("KINDRIC_USER_XDG_DATA_HOME")
            .env("SHELL", "/bin/sh")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "/home/u/.config|none|gone\n");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_shell_file_of_someone_else_is_left_alone() {
        let dir = test_dir::fresh("opencode-shell-other");
        fs::write(shell_file(&dir), "#!/bin/sh\necho mine\n").unwrap();
        assert!(write_shell(&dir).unwrap().is_some_and(|warning| warning.contains("left as it is")));
        assert_eq!(fs::read_to_string(shell_file(&dir)).unwrap(), "#!/bin/sh\necho mine\n");
        fs::write(shell_file(&dir), "#!/bin/sh\n# Written by kindric-accounts: old\n").unwrap();
        assert_eq!(write_shell(&dir).unwrap(), None, "an older one of ours is refreshed");
        assert_eq!(fs::read_to_string(shell_file(&dir)).unwrap(), shell_script());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_config_is_kindrics_until_the_user_changes_it() {
        let dir = test_dir::fresh("opencode-config");
        let file = config_dir(&dir).join("opencode.json");
        let read = || serde_json::from_str::<Value>(&fs::read_to_string(&file).unwrap()).unwrap();
        files::create_private_dir(&config_dir(&dir)).unwrap();
        assert_eq!(write_config(&dir, None).unwrap(), None);
        assert_eq!(read()["shell"], shell_file(&dir).display().to_string());
        assert!(read().get("mcp").is_none());

        // Written by kindric-accounts before the messages server: it gets it.
        let server = mcp_server(&dir, Path::new("/usr/bin/kindric"), "oc");
        assert_eq!(write_config(&dir, Some(&server)).unwrap(), None);
        assert_eq!(read()["mcp"]["kindric"], server);
        assert_eq!(write_config(&dir, Some(&server)).unwrap(), None, "already there");

        // The user's own: left alone, with what it lacks.
        fs::write(&file, r#"{"model": "ollama/qwen"}"#).unwrap();
        let warning = write_config(&dir, Some(&server)).unwrap().unwrap();
        assert!(warning.contains("does not name") && warning.contains("has no messages server"), "{warning}");
        assert_eq!(fs::read_to_string(&file).unwrap(), r#"{"model": "ollama/qwen"}"#);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_messages_server_starts_through_kindric_shell() {
        let dir = Path::new("/data/oc");
        let server = mcp_server(dir, Path::new("/opt/it's/kindric"), "oc");
        assert_eq!(server["type"], "local");
        assert_eq!(server["command"][0], shell_file(dir).display().to_string());
        assert_eq!(server["command"][1], "-c");
        assert_eq!(server["command"][2], r"exec '/opt/it'\''s/kindric' mcp --account 'oc'");
    }

    #[test]
    fn ports_already_taken_are_skipped() {
        assert_eq!(first_free(&BTreeSet::new()).unwrap(), FIRST_PORT);
        assert_eq!(first_free(&BTreeSet::from([FIRST_PORT, FIRST_PORT + 1, FIRST_PORT + 3])).unwrap(), FIRST_PORT + 2);
        let dir = test_dir::fresh("opencode-port");
        files::create_private_dir(&config_dir(&dir)).unwrap();
        fs::write(config_dir(&dir).join("service.json"), "{\n  \"port\": 49376,\n  \"password\": \"p\"\n}").unwrap();
        assert_eq!(port_of(&dir), Some(49376));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn what_opencode_prints_is_read() {
        assert_eq!(runs("stopped\n"), Some(false));
        assert_eq!(runs("http://127.0.0.1:49375\n"), Some(true));
        assert_eq!(runs("Error: no"), None);
        assert_eq!(has_credentials("[]"), Some(false));
        assert_eq!(has_credentials(r#"{"data":[{"id":"cred_1"}]}"#), Some(true));
        assert_eq!(has_credentials("No authenticated integrations"), None);
        assert_eq!(active_count(r#"{"data":{"ses_a":{"type":"running"},"ses_b":{"type":"running"}}}"#), 2);
        assert_eq!(active_count(r#"{"data":{}}"#), 0);
        assert_eq!(active_count("HTTP 401"), 0);
    }
}
