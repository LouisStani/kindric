//! The sessions of an OpenCode account, asked of its background service over HTTP: one request
//! costs a few milliseconds, where `opencode api` would start an OpenCode process each time. The
//! service listens on 127.0.0.1, at the port and with the password OpenCode writes in the
//! account's `service.json` (read here, never written), behind HTTP Basic authentication.
//! Read on OpenCode 2.3.2 (2026-11-22): its sessions all live in the service, a session waiting
//! for a permission or an answer still counts as running, and the service writes
//! `data/opencode/opencode.db-wal` at each change.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use kindric_accounts::opencode as account;
use serde::Deserialize;
use serde_json::{Value, json};

/// Where every service listens unless its account gave it a port.
const DEFAULT_PORT: u16 = 49374;
const TIMEOUT: Duration = Duration::from_secs(3);
/// A list of fifty sessions is a few KB; anything far larger is not the service.
const MAX_RESPONSE: u64 = 16 * 1024 * 1024;
/// The sessions at rest listed, the most recent first, as for Codex.
pub const STOPPED_LISTED: usize = 50;
/// Items of a page of a whole list, and the most pages read of one.
const PAGE: usize = 100;
const MAX_PAGES: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Working,
    /// A permission the user must give.
    WaitingOnPermission,
    /// A question of the model (its `question` tool) the user must answer.
    WaitingOnAnswer,
    AtRest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub directory: String,
    pub status: Status,
}

#[derive(Deserialize)]
struct ServiceFile {
    port: Option<u16>,
    password: Option<String>,
}

/// The service of one account, as its file says. Nothing tells whether it runs until it is asked.
pub struct Service {
    port: u16,
    authorization: String,
}

/// The base64 of `bytes` (RFC 4648, with padding), for the `Authorization` header.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let value = group.iter().enumerate().fold(0u32, |value, (index, byte)| value | u32::from(*byte) << (16 - 8 * index));
        for index in 0..4 {
            if index <= group.len() {
                text.push(char::from(ALPHABET[(value >> (18 - 6 * index)) as usize & 63]));
            } else {
                text.push('=');
            }
        }
    }
    text
}

/// The body of an HTTP response, its status checked: `Content-Length` or chunked, else up to the
/// end, since every request asks the service to close the connection.
fn parse_response(response: &[u8]) -> io::Result<Vec<u8>> {
    let Some(split) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Err(io::Error::other("the OpenCode service gave an answer that is not HTTP"));
    };
    let head = String::from_utf8_lossy(&response[..split]);
    let body = &response[split + 4..];
    let mut lines = head.lines();
    let status = lines.next().unwrap_or_default();
    let code = status.split_whitespace().nth(1).and_then(|code| code.parse::<u16>().ok());
    let header = |name: &str| {
        lines.clone().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim().to_string())
        })
    };
    let body = if header("transfer-encoding").is_some_and(|value| value.eq_ignore_ascii_case("chunked")) {
        dechunk(body)?
    } else if let Some(length) = header("content-length").and_then(|value| value.parse::<usize>().ok()) {
        body.get(..length).ok_or_else(|| io::Error::other("the OpenCode service's answer was cut short"))?.to_vec()
    } else {
        body.to_vec()
    };
    match code {
        Some(200..=299) => Ok(body),
        _ => Err(io::Error::new(error_kind(code), format!("the OpenCode service answered {status:?}: {}", said(&body)))),
    }
}

/// A session of the service, for the statistics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsedSession {
    pub id: String,
    /// The session it is a subagent of.
    pub parent: Option<String>,
    pub directory: String,
    /// When it last changed, as the service says (milliseconds).
    pub updated: i64,
}

fn used_session(info: &Value) -> Option<UsedSession> {
    Some(UsedSession {
        id: info.get("id")?.as_str()?.to_string(),
        parent: info.get("parentID").and_then(Value::as_str).map(String::from),
        directory: directory_of(info).unwrap_or_default(),
        updated: info.pointer("/time/updated")?.as_i64()?,
    })
}

/// A reply of a model: its tokens, the cache apart, and what it cost.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    /// Unix milliseconds.
    pub at: i64,
    pub model: String,
    /// US dollars, at the price OpenCode knows; 0 for a model without one.
    pub cost: f64,
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

