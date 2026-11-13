//! What the welcome screen lists before the first session: Claude Code and its version, an
//! account, kindric-accounts, the MCP server that carries messages between sessions, and a
//! first session. Each line says what is missing and what to run. A line also warns when Claude
//! Code has subcommands kindric does not know.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use kindric_accounts::injection::SUBCOMMANDS;
use kindric_accounts::{Paths, launch, registry};
use serde::Serialize;
use serde_json::Value;

use crate::accounts::{self, DEFAULT_ACCOUNT, LAUNCHER};

/// The Claude Code version kindric was tested with. Older ones get a warning only:
/// the oldest one that works is not known.
pub const TESTED_CLAUDE: (u32, u32, u32) = (2, 1, 312);

/// Name of the messages server in Claude Code's MCP configuration.
pub const MESSAGES_SERVER: &str = "kindric";

/// Claude Code's own installer.
const INSTALL_CLAUDE: &str = if cfg!(windows) {
    "irm https://claude.ai/install.ps1 | iex"
} else {
    "curl -fsSL https://claude.ai/install.sh | bash"
};

const INSTALL_LAUNCHER: &str = "https://github.com/LouisStani/kindric/blob/main/crates/accounts/README.md#installation";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeCheck {
    /// What `claude --version` printed, trimmed; None when it could not run.
    pub version: Option<String>,
    /// At least `TESTED_CLAUDE`. False when the version could not be read.
    pub tested: bool,
    pub error: Option<String>,
}

/// Where the messages server is configured: kindric-accounts shares it between its accounts,
/// but never with `default`, which needs its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessagesCheck {
    /// An account of kindric-accounts that is not isolated, to add the server in; None when
    /// there is none.
    pub account: Option<String>,
    /// One of those accounts has it in its user scope.
    pub accounts: bool,
    /// `default` is Claude Code's own configuration, not an account of that name, and is not
    /// isolated: it carries messages once logged in.
    pub default_carries: bool,
    /// `~/.claude.json` has it in its user scope.
    pub default: bool,
}

/// Everything the list is made from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub claude: ClaudeCheck,
    /// The subcommands of `claude --help` that kindric does not know, or why they were not read.
    pub subcommands: Result<Vec<String>, String>,
    /// The version line of kindric-accounts, or why it cannot be used.
    pub launcher: Result<String, String>,
    pub messages: MessagesCheck,
    /// kindric-accounts has accounts of its own.
    pub registry: bool,
    /// The accounts the app lists: `default` when logged in, and those of kindric-accounts.
    pub accounts: Vec<String>,
    pub sessions: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum State {
    #[serde(rename = "ok")]
    Ok,
    /// Keeps the welcome screen up.
    #[serde(rename = "to do")]
    ToDo,
    #[serde(rename = "warning")]
    Warning,
    #[serde(rename = "optional")]
    Optional,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Line {
    pub state: State,
    pub text: String,
    /// To copy: commands to run in a terminal, or the page to read.
    pub copy: Vec<String>,
    /// Offers to start a session.
    pub start: bool,
    /// Offers to run the commands of `copy` for you: `add_messages_server`.
    pub add: bool,
}

impl Line {
    fn new(state: State, text: impl Into<String>, copy: Vec<String>) -> Self {
        Self { state, text: text.into(), copy, start: false, add: false }
    }
}

/// Where the messages server is missing: an account of kindric-accounts, which shares it with
/// its other accounts, or `default`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Side {
    Account(String),
    Default,
}

const ADD_ARGS: [&str; 8] = ["mcp", "add", "-s", "user", MESSAGES_SERVER, "--", "kindric", "mcp"];

impl Side {
    /// `kindric-accounts run <account> mcp add …`, or `claude mcp add …` without
    /// `CLAUDE_CONFIG_DIR` on `default`, as its sessions run. Claude Code writes its own file.
    pub fn command(&self) -> Command {
        let mut command = match self {
            Side::Account(account) => {
                let mut command = Command::new(LAUNCHER);
                command.args(["run", account]);
                command
            },
            Side::Default => {
                let mut command = Command::new("claude");
                for variable in accounts::default_removed_env() {
                    command.env_remove(variable);
                }
                command
            },
        };
        command.args(ADD_ARGS).stdin(Stdio::null());
        command
    }

