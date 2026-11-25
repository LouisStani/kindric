mod menu;

use std::ffi::OsString;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use kindric_accounts::agent::{self, Agent};
use kindric_accounts::provider::{self, Kind, Provider};
use kindric_accounts::{Error, Paths, Result, account, injection, isolation, launch, mcp, registry, status, usage};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
Usage:
  kindric-accounts                   menu: pick an account and start claude here
  kindric-accounts add <name>        create the account and its directory (700)
  kindric-accounts add <name> --provider api|openrouter|ollama|gateway [--url <address>] [--model <name>]
                                     create an account whose model comes from an API key or a gateway
  kindric-accounts add <name> --agent codex
                                     create an account that runs Codex instead of Claude Code
  kindric-accounts add <name> --agent opencode
                                     create an account that runs OpenCode, with a port of its own
  kindric-accounts token <name>      keep the key of a provider account (read from standard input)
  kindric-accounts list [--json]     list accounts
  kindric-accounts status [--json]   show login status, plan and active sessions of each account
  kindric-accounts remove <name> [--delete-dir|--keep-dir]
                                     remove the account (directory deleted only if confirmed)
  kindric-accounts run <name> [...]  start claude (or codex, opencode) with this account here (args passed to it)
  kindric-accounts isolate <name>    stop sharing MCP servers with and from this account
  kindric-accounts unisolate <name>  share MCP servers again
  kindric-accounts isolated [--json] list isolated accounts
  kindric-accounts statusline        status line of the sessions it starts (reads Claude Code's input)
  kindric-accounts --version         print the version
";

/// What a command asks the process to do once it is done.
enum Exit {
    Success,
    Usage,
    /// Exit code of Claude Code, where it cannot replace this process (Windows).
    #[cfg_attr(unix, allow(dead_code))]
    Code(i32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DirChoice {
    Ask,
    Delete,
    Keep,
}

fn wants_json(args: &[String]) -> Option<bool> {
    match args {
        [] => Some(false),
        [flag] if flag == "--json" => Some(true),
        _ => None,
    }
}

fn dir_choice(args: &[String]) -> Option<DirChoice> {
    match args {
        [] => Some(DirChoice::Ask),
        [flag] if flag == "--delete-dir" => Some(DirChoice::Delete),
        [flag] if flag == "--keep-dir" => Some(DirChoice::Keep),
        _ => None,
    }
}

fn print_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("kindric-accounts: {warning}");
    }
}

/// Asks on the terminal; without one, the answer is no.
fn confirm(prompt: &str) -> bool {
    if !io::stdin().is_terminal() {
        return false;
    }
    eprint!("{prompt} [y/N] ");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    if io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim(), "y" | "Y" | "o" | "O")
}

/// `--provider <kind> [--url <address>] [--model <name>]`, in any order; `None` for a wrong option.
fn provider_options(args: &[String]) -> Option<Result<Option<Provider>>> {
    let (mut kind, mut url, mut model) = (None, None, None);
    let mut rest = args.iter();
    while let Some(option) = rest.next() {
        let value = rest.next()?;
        let slot = match option.as_str() {
            "--provider" => &mut kind,
            "--url" => &mut url,
            "--model" => &mut model,
            _ => return None,
        };
        if slot.replace(value.as_str()).is_some() {
            return None;
        }
    }
    let Some(kind) = kind else {
        return (url.is_none() && model.is_none()).then_some(Ok(None));
    };
    let Some(kind) = Kind::parse(kind) else {
        return Some(Err(Error(format!("unknown provider {kind:?}: use {}", Kind::NAMES.join(", ")))));
    };
    Some(Provider::new(kind, url, model).map(Some))
}

/// `--agent <name>` taken out of `args`, the rest left as it is; `None` for `--agent` without a
/// value or given twice.
fn agent_option(args: &[String]) -> Option<(Option<&str>, Vec<String>)> {
    let mut agent = None;
    let mut rest = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--agent" {
            if agent.replace(args.next()?.as_str()).is_some() {
                return None;
            }
        } else {
            rest.push(arg.clone());
        }
    }
    Some((agent, rest))
}