fn reply(message: &Value) -> Option<Reply> {
    if message.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let tokens = |pointer: &str| message.pointer(pointer).and_then(Value::as_u64).unwrap_or(0);
    Some(Reply {
        at: message.pointer("/time/created")?.as_i64()?,
        model: message.pointer("/model/id").and_then(Value::as_str).unwrap_or_default().to_string(),
        cost: message.get("cost").and_then(Value::as_f64).filter(|cost| cost.is_finite() && *cost > 0.0).unwrap_or(0.0),
        input: tokens("/tokens/input"),
        output: tokens("/tokens/output"),
        reasoning: tokens("/tokens/reasoning"),
        cache_read: tokens("/tokens/cache/read"),
        cache_write: tokens("/tokens/cache/write"),
    })
}

/// The kind of the error for an HTTP status: 404 is a session or request the service does not
/// know (any more), 409 a question already answered.
fn error_kind(code: Option<u16>) -> io::ErrorKind {
    match code {
        Some(404) => io::ErrorKind::NotFound,
        Some(409) => io::ErrorKind::AlreadyExists,
        _ => io::ErrorKind::Other,
    }
}

/// What an error answer of the service says: its `message`, else its text.
fn said(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("message").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| String::from_utf8_lossy(body).trim().chars().take(200).collect())
}

