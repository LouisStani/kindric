//! Codex accounts (since 2.3.0): finding `codex`, its login, and the daemon of the account.
//!
//! Codex keeps its sessions in a daemon, one per `CODEX_HOME`, which answers JSON-RPC over a
//! WebSocket on a Unix socket, `<CODEX_HOME>/app-server-control/app-server-control.sock`. The
//! protocol is the one `codex app-server generate-json-schema` describes (read on 0.171.0).

use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::error::{Result, fail};
use crate::files;
use crate::launch;
use crate::shared;
use crate::usage;

/// Removed before starting Codex: a key exported in the shell must never override the login of
/// the account the user picked, nor bill another one.
pub const STRIPPED_ENV: [&str; 2] = ["OPENAI_API_KEY", "CODEX_API_KEY"];

/// The variables of a Claude Code or Codex session that runs kindric-accounts: the daemon keeps
/// the environment it starts with and hands it to every command of every session, where
/// `kindric ask` would then speak for the wrong session (seen on the bench, 2026-11-24).
const SESSION_ENV: [&str; 3] = ["CLAUDE_CONFIG_DIR", "CLAUDE_JOB_DIR", "CODEX_THREAD_ID"];

/// Every variable of Claude Code's own (`CLAUDE_CODE_SESSION_ID`…) goes too.
const SESSION_PREFIX: &str = "CLAUDE_CODE_";

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

/// `codex` for the account whose `CODEX_HOME` is `dir`, without the keys and the session
/// variables of this process.
pub fn command(codex: &Path, dir: &Path) -> Command {
    command_from(codex, dir, std::env::vars_os().map(|(name, _)| name))
}

/// `codex` for the account in `dir`, given the names of the variables of the environment.
fn command_from(codex: &Path, dir: &Path, names: impl Iterator<Item = OsString>) -> Command {
    let mut command = Command::new(codex);
    for variable in STRIPPED_ENV {
        command.env_remove(variable);
    }
    for name in names {
        if name.to_str().is_some_and(|text| SESSION_ENV.contains(&text) || text.starts_with(SESSION_PREFIX)) {
            command.env_remove(&name);
        }
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

/// What a message of Codex is for the request `id`: its result, its error, or nothing (a
/// notification, a request of Codex, the answer to another request).
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

/// Where a session of Codex is, as `thread/read` and `thread/list` say it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadStatus {
    /// Not in the daemon: paused, its conversation on disk.
    NotLoaded,
    Idle,
    SystemError,
    /// A turn runs.
    Working,
    /// A command or a change waits for the user's approval.
    WaitingOnApproval,
    /// A question of the model waits for the user's answer.
    WaitingOnUserInput,
}

/// A session of a Codex account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub id: String,
    pub name: Option<String>,
    pub cwd: String,
    pub status: ThreadStatus,
}

/// `{"type": "active", "activeFlags": [...]}` and the like; an unknown status counts as idle.
fn thread_status(status: Option<&Value>) -> ThreadStatus {
    let kind = status.and_then(|status| status.get("type")).and_then(Value::as_str);
    let has_flag = |flag: &str| {
        status
            .and_then(|status| status.get("activeFlags"))
            .and_then(Value::as_array)
            .is_some_and(|flags| flags.iter().any(|value| value.as_str() == Some(flag)))
    };
    match kind {
        Some("notLoaded") => ThreadStatus::NotLoaded,
        Some("systemError") => ThreadStatus::SystemError,
        Some("active") if has_flag("waitingOnApproval") => ThreadStatus::WaitingOnApproval,
        Some("active") if has_flag("waitingOnUserInput") => ThreadStatus::WaitingOnUserInput,
        Some("active") => ThreadStatus::Working,
        _ => ThreadStatus::Idle,
    }
}

/// A session as Codex describes it; `None` without an id, and for a sub-agent, which belongs to
/// the session that started it.
fn thread(value: &Value) -> Option<Thread> {
    if value.get("parentThreadId").is_some_and(|parent| !parent.is_null()) {
        return None;
    }
    let text = |field: &str| value.get(field).and_then(Value::as_str).map(String::from);
    Some(Thread {
        id: text("id")?,
        name: text("name").filter(|name| !name.is_empty()),
        cwd: text("cwd").unwrap_or_default(),
        status: thread_status(value.get("status")),
    })
}

/// A message of Codex that asks this client something: it has both a method and an id.
fn is_request_of_codex(message: &Value) -> bool {
    message.get("method").is_some() && message.get("id").is_some()
}

/// How long Codex has to send an open question to a client that joined the session.
const APPROVAL_WAIT: Duration = Duration::from_millis(1500);

/// A question a Codex session waits for: may this command run, may these files change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub thread: String,
    /// The item the question is about; the answer names it.
    pub item: String,
    /// `false` for a command, `true` for a change of files.
    pub file_change: bool,
    /// The command, as the user would type it.
    pub command: Option<String>,
    /// Why the model asks.
    pub reason: Option<String>,
}