    /// The command as typed in a terminal.
    pub fn shown(&self) -> String {
        let program = match self {
            Side::Account(account) => format!("{LAUNCHER} run {account}"),
            Side::Default => "claude".into(),
        };
        format!("{program} {}", ADD_ARGS.join(" "))
    }
}

/// Parses `2.1.316 (Claude Code)` into (2, 1, 316).
pub fn parse_claude_version(output: &str) -> Option<(u32, u32, u32)> {
    let version = output.split_whitespace().next()?;
    let mut parts = version.split('.').map(str::parse::<u32>);
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(Ok(major)), Some(Ok(minor)), Some(Ok(patch)), None) => Some((major, minor, patch)),
        _ => None,
    }
}

fn claude_check(output: &str) -> ClaudeCheck {
    let version = output.trim().to_string();
    let tested = parse_claude_version(&version).is_some_and(|found| found >= TESTED_CLAUDE);
    ClaudeCheck { version: Some(version), tested, error: None }
}

fn check_claude() -> ClaudeCheck {
    let failed = |error: String| ClaudeCheck { version: None, tested: false, error: Some(error) };
    let claude = match launch::require_claude() {
        Ok(claude) => claude,
        Err(error) => return failed(error.to_string()),
    };
    match Command::new(claude).arg("--version").stdin(Stdio::null()).output() {
        Ok(output) if output.status.success() => claude_check(&String::from_utf8_lossy(&output.stdout)),
        Ok(output) => failed(format!("claude --version: {}", String::from_utf8_lossy(&output.stderr).trim())),
        Err(error) => failed(format!("claude --version: {error}")),
    }
}

/// The subcommands listed under `Commands:` in `claude --help` that are not in `SUBCOMMANDS`:
/// kindric would read them as the start of a conversation and give them the memory. Aliases
/// count, `stop|kill` gives both; `help` is left out.
pub fn unknown_subcommands(help: &str) -> Vec<String> {
    help.lines()
        .skip_while(|line| line.trim_end() != "Commands:")
        .skip(1)
        .take_while(|line| line.trim().is_empty() || line.starts_with(' '))
        .filter(|line| line.starts_with("  ") && !line[2..].starts_with(' '))
        .filter_map(|line| line.split_whitespace().next())
        .flat_map(|names| names.split('|'))
        .filter(|name| *name != "help" && !SUBCOMMANDS.contains(name))
        .map(String::from)
        .collect()
}

fn check_subcommands() -> Result<Vec<String>, String> {
    let claude = launch::require_claude().map_err(|error| error.to_string())?;
    match Command::new(claude).arg("--help").stdin(Stdio::null()).output() {
        Ok(output) if output.status.success() => Ok(unknown_subcommands(&String::from_utf8_lossy(&output.stdout))),
        Ok(output) => Err(format!("claude --help: {}", String::from_utf8_lossy(&output.stderr).trim())),
        Err(error) => Err(format!("claude --help: {error}")),
    }
}

/// The claude found in `PATH` as it is on disk: what it said holds while this does not change.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

fn stamp() -> Option<Stamp> {
    let path = launch::require_claude().ok()?;
    let metadata = fs::metadata(&path).ok()?;
    Some(Stamp { path, len: metadata.len(), modified: metadata.modified().ok() })
}

/// What `claude --version` and `claude --help` said, and of which claude.
#[derive(Clone, Debug)]
pub struct ClaudeFacts {
    stamp: Option<Stamp>,
    claude: ClaudeCheck,
    subcommands: Result<Vec<String>, String>,
}

/// Whether `previous` still speaks for the claude found now: a claude not found is looked for
/// again each time.
fn still_holds(previous: Option<&ClaudeFacts>, now: Option<&Stamp>) -> bool {
    previous.is_some_and(|previous| now.is_some() && previous.stamp.as_ref() == now)
}