fn dechunk(mut body: &[u8]) -> io::Result<Vec<u8>> {
    let broken = || io::Error::other("the OpenCode service sent a broken chunked answer");
    let mut out = Vec::new();
    loop {
        let line_end = body.windows(2).position(|window| window == b"\r\n").ok_or_else(broken)?;
        let size_text = String::from_utf8_lossy(&body[..line_end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or_default().trim(), 16).map_err(|_| broken())?;
        if size == 0 {
            return Ok(out);
        }
        let start = line_end + 2;
        out.extend_from_slice(body.get(start..start + size).ok_or_else(broken)?);
        body = body.get(start + size + 2..).ok_or_else(broken)?;
    }
}

impl Service {
    /// The service of the account in `dir`, or None when OpenCode never wrote its password: it
    /// never started.
    pub fn of(dir: &Path) -> io::Result<Option<Self>> {
        let file = account::config_dir(dir).join("service.json");
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io::Error::new(error.kind(), format!("cannot read {}: {error}", file.display()))),
        };
        let read: ServiceFile =
            serde_json::from_str(&text).map_err(|error| io::Error::other(format!("{} is not valid: {error}", file.display())))?;
        let Some(password) = read.password else {
            return Ok(None);
        };
        let authorization = format!("Basic {}", base64(format!("opencode:{password}").as_bytes()));
        Ok(Some(Self { port: read.port.unwrap_or(DEFAULT_PORT), authorization }))
    }

    /// The service of the account in `dir` if it runs: it answers.
    #[cfg(test)]
    pub(crate) fn on_port(port: u16) -> Self {
        Self { port, authorization: "Basic eA==".into() }
    }

    pub fn running(dir: &Path) -> io::Result<Option<Self>> {
        let Some(service) = Self::of(dir)? else {
            return Ok(None);
        };
        match service.request("GET", "/api/session/active", None) {
            Ok(_) => Ok(Some(service)),
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn request_text(&self, method: &str, path: &str, body: Option<&Value>) -> String {
        let body = body.map(Value::to_string).unwrap_or_default();
        format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: {}\r\nConnection: close\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            self.port,
            self.authorization,
            body.len()
        )
    }

    /// The JSON the service answers to `method path`, `null` for an empty answer.
    fn request(&self, method: &str, path: &str, body: Option<&Value>) -> io::Result<Value> {
        let address = SocketAddr::from(([127, 0, 0, 1], self.port));
        let mut stream = TcpStream::connect_timeout(&address, TIMEOUT)?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        stream.write_all(self.request_text(method, path, body).as_bytes())?;
        let mut response = Vec::new();
        stream.take(MAX_RESPONSE).read_to_end(&mut response)?;
        let body = parse_response(&response)?;
        if body.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&body).map_err(|error| io::Error::other(format!("the OpenCode service's answer to {path} is not JSON: {error}")))
    }

    fn items(&self, path: &str) -> io::Result<Vec<Value>> {
        Ok(data_items(&self.request("GET", path, None)?).collect())
    }

    /// The permissions the session `id` waits for: `[{id "per_…", action, resources, …}]`.
    pub fn permissions(&self, id: &str) -> io::Result<Vec<Value>> {
        self.items(&format!("/api/session/{id}/permission"))
    }

    /// The forms the session `id` waits for, its questions among them: `[{id "frm_…", metadata,
    /// fields, …}]`.
    pub fn forms(&self, id: &str) -> io::Result<Vec<Value>> {
        self.items(&format!("/api/session/{id}/form"))
    }

    /// Allows the permission `request` of the session `id` once, or refuses it.
    pub fn reply_permission(&self, id: &str, request: &str, allow: bool) -> io::Result<()> {
        let decision = if allow { "once" } else { "reject" };
        self.request("POST", &format!("/api/session/{id}/permission/{request}/reply"), Some(&json!({ "decision": decision }))).map(|_| ())
    }

    /// Answers the form `form` of the session `id`: `answer` holds a value for each field key.
    pub fn reply_form(&self, id: &str, form: &str, answer: Value) -> io::Result<()> {
        self.request("POST", &format!("/api/session/{id}/form/{form}/reply"), Some(&json!({ "answer": answer }))).map(|_| ())
    }

    /// Withdraws the form `form` of the session `id`, unanswered: the model goes on without it.
    pub fn cancel_form(&self, id: &str, form: &str) -> io::Result<()> {
        self.request("DELETE", &format!("/api/session/{id}/form/{form}"), None).map(|_| ())
    }

    /// What the running session `id` waits for, if anything.
    fn running_status(&self, id: &str) -> io::Result<Status> {
        if !self.permissions(id)?.is_empty() {
            return Ok(Status::WaitingOnPermission);
        }
        if !self.forms(id)?.is_empty() {
            return Ok(Status::WaitingOnAnswer);
        }
        Ok(Status::Working)
    }

    /// The sessions of the account: those running, then, with `include_stopped`, the most recent of
    /// the others. Not the sessions of its subagents, nor those archived.
    pub fn sessions(&self, include_stopped: bool) -> io::Result<Vec<Session>> {
        let running = active_ids(&self.request("GET", "/api/session/active", None)?);
        let listed = self.request("GET", &format!("/api/session?archived=false&parentID=null&order=desc&limit={STOPPED_LISTED}"), None)?;
        let mut infos: BTreeMap<String, Value> = data_items(&listed).map(|info| (id_of(&info), info)).collect();
        for id in &running {
            if !infos.contains_key(id) {
                let info = self.request("GET", &format!("/api/session/{id}"), None)?;
                infos.insert(id.clone(), info.get("data").cloned().unwrap_or(Value::Null));
            }
        }
        let mut sessions = Vec::new();
        for (id, info) in infos {
            if info.get("parentID").is_some_and(|parent| !parent.is_null()) || is_archived(&info) {
                continue;
            }
            let status = if running.contains(&id) { self.running_status(&id)? } else { Status::AtRest };
            if status == Status::AtRest && !include_stopped {
                continue;
            }
            sessions.push(session(id, &info, status));
        }
        Ok(sessions)
    }

    /// When each of the most recent sessions last changed (`time.updated`), in Unix seconds.
    pub fn updated_times(&self) -> io::Result<BTreeMap<String, i64>> {
        let listed = self.request("GET", &format!("/api/session?archived=false&parentID=null&order=desc&limit={STOPPED_LISTED}"), None)?;
        Ok(data_items(&listed).filter_map(|info| Some((id_of(&info), updated_seconds(&info)?))).collect())
    }

    /// Every page of `path`, a list of the service given 100 items at a time.
    fn all_items(&self, path: &str) -> io::Result<Vec<Value>> {
        let separator = if path.contains('?') { '&' } else { '?' };
        let page_path = format!("{path}{separator}limit={PAGE}");
        let mut items = Vec::new();
        let mut next = page_path.clone();
        for _ in 0..MAX_PAGES {
            let page = self.request("GET", &next, None)?;
            let count = items.len();
            items.extend(data_items(&page));
            // The last page has a cursor too, to an empty one.
            let cursor = page.pointer("/cursor/next").and_then(Value::as_str);
            let Some(cursor) = cursor.filter(|_| items.len() > count) else {
                return Ok(items);
            };
            next = format!("{page_path}&cursor={cursor}");
        }
        Ok(items)
    }

    /// Every session of the account, those archived and those of subagents included.
    pub fn used_sessions(&self) -> io::Result<Vec<UsedSession>> {
        Ok(self.all_items("/api/session")?.iter().filter_map(used_session).collect())
    }

    /// The replies of the session `id`.
    pub fn replies(&self, id: &str) -> io::Result<Vec<Reply>> {
        Ok(self.all_items(&format!("/api/session/{id}/message?type=assistant"))?.iter().filter_map(reply).collect())
    }

    /// The folder the session `id` works in, if the service knows it.
    pub fn directory(&self, id: &str) -> io::Result<Option<PathBuf>> {
        let info = self.request("GET", &format!("/api/session/{id}"), None)?;
        Ok(directory_of(info.get("data").unwrap_or(&Value::Null)).filter(|directory| !directory.is_empty()).map(PathBuf::from))
    }

    /// Starts a session in `directory`, titled `title` unless it is empty. It waits for its first
    /// message. Returns its id.
    pub fn create(&self, directory: &Path, title: &str) -> io::Result<String> {
        let answer = self.request("POST", "/api/session", Some(&create_body(directory, title)))?;
        answer
            .get("data")
            .and_then(|data| data.get("id"))
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| io::Error::other("the OpenCode service gave no id for the new session"))
    }

    /// Stops the turn the session `id` runs, if any: the session stays, at rest.
    pub fn interrupt(&self, id: &str) -> io::Result<()> {
        self.request("POST", &format!("/api/session/{id}/interrupt"), None).map(|_| ())
    }

    /// Archives the session `id`: it leaves the lists of OpenCode and of kindric, and
    /// `PATCH {"archived": false}` brings it back.
    pub fn archive(&self, id: &str) -> io::Result<()> {
        self.request("PATCH", &format!("/api/session/{id}"), Some(&json!({ "archived": true }))).map(|_| ())
    }

    /// Gives the session `id` `text` as the user's message: it starts a turn at rest, or waits for
    /// the end of the one running.
    pub fn prompt(&self, id: &str, text: &str) -> io::Result<()> {
        self.request("POST", &format!("/api/session/{id}/prompt"), Some(&json!({ "text": text, "delivery": "queue" }))).map(|_| ())
    }

    /// The messages of the session `id`, newest first: `user` ones with their `text`, `assistant`
    /// ones with their `content` (`text` and `tool` parts), `idle` ones at the end of each turn.
    pub fn messages(&self, id: &str) -> io::Result<Vec<Value>> {
        let answer = self.request("GET", &format!("/api/session/{id}/message"), None)?;
        Ok(match answer {
            Value::Array(items) => items,
            other => data_items(&other).collect(),
        })
    }
}

