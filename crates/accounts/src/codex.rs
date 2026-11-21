//! Codex accounts (since 2.3.0): finding `codex`, its login, and the daemon of the account.
//!
//! Codex keeps its sessions in a daemon, one per `CODEX_HOME`, which answers JSON-RPC over a
//! WebSocket on a Unix socket, `<CODEX_HOME>/app-server-control/app-server-control.sock`. The
//! protocol is the one `codex app-server generate-json-schema` describes (read on 0.171.0).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::error::{Result, fail};
use crate::launch;

/// Removed before starting Codex: a key exported in the shell must never override the login of
/// the account the user picked, nor bill another one.
pub const STRIPPED_ENV: [&str; 2] = ["OPENAI_API_KEY", "CODEX_API_KEY"];

#[cfg(windows)]
const CODEX_NAMES: [&str; 3] = ["codex.exe", "codex.cmd", "codex"];
#[cfg(not(windows))]
const CODEX_NAMES: [&str; 1] = ["codex"];

/// The longest socket path a Unix system takes: 104 bytes with the final zero on macOS, 108 on
/// Linux.
pub const SOCKET_PATH_MAX: usize = 103;

pub fn find_codex_in(path: &OsString) -> Option<PathBuf> {
    launch::find_in(path, &CODEX_NAMES)
}

pub fn require_codex() -> Result<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    match find_codex_in(&path) {
        Some(codex) => Ok(codex),
        None => fail("codex not found in PATH: install Codex, then check that `codex --version` works here"),
    }
}

/// `codex` for the account whose `CODEX_HOME` is `dir`, without the keys of the shell.
pub fn command(codex: &Path, dir: &Path) -> Command {
    let mut command = Command::new(codex);
    for variable in STRIPPED_ENV {
        command.env_remove(variable);
    }
    command.env("CODEX_HOME", dir);
    command
}

pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join("app-server-control").join("app-server-control.sock")
}

/// A socket path the system refuses would leave the daemon out of reach: Codex itself then
/// answers nothing. Windows has no such limit.
pub fn check_socket_path(dir: &Path) -> Result<()> {
    let socket = socket_path(dir);
    let length = socket.as_os_str().len();
    if cfg!(unix) && length > SOCKET_PATH_MAX {
        return fail(format!(
            "the Codex daemon of this account would listen on {} ({length} bytes), longer than the {SOCKET_PATH_MAX} a Unix socket allows: take a shorter account name",
            socket.display()
        ));
    }
    Ok(())
}