/// The question in `request`, a request of Codex; `None` for anything else.
fn approval(request: &Value) -> Option<Approval> {
    let file_change = match request.get("method")?.as_str()? {
        "item/commandExecution/requestApproval" => false,
        "item/fileChange/requestApproval" => true,
        _ => return None,
    };
    let params = request.get("params")?;
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(String::from).filter(|text| !text.is_empty());
    let typed = params.get("commandActions").and_then(Value::as_array).and_then(|actions| actions.first()).and_then(|action| action.get("command"));
    Some(Approval {
        thread: text(params.get("threadId"))?,
        item: text(params.get("itemId"))?,
        file_change,
        command: text(typed).or_else(|| text(params.get("command"))),
        reason: text(params.get("reason")),
    })
}

/// The approval the session `thread` of the account in `dir` waits for, read from its daemon and
/// left open; `None` when the daemon does not run or the session waits for none.
pub fn pending_approval(dir: &Path, thread: &str) -> io::Result<Option<Approval>> {
    let Some(mut daemon) = AppServer::connect_daemon(dir)? else {
        return Ok(None);
    };
    Ok(daemon.join_for_approval(thread)?.map(|(_, approval)| approval))
}

/// Answers the approval about `item` that the session `thread` of the account in `dir` waits
/// for: `accept`, or decline it. `false` when that question is no longer open (answered in the
/// TUI meanwhile).
pub fn answer_approval(dir: &Path, thread: &str, item: &str, accept: bool) -> io::Result<bool> {
    let Some(mut daemon) = AppServer::connect_daemon(dir)? else {
        return Ok(false);
    };
    let Some((id, approval)) = daemon.join_for_approval(thread)? else {
        return Ok(false);
    };
    if approval.item != item {
        return Ok(false);
    }
    let decision = if accept { "accept" } else { "decline" };
    daemon.transport.send(&json!({"jsonrpc": "2.0", "id": id, "result": {"decision": decision}}).to_string())?;
    Ok(true)
}

/// The sessions of one page of `thread/list`.
fn thread_page(result: &Value) -> Vec<Thread> {
    result.get("data").and_then(Value::as_array).map(|threads| threads.iter().filter_map(thread).collect()).unwrap_or_default()
}

/// How many sessions at rest are listed: the most recent ones.
pub const STOPPED_LISTED: usize = 50;

/// The most ids `thread/list` takes in `excludedThreadIds`.
const MAX_EXCLUDED: usize = 100;

/// How long Codex has to answer a request, notifications included.
const TIMEOUT: Duration = Duration::from_secs(3);

/// How long a Codex app server started for one question has to come up.
const START_TIMEOUT: Duration = Duration::from_secs(15);

/// One line, one message: how the app server of Codex talks on its standard input and output.
struct Piped {
    child: Child,
    input: ChildStdin,
    lines: Receiver<io::Result<String>>,
}

impl Drop for Piped {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

enum Transport {
    /// The daemon of the account, over its socket.
    #[cfg(unix)]
    Socket(Box<tungstenite::WebSocket<std::os::unix::net::UnixStream>>),
    /// A Codex app server started for the question, which ends with it.
    Stdio(Piped),
}

impl Transport {
    fn send(&mut self, text: &str) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Self::Socket(socket) => socket.send(tungstenite::Message::text(text)).map_err(io::Error::other),
            Self::Stdio(stdio) => {
                stdio.input.write_all(text.as_bytes())?;
                stdio.input.write_all(b"\n")?;
                stdio.input.flush()
            },
        }
    }

    /// The next message, or `None` for a frame that holds none (ping, binary).
    fn receive(&mut self, timeout: Duration) -> io::Result<Option<String>> {
        match self {
            #[cfg(unix)]
            Self::Socket(socket) => {
                socket.get_mut().set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
                match socket.read() {
                    Ok(tungstenite::Message::Text(text)) => Ok(Some(text.as_str().to_string())),
                    Ok(_) => Ok(None),
                    Err(tungstenite::Error::Io(error)) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                        Err(io::Error::new(io::ErrorKind::TimedOut, "no answer in time"))
                    },
                    Err(error) => Err(io::Error::other(error)),
                }
            },
            Self::Stdio(stdio) => match stdio.lines.recv_timeout(timeout) {
                Ok(line) => line.map(Some),
                Err(RecvTimeoutError::Timeout) => Err(io::Error::new(io::ErrorKind::TimedOut, "no answer in time")),
                Err(RecvTimeoutError::Disconnected) => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "codex app-server ended")),
            },
        }
    }
}

/// A conversation with Codex's app server for one account: its daemon, or one started for it.
pub struct AppServer {
    transport: Transport,
    next_id: u64,
    /// What the messages say they come from, for errors.
    what: &'static str,
    /// The requests of Codex that came while an answer was awaited, oldest first.
    asked: Vec<Value>,
}