fn create_body(directory: &Path, title: &str) -> Value {
    let mut body = json!({ "location": { "directory": directory } });
    if !title.trim().is_empty() {
        body["title"] = json!(title.trim());
    }
    body
}

fn active_ids(answer: &Value) -> Vec<String> {
    answer.get("data").and_then(Value::as_object).map(|running| running.keys().cloned().collect()).unwrap_or_default()
}

fn data_items(answer: &Value) -> impl Iterator<Item = Value> + '_ {
    answer.get("data").and_then(Value::as_array).into_iter().flatten().cloned()
}

fn id_of(info: &Value) -> String {
    info.get("id").and_then(Value::as_str).unwrap_or_default().to_string()
}

fn is_archived(info: &Value) -> bool {
    info.get("time").and_then(|time| time.get("archived")).is_some_and(|archived| !archived.is_null())
}

/// `time.updated`, which OpenCode gives in milliseconds, in seconds.
fn updated_seconds(info: &Value) -> Option<i64> {
    let millis = info.get("time")?.get("updated")?.as_f64()?;
    Some((millis / 1000.0) as i64)
}

fn directory_of(info: &Value) -> Option<String> {
    info.get("location").and_then(|location| location.get("directory")).and_then(Value::as_str).map(String::from)
}

fn session(id: String, info: &Value, status: Status) -> Session {
    let title = info.get("title").and_then(Value::as_str).unwrap_or_default().to_string();
    Session { id, title, directory: directory_of(info).unwrap_or_default(), status }
}