/// Whether the account is signed in: `codex login status` exits with 0 when it is, 1 when it is
/// not. Codex reads its own credentials; only the exit code is used. `None`: Codex did not say.
pub fn logged_in(codex: &Path, dir: &Path) -> Option<bool> {
    let status = command(codex, dir)
        .args(["login", "status"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// What a message of the daemon is for the request `id`: its result, its error, or nothing (a
/// notification, a request of the daemon, the answer to another request).
#[cfg_attr(not(unix), allow(dead_code))]
fn answer_to(id: u64, message: &Value) -> Option<std::result::Result<Value, String>> {
    if message.get("method").is_some() || message.get("id")?.as_u64()? != id {
        return None;
    }
    if let Some(error) = message.get("error") {
        let text = error.get("message").and_then(Value::as_str).unwrap_or("error without a message");
        return Some(Err(text.to_string()));
    }
    Some(Ok(message.get("result").cloned().unwrap_or(Value::Null)))
}

/// The ids of one page of `thread/loaded/list`, and the cursor of the next page.
#[cfg_attr(not(unix), allow(dead_code))]
fn loaded_page(result: &Value) -> (Vec<String>, Option<String>) {
    let ids = result
        .get("data")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default();
    let next = result.get("nextCursor").and_then(Value::as_str).map(String::from);
    (ids, next)
}

#[cfg(unix)]
pub use client::Daemon;

#[cfg(unix)]
mod client {
    use std::io;
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};
    use tungstenite::client::IntoClientRequest;
    use tungstenite::{Message, WebSocket};

    use super::{answer_to, loaded_page, socket_path};

    /// How long the daemon has to answer a request, notifications included.
    const TIMEOUT: Duration = Duration::from_secs(3);

    /// More pages than this is a daemon that keeps giving the same cursor.
    const MAX_PAGES: usize = 100;

    /// A connection to the daemon of one account.
    pub struct Daemon {
        socket: WebSocket<UnixStream>,
        next_id: u64,
    }

    impl Daemon {
        /// `None` when the daemon does not run: no socket, or nobody listening on it.
        pub fn connect(dir: &Path) -> io::Result<Option<Self>> {
            let path = socket_path(dir);
            let stream = match UnixStream::connect(&path) {
                Ok(stream) => stream,
                Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => {
                    return Ok(None);
                },
                Err(error) => return Err(io::Error::new(error.kind(), format!("Codex daemon at {}: {error}", path.display()))),
            };
            stream.set_read_timeout(Some(TIMEOUT))?;
            stream.set_write_timeout(Some(TIMEOUT))?;
            let request = "ws://localhost/".into_client_request().map_err(io::Error::other)?;
            let (socket, _) = tungstenite::client(request, stream)
                .map_err(|error| io::Error::other(format!("Codex daemon at {}: {error}", path.display())))?;
            let mut daemon = Self { socket, next_id: 0 };
            let client = json!({"clientInfo": {"name": "kindric-accounts", "version": env!("CARGO_PKG_VERSION")}});
            daemon.request("initialize", client)?;
            daemon.send(&json!({"jsonrpc": "2.0", "method": "initialized"}))?;
            Ok(Some(daemon))
        }

        fn send(&mut self, message: &Value) -> io::Result<()> {
            self.socket.send(Message::text(message.to_string())).map_err(io::Error::other)
        }

        /// The result of `method`, skipping the notifications that come meanwhile.
        pub fn request(&mut self, method: &str, params: Value) -> io::Result<Value> {
            self.next_id += 1;
            let id = self.next_id;
            let failed = |error: &dyn std::fmt::Display| io::Error::other(format!("Codex daemon, {method}: {error}"));
            self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
            let deadline = Instant::now() + TIMEOUT;
            while Instant::now() < deadline {
                let message = self.socket.read().map_err(|error| failed(&error))?;
                let Message::Text(text) = message else {
                    continue;
                };
                let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                if let Some(answer) = answer_to(id, &value) {
                    return answer.map_err(|error| failed(&error));
                }
            }
            Err(failed(&"no answer in time"))
        }

        /// The sessions loaded in the daemon: they run, at work or waiting.
        pub fn loaded_threads(&mut self) -> io::Result<Vec<String>> {
            let mut ids = Vec::new();
            let mut cursor: Option<String> = None;
            for _ in 0..MAX_PAGES {
                let result = self.request("thread/loaded/list", json!({ "cursor": cursor }))?;
                let (page, next) = loaded_page(&result);
                ids.extend(page);
                match next {
                    Some(next) => cursor = Some(next),
                    None => return Ok(ids),
                }
            }
            Err(io::Error::other("Codex daemon, thread/loaded/list: the pages never end"))
        }
    }
}

/// The sessions running on the account: those loaded in its daemon, 0 when it does not run or
/// cannot be asked. Windows: not counted yet.
pub fn running_sessions(dir: &Path) -> usize {
    #[cfg(unix)]
    {
        Daemon::connect(dir)
            .ok()
            .flatten()
            .and_then(|mut daemon| daemon.loaded_threads().ok())
            .map_or(0, |ids| ids.len())
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_the_answer_to_the_request_counts() {
        let notification = json!({"jsonrpc": "2.0", "method": "thread/started", "params": {}});
        let daemon_request = json!({"jsonrpc": "2.0", "id": 1, "method": "item/commandExecution/requestApproval", "params": {}});
        let other = json!({"jsonrpc": "2.0", "id": 2, "result": {}});
        assert_eq!(answer_to(1, &notification), None);
        assert_eq!(answer_to(1, &daemon_request), None, "the daemon asks, it does not answer");
        assert_eq!(answer_to(1, &other), None);
        assert_eq!(answer_to(2, &other), Some(Ok(json!({}))));
        let error = json!({"jsonrpc": "2.0", "id": 3, "error": {"code": -32600, "message": "unknown thread"}});
        assert_eq!(answer_to(3, &error), Some(Err("unknown thread".to_string())));
    }

    #[test]
    fn loaded_threads_are_read_page_by_page() {
        // As the daemon of 0.171.0 answered on the test bench.
        let last = json!({"data": ["01a1ff2a-77e6-7382-a87e-6bb95a3c1b59"], "nextCursor": null});
        assert_eq!(loaded_page(&last), (vec!["01a1ff2a-77e6-7382-a87e-6bb95a3c1b59".to_string()], None));
        let first = json!({"data": ["a", "b"], "nextCursor": "c2"});
        assert_eq!(loaded_page(&first), (vec!["a".to_string(), "b".to_string()], Some("c2".to_string())));
        assert_eq!(loaded_page(&json!({})), (Vec::new(), None));
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_path_too_long_for_the_system_is_refused() {
        let short = Path::new("/home/u/.local/share/kindric-accounts/work");
        assert!(check_socket_path(short).is_ok());
        let long = PathBuf::from(format!("/home/u/.local/share/kindric-accounts/{}", "n".repeat(40)));
        let error = check_socket_path(&long).unwrap_err();
        assert!(error.0.contains("take a shorter account name"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn no_daemon_means_no_session() {
        let dir = crate::files::test_dir::fresh("codex-no-daemon");
        assert!(Daemon::connect(&dir).unwrap().is_none());
        assert_eq!(running_sessions(&dir), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