impl ClaudeFacts {
    /// `previous` again while the same claude is found, else read again: the two calls cost about
    /// 0.4 s of processor, and the list is checked again each time a session starts. A claude
    /// updated behind a shim of the same size and date (mise) is seen at the next start of the app.
    pub fn current(previous: Option<&ClaudeFacts>) -> ClaudeFacts {
        let now = stamp();
        match previous {
            Some(previous) if still_holds(Some(previous), now.as_ref()) => previous.clone(),
            _ => ClaudeFacts { stamp: now, claude: check_claude(), subcommands: check_subcommands() },
        }
    }
}

/// Shown only when something is off, and never when Claude Code itself is missing: its own line
/// says so.
fn subcommands_line(subcommands: &Result<Vec<String>, String>, claude: &ClaudeCheck) -> Option<Line> {
    match subcommands {
        Ok(unknown) if unknown.is_empty() => None,
        Ok(unknown) => {
            let text = format!("Claude Code subcommands unknown to kindric, which gives them the memory: {}", unknown.join(", "));
            Some(Line::new(State::Warning, text, Vec::new()))
        },
        Err(_) if claude.version.is_none() => None,
        Err(error) => Some(Line::new(State::Warning, format!("Claude Code subcommands not checked: {error}"), Vec::new())),
    }
}

fn check_launcher() -> Result<String, String> {
    accounts::check_launcher_version().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => format!("{LAUNCHER} is not installed"),
        _ => error.to_string(),
    })
}

/// Whether the Claude Code state file at `path` has the messages server in its user scope.
/// A missing or unreadable file has none.
pub fn has_messages_server(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|state| state.get("mcpServers")?.as_object().map(|servers| servers.contains_key(MESSAGES_SERVER)))
        .unwrap_or(false)
}

/// `~/.claude.json`, next to `~/.claude`: where Claude Code keeps the state of `default`.
fn default_state_file(paths: &Paths) -> Option<std::path::PathBuf> {
    paths.shared_root.parent().map(|home| home.join(".claude.json"))
}

pub fn check_messages(paths: &Paths) -> Result<MessagesCheck, String> {
    let isolated = registry::list_isolated(paths).map_err(|error| error.to_string())?;
    let registered = registry::list_accounts(paths).map_err(|error| error.to_string())?;
    let carriers: Vec<&String> = registered.iter().filter(|account| !isolated.contains(account)).collect();
    let accounts = carriers.iter().any(|account| has_messages_server(&paths.account_dir(account).join(".claude.json")));
    let named_default = |accounts: &[String]| accounts.iter().any(|account| account == DEFAULT_ACCOUNT);
    Ok(MessagesCheck {
        account: carriers.first().map(|account| account.to_string()),
        accounts,
        default_carries: !named_default(&registered) && !named_default(&isolated),
        default: default_state_file(paths).is_some_and(|file| has_messages_server(&file)),
    })
}

fn claude_line(claude: &ClaudeCheck) -> Line {
    let Some(version) = &claude.version else {
        let error = claude.error.as_deref().unwrap_or("not found");
        return Line::new(State::ToDo, format!("Claude Code: {error}"), vec![INSTALL_CLAUDE.into()]);
    };
    let shown = parse_claude_version(version)
        .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"))
        .unwrap_or_else(|| version.clone());
    if claude.tested {
        return Line::new(State::Ok, format!("Claude Code {shown}"), Vec::new());
    }
    let (major, minor, patch) = TESTED_CLAUDE;
    let text = format!("Claude Code {shown}: kindric was tested with {major}.{minor}.{patch} and later");
    Line::new(State::Warning, text, vec!["claude update".into()])
}

fn account_line(accounts: &[String]) -> Line {
    if accounts.is_empty() {
        let text = format!("an account: run claude in a terminal and log in, or add one with {LAUNCHER}");
        return Line::new(State::ToDo, text, vec!["claude".into()]);
    }
    Line::new(State::Ok, format!("accounts: {}", accounts.join(", ")), Vec::new())
}

/// Needed only when kindric-accounts has accounts: `default` runs with `claude` alone.
fn launcher_line(launcher: &Result<String, String>, registry: bool) -> Line {
    let optional = if registry { "" } else { " · only needed for more accounts" };
    match launcher {
        Ok(version) => Line::new(State::Ok, format!("{version}{optional}"), Vec::new()),
        Err(error) if registry => Line::new(State::ToDo, error.clone(), vec![INSTALL_LAUNCHER.into()]),
        Err(error) => Line::new(State::Optional, format!("{error}{optional}"), vec![INSTALL_LAUNCHER.into()]),
    }
}