/// Starts the service of the account in `dir` unless it runs, with the environment of
/// `kindric-accounts run`: the service passes its own to the commands of every session.
pub fn ensure_service(dir: &Path) -> io::Result<Service> {
    if let Some(service) = Service::running(dir)? {
        return Ok(service);
    }
    let opencode = account::require_opencode().map_err(|error| io::Error::new(io::ErrorKind::NotFound, error.0))?;
    let output = account::command(&opencode, dir).args(["service", "start"]).stdin(Stdio::null()).output()?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        let last = said.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("no message");
        return Err(io::Error::other(format!("opencode service start: {}", last.trim())));
    }
    Service::running(dir)?.ok_or_else(|| io::Error::other("the OpenCode service started but does not answer"))
}

/// The sessions of the OpenCode account in `dir`: those running, then, with `include_stopped`,
/// the most recent of the others. Those at rest live in the service alone: with
/// `include_stopped`, a service that does not run is started, and left running (decision 440).
/// The sessions of the account in `dir`, those at rest included, while its service runs; none
/// else, and the service is not started for that.
pub fn sessions_if_running(dir: &Path) -> io::Result<Vec<Session>> {
    match Service::running(dir)? {
        Some(service) => service.sessions(true),
        None => Ok(Vec::new()),
    }
}

/// Gives the session `id` of the account in `dir` a prompt, queued after the turn it runs: how a
/// session of OpenCode, which has no hooks, learns of a message.
pub fn tell(dir: &Path, id: &str, text: &str) -> io::Result<()> {
    running_service(dir)?.prompt(id, text)
}

/// The messages of the session `id` of the account in `dir`, newest first (see `Service::messages`).
pub fn session_messages(dir: &Path, id: &str) -> io::Result<Vec<Value>> {
    running_service(dir)?.messages(id)
}

fn running_service(dir: &Path) -> io::Result<Service> {
    Service::running(dir)?.ok_or_else(|| io::Error::other("its OpenCode service does not run"))
}

pub fn sessions(dir: &Path, include_stopped: bool) -> io::Result<Vec<Session>> {
    let service = if include_stopped { Some(ensure_service(dir)?) } else { Service::running(dir)? };
    match service {
        Some(service) => service.sessions(include_stopped),
        None => Ok(Vec::new()),
    }
}

/// A fake service for the tests of this crate.
#[cfg(test)]
pub(crate) mod fake {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc::{self, Receiver};
    use std::thread;

    /// A port nothing listens on: below 1024, no program of the user's takes it. A port just freed
    /// will not do: a process another test starts may hold it open a moment longer.
    pub const NO_SERVICE: u16 = 1;