fn cmd_add(paths: &Paths, name: &str, args: &[String]) -> Result<Exit> {
    let Some((agent, args)) = agent_option(args) else {
        return Ok(Exit::Usage);
    };
    match agent.map(|agent| (agent, Agent::parse(agent))) {
        Some((_, Some(Agent::Codex))) => return cmd_add_codex(paths, name, &args),
        Some((_, Some(Agent::OpenCode))) => return cmd_add_opencode(paths, name, &args),
        Some((_, Some(Agent::ClaudeCode))) | None => {},
        Some((unknown, None)) => {
            return Err(Error(format!("unknown agent {unknown:?}: use {}", Agent::NAMES.join(", "))));
        },
    }
    let Some(provider) = provider_options(&args) else {
        return Ok(Exit::Usage);
    };
    let provider = provider?;
    let created = account::add_with(paths, name, provider.as_ref())?;
    print_warnings(&created.warnings);
    println!("Account \"{name}\" created: {}", created.dir.display());
    match provider {
        None => println!("First launch: kindric-accounts run {name}, then /login"),
        Some(provider) if provider.provider.needs_token() => {
            println!("Give its {} key: kindric-accounts token {name}", provider.provider.name());
        },
        Some(_) => println!("Ready: kindric-accounts run {name}"),
    }
    Ok(Exit::Success)
}

fn cmd_add_codex(paths: &Paths, name: &str, args: &[String]) -> Result<Exit> {
    if !args.is_empty() {
        return Err(Error("a Codex account takes no --provider, --url or --model: it signs in with its ChatGPT login or an OpenAI key".into()));
    }
    let created = account::add_codex(paths, name)?;
    print_warnings(&created.warnings);
    println!("Codex account \"{name}\" created: {}", created.dir.display());
    println!("Sign in: kindric-accounts run {name} login");
    Ok(Exit::Success)
}

fn cmd_add_opencode(paths: &Paths, name: &str, args: &[String]) -> Result<Exit> {
    if !args.is_empty() {
        return Err(Error("an OpenCode account takes no --provider, --url or --model: OpenCode keeps its own providers".into()));
    }
    let created = account::add_opencode(paths, name)?;
    print_warnings(&created.warnings);
    println!("OpenCode account \"{name}\" created: {}", created.dir.display());
    println!("Sign in: kindric-accounts run {name} auth login");
    Ok(Exit::Success)
}

/// The provider of `name`, which must be a provider account.
fn require_provider(paths: &Paths, name: &str) -> Result<(PathBuf, Provider)> {
    let dir = launch::require_account(paths, name)?;
    match agent::read(&dir)? {
        Agent::Codex => {
            return Err(Error(format!("{name} is a Codex account: Codex keeps its own key, give it with kindric-accounts run {name} login --with-api-key")));
        },
        Agent::OpenCode => {
            return Err(Error(format!("{name} is an OpenCode account: OpenCode keeps its own keys, give them with kindric-accounts run {name} auth login")));
        },
        Agent::ClaudeCode => {},
    }
    match provider::read(&dir)? {
        Some(provider) => Ok((dir, provider)),
        None => Err(Error(format!("{name} is a Claude account: sign in with kindric-accounts run {name}, then /login"))),
    }
}

/// `token <name>`: keeps the key read on standard input, without echo on a terminal.
/// `token <name> --print`: prints it, for Claude Code's `apiKeyHelper`.
fn cmd_token(paths: &Paths, name: &str, args: &[String]) -> Result<Exit> {
    let print = match args {
        [] => false,
        [flag] if flag == "--print" => true,
        _ => return Ok(Exit::Usage),
    };
    let (dir, provider) = require_provider(paths, name)?;
    if print {
        println!("{}", provider::key_for_session(&dir, &provider, name)?);
        return Ok(Exit::Success);
    }
    let token = if io::stdin().is_terminal() {
        eprint!("{} key for {name} (not shown): ", provider.provider.name());
        dialoguer::console::Term::stderr().read_secure_line()?
    } else {
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        line
    };
    provider::save_token(&dir, &token)?;
    println!("Key of \"{name}\" kept in {}", provider::token_file(&dir).display());
    Ok(Exit::Success)
}

