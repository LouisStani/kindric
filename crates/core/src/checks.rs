//! What the welcome screen checks before the first session: Claude Code and its version,
//! claude-accounts, and the MCP server that carries messages between sessions.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use claude_accounts::{Paths, launch, registry};
use serde::Serialize;
use serde_json::Value;

use crate::accounts;

/// The Claude Code version claude-sessions was tested with. Older ones get a warning only:
/// the oldest one that works is not known.
pub const TESTED_CLAUDE: (u32, u32, u32) = (2, 1, 312);

/// Name of the messages server in Claude Code's MCP configuration.
pub const MESSAGES_SERVER: &str = "claude-sessions";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClaudeCheck {
    /// What `claude --version` printed, trimmed; None when it could not run.
    pub version: Option<String>,
    /// At least `TESTED_CLAUDE`. False when the version could not be read.
    pub tested: bool,
    pub error: Option<String>,
}

/// Whether the messages server is configured, on each side that needs it: claude-accounts
/// shares it between its accounts, but never with `default`, which needs its own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MessagesCheck {
    /// An account of claude-accounts that is not isolated has it in its user scope.
    pub accounts: bool,
    /// `~/.claude.json` has it in its user scope.
    pub default: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Checks {
    pub claude: ClaudeCheck,
    /// Why claude-accounts cannot be used (missing, too old), if so.
    pub launcher_error: Option<String>,
    pub messages: MessagesCheck,
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
    let accounts = registry::list_accounts(paths)
        .map_err(|error| error.to_string())?
        .iter()
        .filter(|account| !isolated.contains(account))
        .any(|account| has_messages_server(&paths.account_dir(account).join(".claude.json")));
    let default = default_state_file(paths).is_some_and(|file| has_messages_server(&file));
    Ok(MessagesCheck { accounts, default })
}

/// Every check, about a second at most: `claude --version` and `claude-accounts --version`.
pub fn run() -> Result<Checks, String> {
    let paths = accounts::paths().map_err(|error| error.to_string())?;
    Ok(Checks {
        claude: check_claude(),
        launcher_error: accounts::check_launcher_version().err().map(|error| error.to_string()),
        messages: check_messages(&paths)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

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
        let root = std::env::temp_dir().join(format!("claude-sessions-checks-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch directory");
        root
    }

    const WITH_SERVER: &str = r#"{"mcpServers":{"claude-sessions":{"type":"stdio","command":"claude-sessions","args":["mcp"]}}}"#;

    #[test]
    fn only_the_user_scope_counts() {
        let root = scratch("scope");
        let file = root.join(".claude.json");
        assert!(!has_messages_server(&file), "missing file");
        fs::write(&file, WITH_SERVER).unwrap();
        assert!(has_messages_server(&file));
        fs::write(&file, r#"{"mcpServers":{"sentry":{"type":"http","url":"https://mcp.sentry.dev/mcp"}}}"#).unwrap();
        assert!(!has_messages_server(&file), "another server only");
        fs::write(&file, r#"{"projects":{"/p":{"mcpServers":{"claude-sessions":{}}}}}"#).unwrap();
        assert!(!has_messages_server(&file), "project scope");
        fs::write(&file, "{").unwrap();
        assert!(!has_messages_server(&file), "broken file");
        fs::remove_dir_all(&root).unwrap();
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
        assert_eq!(check_messages(&paths).unwrap(), MessagesCheck { accounts: false, default: false });

        fs::write(paths.account_dir("work").join(".claude.json"), WITH_SERVER).unwrap();
        fs::write(paths.isolated_file(), "work\n").unwrap();
        assert_eq!(check_messages(&paths).unwrap(), MessagesCheck { accounts: false, default: false }, "isolated");

        fs::write(paths.account_dir("perso").join(".claude.json"), WITH_SERVER).unwrap();
        assert_eq!(check_messages(&paths).unwrap(), MessagesCheck { accounts: true, default: false });

        fs::write(root.join("home").join(".claude.json"), WITH_SERVER).unwrap();
        assert_eq!(check_messages(&paths).unwrap(), MessagesCheck { accounts: true, default: true });
        fs::remove_dir_all(&root).unwrap();
    }
}