    /// A fake service on a free port: answers each request with the body its path maps to, as the
    /// real one does (content-length, connection closed), 404 for any other path. Returns the port
    /// and each request seen, whole.
    pub fn service(routes: Vec<(&'static str, &'static str)>) -> (u16, Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen, requests) = mpsc::channel();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut buffer = [0u8; 16384];
                let read = stream.read(&mut buffer).unwrap();
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let line = request.lines().next().unwrap_or_default().to_string();
                let target = line.split_whitespace().nth(1).unwrap_or_default().to_string();
                let found = routes.iter().find(|(path, _)| target == *path);
                let (code, body) = found.map_or(("404 Not Found", "{\"message\":\"no route\"}"), |(_, body)| ("200 OK", body));
                let answer = format!("HTTP/1.1 {code}\r\ncontent-length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(answer.as_bytes()).unwrap();
                if seen.send(request).is_err() {
                    return;
                }
            }
        });
        (port, requests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_4648() {
        let cases = [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")];
        for (plain, encoded) in cases {
            assert_eq!(base64(plain.as_bytes()), encoded, "{plain:?}");
        }
    }

    #[test]
    fn a_response_is_read_by_its_length_or_its_chunks() {
        let sized = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 11\r\n\r\n{\"data\":[]}trailing";
        assert_eq!(parse_response(sized).unwrap(), b"{\"data\":[]}");
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"da\r\n7;x=1\r\nta\":[]}\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked).unwrap(), b"{\"data\":[]}");
        assert_eq!(parse_response(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap(), b"");
    }

    #[test]
    fn an_error_answer_says_what_the_service_said() {
        let body = r#"{"_tag":"SessionNotFoundError","sessionID":"ses_x","message":"Session not found: ses_x"}"#;
        let missing = format!("HTTP/1.1 404 Not Found\r\ncontent-length: {}\r\n\r\n{body}", body.len());
        let error = parse_response(missing.as_bytes()).unwrap_err().to_string();
        assert!(error.contains("404") && error.contains("Session not found: ses_x"), "{error}");
        assert!(parse_response(b"HTTP/1.1 401 Unauthorized\r\n\r\n").is_err());
        assert!(parse_response(b"garbage").is_err());
        assert!(parse_response(b"HTTP/1.1 200 OK\r\ncontent-length: 50\r\n\r\nshort").is_err());
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n").is_err());
    }

    fn account_with(label: &str, file: Option<&str>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kindric-opencode-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        if let Some(text) = file {
            fs::create_dir_all(account::config_dir(&dir)).unwrap();
            fs::write(account::config_dir(&dir).join("service.json"), text).unwrap();
        }
        dir
    }

    #[test]
    fn the_service_file_gives_the_port_and_the_password() {
        assert!(Service::of(&account_with("none", None)).unwrap().is_none(), "never set up");
        assert!(Service::of(&account_with("port", Some(r#"{"port": 49375}"#))).unwrap().is_none(), "never started: no password");
        let service = Service::of(&account_with("both", Some(r#"{"port": 49375, "password": "secret"}"#))).unwrap().unwrap();
        assert_eq!(service.port, 49375);
        assert_eq!(service.authorization, format!("Basic {}", base64(b"opencode:secret")));
        let default = Service::of(&account_with("default", Some(r#"{"password": "secret"}"#))).unwrap().unwrap();
        assert_eq!(default.port, DEFAULT_PORT);
        assert!(Service::of(&account_with("broken", Some("{"))).is_err());
    }

    #[test]
    fn a_request_names_its_body_length_and_closes() {
        let service = Service { port: 49375, authorization: "Basic eA==".into() };
        let text = service.request_text("PATCH", "/api/session/ses_a", Some(&json!({ "archived": true })));
        assert!(text.starts_with("PATCH /api/session/ses_a HTTP/1.1\r\n"), "{text}");
        assert!(text.contains("\r\nAuthorization: Basic eA==\r\n") && text.contains("\r\nConnection: close\r\n"));
        assert!(text.ends_with("Content-Length: 17\r\n\r\n{\"archived\":true}"), "{text}");
        assert!(service.request_text("GET", "/api/session", None).ends_with("Content-Length: 0\r\n\r\n"));
    }

    #[test]
    fn a_new_session_gets_its_folder_and_a_title_only_if_given() {
        assert_eq!(create_body(Path::new("/w/api"), " review "), json!({ "location": { "directory": "/w/api" }, "title": "review" }));
        assert_eq!(create_body(Path::new("/w/api"), " "), json!({ "location": { "directory": "/w/api" } }));
    }

    fn fake_service(routes: Vec<(&'static str, &'static str)>) -> (u16, std::sync::mpsc::Receiver<String>) {
        fake::service(routes)
    }

    /// The first line of the next request the fake service saw.
    fn first_line(requests: &std::sync::mpsc::Receiver<String>) -> String {
        requests.recv().unwrap().lines().next().unwrap_or_default().to_string()
    }

    const LIST: &str = "/api/session?archived=false&parentID=null&order=desc&limit=50";

    fn sessions_of(routes: Vec<(&'static str, &'static str)>, include_stopped: bool) -> Vec<Session> {
        let (port, _requests) = fake_service(routes);
        Service { port, authorization: "Basic eA==".into() }.sessions(include_stopped).unwrap()
    }

    #[test]
    fn running_sessions_say_what_they_wait_for_and_the_others_rest() {
        let routes = vec![
            ("/api/session/active", r#"{"data":{"ses_perm":{"type":"running"},"ses_form":{"type":"running"},"ses_work":{"type":"running"}}}"#),
            (
                LIST,
                r#"{"data":[
                {"id":"ses_perm","title":"perm","location":{"directory":"/w/a"},"time":{"created":3,"updated":3}},
                {"id":"ses_form","title":"form","location":{"directory":"/w/b"},"time":{"created":2,"updated":2}},
                {"id":"ses_rest","title":"rest","location":{"directory":"/w/c"},"time":{"created":1,"updated":1}}],"cursor":{}}"#,
            ),
            ("/api/session/ses_work", r#"{"data":{"id":"ses_work","title":"","location":{"directory":"/w/d"},"time":{"created":0,"updated":0}}}"#),
            ("/api/session/ses_perm/permission", r#"{"data":[{"id":"per_1","sessionID":"ses_perm","action":"shell"}]}"#),
            ("/api/session/ses_form/permission", r#"{"data":[]}"#),
            ("/api/session/ses_form/form", r#"{"data":[{"id":"frm_1","sessionID":"ses_form"}]}"#),
            ("/api/session/ses_work/permission", r#"{"data":[]}"#),
            ("/api/session/ses_work/form", r#"{"data":[]}"#),
        ];
        let all = sessions_of(routes.clone(), true);
        let status = |id: &str| all.iter().find(|session| session.id == id).map(|session| session.status.clone());
        assert_eq!(status("ses_perm"), Some(Status::WaitingOnPermission));
        assert_eq!(status("ses_form"), Some(Status::WaitingOnAnswer));
        assert_eq!(status("ses_work"), Some(Status::Working), "running, older than the fifty listed");
        assert_eq!(status("ses_rest"), Some(Status::AtRest));
        let perm = all.iter().find(|session| session.id == "ses_perm").unwrap();
        assert_eq!((perm.title.as_str(), perm.directory.as_str()), ("perm", "/w/a"));
        let (port, _requests) = fake_service(routes.clone());
        let times = Service { port, authorization: "Basic eA==".into() }.updated_times().unwrap();
        assert_eq!((times.len(), times.get("ses_perm")), (3, Some(&0)), "the fifty listed, 3 ms rounded down");
        assert_eq!(updated_seconds(&json!({ "time": { "updated": 1795341309318_i64 } })), Some(1795341309));
        let live = sessions_of(routes, false);
        assert_eq!(live.len(), 3, "without the sessions at rest: {live:?}");
        assert!(live.iter().all(|session| session.status != Status::AtRest));
    }

    #[test]
    fn subagents_and_archived_sessions_are_left_out() {
        let routes = vec![
            ("/api/session/active", r#"{"data":{"ses_child":{"type":"running"}}}"#),
            (LIST, r#"{"data":[{"id":"ses_old","title":"old","location":{"directory":"/w"},"time":{"created":1,"updated":1,"archived":5}}],"cursor":{}}"#),
            ("/api/session/ses_child", r#"{"data":{"id":"ses_child","parentID":"ses_main","title":"sub","location":{"directory":"/w"},"time":{"created":2,"updated":2}}}"#),
        ];
        assert_eq!(sessions_of(routes, true), Vec::new());
    }

    #[test]
    fn a_service_that_does_not_run_is_none_and_one_that_answers_is_found() {
        let stopped = account_with("stopped", Some(&format!(r#"{{"port": {}, "password": "p"}}"#, fake::NO_SERVICE)));
        assert!(Service::running(&stopped).unwrap().is_none());
        let (port, requests) = fake_service(vec![("/api/session/active", r#"{"data":{}}"#)]);
        let running = account_with("running", Some(&format!(r#"{{"port": {port}, "password": "p"}}"#)));
        assert!(Service::running(&running).unwrap().is_some());
        assert_eq!(first_line(&requests), "GET /api/session/active HTTP/1.1");
    }

    #[test]
    fn interrupt_archive_prompt_and_create_ask_the_right_paths() {
        let (port, requests) = fake_service(vec![
            ("/api/session/ses_a/interrupt", r#"{"interrupted":true}"#),
            ("/api/session/ses_a", ""),
            ("/api/session/ses_a/prompt", r#"{"data":{"id":"msg_1"}}"#),
            ("/api/session", r#"{"data":{"id":"ses_new"}}"#),
        ]);
        let service = Service { port, authorization: "Basic eA==".into() };
        service.interrupt("ses_a").unwrap();
        assert_eq!(first_line(&requests), "POST /api/session/ses_a/interrupt HTTP/1.1");
        service.archive("ses_a").unwrap();
        assert_eq!(first_line(&requests), "PATCH /api/session/ses_a HTTP/1.1");
        service.prompt("ses_a", "go on").unwrap();
        assert_eq!(first_line(&requests), "POST /api/session/ses_a/prompt HTTP/1.1");
        assert_eq!(service.create(Path::new("/w"), "t").unwrap(), "ses_new");
        let error = service.interrupt("ses_gone").unwrap_err().to_string();
        assert!(error.contains("404"), "{error}");
    }
}
