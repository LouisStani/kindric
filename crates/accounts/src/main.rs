mod menu;

use std::ffi::OsString;
use std::io::{self, BufRead, IsTerminal, Write};
use std::process::ExitCode;

use claude_accounts::{Paths, Result, account, isolation, launch, mcp, registry, status};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
Usage:
  claude-accounts                   menu: pick an account and start claude here
  claude-accounts add <name>        create the account and its directory (700)
  claude-accounts list [--json]     list accounts
  claude-accounts status [--json]   show login status, plan and active sessions of each account
  claude-accounts remove <name> [--delete-dir|--keep-dir]
                                    remove the account (directory deleted only if confirmed)
  claude-accounts run <name> [...]  start claude with this account here (args passed to claude)
  claude-accounts isolate <name>    stop sharing MCP servers with and from this account
  claude-accounts unisolate <name>  share MCP servers again
  claude-accounts isolated [--json] list isolated accounts
  claude-accounts --version         print the version
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
        eprintln!("claude-accounts: {warning}");
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

fn cmd_add(paths: &Paths, name: &str) -> Result<Exit> {
    let created = account::add(paths, name)?;
    print_warnings(&created.warnings);
    println!("Account \"{name}\" created: {}", created.dir.display());
    println!("First launch: claude-accounts run {name}, then /login");
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
        eprintln!("No accounts. Create one: claude-accounts add <name>");
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
    serde_json::to_string(value).map_err(|error| claude_accounts::Error(error.to_string()))
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

fn cmd_run(paths: &Paths, name: &str, claude_args: &[OsString]) -> Result<Exit> {
    launch::require_account(paths, name)?;
    launch::require_claude()?;
    let shared_mcp = mcp::prepare(paths, name)?;
    print_warnings(&shared_mcp.warnings);
    let prepared = launch::prepare(paths, name, shared_mcp.file.as_deref(), claude_args)?;
    print_warnings(&prepared.warnings);
    #[cfg(unix)]
    return Err(launch::exec(prepared.command));
    #[cfg(windows)]
    return launch::run_and_wait(prepared.command).map(Exit::Code);
}

fn dispatch(args: &[String], raw_args: &[OsString]) -> Result<Exit> {
    let command = args.first().map(String::as_str).unwrap_or("");
    match command {
        "-V" | "--version" => {
            println!("claude-accounts {VERSION}");
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
        ("add", Some(name)) => cmd_add(&paths, name),
        ("list", _) => cmd_list(&paths, &args[1..]),
        ("status", _) => cmd_status(&paths, &args[1..]),
        ("remove", Some(name)) => cmd_remove(&paths, name, &args[2..]),
        ("run", Some(name)) => cmd_run(&paths, name, &raw_args[2..]),
        ("isolate", Some(name)) => cmd_isolate(&paths, name),
        ("unisolate", Some(name)) => cmd_unisolate(&paths, name),
        ("isolated", _) => cmd_isolated(&paths, &args[1..]),
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
            eprintln!("claude-accounts: {error}");
            ExitCode::FAILURE
        },
    }
}