fn cmd_list(paths: &Paths, args: &[String]) -> Result<Exit> {
    let Some(json) = wants_json(args) else {
        return Ok(Exit::Usage);
    };
    let names = registry::list_accounts(paths)?;
    if json {
        println!("{}", to_json(&names)?);
    } else if names.is_empty() {
        eprintln!("No accounts. Create one: kindric-accounts add <name>");
    } else {
        for name in names {
            println!("{name}");
        }
    }
    Ok(Exit::Success)
}

fn cmd_remove(paths: &Paths, name: &str, args: &[String]) -> Result<Exit> {
    let Some(choice) = dir_choice(args) else {
        return Ok(Exit::Usage);
    };
    cmd_remove_with(paths, name, choice)
}

fn cmd_remove_with(paths: &Paths, name: &str, choice: DirChoice) -> Result<Exit> {
    account::forget(paths, name)?;
    println!("Account \"{name}\" removed from the list.");
    let dir = paths.account_dir(name);
    if !dir.is_dir() {
        return Ok(Exit::Success);
    }
    let delete = match choice {
        DirChoice::Delete => true,
        DirChoice::Keep => false,
        DirChoice::Ask => {
            confirm(&format!("Also delete {} (credentials and history of this account)?", dir.display()))
        },
    };
    if delete {
        account::delete_dir(paths, name)?;
        println!("Directory deleted.");
    } else {
        println!("Directory kept: {}", dir.display());
    }
    Ok(Exit::Success)
}

fn to_json(value: &impl serde::Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(|error| kindric_accounts::Error(error.to_string()))
}

fn cmd_status(paths: &Paths, args: &[String]) -> Result<Exit> {
    let Some(json) = wants_json(args) else {
        return Ok(Exit::Usage);
    };
    let names = registry::list_accounts(paths)?;
    if names.is_empty() {
        if json {
            println!("[]");
            return Ok(Exit::Success);
        }
        return cmd_list(paths, &[]);
    }
    let statuses = status::collect(paths, &names)?;
    if json {
        println!("{}", to_json(&statuses)?);
    } else {
        for line in status::format_lines(&statuses) {
            println!("{line}");
        }
    }
    Ok(Exit::Success)
}

fn cmd_isolate(paths: &Paths, name: &str) -> Result<Exit> {
    if isolation::isolate(paths, name)? {
        println!("Account \"{name}\" isolated: it no longer gives or receives MCP servers.");
        println!("Restart the sessions already running, on this account and on the others, to apply it.");
    } else {
        println!("Account \"{name}\" is already isolated.");
    }
    Ok(Exit::Success)
}

fn cmd_unisolate(paths: &Paths, name: &str) -> Result<Exit> {
    if isolation::unisolate(paths, name)? {
        println!("Account \"{name}\" shares MCP servers again from its next launch.");
    } else {
        println!("Account \"{name}\" is not isolated.");
    }
    Ok(Exit::Success)
}

fn cmd_isolated(paths: &Paths, args: &[String]) -> Result<Exit> {
    let Some(json) = wants_json(args) else {
        return Ok(Exit::Usage);
    };
    let names = registry::list_isolated(paths)?;
    if json {
        println!("{}", to_json(&names)?);
    } else {
        for name in names {
            println!("{name}");
        }
    }
    Ok(Exit::Success)
}

/// Where the conversation `claude <args>` opens starts, so it gets the memory of its zone; none
/// for a subcommand. A current directory that cannot be read only costs the memory.
fn conversation_dir(claude_args: &[OsString]) -> Option<PathBuf> {
    if !injection::opens_conversation(claude_args) {
        return None;
    }
    match std::env::current_dir() {
        Ok(directory) => Some(directory),
        Err(error) => {
            print_warnings(&[format!("memory not loaded: current directory: {error}")]);
            None
        },
    }
}

/// Replaces this process with `prepared`, or waits for it where that cannot be done (Windows).
fn start(prepared: launch::Launch) -> Result<Exit> {
    print_warnings(&prepared.warnings);
    #[cfg(unix)]
    return Err(launch::exec(prepared.command));
    #[cfg(windows)]
    return launch::run_and_wait(prepared.command).map(Exit::Code);
}