impl AppServer {
    /// The daemon of the account in `dir`; `None` when it does not run: no socket, or nobody
    /// listening on it. Always `None` on Windows for now.
    pub fn connect_daemon(dir: &Path) -> io::Result<Option<Self>> {
        #[cfg(unix)]
        {
            use tungstenite::client::IntoClientRequest;
            let path = socket_path(dir);
            let stream = match std::os::unix::net::UnixStream::connect(&path) {
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
            Self::initialized(Transport::Socket(Box::new(socket)), "Codex daemon", TIMEOUT).map(Some)
        }
        #[cfg(not(unix))]
        {
            let _ = dir;
            Ok(None)
        }
    }

    /// A Codex app server of its own, `codex app-server` on standard input and output, for the
    /// account in `dir`: it reads the sessions on disk, and ends when this value is dropped.
    pub fn start(codex: &Path, dir: &Path) -> io::Result<Self> {
        let mut child = command(codex, dir)
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| io::Error::new(error.kind(), format!("cannot start codex app-server: {error}")))?;
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err(io::Error::other("codex app-server: no standard input or output"));
        };
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if sender.send(line).is_err() {
                    return;
                }
            }
        });
        Self::initialized(Transport::Stdio(Piped { child, input, lines }), "codex app-server", START_TIMEOUT)
    }

    fn initialized(transport: Transport, what: &'static str, timeout: Duration) -> io::Result<Self> {
        let mut server = Self { transport, next_id: 0, what, asked: Vec::new() };
        let client = json!({"clientInfo": {"name": "kindric-accounts", "version": env!("CARGO_PKG_VERSION")}});
        server.request_within("initialize", client, timeout)?;
        server.transport.send(&json!({"jsonrpc": "2.0", "method": "initialized"}).to_string())?;
        Ok(server)
    }

    /// The result of `method`, skipping the notifications that come meanwhile.
    pub fn request(&mut self, method: &str, params: Value) -> io::Result<Value> {
        self.request_within(method, params, TIMEOUT)
    }

    fn request_within(&mut self, method: &str, params: Value, timeout: Duration) -> io::Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let what = self.what;
        let failed = |error: &dyn std::fmt::Display| io::Error::other(format!("{what}, {method}: {error}"));
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.transport.send(&message.to_string()).map_err(|error| failed(&error))?;
        let deadline = Instant::now() + timeout;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let text = match self.transport.receive(left) {
                Ok(Some(text)) => text,
                Ok(None) => continue,
                Err(error) if error.kind() == io::ErrorKind::TimedOut => continue,
                Err(error) => return Err(failed(&error)),
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if let Some(answer) = answer_to(id, &value) {
                return answer.map_err(|error| failed(&error));
            }
            if is_request_of_codex(&value) {
                self.asked.push(value);
            }
        }
        Err(failed(&"no answer in time"))
    }

    /// The next request Codex sends this client, waiting at most `timeout`; `None` if none came.
    fn next_request(&mut self, timeout: Duration) -> io::Result<Option<Value>> {
        if !self.asked.is_empty() {
            return Ok(Some(self.asked.remove(0)));
        }
        let deadline = Instant::now() + timeout;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let text = match self.transport.receive(left) {
                Ok(Some(text)) => text,
                Ok(None) => continue,
                Err(error) if error.kind() == io::ErrorKind::TimedOut => return Ok(None),
                Err(error) => return Err(error),
            };
            if let Ok(value) = serde_json::from_str::<Value>(&text)
                && is_request_of_codex(&value)
            {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    /// The approval the session `thread` waits for, and the request that asks it, once this
    /// client joined the session: Codex sends a question still open to each client that joins.
    fn join_for_approval(&mut self, thread: &str) -> io::Result<Option<(Value, Approval)>> {
        self.request("thread/resume", json!({ "threadId": thread }))?;
        while let Some(request) = self.next_request(APPROVAL_WAIT)? {
            if let Some(approval) = approval(&request).filter(|approval| approval.thread == thread) {
                let id = request.get("id").cloned().unwrap_or(Value::Null);
                return Ok(Some((id, approval)));
            }
        }
        Ok(None)
    }

    /// The sessions loaded in the daemon: they run, at work or waiting.
    pub fn loaded_threads(&mut self) -> io::Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        // More pages than ids is a daemon that keeps giving the same cursor.
        for _ in 0..1000 {
            let result = self.request("thread/loaded/list", json!({ "cursor": cursor }))?;
            let (page, next) = loaded_page(&result);
            ids.extend(page);
            match next {
                Some(next) => cursor = Some(next),
                None => return Ok(ids),
            }
        }
        Err(io::Error::other(format!("{}, thread/loaded/list: the pages never end", self.what)))
    }

    /// The session `id`, with its state; `None` for a sub-agent.
    pub fn read_thread(&mut self, id: &str) -> io::Result<Option<Thread>> {
        let result = self.request("thread/read", json!({ "threadId": id }))?;
        Ok(result.get("thread").and_then(thread))
    }

    /// The `limit` sessions used last, archived ones left out, and `excluded` too.
    pub fn recent_threads(&mut self, limit: usize, excluded: &[String]) -> io::Result<Vec<Thread>> {
        let excluded: Vec<&String> = excluded.iter().take(MAX_EXCLUDED).collect();
        let params = json!({"limit": limit, "sortKey": "updated_at", "archived": false, "excludedThreadIds": excluded});
        Ok(thread_page(&self.request("thread/list", params)?))
    }
}