/// `default` carries messages and the app lists it, so it is logged in.
fn default_side(messages: &MessagesCheck, accounts: &[String]) -> bool {
    messages.default_carries && accounts.iter().any(|account| account == DEFAULT_ACCOUNT)
}

/// The sides that lack the server, among those that carry messages.
pub fn missing_sides(messages: &MessagesCheck, accounts: &[String]) -> Vec<Side> {
    let mut sides = Vec::new();
    if let Some(account) = &messages.account
        && !messages.accounts
    {
        sides.push(Side::Account(account.clone()));
    }
    if default_side(messages, accounts) && !messages.default {
        sides.push(Side::Default);
    }
    sides
}

/// One command per side that lacks the server, and the offer to run them.
fn messages_line(messages: &MessagesCheck, accounts: &[String]) -> Line {
    if messages.account.is_none() && !default_side(messages, accounts) {
        return if accounts.is_empty() {
            Line::new(State::ToDo, "messages between sessions: an account first", Vec::new())
        } else {
            Line::new(State::Optional, "messages between sessions: every account is isolated", Vec::new())
        };
    }
    let copy: Vec<String> = missing_sides(messages, accounts).iter().map(Side::shown).collect();
    if copy.is_empty() {
        return Line::new(State::Ok, "messages between sessions", copy);
    }
    let text = "messages between sessions: add the server once, sessions started after get them";
    Line { add: true, ..Line::new(State::ToDo, text, copy) }
}

/// Runs the command of every side that lacks the server, for the accounts the app lists;
/// stops at the first that fails, with what it said.
pub fn add_messages_server(accounts: &[String]) -> Result<(), String> {
    let paths = accounts::paths().map_err(|error| error.to_string())?;
    for side in missing_sides(&check_messages(&paths)?, accounts) {
        let output = side.command().output().map_err(|error| format!("{}: {error}", side.shown()))?;
        if !output.status.success() {
            let said = format!("{}{}", String::from_utf8_lossy(&output.stderr), String::from_utf8_lossy(&output.stdout));
            return Err(format!("{}: {}", side.shown(), said.trim()));
        }
    }
    Ok(())
}

fn session_line(sessions: usize) -> Line {
    if sessions == 0 {
        return Line { start: true, ..Line::new(State::ToDo, "a first session", Vec::new()) };
    }
    let plural = if sessions == 1 { "" } else { "s" };
    Line::new(State::Ok, format!("{sessions} session{plural}"), Vec::new())
}

/// The list, in the order things get set up.
pub fn lines(found: &Found) -> Vec<Line> {
    let mut lines = vec![claude_line(&found.claude)];
    lines.extend(subcommands_line(&found.subcommands, &found.claude));
    lines.extend([
        account_line(&found.accounts),
        launcher_line(&found.launcher, found.registry),
        messages_line(&found.messages, &found.accounts),
        session_line(found.sessions),
    ]);
    lines
}

