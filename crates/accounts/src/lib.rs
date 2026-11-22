//! kindric-accounts: several Claude Code, Codex and OpenCode accounts, one shared configuration.
//! Each Claude Code account has its own `CLAUDE_CONFIG_DIR` (credentials, history, sessions);
//! `CLAUDE.md`, skills, agents, commands, plugins and themes are shared with `~/.claude`.
//! A Codex account has its own `CODEX_HOME`, and shares `AGENTS.md`, skills and prompts with
//! `~/.codex`. An OpenCode account has its own XDG directories, and shares `AGENTS.md`, agents
//! and commands with the user's OpenCode configuration. The library never reads credentials and
//! makes no network requests: it only talks to the local daemon of a Codex account.

pub mod account;
pub mod agent;
pub mod codex;
pub mod error;
pub mod files;
pub mod generate;
pub mod injection;
pub mod isolation;
pub mod launch;
pub mod mcp;
pub mod memory;
pub mod opencode;
pub mod paths;
pub mod provider;
pub mod registry;
pub mod shared;
pub mod status;
pub mod theme;
pub mod usage;

pub use error::{Error, Result};
pub use paths::Paths;