/// The sessions of the Codex account in `dir`: those loaded in its daemon, then, with
/// `include_stopped`, the most recent of the others. When the daemon does not run, none is
/// loaded: those at rest are read by a Codex app server started for the question.
pub fn sessions(dir: &Path, include_stopped: bool) -> io::Result<Vec<Thread>> {
    let (mut server, loaded) = match AppServer::connect_daemon(dir)? {
        Some(mut daemon) => {
            let loaded = daemon.loaded_threads()?;
            (daemon, loaded)
        },
        None if include_stopped => {
            let codex = require_codex().map_err(|error| io::Error::new(io::ErrorKind::NotFound, error.0))?;
            (AppServer::start(&codex, dir)?, Vec::new())
        },
        None => return Ok(Vec::new()),
    };
    let mut threads = Vec::new();
    for id in &loaded {
        threads.extend(server.read_thread(id)?);
    }
    if include_stopped {
        threads.extend(server.recent_threads(STOPPED_LISTED, &loaded)?);
    }
    Ok(threads)
}

/// The name a new session gets: `name`, else the last folder of `cwd`, the title kindric shows for
/// a session without a name anyway.
fn session_name(name: &str, cwd: &Path) -> String {
    if !name.trim().is_empty() {
        return name.trim().to_string();
    }
    cwd.file_name().map_or_else(|| "codex".to_string(), |folder| folder.to_string_lossy().into_owned())
}

/// Starts a session of the account in `dir`, in the folder `cwd`, in the daemon of the account,
/// which must run (see `ensure_daemon`). The session waits for its first message; `codex resume`
/// joins it. It is always named (see `session_name`): Codex 0.171.0 writes the file of a session
/// only at its first turn or when it is named, and `codex resume` refuses a session without one
/// ("no rollout found"), so a pane opened on an unnamed new session closed at once. Returns its id.
pub fn start_session(dir: &Path, cwd: &Path, name: &str) -> io::Result<String> {
    let Some(mut daemon) = AppServer::connect_daemon(dir)? else {
        return Err(io::Error::other("the Codex daemon of this account does not run"));
    };
    let started = daemon.request("thread/start", json!({ "cwd": cwd }))?;
    let Some(id) = started.get("thread").and_then(|thread| thread.get("id")).and_then(Value::as_str) else {
        return Err(io::Error::other("Codex daemon, thread/start: no session id in the answer"));
    };
    daemon.request("thread/name/set", json!({ "threadId": id, "name": session_name(name, cwd) }))?;
    Ok(id.to_string())
}

/// The turn of a session that is running, from one page of `thread/turns/list`.
fn turn_in_progress(result: &Value) -> Option<String> {
    let turns = result.get("data")?.as_array()?;
    let running = turns.iter().find(|turn| turn.get("status").and_then(Value::as_str) == Some("inProgress"))?;
    running.get("id").and_then(Value::as_str).map(String::from)
}

/// Stops the turn the session `id` runs, if any: Codex keeps the conversation and waits for the
/// next message. Nothing to do when the daemon of the account in `dir` does not run.
pub fn interrupt(dir: &Path, id: &str) -> io::Result<()> {
    let Some(mut daemon) = AppServer::connect_daemon(dir)? else {
        return Ok(());
    };
    let turns = daemon.request("thread/turns/list", json!({ "threadId": id, "limit": 5 }))?;
    if let Some(turn) = turn_in_progress(&turns) {
        daemon.request("turn/interrupt", json!({ "threadId": id, "turnId": turn }))?;
    }
    Ok(())
}

/// Archives the session `id` of the account in `dir`: it leaves the lists of Codex and of
/// kindric, its conversation stays on disk, and `codex unarchive` brings it back. Through the
/// daemon when it runs, else through a Codex app server started for it.
pub fn archive(dir: &Path, id: &str) -> io::Result<()> {
    let mut server = match AppServer::connect_daemon(dir)? {
        Some(daemon) => daemon,
        None => {
            let codex = require_codex().map_err(|error| io::Error::new(io::ErrorKind::NotFound, error.0))?;
            AppServer::start(&codex, dir)?
        },
    };
    server.request("thread/archive", json!({ "threadId": id })).map(|_| ())
}

/// The folder the session `id` of the account in `dir` works in, read from its daemon; `None`
/// when the daemon does not run or does not know the session.
pub fn session_cwd(dir: &Path, id: &str) -> io::Result<Option<PathBuf>> {
    let Some(mut daemon) = AppServer::connect_daemon(dir)? else {
        return Ok(None);
    };
    let thread = daemon.read_thread(id)?;
    Ok(thread.map(|thread| PathBuf::from(thread.cwd)).filter(|cwd| !cwd.as_os_str().is_empty()))
}

/// Where `codex --remote` reaches the daemon of the account in `dir`.
pub fn remote_address(dir: &Path) -> String {
    format!("unix://{}", socket_path(dir).display())
}

/// Starts the daemon of the account in `dir` unless it runs: `codex app-server daemon start`
/// returns once it listens. A session at rest is then loaded by the first client that joins it.
pub fn ensure_daemon(codex: &Path, dir: &Path) -> io::Result<()> {
    if AppServer::connect_daemon(dir)?.is_some() {
        return Ok(());
    }
    let output = command(codex, dir).args(["app-server", "daemon", "start"]).stdin(Stdio::null()).output()?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let last = said.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("no message");
    Err(io::Error::other(format!("codex app-server daemon start: {}", last.trim())))
}

