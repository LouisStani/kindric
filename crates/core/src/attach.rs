//! The commands that show a background session in a terminal, stop it or remove it, the same for every
//! interface, and the one that lets Claude Code ask whether a folder is trusted.

use std::path::{Path, PathBuf};
use std::process::Command;

use kindric_accounts::{Paths, codex};

use crate::accounts::{self, LAUNCHER, default_removed_env};

pub struct AttachCommand {
    pub program: String,
    pub args: Vec<String>,
    /// Claude Code draws in colour only when the terminal says it can.
    pub env: Vec<(String, String)>,
    /// Variables the program must not inherit.
    pub env_remove: Vec<String>,
    /// Where the program starts; None keeps the app's own directory.
    pub directory: Option<PathBuf>,
}

fn colours() -> Vec<(String, String)> {
    vec![("TERM".into(), "xterm-256color".into()), ("COLORTERM".into(), "truecolor".into())]
}

/// `kindric-accounts run <account> attach <short_id>`: kindric-accounts sets the account up,
/// then Claude Code attaches. On the implicit `default`, `claude attach <short_id>` without
/// `CLAUDE_CONFIG_DIR`. Ending this process only detaches; the session keeps running.
pub fn attach_command(account: &str, short_id: &str, implicit_default: bool) -> AttachCommand {
    if implicit_default {
        return AttachCommand {
            program: "claude".into(),
            args: ["attach", short_id].map(String::from).to_vec(),
            env: colours(),
            env_remove: default_removed_env().into_iter().map(String::from).collect(),
            directory: None,
        };
    }
    AttachCommand {
        program: LAUNCHER.into(),
        args: ["run", account, "attach", short_id].map(String::from).to_vec(),
        env: colours(),
        env_remove: Vec::new(),
        directory: None,
    }
}

/// `kindric-accounts run <account> resume <id> --remote unix://<socket>`: the Codex TUI joins the
/// session in the daemon of the account in `dir`. Ending this process leaves the session, and a
/// turn it runs, in the daemon. Windows has no socket: the TUI finds the daemon by itself.
/// `cwd`, the folder of the session: started elsewhere, the TUI first asks which folder to use.
pub fn codex_attach_command(account: &str, thread_id: &str, dir: &Path, cwd: Option<PathBuf>) -> AttachCommand {
    let mut args: Vec<String> = ["run", account, "resume", thread_id].map(String::from).to_vec();
    if cfg!(unix) {
        args.extend(["--remote".to_string(), codex::remote_address(dir)]);
    }
    let directory = cwd.filter(|cwd| cwd.is_dir());
    AttachCommand { program: LAUNCHER.into(), args, env: colours(), env_remove: Vec::new(), directory }
}

/// The command that shows the session `short_id` of `account` in a pane, whatever its account
/// runs. For Codex, the daemon of the account is started first when it does not run: a session at
/// rest lives on disk until a client joins it.
pub fn session_attach_command(account: &str, short_id: &str, implicit_default: bool) -> Result<AttachCommand, String> {
    let paths = Paths::from_env().map_err(|error| error.to_string())?;
    if implicit_default || !accounts::is_codex(&paths, account) {
        return Ok(attach_command(account, short_id, implicit_default));
    }
    let dir = paths.account_dir(account);
    if cfg!(unix) {
        let codex = codex::require_codex().map_err(|error| error.to_string())?;
        codex::ensure_daemon(&codex, &dir).map_err(|error| format!("{account}: {error}"))?;
    }
    let cwd = codex::session_cwd(&dir, short_id).map_err(|error| format!("{account}: {error}"))?;
    Ok(codex_attach_command(account, short_id, &dir, cwd))
}

/// Whether `account` is an account of kindric-accounts that runs Codex.
pub fn is_codex_account(account: &str, implicit_default: bool) -> bool {
    !implicit_default && Paths::from_env().is_ok_and(|paths| accounts::is_codex(&paths, account))
}

/// `kindric-accounts run <account> queue --thread <id> --message <text> --resume`: Codex keeps
/// the message for the session and starts a turn with it as soon as it is free, waking it up when
/// it was at rest. Typing into its screen, as for Claude Code, is not needed.
pub fn codex_message_command(account: &str, thread_id: &str, text: &str, dir: &Path) -> Command {
    let mut command = Command::new(LAUNCHER);
    command.args(["run", account, "queue", "--thread", thread_id, "--message", text, "--resume"]);
    if cfg!(unix) {
        command.args(["--remote", &codex::remote_address(dir)]);
    }
    command
}