fn cmd_run(paths: &Paths, name: &str, claude_args: &[OsString]) -> Result<Exit> {
    let dir = launch::require_account(paths, name)?;
    match agent::read(&dir)? {
        Agent::Codex => return start(launch::prepare_codex(paths, name, claude_args)?),
        Agent::OpenCode => return start(launch::prepare_opencode(paths, name, claude_args)?),
        Agent::ClaudeCode => {},
    }
    launch::require_claude()?;
    if injection::opens_conversation(claude_args)
        && let Some(provider) = provider::read(&dir)?
        && !provider::is_ready(&dir, &provider)
    {
        return Err(Error(format!("{name} has no {} key yet: kindric-accounts token {name}", provider.provider.name())));
    }
    let shared_mcp = mcp::prepare(paths, name)?;
    print_warnings(&shared_mcp.warnings);
    let conversation_dir = conversation_dir(claude_args);
    let prepared = launch::prepare(paths, name, shared_mcp.file.as_deref(), claude_args, conversation_dir.as_deref(), None)?;
    start(prepared)
}

/// Run by Claude Code for the status line of a session kindric-accounts started: writes down how
/// much of its limits the account used, then shows the user's own status line. Writing down
/// fails without hiding the status line.
fn cmd_statusline(paths: &Paths) -> Result<Exit> {
    let mut input = Vec::new();
    if let Err(error) = std::io::stdin().read_to_end(&mut input) {
        eprintln!("kindric-accounts statusline: {error}");
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR");
    if let Err(error) = usage::record(paths, config_dir.as_deref(), &input, now) {
        eprintln!("kindric-accounts statusline: usage not written: {error}");
    }
    match usage::run_user_statusline(paths, &input) {
        Ok(Some((output, code))) => {
            let mut stdout = std::io::stdout();
            if stdout.write_all(&output).and_then(|()| stdout.flush()).is_err() {
                return Ok(Exit::Code(1));
            }
            Ok(if code == 0 { Exit::Success } else { Exit::Code(code) })
        },
        Ok(None) => Ok(Exit::Success),
        Err(error) => Err(Error(format!("your status line could not run: {error}"))),
    }
}

fn dispatch(args: &[String], raw_args: &[OsString]) -> Result<Exit> {
    let command = args.first().map(String::as_str).unwrap_or("");
    match command {
        "-V" | "--version" => {
            println!("kindric-accounts {VERSION}");
            return Ok(Exit::Success);
        },
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            return Ok(Exit::Success);
        },
        _ => {},
    }
    let paths = Paths::from_env()?;
    let name = args.get(1).filter(|name| !name.is_empty());
    match (command, name) {
        ("", _) => menu::run(&paths),
        ("add", Some(name)) if name == "--help" || name == "-h" => {
            print!("{USAGE}");
            Ok(Exit::Success)
        },
        ("add", Some(name)) => cmd_add(&paths, name, &args[2..]),
        ("token", Some(name)) => cmd_token(&paths, name, &args[2..]),
        ("list", _) => cmd_list(&paths, &args[1..]),
        ("status", _) => cmd_status(&paths, &args[1..]),
        ("remove", Some(name)) => cmd_remove(&paths, name, &args[2..]),
        ("run", Some(name)) => cmd_run(&paths, name, &raw_args[2..]),
        ("isolate", Some(name)) => cmd_isolate(&paths, name),
        ("unisolate", Some(name)) => cmd_unisolate(&paths, name),
        ("isolated", _) => cmd_isolated(&paths, &args[1..]),
        (usage::SUBCOMMAND, _) => cmd_statusline(&paths),
        _ => Ok(Exit::Usage),
    }
}

fn main() -> ExitCode {
    let raw_args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let args: Vec<String> = raw_args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
    match dispatch(&args, &raw_args) {
        Ok(Exit::Success) => ExitCode::SUCCESS,
        Ok(Exit::Code(code)) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Ok(Exit::Usage) => {
            eprint!("{USAGE}");
            ExitCode::FAILURE
        },
        Err(error) => {
            eprintln!("kindric-accounts: {error}");
            ExitCode::FAILURE
        },
    }
}