/// The sessions running on the account: those loaded in its daemon, 0 when it does not run or
/// cannot be asked. Windows: not counted yet.
pub fn running_sessions(dir: &Path) -> usize {
    AppServer::connect_daemon(dir)
        .ok()
        .flatten()
        .and_then(|mut daemon| daemon.loaded_threads().ok())
        .map_or(0, |ids| ids.len())
}

/// kindric's hooks a Codex session gets, among `all`: the user away, how to ask the user, and the
/// notice of unread messages, read with the `read_messages` tool of kindric's MCP server (see
/// `prepare_mcp`). Not:
/// - the questions put on kindric's page: Codex waits for this hook before it shows the question
///   in its TUI, which would stay blank until the page answers;
/// - the wait for a message, which relies on a feature of Claude Code (`asyncRewake`);
/// - those that read Claude Code's transcript or tools.
pub fn kindric_hooks(all: usage::KindricHooks) -> usage::KindricHooks {
    usage::KindricHooks { away: all.away, start: all.start, unread: all.unread, ..usage::KindricHooks::default() }
}

/// Codex's hooks file of the account in `dir`, in Claude Code's form.
pub fn hooks_file(dir: &Path) -> PathBuf {
    dir.join("hooks.json")
}

/// The fingerprint of the hooks file kindric-accounts wrote last, so that it never replaces one
/// written by someone else.
fn hooks_mark(dir: &Path) -> PathBuf {
    dir.join(".kindric-hooks")
}

/// The fingerprint of the hooks file Codex was last asked to trust: trusting is asked again only
/// when the file changed, or when it failed.
fn trusted_mark(dir: &Path) -> PathBuf {
    dir.join(".kindric-hooks-trusted")
}

/// Writes kindric's `hooks` in the hooks file of the Codex account in `dir`, unless the file is not
/// the one kindric-accounts wrote. Returns a warning when it was left alone.
pub fn write_hooks(dir: &Path, hooks: &usage::KindricHooks) -> io::Result<Option<String>> {
    let file = hooks_file(dir);
    let (text, _) = usage::with_additions(None, None, hooks);
    let content = format!("{text}\n");
    let existing = match fs::read_to_string(&file) {
        Ok(existing) => Some(existing),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(io::Error::new(error.kind(), format!("{}: {error}", file.display()))),
    };
    if let Some(existing) = &existing {
        let mark = fs::read_to_string(hooks_mark(dir)).unwrap_or_default();
        if mark.trim() != shared::fingerprint(existing.as_bytes()) {
            return Ok(Some(format!("{} was not written by kindric-accounts: kindric's hooks are not added to it", file.display())));
        }
        if *existing == content {
            return Ok(None);
        }
    }
    files::write_atomic(&file, &content)?;
    files::write_atomic(&hooks_mark(dir), &shared::fingerprint(content.as_bytes()))?;
    Ok(None)
}

/// The hooks of `hooks_file` that Codex does not trust yet, as `hooks/list` lists them: the key
/// of each and the hash Codex expects for it.
fn untrusted_hooks(listed: &Value, hooks_file: &Path) -> Vec<(String, String)> {
    // Codex names the file by its real path: compared as written, then resolved, never two files
    // that cannot be resolved taken for the same one.
    let resolved = fs::canonicalize(hooks_file).ok();
    let same_file = |path: &str| {
        Path::new(path) == hooks_file || resolved.as_ref().is_some_and(|resolved| fs::canonicalize(path).ok().as_ref() == Some(resolved))
    };
    listed
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("hooks")?.as_array())
        .flatten()
        .filter(|hook| hook.get("sourcePath").and_then(Value::as_str).is_some_and(same_file))
        .filter(|hook| hook.get("trustStatus").and_then(Value::as_str) != Some("trusted"))
        .filter_map(|hook| Some((hook.get("key")?.as_str()?.to_string(), hook.get("currentHash")?.as_str()?.to_string())))
        .collect()
}

/// The edits of `config/batchWrite` that turn hooks on and trust `hooks`. The keys hold dots and
/// colons: they go as keys of one object, merged into `hooks.state`, never inside a key path.
fn trust_edits(hooks: &[(String, String)]) -> Value {
    let state: serde_json::Map<String, Value> =
        hooks.iter().map(|(key, hash)| (key.clone(), json!({ "trusted_hash": hash }))).collect();
    json!([
        { "keyPath": "features.hooks", "value": true, "mergeStrategy": "replace" },
        { "keyPath": "hooks.state", "value": state, "mergeStrategy": "upsert" },
    ])
}

/// Has Codex trust the hooks kindric wrote for the account in `dir`, and turn hooks on: Codex
/// writes its own `config.toml`, kindric never does.
pub fn trust_hooks(server: &mut AppServer, dir: &Path) -> io::Result<()> {
    let listed = server.request("hooks/list", json!({ "cwds": [dir] }))?;
    let untrusted = untrusted_hooks(&listed, &hooks_file(dir));
    if untrusted.is_empty() {
        return Ok(());
    }
    server.request("config/batchWrite", json!({ "edits": trust_edits(&untrusted), "reloadUserConfig": true })).map(|_| ())
}