/// The list for the accounts and sessions the app shows now, with what claude said (see
/// `ClaudeFacts::current`). Runs `kindric-accounts --version`.
pub fn run(accounts: Vec<String>, sessions: usize, claude: &ClaudeFacts) -> Result<Vec<Line>, String> {
    let paths = accounts::paths().map_err(|error| error.to_string())?;
    let registry = !registry::list_accounts(&paths).map_err(|error| error.to_string())?.is_empty();
    let found = Found {
        claude: claude.claude.clone(),
        subcommands: claude.subcommands.clone(),
        launcher: check_launcher(),
        messages: check_messages(&paths)?,
        registry,
        accounts,
        sessions,
    };
    Ok(lines(&found))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_claude_said_holds_while_the_same_claude_is_found() {
        let stamp = |len| Stamp { path: PathBuf::from("/bin/claude"), len, modified: None };
        let facts = ClaudeFacts {
            stamp: Some(stamp(10)),
            claude: ClaudeCheck { version: Some("2.1.323".into()), tested: true, error: None },
            subcommands: Ok(Vec::new()),
        };
        assert!(still_holds(Some(&facts), Some(&stamp(10))));
        assert!(!still_holds(Some(&facts), Some(&stamp(11))), "another claude: asked again");
        assert!(!still_holds(Some(&facts), None), "no claude found: looked for again");
        assert!(!still_holds(None, Some(&stamp(10))), "never asked yet");
        let not_found = ClaudeFacts { stamp: None, ..facts };
        assert!(!still_holds(Some(&not_found), None));
    }

    #[test]
    fn claude_versions_are_read_and_compared_with_the_tested_one() {
        assert_eq!(parse_claude_version("2.1.316 (Claude Code)\n"), Some((2, 1, 316)));
        assert_eq!(parse_claude_version("2.1"), None);
        assert_eq!(parse_claude_version(""), None);
        assert!(claude_check("2.1.316 (Claude Code)\n").tested);
        assert!(claude_check("2.1.312 (Claude Code)").tested);
        assert!(!claude_check("2.1.311 (Claude Code)").tested);
        assert!(!claude_check("something else").tested);
        assert_eq!(claude_check("2.1.316 (Claude Code)\n").version.as_deref(), Some("2.1.316 (Claude Code)"));
    }

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-checks-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch directory");
        root
    }

    const WITH_SERVER: &str = r#"{"mcpServers":{"kindric":{"type":"stdio","command":"kindric","args":["mcp"]}}}"#;

    #[test]
    fn only_the_user_scope_counts() {
        let root = scratch("scope");
        let file = root.join(".claude.json");
        assert!(!has_messages_server(&file), "missing file");
        fs::write(&file, WITH_SERVER).unwrap();
        assert!(has_messages_server(&file));
        fs::write(&file, r#"{"mcpServers":{"sentry":{"type":"http","url":"https://mcp.sentry.dev/mcp"}}}"#).unwrap();
        assert!(!has_messages_server(&file), "another server only");
        fs::write(&file, r#"{"projects":{"/p":{"mcpServers":{"kindric":{}}}}}"#).unwrap();
        assert!(!has_messages_server(&file), "project scope");
        fs::write(&file, "{").unwrap();
        assert!(!has_messages_server(&file), "broken file");
        fs::remove_dir_all(&root).unwrap();
    }

    fn messages(account: Option<&str>, accounts: bool, default_carries: bool, default: bool) -> MessagesCheck {
        MessagesCheck { account: account.map(String::from), accounts, default_carries, default }
    }

    #[test]
    fn accounts_count_unless_isolated_and_default_has_its_own() {
        let root = scratch("sides");
        let paths = Paths::from_roots(root.join("config"), root.join("data"), root.join("home"));
        fs::create_dir_all(&paths.config_dir).unwrap();
        fs::create_dir_all(paths.account_dir("work")).unwrap();
        fs::create_dir_all(paths.account_dir("perso")).unwrap();
        fs::create_dir_all(&paths.shared_root).unwrap();
        fs::write(paths.registry_file(), "work\nperso\n").unwrap();
        assert_eq!(check_messages(&paths).unwrap(), messages(Some("work"), false, true, false));

        fs::write(paths.account_dir("work").join(".claude.json"), WITH_SERVER).unwrap();
        fs::write(paths.isolated_file(), "work\n").unwrap();
        assert_eq!(check_messages(&paths).unwrap(), messages(Some("perso"), false, true, false), "isolated");

        fs::write(paths.account_dir("perso").join(".claude.json"), WITH_SERVER).unwrap();
        assert_eq!(check_messages(&paths).unwrap(), messages(Some("perso"), true, true, false));

        fs::write(root.join("home").join(".claude.json"), WITH_SERVER).unwrap();
        assert_eq!(check_messages(&paths).unwrap(), messages(Some("perso"), true, true, true));

        fs::write(paths.isolated_file(), "work\ndefault\n").unwrap();
        assert!(!check_messages(&paths).unwrap().default_carries, "default isolated");
        fs::write(paths.isolated_file(), "").unwrap();
        fs::write(paths.registry_file(), "work\nperso\ndefault\n").unwrap();
        assert!(!check_messages(&paths).unwrap().default_carries, "an account named default");
        fs::remove_dir_all(&root).unwrap();
    }

    fn found() -> Found {
        Found {
            claude: claude_check("2.1.316 (Claude Code)"),
            subcommands: Ok(Vec::new()),
            launcher: Ok("kindric-accounts 2.0.0".into()),
            messages: messages(Some("louis"), true, true, true),
            registry: true,
            accounts: vec!["default".into(), "louis".into()],
            sessions: 2,
        }
    }

    fn states(found: &Found) -> Vec<State> {
        lines(found).iter().map(|line| line.state).collect()
    }

    #[test]
    fn a_machine_set_up_has_nothing_to_do() {
        let lines = lines(&found());
        assert!(lines.iter().all(|line| line.state == State::Ok && line.copy.is_empty() && !line.start), "{lines:?}");
        assert_eq!(lines[0].text, "Claude Code 2.1.316");
        assert_eq!(lines[1].text, "accounts: default, louis");
        assert_eq!(lines[2].text, "kindric-accounts 2.0.0");
        assert_eq!(lines[4].text, "2 sessions");
    }

    #[test]
    fn a_new_machine_lists_everything_in_order() {
        let found = Found {
            claude: ClaudeCheck { version: None, tested: false, error: Some("claude not found".into()) },
            subcommands: Err("claude not found in PATH".into()),
            launcher: Err("kindric-accounts is not installed".into()),
            messages: messages(None, false, true, false),
            registry: false,
            accounts: Vec::new(),
            sessions: 0,
        };
        let lines = lines(&found);
        let shown: Vec<_> = lines.iter().map(|line| line.state).collect();
        assert_eq!(shown, [State::ToDo, State::ToDo, State::Optional, State::ToDo, State::ToDo]);
        assert_eq!(lines[0].copy, [INSTALL_CLAUDE]);
        assert_eq!(lines[1].copy, ["claude"]);
        assert_eq!(lines[2].copy, [INSTALL_LAUNCHER]);
        assert!(lines[3].copy.is_empty());
        assert!(lines[4].start);
    }

    /// `claude --help` of Claude Code 2.1.318, cut down: options, then commands with an alias, a
    /// description on two lines, and one subcommand kindric does not know.
    const HELP: &str = "Usage: claude [options] [command] [prompt]

Options:
  -p, --print                           Print response and exit
  -h, --help                            Display help for command

Commands:
  agents [options]                      Manage background agents
  attach <id|name>                      Open a background session in this
                                        terminal. Part of the session name works
  plugin|plugins                        Manage Claude Code plugins
  stop|kill <id>                        Stop a background session
  sandbox|sb [options]                  Something new
  help [command]                        display help for command
";

    #[test]
    fn subcommands_are_read_under_commands_with_their_aliases() {
        assert_eq!(unknown_subcommands(HELP), ["sandbox", "sb"]);
        let known = HELP.replace("  sandbox|sb [options]                  Something new\n", "");
        assert!(unknown_subcommands(&known).is_empty());
        assert!(unknown_subcommands("Usage: claude\n").is_empty());
    }

    #[test]
    fn the_installed_list_is_known() {
        let installed = "Commands:\n".to_string() + &SUBCOMMANDS.map(|name| format!("  {name} [options]  d\n")).concat();
        assert!(unknown_subcommands(&installed).is_empty());
    }

    #[test]
    fn unknown_subcommands_are_a_warning_after_claude_code() {
        let unknown = lines(&Found { subcommands: Ok(vec!["sandbox".into(), "sb".into()]), ..found() });
        assert_eq!(unknown.len(), 6);
        assert_eq!(unknown[1].state, State::Warning);
        assert!(unknown[1].text.ends_with(": sandbox, sb"), "{}", unknown[1].text);
        let unread = Found { subcommands: Err("claude --help: boom".into()), ..found() };
        assert_eq!(lines(&unread)[1].text, "Claude Code subcommands not checked: claude --help: boom");
    }

    #[test]
    fn an_old_claude_code_is_a_warning_only() {
        let found = Found { claude: claude_check("2.1.300 (Claude Code)"), ..found() };
        let line = &lines(&found)[0];
        assert_eq!(line.state, State::Warning);
        assert!(line.text.starts_with("Claude Code 2.1.300: "), "{}", line.text);
        assert_eq!(line.copy, ["claude update"]);
    }

    #[test]
    fn kindric_accounts_is_needed_only_for_its_own_accounts() {
        let alone = Found { launcher: Err("kindric-accounts is not installed".into()), registry: false, ..found() };
        assert_eq!(states(&alone)[2], State::Optional);
        let used = Found { registry: true, ..alone.clone() };
        assert_eq!(states(&used)[2], State::ToDo);
        let installed = Found { registry: false, ..found() };
        assert_eq!(lines(&installed)[2].text, "kindric-accounts 2.0.0 · only needed for more accounts");
    }

    #[test]
    fn each_side_without_the_server_gets_its_command() {
        let add = "mcp add -s user kindric -- kindric mcp";
        let both = Found { messages: messages(Some("louis"), false, true, false), ..found() };
        let line = &lines(&both)[3];
        assert_eq!(line.state, State::ToDo);
        assert_eq!(line.copy, [format!("kindric-accounts run louis {add}"), format!("claude {add}")]);
        assert!(line.add);
        assert!(lines(&found()).iter().all(|line| !line.add), "nothing to add");
        assert!(!lines(&Found { accounts: Vec::new(), messages: messages(None, false, true, false), ..found() })[3].add, "no account yet");

        let default_only = Found { messages: messages(None, false, true, false), accounts: vec!["default".into()], ..found() };
        assert_eq!(lines(&default_only)[3].copy, [format!("claude {add}")]);

        let logged_out = Found { messages: messages(Some("louis"), true, true, false), accounts: vec!["louis".into()], ..found() };
        assert_eq!(states(&logged_out)[3], State::Ok, "default not logged in needs nothing");

        let isolated = Found { messages: messages(Some("louis"), true, false, false), ..found() };
        assert_eq!(states(&isolated)[3], State::Ok, "default isolated needs nothing");

        let none = Found { messages: messages(None, false, false, false), accounts: vec!["work".into()], ..found() };
        assert_eq!(states(&none)[3], State::Optional);
    }

    fn parts(command: &Command) -> Vec<String> {
        std::iter::once(command.get_program()).chain(command.get_args()).map(|part| part.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn the_command_run_is_the_one_shown() {
        let account = Side::Account("louis".into());
        assert_eq!(parts(&account.command()).join(" "), account.shown());
        assert_eq!(parts(&account.command())[..3], ["kindric-accounts", "run", "louis"]);
        assert!(account.command().get_envs().next().is_none(), "kindric-accounts sets the account up");

        let default = Side::Default.command();
        assert_eq!(parts(&default).join(" "), Side::Default.shown());
        assert_eq!(Side::Default.shown(), "claude mcp add -s user kindric -- kindric mcp");
        let removed: Vec<_> = default.get_envs().filter(|(_, value)| value.is_none()).map(|(name, _)| name).collect();
        assert!(removed.contains(&std::ffi::OsStr::new("CLAUDE_CONFIG_DIR")));
        assert!(removed.contains(&std::ffi::OsStr::new("ANTHROPIC_API_KEY")));
    }

    #[test]
    fn only_the_sides_without_the_server_are_added() {
        let accounts = ["default".to_string(), "louis".to_string()];
        let sides = |account, has_it, default| missing_sides(&messages(account, has_it, true, default), &accounts);
        assert_eq!(sides(Some("louis"), false, false), [Side::Account("louis".into()), Side::Default]);
        assert_eq!(sides(Some("louis"), true, false), [Side::Default]);
        assert_eq!(sides(Some("louis"), false, true), [Side::Account("louis".into())]);
        assert_eq!(sides(None, false, true), []);
        assert_eq!(missing_sides(&messages(None, false, true, false), &["louis".to_string()]), [], "default logged out");
    }

    #[test]
    fn sessions_are_counted() {
        assert_eq!(lines(&Found { sessions: 1, ..found() })[4].text, "1 session");
        let none = &lines(&Found { sessions: 0, ..found() })[4];
        assert_eq!((none.state, none.start), (State::ToDo, true));
    }
}