/// Sends `text` to the Codex session `thread_id` of `account` as the user's message.
pub fn send_codex_message(account: &str, thread_id: &str, text: &str) -> Result<(), String> {
    let paths = Paths::from_env().map_err(|error| error.to_string())?;
    let dir = paths.account_dir(account);
    if cfg!(unix) {
        let codex = codex::require_codex().map_err(|error| error.to_string())?;
        codex::ensure_daemon(&codex, &dir).map_err(|error| format!("{account}: {error}"))?;
    }
    let output = codex_message_command(account, thread_id, text, &dir).output().map_err(|error| format!("{LAUNCHER}: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    Err(format!("cannot send the message to {thread_id}: {}", said.trim()))
}

/// Claude Code itself, not in the background, in `directory`: `claude --bg` refuses a folder the
/// account has not trusted, and only this asks. Claude Code keeps the answer in its own files.
/// `kindric-accounts run <account>`, or `claude` without `CLAUDE_CONFIG_DIR` on the implicit
/// `default`.
pub fn trust_command(account: &str, directory: &Path, implicit_default: bool) -> AttachCommand {
    if implicit_default {
        return AttachCommand {
            program: "claude".into(),
            args: Vec::new(),
            env: colours(),
            env_remove: default_removed_env().into_iter().map(String::from).collect(),
            directory: Some(directory.to_path_buf()),
        };
    }
    AttachCommand {
        program: LAUNCHER.into(),
        args: ["run", account].map(String::from).to_vec(),
        env: colours(),
        env_remove: Vec::new(),
        directory: Some(directory.to_path_buf()),
    }
}

/// `kindric-accounts run <account> auth login`: Claude Code signs the account in and keeps the
/// credentials in the account's own directory. Accounts of kindric-accounts only.
pub fn login_command(account: &str, codex: bool) -> AttachCommand {
    // Claude Code signs in with `claude auth login`, Codex with `codex login`.
    let login: &[&str] = if codex { &["login"] } else { &["auth", "login"] };
    AttachCommand {
        program: LAUNCHER.into(),
        args: ["run", account].iter().chain(login).map(|arg| (*arg).to_string()).collect(),
        env: colours(),
        env_remove: Vec::new(),
        directory: None,
    }
}

/// `kindric-accounts run <account> stop <short_id>`, or `claude stop <short_id>` on the
/// implicit `default`: the session stops and keeps its conversation; `attach` starts it again.
pub fn stop_command(account: &str, short_id: &str, implicit_default: bool) -> Command {
    session_command(account, "stop", short_id, implicit_default)
}

/// `kindric-accounts run <account> rm <short_id>`, or `claude rm <short_id>` on the implicit
/// `default`: the session stops if it runs, leaves Claude Code's list, its folder `jobs/<id>/`
/// goes, its conversation stays for `claude --resume`. Claude Code refuses while its worktree
/// holds commits not pushed.
pub fn remove_command(account: &str, short_id: &str, implicit_default: bool) -> Command {
    session_command(account, "rm", short_id, implicit_default)
}

fn session_command(account: &str, verb: &str, short_id: &str, implicit_default: bool) -> Command {
    if implicit_default {
        let mut command = Command::new("claude");
        for variable in default_removed_env() {
            command.env_remove(variable);
        }
        command.args([verb, short_id]);
        return command;
    }
    let mut command = Command::new(LAUNCHER);
    command.args(["run", account, verb, short_id]);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attaches_through_kindric_accounts_with_colours() {
        let command = attach_command("lystriq", "378ce649", false);
        assert_eq!(command.program, "kindric-accounts");
        assert_eq!(command.args, ["run", "lystriq", "attach", "378ce649"]);
        assert!(command.env.contains(&("TERM".into(), "xterm-256color".into())));
        assert!(command.env.contains(&("COLORTERM".into(), "truecolor".into())));
        assert!(command.env_remove.is_empty());
        assert_eq!(command.directory, None);
    }

    #[test]
    fn asks_for_trust_through_kindric_accounts_in_the_folder() {
        let command = trust_command("lystriq", Path::new("/work/api"), false);
        assert_eq!(command.program, "kindric-accounts");
        assert_eq!(command.args, ["run", "lystriq"]);
        assert_eq!(command.directory, Some(PathBuf::from("/work/api")));
        assert!(command.env.contains(&("TERM".into(), "xterm-256color".into())));
        assert!(command.env_remove.is_empty());
    }

    #[test]
    fn a_codex_session_is_joined_through_its_daemon() {
        let dir = Path::new("/d/kindric-accounts/cx");
        let command = codex_attach_command("cx", "01a1ff52", dir, Some(std::env::temp_dir()));
        assert_eq!(command.directory, Some(std::env::temp_dir()), "the TUI starts in the folder of the session");
        assert_eq!(codex_attach_command("cx", "01a1ff52", dir, Some(PathBuf::from("/gone/away"))).directory, None);
        assert_eq!(command.program, "kindric-accounts");
        if cfg!(unix) {
            assert_eq!(command.args, ["run", "cx", "resume", "01a1ff52", "--remote", "unix:///d/kindric-accounts/cx/app-server-control/app-server-control.sock"]);
        } else {
            assert_eq!(command.args, ["run", "cx", "resume", "01a1ff52"]);
        }
        assert!(command.env.contains(&("TERM".into(), "xterm-256color".into())));
        let message = codex_message_command("cx", "01a1ff52", "go on", dir);
        let args: Vec<_> = message.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert_eq!(&args[..8], ["run", "cx", "queue", "--thread", "01a1ff52", "--message", "go on", "--resume"]);
    }

    #[test]
    fn logs_in_through_kindric_accounts() {
        let command = login_command("work", false);
        assert_eq!(command.program, "kindric-accounts");
        assert_eq!(command.args, ["run", "work", "auth", "login"]);
        assert_eq!(login_command("cx", true).args, ["run", "cx", "login"]);
        assert!(command.env.contains(&("TERM".into(), "xterm-256color".into())));
        assert!(command.env_remove.is_empty());
        assert_eq!(command.directory, None);
    }

    #[test]
    fn the_implicit_default_asks_for_trust_with_claude_alone() {
        let command = trust_command("default", Path::new("/work/api"), true);
        assert_eq!(command.program, "claude");
        assert!(command.args.is_empty());
        assert_eq!(command.directory, Some(PathBuf::from("/work/api")));
        assert!(command.env_remove.contains(&"CLAUDE_CONFIG_DIR".to_string()));
    }

    #[test]
    fn the_implicit_default_attaches_with_claude_alone() {
        let command = attach_command("default", "378ce649", true);
        assert_eq!(command.program, "claude");
        assert_eq!(command.args, ["attach", "378ce649"]);
        assert!(command.env.contains(&("COLORTERM".into(), "truecolor".into())));
        assert!(command.env_remove.contains(&"CLAUDE_CONFIG_DIR".to_string()));
        assert!(command.env_remove.contains(&"ANTHROPIC_API_KEY".to_string()));
    }

    #[test]
    fn stops_through_kindric_accounts() {
        let command = stop_command("lystriq", "378ce649", false);
        assert_eq!(command.get_program(), "kindric-accounts");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["run", "lystriq", "stop", "378ce649"]);
    }

    #[test]
    fn the_implicit_default_stops_with_claude_alone() {
        let command = stop_command("default", "378ce649", true);
        assert_eq!(command.get_program(), "claude");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["stop", "378ce649"]);
        let removed: Vec<_> = command.get_envs().filter(|(_, value)| value.is_none()).map(|(name, _)| name).collect();
        assert!(removed.contains(&std::ffi::OsStr::new("CLAUDE_CONFIG_DIR")));
    }

    #[test]
    fn removes_like_it_stops() {
        let command = remove_command("lystriq", "378ce649", false);
        assert_eq!(command.get_program(), "kindric-accounts");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["run", "lystriq", "rm", "378ce649"]);
        let default = remove_command("default", "378ce649", true);
        assert_eq!(default.get_program(), "claude");
        assert_eq!(default.get_args().collect::<Vec<_>>(), ["rm", "378ce649"]);
    }
}
