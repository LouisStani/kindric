//! The commands that show a background session in a terminal or stop it, the same for every
//! interface, and the one that lets Claude Code ask whether a folder is trusted.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::accounts::{LAUNCHER, default_removed_env};

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
pub fn login_command(account: &str) -> AttachCommand {
    AttachCommand {
        program: LAUNCHER.into(),
        args: ["run", account, "auth", "login"].map(String::from).to_vec(),
        env: colours(),
        env_remove: Vec::new(),
        directory: None,
    }
}

/// `kindric-accounts run <account> stop <short_id>`, or `claude stop <short_id>` on the
/// implicit `default`: the session stops and keeps its conversation; `attach` starts it again.
pub fn stop_command(account: &str, short_id: &str, implicit_default: bool) -> Command {
    if implicit_default {
        let mut command = Command::new("claude");
        for variable in default_removed_env() {
            command.env_remove(variable);
        }
        command.args(["stop", short_id]);
        return command;
    }
    let mut command = Command::new(LAUNCHER);
    command.args(["run", account, "stop", short_id]);
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
    fn logs_in_through_kindric_accounts() {
        let command = login_command("work");
        assert_eq!(command.program, "kindric-accounts");
        assert_eq!(command.args, ["run", "work", "auth", "login"]);
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
}