/// Gives the Codex account in `dir` kindric's `hooks`, trusted: through its daemon when it runs,
/// else through a Codex app server started for it. Returns what went wrong, as warnings: a session
/// without kindric's hooks still works.
pub fn prepare_hooks(codex: &Path, dir: &Path, hooks: &usage::KindricHooks) -> Vec<String> {
    let mut warnings = Vec::new();
    // No `kindric` in PATH: nothing to give, and a file written before stays as it was.
    if hooks.is_empty() {
        return warnings;
    }
    match write_hooks(dir, hooks) {
        Ok(Some(warning)) => {
            warnings.push(warning);
            return warnings;
        },
        Ok(None) => {},
        Err(error) => {
            warnings.push(format!("kindric's hooks not written: {error}"));
            return warnings;
        },
    }
    let written = fs::read_to_string(hooks_mark(dir)).unwrap_or_default();
    if fs::read_to_string(trusted_mark(dir)).is_ok_and(|trusted| trusted == written) {
        return warnings;
    }
    match app_server(codex, dir).and_then(|mut server| trust_hooks(&mut server, dir)) {
        Ok(()) => {
            if let Err(error) = files::write_atomic(&trusted_mark(dir), &written) {
                warnings.push(format!("{}: {error}", trusted_mark(dir).display()));
            }
        },
        Err(error) => warnings.push(format!("kindric's hooks not trusted by Codex: {error}")),
    }
    warnings
}

/// What kindric gives the Codex account `name` in `dir` before a session: its hooks and its
/// messages server, with the `kindric` in `PATH`. Returns what went wrong, as warnings.
pub fn prepare_kindric(codex: &Path, dir: &Path, name: &str, isolated: bool) -> Vec<String> {
    let mut warnings = prepare_hooks(codex, dir, &kindric_hooks(usage::kindric_hooks()));
    warnings.extend(prepare_mcp(codex, dir, name, usage::kindric_program().as_deref(), isolated));
    warnings
}

/// The daemon of the Codex account in `dir` when it runs, else a Codex app server started for it.
fn app_server(codex: &Path, dir: &Path) -> io::Result<AppServer> {
    match AppServer::connect_daemon(dir)? {
        Some(daemon) => Ok(daemon),
        None => AppServer::start(codex, dir),
    }
}

/// The name of kindric's MCP server in Codex's configuration.
const MESSAGES_SERVER: &str = "kindric";

/// The fingerprint of kindric's MCP server as Codex was last asked to declare it: asked again only
/// when it changed, or when it failed.
fn mcp_mark(dir: &Path) -> PathBuf {
    dir.join(".kindric-mcp")
}

/// What Codex gives its MCP servers is only HOME, LANG, LOGNAME, PATH, SHELL, TERM and USER (Codex
/// 0.172.0): kindric finds its data and kindric-accounts' with these too.
const MCP_ENV: [&str; 2] = ["XDG_CONFIG_HOME", "XDG_DATA_HOME"];

/// The edit of `config/batchWrite` that declares kindric's MCP server for the Codex account `name`.
/// Codex names the session in each call, so the server only needs the account. Its tools are allowed
/// without asking: else every message stops the session on a permission prompt (Codex 0.172.0).
fn mcp_edits(kindric: &Path, name: &str) -> Value {
    json!([{
        "keyPath": format!("mcp_servers.{MESSAGES_SERVER}"),
        "value": {
            "command": kindric,
            "args": ["mcp", "--account", name],
            "env_vars": MCP_ENV,
            "default_tools_approval_mode": "approve",
        },
        "mergeStrategy": "replace",
    }])
}

