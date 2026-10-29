//! Where claude-accounts keeps its files. Unix paths are the ones of the 1.x Bash
//! script, so existing accounts work without any migration.

use std::path::PathBuf;

use crate::error::{Result, fail};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// Account list and isolated list.
    pub config_dir: PathBuf,
    /// One `CLAUDE_CONFIG_DIR` per account, plus `.shared-mcp/`.
    pub data_root: PathBuf,
    /// `~/.claude`, the configuration every account shares.
    pub shared_root: PathBuf,
}

/// A variable that is set but empty counts as unset, like `${VAR:-default}` in Bash.
#[cfg(unix)]
fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from)
}

impl Paths {
    #[cfg(unix)]
    pub fn from_env() -> Result<Self> {
        let Some(home) = env_path("HOME") else {
            return fail("HOME is not set");
        };
        let config = env_path("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let data = env_path("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        Ok(Self::from_roots(config, data, home))
    }

    #[cfg(windows)]
    pub fn from_env() -> Result<Self> {
        let (Some(home), Some(config), Some(data)) = (dirs::home_dir(), dirs::config_dir(), dirs::data_dir()) else {
            return fail("cannot find the user profile and AppData directories");
        };
        Ok(Self::from_roots(config, data, home))
    }

    pub fn from_roots(config: PathBuf, data: PathBuf, home: PathBuf) -> Self {
        Self {
            config_dir: config.join("claude-accounts"),
            data_root: data.join("claude-accounts"),
            shared_root: home.join(".claude"),
        }
    }

    pub fn registry_file(&self) -> PathBuf {
        self.config_dir.join("accounts")
    }

    pub fn isolated_file(&self) -> PathBuf {
        self.config_dir.join("isolated")
    }

    pub fn account_dir(&self, name: &str) -> PathBuf {
        self.data_root.join(name)
    }

    pub fn shared_mcp_file(&self, name: &str) -> PathBuf {
        self.data_root.join(".shared-mcp").join(format!("{name}.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_the_bash_script() {
        let paths = Paths::from_roots("/h/.config".into(), "/h/.local/share".into(), "/h".into());
        assert_eq!(paths.registry_file(), PathBuf::from("/h/.config/claude-accounts/accounts"));
        assert_eq!(paths.isolated_file(), PathBuf::from("/h/.config/claude-accounts/isolated"));
        assert_eq!(paths.account_dir("work"), PathBuf::from("/h/.local/share/claude-accounts/work"));
        assert_eq!(
            paths.shared_mcp_file("work"),
            PathBuf::from("/h/.local/share/claude-accounts/.shared-mcp/work.json")
        );
        assert_eq!(paths.shared_root, PathBuf::from("/h/.claude"));
    }
}
