//! claude-accounts: several Claude Code accounts, one shared configuration.
//! Each account has its own `CLAUDE_CONFIG_DIR` (credentials, history, sessions);
//! `CLAUDE.md`, skills, agents, commands, plugins and themes are shared with `~/.claude`.
//! The library never reads credentials and makes no network requests.

pub mod account;
pub mod error;
pub mod files;
pub mod isolation;
pub mod launch;
pub mod mcp;
pub mod paths;
pub mod registry;
pub mod shared;
pub mod status;

pub use error::{Error, Result};
pub use paths::Paths;