/// Declares kindric's MCP server (messages between sessions, artifacts) in the Codex account `name`
/// in `dir`, through its daemon or an app server: Codex writes its own `config.toml`. Nothing for an
/// isolated account, whose sessions take no part in messages, nor without a `kindric` that runs it.
/// Returns what went wrong, as warnings: a session without the server still works.
pub fn prepare_mcp(codex: &Path, dir: &Path, name: &str, kindric: Option<&Path>, isolated: bool) -> Vec<String> {
    let Some(kindric) = kindric.filter(|_| !isolated) else {
        return Vec::new();
    };
    let edits = mcp_edits(kindric, name);
    let wanted = shared::fingerprint(edits.to_string().as_bytes());
    if fs::read_to_string(mcp_mark(dir)).is_ok_and(|done| done == wanted) || !usage::runs_mcp_for(kindric, name) {
        return Vec::new();
    }
    let declared = app_server(codex, dir)
        .and_then(|mut server| server.request("config/batchWrite", json!({ "edits": edits, "reloadUserConfig": true })).map(|_| ()));
    match declared {
        Ok(()) => match files::write_atomic(&mcp_mark(dir), &wanted) {
            Ok(()) => Vec::new(),
            Err(error) => vec![format!("{}: {error}", mcp_mark(dir).display())],
        },
        Err(error) => vec![format!("kindric's messages server not declared to Codex: {error}")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn codex_runs_without_the_keys_and_the_session_variables_of_the_shell() {
        let names = ["PATH", "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CONFIG_DIR", "CLAUDE_JOB_DIR", "CODEX_THREAD_ID", "CLAUDE_EFFORT"];
        let command = command_from(Path::new("/usr/bin/codex"), Path::new("/data/cx"), names.iter().map(OsString::from));
        let env_of = |name: &str| command.get_envs().find(|(key, _)| *key == OsStr::new(name)).map(|(_, value)| value);
        for gone in ["CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CONFIG_DIR", "CLAUDE_JOB_DIR", "CODEX_THREAD_ID", "OPENAI_API_KEY", "CODEX_API_KEY"] {
            assert_eq!(env_of(gone), Some(None), "{gone}");
        }
        assert_eq!(env_of("CODEX_HOME"), Some(Some(OsStr::new("/data/cx"))));
        assert_eq!(env_of("PATH"), None, "the rest is left as it is");
        assert_eq!(env_of("CLAUDE_EFFORT"), None, "not the identity of a session");
    }

    #[test]
    fn the_messages_server_runs_for_the_account_and_asks_nothing() {
        let edits = mcp_edits(Path::new("/usr/bin/kindric"), "cx");
        assert_eq!(
            edits,
            json!([{
                "keyPath": "mcp_servers.kindric",
                "value": {
                    "command": "/usr/bin/kindric",
                    "args": ["mcp", "--account", "cx"],
                    "env_vars": ["XDG_CONFIG_HOME", "XDG_DATA_HOME"],
                    "default_tools_approval_mode": "approve",
                },
                "mergeStrategy": "replace",
            }])
        );
    }

    #[test]
    fn no_messages_server_for_an_isolated_account_or_without_kindric() {
        let dir = std::env::temp_dir().join(format!("kindric-codex-mcp-{}", std::process::id()));
        // Neither reaches Codex: a missing program would otherwise come back as a warning.
        let codex = Path::new("/nonexistent/codex");
        assert!(prepare_mcp(codex, &dir, "cx", Some(Path::new("/usr/bin/kindric")), true).is_empty());
        assert!(prepare_mcp(codex, &dir, "cx", None, false).is_empty());
        assert!(!mcp_mark(&dir).exists());
    }

    #[test]
    fn a_new_session_is_named_after_its_folder_when_no_name_is_given() {
        let folder = Path::new("/home/u").join("api");
        assert_eq!(session_name(" review ", &folder), "review");
        assert_eq!(session_name("", &folder), "api");
        assert_eq!(session_name("  ", &folder), "api");
        assert_eq!(session_name("", Path::new("/")), "codex", "a folder without a name");
    }
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

    #[test]
    fn each_status_of_codex_is_read() {
        let status = |value: Value| thread_status(Some(&value));
        assert_eq!(status(json!({"type": "notLoaded"})), ThreadStatus::NotLoaded);
        assert_eq!(status(json!({"type": "idle"})), ThreadStatus::Idle);
        assert_eq!(status(json!({"type": "systemError"})), ThreadStatus::SystemError);
        assert_eq!(status(json!({"type": "active", "activeFlags": []})), ThreadStatus::Working);
        assert_eq!(status(json!({"type": "active", "activeFlags": ["waitingOnApproval"]})), ThreadStatus::WaitingOnApproval);
        assert_eq!(status(json!({"type": "active", "activeFlags": ["waitingOnUserInput"]})), ThreadStatus::WaitingOnUserInput);
        assert_eq!(
            status(json!({"type": "active", "activeFlags": ["waitingOnUserInput", "waitingOnApproval"]})),
            ThreadStatus::WaitingOnApproval,
            "a permission first, as for Claude Code"
        );
        assert_eq!(status(json!({"type": "somethingNew"})), ThreadStatus::Idle);
        assert_eq!(thread_status(None), ThreadStatus::Idle);
    }

    #[test]
    fn sessions_are_read_from_thread_list_without_sub_agents() {
        // Fields as codex 0.171.0 gave them on the test bench, the rest left out.
        let page = json!({"data": [
            {"id": "01a155e3", "name": "Conseiller une stack", "cwd": "/home/u/Socials", "parentThreadId": null, "status": {"type": "notLoaded"}},
            {"id": "01a1505a", "name": null, "cwd": "/home/u/Thr", "status": {"type": "idle"}},
            {"id": "01a1ff2b", "name": "", "cwd": "/w", "parentThreadId": "01a1505a", "status": {"type": "idle"}},
            {"name": "no id"}
        ], "nextCursor": null});
        let threads = thread_page(&page);
        assert_eq!(threads, [
            Thread { id: "01a155e3".into(), name: Some("Conseiller une stack".into()), cwd: "/home/u/Socials".into(), status: ThreadStatus::NotLoaded },
            Thread { id: "01a1505a".into(), name: None, cwd: "/home/u/Thr".into(), status: ThreadStatus::Idle },
        ]);
    }

    #[test]
    fn an_approval_is_read_from_the_request_of_codex() {
        // As the daemon of codex 0.171.0 sent it on the test bench, the rest left out.
        let command = json!({"jsonrpc": "2.0", "id": 0, "method": "item/commandExecution/requestApproval", "params": {
            "threadId": "01a1ff2c", "itemId": "c1", "reason": "test kindric",
            "command": "/usr/bin/bash -lc 'touch /tmp/x'", "commandActions": [{"type": "unknown", "command": "touch /tmp/x"}]}});
        assert_eq!(approval(&command), Some(Approval {
            thread: "01a1ff2c".into(), item: "c1".into(), file_change: false,
            command: Some("touch /tmp/x".into()), reason: Some("test kindric".into()),
        }));
        let files = json!({"id": 1, "method": "item/fileChange/requestApproval", "params": {"threadId": "t", "itemId": "f1"}});
        assert_eq!(approval(&files).map(|approval| (approval.file_change, approval.command)), Some((true, None)));
        assert_eq!(approval(&json!({"id": 2, "method": "item/tool/requestUserInput", "params": {"threadId": "t", "itemId": "q"}})), None);
        assert!(is_request_of_codex(&command));
        assert!(!is_request_of_codex(&json!({"method": "thread/started", "params": {}})));
        assert!(!is_request_of_codex(&json!({"id": 3, "result": {}})));
    }

    #[test]
    fn only_a_turn_in_progress_is_interrupted() {
        let page = json!({"data": [
            {"id": "t2", "status": "inProgress"},
            {"id": "t1", "status": "completed"}
        ]});
        assert_eq!(turn_in_progress(&page).as_deref(), Some("t2"));
        let over = json!({"data": [{"id": "t1", "status": "completed"}, {"id": "t0", "status": "interrupted"}]});
        assert_eq!(turn_in_progress(&over), None);
        assert_eq!(turn_in_progress(&json!({})), None);
    }

    fn some_hooks() -> usage::KindricHooks {
        usage::KindricHooks {
            unread: Some("k unread".into()),
            away: Some("k away-hook".into()),
            wake: Some("k wait-message".into()),
            permission: Some("k permission-hook".into()),
            start: Some("k overview-start".into()),
            artifacts: Some("k artifact-check".into()),
            tasks: Some("k task-check".into()),
            question: Some("k question-check".into()),
        }
    }

    #[test]
    fn a_codex_session_gets_the_hooks_that_work_in_codex() {
        let hooks = kindric_hooks(some_hooks());
        let expected = usage::KindricHooks {
            away: Some("k away-hook".into()),
            start: Some("k overview-start".into()),
            unread: Some("k unread".into()),
            ..usage::KindricHooks::default()
        };
        assert_eq!(hooks, expected, "no page question, no wait for a message, no check of Claude Code's transcript");
    }

    #[test]
    fn the_hooks_file_is_written_only_when_it_is_kindrics() {
        let dir = crate::files::test_dir::fresh("codex-hooks");
        let hooks = kindric_hooks(some_hooks());
        assert_eq!(write_hooks(&dir, &hooks).unwrap(), None);
        let written: Value = serde_json::from_str(&fs::read_to_string(hooks_file(&dir)).unwrap()).unwrap();
        assert_eq!(written["hooks"]["SessionStart"][0]["hooks"][0]["command"], "k overview-start");
        assert_eq!(written["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        assert_eq!(written["hooks"]["Stop"].as_array().map(Vec::len), Some(1), "only the away hook on Stop");
        // Written again by kindric: replaced.
        let fewer = usage::KindricHooks { start: None, ..hooks.clone() };
        assert_eq!(write_hooks(&dir, &fewer).unwrap(), None);
        assert!(!fs::read_to_string(hooks_file(&dir)).unwrap().contains("overview-start"));
        // Changed by someone else: left alone, with a warning.
        fs::write(hooks_file(&dir), r#"{"hooks": {}}"#).unwrap();
        let warning = write_hooks(&dir, &hooks).unwrap().unwrap();
        assert!(warning.contains("not written by kindric-accounts"), "{warning}");
        assert_eq!(fs::read_to_string(hooks_file(&dir)).unwrap(), r#"{"hooks": {}}"#);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_kindrics_untrusted_hooks_are_trusted_through_one_merge() {
        let file = Path::new("/d/cx/hooks.json");
        // As hooks/list answered on the test bench (codex 0.171.0), the rest left out.
        let listed = json!({"data": [{"cwd": "/d/cx", "hooks": [
            {"key": "/d/cx/hooks.json:stop:0:0", "sourcePath": "/d/cx/hooks.json", "trustStatus": "untrusted", "currentHash": "sha256:aa"},
            {"key": "/d/cx/hooks.json:session_start:0:0", "sourcePath": "/d/cx/hooks.json", "trustStatus": "trusted", "currentHash": "sha256:bb"},
            {"key": "postman@postman:hooks/codex-hooks.json:session_start:0:0", "sourcePath": "/p/codex-hooks.json", "trustStatus": "untrusted", "currentHash": "sha256:cc"}
        ]}]});
        let untrusted = untrusted_hooks(&listed, file);
        assert_eq!(untrusted, [("/d/cx/hooks.json:stop:0:0".to_string(), "sha256:aa".to_string())]);
        let edits = trust_edits(&untrusted);
        assert_eq!(edits[0], json!({"keyPath": "features.hooks", "value": true, "mergeStrategy": "replace"}));
        assert_eq!(edits[1]["keyPath"], "hooks.state");
        assert_eq!(edits[1]["mergeStrategy"], "upsert");
        assert_eq!(edits[1]["value"]["/d/cx/hooks.json:stop:0:0"]["trusted_hash"], "sha256:aa");
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
        let dir = crate::files::test_dir::fresh_short("nd");
        assert!(AppServer::connect_daemon(&dir).unwrap().is_none());
        assert_eq!(running_sessions(&dir), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
