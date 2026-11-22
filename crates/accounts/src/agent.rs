//! Which program an account runs: Claude Code, OpenAI's Codex (since 2.3.0) or OpenCode (since 2.4.0).
//!
//! `<account>/kindric-agent.json` names it. An account without the file runs Claude Code, so the
//! accounts made before 2.3.0 stay as they were. A Codex account's directory is its `CODEX_HOME`;
//! an OpenCode account's holds the four XDG directories OpenCode runs with.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::files;

const AGENT_FILE: &str = "kindric-agent.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Agent {
    #[default]
    ClaudeCode,
    Codex,
    #[serde(rename = "opencode")]
    OpenCode,
}

impl Agent {
    pub const NAMES: [&str; 3] = ["claude-code", "codex", "opencode"];

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "claude-code" => Some(Self::ClaudeCode),
            "codex" => Some(Self::Codex),
            "opencode" => Some(Self::OpenCode),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }
}

#[derive(Serialize, Deserialize)]
struct AgentFile {
    agent: Agent,
}

pub fn agent_file(dir: &Path) -> PathBuf {
    dir.join(AGENT_FILE)
}

/// The program of the account in `dir`; Claude Code when the file is missing.
pub fn read(dir: &Path) -> Result<Agent> {
    let file = agent_file(dir);
    let text = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Agent::ClaudeCode),
        Err(error) => return Err(Error(format!("cannot read {}: {error}", file.display()))),
    };
    serde_json::from_str::<AgentFile>(&text)
        .map(|content| content.agent)
        .map_err(|error| Error(format!("{} is not valid: {error}; remove the account and add it again", file.display())))
}

/// Writes the file only for an agent other than Claude Code, which needs none.
pub fn write(dir: &Path, agent: Agent) -> Result<()> {
    if agent == Agent::ClaudeCode {
        return Ok(());
    }
    let file = agent_file(dir);
    let text = serde_json::to_string_pretty(&AgentFile { agent }).map_err(|error| Error(error.to_string()))?;
    files::write_atomic(&file, &format!("{text}\n")).map_err(|error| Error(format!("cannot write {}: {error}", file.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::test_dir;

    #[test]
    fn an_account_without_the_file_runs_claude_code() {
        let dir = test_dir::fresh("agent-missing");
        assert_eq!(read(&dir).unwrap(), Agent::ClaudeCode);
        write(&dir, Agent::ClaudeCode).unwrap();
        assert!(!agent_file(&dir).exists(), "Claude Code needs no file");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_codex_account_keeps_its_agent() {
        let dir = test_dir::fresh("agent-codex");
        write(&dir, Agent::Codex).unwrap();
        assert_eq!(fs::read_to_string(agent_file(&dir)).unwrap(), "{\n  \"agent\": \"codex\"\n}\n");
        assert_eq!(read(&dir).unwrap(), Agent::Codex);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_opencode_account_keeps_its_agent() {
        let dir = test_dir::fresh("agent-opencode");
        write(&dir, Agent::OpenCode).unwrap();
        assert_eq!(fs::read_to_string(agent_file(&dir)).unwrap(), "{\n  \"agent\": \"opencode\"\n}\n");
        assert_eq!(read(&dir).unwrap(), Agent::OpenCode);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_broken_file_is_an_error_that_says_what_to_do() {
        let dir = test_dir::fresh("agent-broken");
        fs::write(agent_file(&dir), r#"{"agent": "vim"}"#).unwrap();
        let error = read(&dir).unwrap_err();
        assert!(error.0.contains("remove the account and add it again"), "{error}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn names_are_parsed_back() {
        for name in Agent::NAMES {
            assert_eq!(Agent::parse(name).map(Agent::name), Some(name));
        }
        assert_eq!(Agent::parse("claude"), None);
    }
}
