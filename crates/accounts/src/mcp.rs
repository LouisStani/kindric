//! MCP servers shared between accounts. At each launch, the user-scope servers of the
//! other accounts are merged into `.shared-mcp/<account>.json` and passed with
//! `--mcp-config`. Claude Code's own files are only read, never written.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::error::Result;
use crate::files;
use crate::paths::Paths;
use crate::registry;

type Servers = Map<String, Value>;

/// Written instead of deleting: a background session keeps the `--mcp-config` path it
/// started with, and Claude Code refuses to start when that file is missing.
const EMPTY: &str = "{\"mcpServers\":{}}\n";

enum State {
    Missing,
    Unreadable,
    Read(Value),
}

fn read_state(paths: &Paths, name: &str) -> State {
    match fs::read_to_string(paths.account_dir(name).join(".claude.json")) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => State::Missing,
        Err(_) => State::Unreadable,
        Ok(text) => serde_json::from_str(&text).map_or(State::Unreadable, State::Read),
    }
}

fn servers_of(value: Option<&Value>) -> Servers {
    value.and_then(Value::as_object).cloned().unwrap_or_default()
}

/// Servers an account gives to the others: its user scope only.
fn user_servers(state: &Value) -> Servers {
    servers_of(state.get("mcpServers"))
}

/// Every server name the launched account defines itself, user or project scope:
/// those are never replaced by a shared one, so the account keeps its own login.
fn own_servers(state: &Value) -> Servers {
    let mut servers = user_servers(state);
    if let Some(projects) = state.get("projects").and_then(Value::as_object) {
        for project in projects.values() {
            servers.extend(servers_of(project.get("mcpServers")));
        }
    }
    servers
}

/// Servers to share with `name`, and warnings about accounts that were skipped.
/// `None` when nothing can be shared safely (the account's own file is unreadable).
fn foreign_servers(paths: &Paths, name: &str) -> Result<Option<(Servers, Vec<String>)>> {
    let own = match read_state(paths, name) {
        State::Missing => Servers::new(),
        State::Unreadable => return Ok(None),
        State::Read(state) => own_servers(&state),
    };
    let isolated = registry::list_isolated(paths)?;
    let mut merged = Servers::new();
    let mut warnings = Vec::new();
    for other in registry::list_accounts(paths)? {
        if other == name || isolated.contains(&other) {
            continue;
        }
        match read_state(paths, &other) {
            State::Missing => {},
            State::Unreadable => warnings.push(format!("MCP servers of {other} skipped (unreadable .claude.json)")),
            State::Read(state) => {
                for (server, config) in user_servers(&state) {
                    merged.entry(server).or_insert(config);
                }
            },
        }
    }
    merged.retain(|server, _| !own.contains_key(server));
    Ok(Some((merged, warnings)))
}

pub struct SharedMcp {
    /// File to pass with `--mcp-config`, if any.
    pub file: Option<PathBuf>,
    pub warnings: Vec<String>,
}

fn write(file: &Path, content: &str) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        files::create_private_dir(dir)?;
    }
    files::write_atomic(file, content)
}

