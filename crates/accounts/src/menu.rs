//! Interactive menu: pick an account and start Claude Code in this terminal.

use std::io::{self, IsTerminal};

use claude_accounts::status::{AccountStatus, LoginState};
use claude_accounts::{Paths, Result, Error, registry, status};
use dialoguer::console::{Term, style};
use dialoguer::{FuzzySelect, Input};

use crate::{DirChoice, Exit};

const ADD: &str = "+ Add an account";
const REMOVE: &str = "- Remove an account";
const BACK: &str = "Back";
const QUIT: &str = "Quit";

fn header(count: usize) -> String {
    let label = match count {
        0 => "No accounts yet".to_string(),
        1 => "1 account ready to launch".to_string(),
        _ => format!("{count} accounts ready to launch"),
    };
    format!("claude-accounts\n{label}\n")
}

fn badge(status: &AccountStatus) -> String {
    let mut badge = match (status.state, &status.plan) {
        (LoginState::LoggedIn, Some(plan)) => format!("logged in · {plan}"),
        (LoginState::LoggedIn, None) => "logged in".to_string(),
        (LoginState::LoggedOut, _) => "logged out".to_string(),
        (LoginState::Unknown, _) => "status unknown".to_string(),
    };
    if status.sessions > 0 {
        badge.push_str(&format!(" · {} active", status.sessions));
    }
    if status.isolated {
        badge.push_str(" · isolated");
    }
    badge
}

/// One line per account; plain names when Claude Code cannot be asked for a status.
pub fn labels(paths: &Paths, names: &[String]) -> Vec<String> {
    let Ok(statuses) = status::collect(paths, names) else {
        return names.to_vec();
    };
    let width = names.iter().map(String::len).max().unwrap_or(0);
    statuses.iter().map(|status| format!("{:<width$}  {}", status.name, badge(status))).collect()
}

fn dialog_error(error: dialoguer::Error) -> Error {
    Error(format!("menu: {error}"))
}

/// Shows `header` and lets the user pick an item; `None` on Escape.
fn choose(header: &str, items: &[String], prompt: &str) -> Result<Option<usize>> {
    let term = Term::stderr();
    eprint!("{header}");
    let choice = FuzzySelect::new()
        .with_prompt(prompt)
        .items(items)
        .default(0)
        .highlight_matches(false)
        .interact_on_opt(&term)
        .map_err(dialog_error);
    let _ = term.clear_last_lines(header.lines().count());
    choice
}

fn show_message(message: &str) {
    eprintln!("{}", style(message).red());
}

/// Asks for a new account name until it is valid and free; `None` if left empty.
fn read_new_name(paths: &Paths) -> Result<Option<String>> {
    loop {
        let name: String = Input::new()
            .with_prompt("New account name (empty to go back)")
            .allow_empty(true)
            .interact_text_on(&Term::stderr())
            .map_err(dialog_error)?;
        let name = name.trim().to_string();
        if name.is_empty() {
            return Ok(None);
        }
        if !registry::is_valid_name(&name) {
            show_message("Invalid name: use 1 to 32 characters among a-z, 0-9, _ and -.");
            continue;
        }
        if registry::account_exists(paths, &name)? {
            show_message(&format!("Account '{name}' already exists."));
            continue;
        }
        return Ok(Some(name));
    }
}

fn remove_from_menu(paths: &Paths, names: &[String]) -> Result<()> {
    let mut items = names.to_vec();
    items.push(BACK.to_string());
    let header = "Remove an account\nCredentials and history can be kept.\n";
    let Some(index) = choose(header, &items, "Account to remove")? else {
        return Ok(());
    };
    if let Some(name) = names.get(index) {
        crate::cmd_remove_with(paths, name, DirChoice::Ask)?;
    }
    Ok(())
}

pub fn run(paths: &Paths) -> Result<Exit> {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return Err(Error("the menu needs an interactive terminal".to_string()));
    }
    loop {
        let names = registry::list_accounts(paths)?;
        let mut items = labels(paths, &names);
        items.push(ADD.to_string());
        if !names.is_empty() {
            items.push(REMOVE.to_string());
        }
        items.push(QUIT.to_string());
        let prompt = "Choose an account to open Claude in this terminal";
        let Some(index) = choose(&header(names.len()), &items, prompt)? else {
            return Ok(Exit::Success);
        };
        if let Some(name) = names.get(index) {
            return crate::cmd_run(paths, name, &[]);
        }
        match items[index].as_str() {
            ADD => {
                if let Some(name) = read_new_name(paths)? {
                    crate::cmd_add(paths, &name)?;
                    return crate::cmd_run(paths, &name, &[]);
                }
            },
            REMOVE => remove_from_menu(paths, &names)?,
            _ => return Ok(Exit::Success),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn status(state: LoginState, plan: Option<&str>, sessions: usize, isolated: bool) -> AccountStatus {
        AccountStatus { name: "x".into(), state, sessions, plan: plan.map(Into::into), dir: PathBuf::new(), isolated }
    }

    #[test]
    fn badges_match_1x() {
        assert_eq!(badge(&status(LoginState::LoggedIn, Some("pro"), 1, false)), "logged in · pro · 1 active");
        assert_eq!(badge(&status(LoginState::LoggedOut, None, 0, true)), "logged out · isolated");
        assert_eq!(badge(&status(LoginState::Unknown, None, 0, false)), "status unknown");
    }

    #[test]
    fn header_counts_accounts() {
        assert!(header(0).contains("No accounts yet"));
        assert!(header(1).contains("1 account ready"));
        assert!(header(3).contains("3 accounts ready"));
    }
}