/// Prepares the shared MCP file of `name`. An isolated account gives nothing and gets
/// an empty file. A write failure means Claude Code starts without shared servers.
pub fn prepare(paths: &Paths, name: &str) -> Result<SharedMcp> {
    let file = paths.shared_mcp_file(name);
    if registry::is_isolated(paths, name)? {
        let file = write(&file, EMPTY).ok().map(|()| file);
        return Ok(SharedMcp { file, warnings: Vec::new() });
    }
    let Some((servers, warnings)) = foreign_servers(paths, name)? else {
        return Ok(SharedMcp { file: None, warnings: Vec::new() });
    };
    if servers.is_empty() {
        if file.exists() {
            let _ = write(&file, EMPTY);
        }
        return Ok(SharedMcp { file: None, warnings });
    }
    let content = format!("{}\n", serde_json::json!({ "mcpServers": servers }));
    let file = write(&file, &content).ok().map(|()| file);
    Ok(SharedMcp { file, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::test_dir;

    fn setup(label: &str, accounts: &[(&str, Option<&str>)]) -> (PathBuf, Paths) {
        let root = test_dir::fresh(label);
        let paths = Paths::from_roots(root.join("config"), root.join("data"), root.clone());
        files::create_private_dir(&paths.config_dir).unwrap();
        for (name, state) in accounts {
            files::append_line(&paths.registry_file(), name).unwrap();
            fs::create_dir_all(paths.account_dir(name)).unwrap();
            if let Some(state) = state {
                fs::write(paths.account_dir(name).join(".claude.json"), state).unwrap();
            }
        }
        (root, paths)
    }

    fn shared(paths: &Paths, name: &str) -> Value {
        serde_json::from_str(&fs::read_to_string(paths.shared_mcp_file(name)).unwrap()).unwrap()
    }

    #[test]
    fn first_account_wins_and_own_names_are_kept() {
        let (root, paths) = setup("merge", &[
            ("me", Some(r#"{"mcpServers":{"both":{"c":"own"}},"projects":{"/p":{"mcpServers":{"proj":{}}}}}"#)),
            ("a", Some(r#"{"mcpServers":{"alpha":{"c":"a"},"both":{"c":"a"},"proj":{"c":"a"}}}"#)),
            ("b", Some(r#"{"mcpServers":{"alpha":{"c":"b"},"beta":{"c":"b"}}}"#)),
        ]);
        let result = prepare(&paths, "me").unwrap();
        assert_eq!(result.file, Some(paths.shared_mcp_file("me")));
        assert_eq!(shared(&paths, "me")["mcpServers"], serde_json::json!({ "alpha": {"c": "a"}, "beta": {"c": "b"} }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn isolated_account_gives_nothing_and_gets_an_empty_file() {
        let (root, paths) = setup("isolated", &[
            ("me", Some("{}")),
            ("work", Some(r#"{"mcpServers":{"secret":{}}}"#)),
            ("perso", Some(r#"{"mcpServers":{"buffer":{}}}"#)),
        ]);
        files::append_line(&paths.isolated_file(), "work").unwrap();
        prepare(&paths, "me").unwrap();
        assert_eq!(shared(&paths, "me")["mcpServers"], serde_json::json!({ "buffer": {} }));
        let work = prepare(&paths, "work").unwrap();
        assert_eq!(work.file, Some(paths.shared_mcp_file("work")));
        assert_eq!(fs::read_to_string(paths.shared_mcp_file("work")).unwrap(), EMPTY);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nothing_to_share_empties_an_old_file_and_passes_none() {
        let (root, paths) = setup("empty", &[("me", None), ("other", Some(r#"{"mcpServers":{"x":{}}}"#))]);
        prepare(&paths, "me").unwrap();
        fs::write(paths.account_dir("other").join(".claude.json"), "{}").unwrap();
        let result = prepare(&paths, "me").unwrap();
        assert_eq!(result.file, None);
        assert_eq!(fs::read_to_string(paths.shared_mcp_file("me")).unwrap(), EMPTY);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unreadable_files_are_skipped_or_stop_sharing() {
        let (root, paths) = setup("broken", &[
            ("me", Some("{}")),
            ("bad", Some(r#"{"mcpServers": {"#)),
            ("good", Some(r#"{"mcpServers":{"g":{}}}"#)),
        ]);
        let result = prepare(&paths, "me").unwrap();
        assert_eq!(result.warnings, ["MCP servers of bad skipped (unreadable .claude.json)"]);
        assert_eq!(shared(&paths, "me")["mcpServers"], serde_json::json!({ "g": {} }));
        fs::write(paths.account_dir("me").join(".claude.json"), "{").unwrap();
        assert_eq!(prepare(&paths, "me").unwrap().file, None);
        fs::remove_dir_all(root).unwrap();
    }
}
