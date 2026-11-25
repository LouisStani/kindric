//! Figures over the transcripts of Claude Code, those of Codex and the sessions of OpenCode:
//! tokens, replies, messages typed, what they cost, and the spans of time each session was active
//! (decisions 347, 384, 387, 470). There can be gigabytes of transcripts: a file is read once, then
//! only what was appended since, and what each file said is kept in a cache. The transcripts are
//! not documented: a line that cannot be read counts for nothing. OpenCode keeps a database: its
//! service is asked, when it runs, for the sessions that changed. Claude Code removes its transcripts
//! after about a month: the figures of those gone are kept in a file of their own (decision 472).
//!
//! Figures are added up per hour in UTC; the page turns hours into its own local days.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use kindric_accounts::agent::{self, Agent};
use kindric_accounts::usage::Cost;
use kindric_accounts::Paths;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::opencode;

/// Bumped when what the cache holds changes: an older cache is then read again from scratch.
const VERSION: u32 = 2;
const CACHE_FILE: &str = "stats.json";
/// The figures of the sessions whose transcript is gone, next to the cache. Never read again from
/// anywhere: `VERSION` does not apply to it.
const GONE_FILE: &str = "stats-gone.json";
/// A pause longer than this ends a span of activity.
const IDLE_GAP_MS: i64 = 30 * 60 * 1000;
const MS_PER_HOUR: i64 = 3_600_000;
/// The model of the replies Claude Code writes itself (an API error): no tokens, not a reply.
const SYNTHETIC_MODEL: &str = "<synthetic>";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
    pub thinking: u64,
    pub web_searches: u64,
    /// Replies of Claude, once each however many lines of the transcript they took.
    pub replies: u64,
    /// Messages the user typed.
    pub prompts: u64,
}

impl Tokens {
    fn add(&mut self, other: &Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_creation += other.cache_creation;
        self.thinking += other.thinking;
        self.web_searches += other.web_searches;
        self.replies += other.replies;
        self.prompts += other.prompts;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Figures {
    pub tokens: Tokens,
    /// Model → replies.
    pub models: BTreeMap<String, u64>,
    /// Millionths of a US dollar, for what is paid by use and whose cost is known.
    pub cost: u64,
}

impl Figures {
    fn add(&mut self, other: &Figures) {
        self.tokens.add(&other.tokens);
        for (model, replies) in &other.models {
            *self.models.entry(model.clone()).or_default() += replies;
        }
        self.cost += other.cost;
    }
}

/// A stretch of activity without a pause longer than `IDLE_GAP_MS`, in Unix milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: i64,
    pub end: i64,
}

/// What one transcript said, up to `read`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Transcript {
    account: String,
    session: String,
    /// A subagent's transcript: its tokens count for its session, its time does not.
    agent: bool,
    project: Option<String>,
    /// Bytes read: up to the end of the last whole line.
    read: u64,
    /// A reply takes one line per block, each with the same usage, one after the other.
    last_reply: Option<String>,
    /// Codex: the model of its turns, and the tokens counted up to the last reply.
    model: String,
    counted: CodexTotal,
    /// Unix hour (UTC) → figures.
    hours: BTreeMap<i64, Figures>,
    spans: Vec<Span>,
}

/// What every transcript read so far said, by path.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    version: u32,
    files: BTreeMap<String, Transcript>,
    /// What the transcripts now gone said, in `GONE_FILE`.
    #[serde(skip)]
    gone: BTreeMap<String, Transcript>,
    /// Why `GONE_FILE` could not be read: it is then left as it is.
    #[serde(skip)]
    gone_unread: Option<String>,
    #[serde(skip)]
    gone_changed: bool,
}

/// What tells a transcript from another, wherever its file is: Codex moves those it archives.
type Identity = (String, String, bool, String);

fn identity(key: &str, transcript: &Transcript) -> Identity {
    let name = key.rsplit(['/', '\\']).next().unwrap_or(key);
    (transcript.account.clone(), transcript.session.clone(), transcript.agent, name.to_string())
}

fn gone_file(cache: &Path) -> PathBuf {
    cache.with_file_name(GONE_FILE)
}

fn read_gone(path: &Path) -> Result<BTreeMap<String, Transcript>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| format!("{} is not valid, the figures of the transcripts gone are left out: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(format!("cannot read {}, the figures of the transcripts gone are left out: {error}", path.display())),
    }
}

/// An account whose sessions count, where its files are, and the program it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub account: String,
    pub dir: PathBuf,
    pub agent: Agent,
}

impl Source {
    /// An unreadable agent file counts as Claude Code, which every older account runs.
    pub fn of(account: String, dir: PathBuf) -> Self {
        let agent = agent::read(&dir).unwrap_or_default();
        Self { account, dir, agent }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HourRow {
    /// Unix hour (UTC).
    pub hour: i64,
    pub account: String,
    pub project: String,
    #[serde(flatten)]
    pub figures: Figures,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionRow {
    pub account: String,
    pub session: String,
    pub project: String,
    /// Its subagents' included.
    pub tokens: Tokens,
    /// Millionths of a US dollar.
    pub cost: u64,
    pub spans: Vec<Span>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub hours: Vec<HourRow>,
    pub sessions: Vec<SessionRow>,
}

/// Where the cache is kept, among kindric's files.
pub fn cache_file(paths: &Paths) -> io::Result<PathBuf> {
    paths
        .kindric_data_dir()
        .map(|dir| dir.join(CACHE_FILE))
        .ok_or_else(|| io::Error::other("kindric-accounts data has no parent directory"))
}

impl Cache {
    /// The cache in `path`. A missing, broken or older one is empty: everything is read again,
    /// except the transcripts gone, whose figures are in a file of their own.
    pub fn load(path: &Path) -> Self {
        let mut cache = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Cache>(&bytes).ok())
            .filter(|cache| cache.version == VERSION)
            .unwrap_or_default();
        match read_gone(&gone_file(path)) {
            Ok(gone) => cache.gone = gone,
            Err(error) => cache.gone_unread = Some(error),
        }
        cache
    }

    /// The figures of the transcripts gone first: until they are written, the cache still has them.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let write = |path: &Path, text: &str| {
            kindric_accounts::files::write_atomic(path, text)
                .map_err(|error| io::Error::new(error.kind(), format!("cannot write {}: {error}", path.display())))
        };
        if self.gone_changed && self.gone_unread.is_none() {
            write(&gone_file(path), &serde_json::to_string(&self.gone).map_err(io::Error::other)?)?;
        }
        write(path, &serde_json::to_string(self).map_err(io::Error::other)?)
    }

    /// Reads what the transcripts of `sources` gained since the last time. The figures of those gone
    /// stay, and those of the accounts of `unseen` as they were; other accounts are forgotten.
    /// Returns what could not be read.
    pub fn refresh(&mut self, sources: &[Source], unseen: &[&str]) -> Vec<String> {
        let mut problems: Vec<String> = self.gone_unread.iter().cloned().collect();
        let mut kept = BTreeMap::new();
        for source in sources {
            let (listed, read_lines): (_, LineReader) = match source.agent {
                Agent::ClaudeCode => (transcripts(&source.dir, &mut problems), claude_line),
                Agent::Codex => (rollouts(&source.dir, &mut problems), codex_line),
                Agent::OpenCode => {
                    self.refresh_opencode(source, &mut kept, &mut problems);
                    continue;
                },
            };
            for found in listed {
                let key = found.path.to_string_lossy().into_owned();
                let previous = self.files.remove(&key).filter(|old| old.account == source.account);
                match update(previous.clone(), &source.account, &found, read_lines) {
                    Ok(transcript) => {
                        kept.insert(key, transcript);
                    },
                    Err(error) => {
                        problems.push(format!("cannot read {}: {error}", found.path.display()));
                        kept.extend(previous.map(|previous| (key, previous)));
                    },
                }
            }
        }
        let mut left = std::mem::take(&mut self.files);
        left.retain(|key, transcript| {
            let unseen = unseen.contains(&transcript.account.as_str());
            if unseen {
                kept.insert(key.clone(), transcript.clone());
            }
            !unseen
        });
        self.version = VERSION;
        self.files = kept;
        let counted: BTreeSet<&str> = sources.iter().map(|source| source.account.as_str()).chain(unseen.iter().copied()).collect();
        self.keep_gone(left, &counted);
        problems
    }

    /// Adds `left`, the transcripts known that were not found, to those gone; a transcript found
    /// again, and the accounts that are not `counted` any more, leave them.
    fn keep_gone(&mut self, left: BTreeMap<String, Transcript>, counted: &BTreeSet<&str>) {
        if self.gone_unread.is_some() {
            // Not to be written over: they stay in the cache until it can be read again.
            self.files.extend(left.into_iter().filter(|(_, transcript)| counted.contains(transcript.account.as_str())));
            return;
        }
        let before = self.gone.clone();
        self.gone.extend(left.into_iter().filter(|(_, transcript)| !transcript.hours.is_empty() || !transcript.spans.is_empty()));
        let found: BTreeSet<Identity> = self.files.iter().map(|(key, transcript)| identity(key, transcript)).collect();
        self.gone.retain(|key, transcript| {
            counted.contains(transcript.account.as_str()) && !found.contains(&identity(key, transcript))
        });
        self.gone_changed |= self.gone != before;
    }

    fn all(&self) -> impl Iterator<Item = &Transcript> {
        self.files.values().chain(self.gone.values())
    }

    /// Reads the sessions of an OpenCode account that changed. What was read stays while its
    /// service does not run: it is not started for this.
    fn refresh_opencode(&mut self, source: &Source, kept: &mut BTreeMap<String, Transcript>, problems: &mut Vec<String>) {
        let mut known: BTreeMap<String, Transcript> = BTreeMap::new();
        self.files.retain(|key, transcript| {
            let ours = transcript.account == source.account && key.starts_with(OPENCODE_KEY);
            if ours {
                known.insert(key.clone(), transcript.clone());
            }
            !ours
        });
        match read_opencode(source, &known, problems) {
            Ok(Some(read)) => {
                self.files.extend(known.into_iter().filter(|(key, _)| !read.contains_key(key)));
                kept.extend(read);
            },
            Ok(None) => kept.extend(known),
            Err(error) => {
                problems.push(format!("cannot read the OpenCode sessions of {}: {error}", source.account));
                kept.extend(known);
            },
        }
    }

    /// Figures added up per hour, account and project, then per session. `costs`: account → the
    /// costs its status lines noted.
    pub fn summary(&self, costs: &BTreeMap<String, Vec<Cost>>) -> Summary {
        let mut hours: BTreeMap<(i64, &str, &str), Figures> = BTreeMap::new();
        let mut sessions: BTreeMap<(&str, &str), SessionRow> = BTreeMap::new();
        for transcript in self.all() {
            let project = transcript.project.as_deref().unwrap_or("");
            let session = sessions.entry((&transcript.account, &transcript.session)).or_insert_with(|| SessionRow {
                account: transcript.account.clone(),
                session: transcript.session.clone(),
                project: project.to_string(),
                tokens: Tokens::default(),
                cost: 0,
                spans: Vec::new(),
            });
            if !transcript.agent {
                session.project = project.to_string();
                session.spans.extend_from_slice(&transcript.spans);
            }
            for (hour, figures) in &transcript.hours {
                session.tokens.add(&figures.tokens);
                session.cost += figures.cost;
                hours.entry((*hour, &transcript.account, project)).or_default().add(figures);
            }
        }
        // Only for a session whose transcript is there: without one, its project is unknown.
        for (account, noted) in costs {
            for (session_id, hour, micros) in cost_steps(noted) {
                let Some(session) = sessions.get_mut(&(account.as_str(), session_id)) else {
                    continue;
                };
                session.cost += micros;
                let project = self.project_of(account, session_id);
                hours.entry((hour, account, project)).or_default().cost += micros;
            }
        }
        Summary {
            hours: hours
                .into_iter()
                .map(|((hour, account, project), figures)| HourRow {
                    hour,
                    account: account.to_string(),
                    project: project.to_string(),
                    figures,
                })
                .collect(),
            sessions: sessions.into_values().collect(),
        }
    }

    fn project_of(&self, account: &str, session: &str) -> &str {
        self.all()
            .find(|transcript| transcript.account == account && transcript.session == session && !transcript.agent)
            .and_then(|transcript| transcript.project.as_deref())
            .unwrap_or("")
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Saturates, and is never negative.
fn micros(dollars: f64) -> u64 {
    (dollars * 1e6).round().max(0.0) as u64
}

/// What each note adds to the cost of its session: `(session, hour, millionths of a dollar)`. A
/// total lower than the one before started again from nothing.
fn cost_steps(noted: &[Cost]) -> Vec<(&str, i64, u64)> {
    let mut totals: BTreeMap<&str, u64> = BTreeMap::new();
    noted
        .iter()
        .filter_map(|cost| {
            let total = micros(cost.total_usd);
            let before = totals.insert(&cost.session_id, total).unwrap_or(0);
            let step = if total >= before { total - before } else { total };
            (step > 0).then(|| (cost.session_id.as_str(), cost.updated_at.div_euclid(3600), step))
        })
        .collect()
}

/// The start of the key of an OpenCode session in the cache, where a transcript has its path.
const OPENCODE_KEY: &str = "opencode:";

/// The sessions of an OpenCode account as its service has them now, those of `known` that did not
/// change taken as they are; `None` when the service does not run.
fn read_opencode(source: &Source, known: &BTreeMap<String, Transcript>, problems: &mut Vec<String>) -> io::Result<Option<BTreeMap<String, Transcript>>> {
    let Some(service) = opencode::Service::running(&source.dir)? else {
        return Ok(None);
    };
    let mut read = BTreeMap::new();
    for session in service.used_sessions()? {
        let key = format!("{OPENCODE_KEY}{}:{}", source.account, session.id);
        let updated = u64::try_from(session.updated).unwrap_or(0);
        match known.get(&key) {
            Some(same) if same.read == updated => {
                read.insert(key, same.clone());
            },
            previous => match service.replies(&session.id) {
                Ok(replies) => {
                    read.insert(key, opencode_transcript(&source.account, &session, updated, &replies));
                },
                Err(error) => {
                    problems.push(format!("cannot read the OpenCode session {} of {}: {error}", session.id, source.account));
                    read.extend(previous.map(|old| (key, old.clone())));
                },
            },
        }
    }
    Ok(Some(read))
}

fn opencode_transcript(account: &str, session: &opencode::UsedSession, updated: u64, replies: &[opencode::Reply]) -> Transcript {
    let mut transcript = Transcript {
        account: account.to_string(),
        session: session.parent.clone().unwrap_or_else(|| session.id.clone()),
        agent: session.parent.is_some(),
        project: Some(session.directory.clone()).filter(|directory| !directory.is_empty()),
        read: updated,
        ..Transcript::default()
    };
    let mut times: Vec<i64> = Vec::new();
    for reply in replies {
        let figures = transcript.hours.entry(reply.at.div_euclid(MS_PER_HOUR)).or_default();
        figures.tokens.add(&Tokens {
            input: reply.input,
            output: reply.output + reply.reasoning,
            cache_read: reply.cache_read,
            cache_creation: reply.cache_write,
            thinking: reply.reasoning,
            replies: 1,
            ..Tokens::default()
        });
        figures.cost += micros(reply.cost);
        *figures.models.entry(reply.model.clone()).or_default() += 1;
        times.push(reply.at);
    }
    if !transcript.agent {
        times.sort_unstable();
        for time in times {
            extend_spans(&mut transcript.spans, time);
        }
    }
    transcript
}

struct Found {
    path: PathBuf,
    session: String,
    agent: bool,
}

/// `<dir>/projects/<project>/<session>.jsonl`, and the subagents of a session in
/// `<dir>/projects/<project>/<session>/subagents/*.jsonl`. A missing directory holds none.
fn transcripts(dir: &Path, problems: &mut Vec<String>) -> Vec<Found> {
    let mut found = Vec::new();
    for project in entries(&dir.join("projects"), problems) {
        for entry in entries(&project, problems) {
            let Some(name) = entry.file_name().map(|name| name.to_string_lossy().into_owned()) else {
                continue;
            };
            if let Some(session) = name.strip_suffix(".jsonl") {
                found.push(Found { path: entry, session: session.to_string(), agent: false });
                continue;
            }
            let agents = entry.join("subagents");
            if !agents.is_dir() {
                continue;
            }
            for agent in entries(&agents, problems) {
                if agent.extension().is_some_and(|extension| extension == "jsonl") {
                    found.push(Found { path: agent, session: name.clone(), agent: true });
                }
            }
        }
    }
    found
}

/// The length of a session id of Codex, a UUID, at the end of the name of its file.
const UUID_LENGTH: usize = 36;

/// The sessions of a Codex account: `<dir>/sessions/<year>/<month>/<day>/rollout-<time>-<id>.jsonl`,
/// and those archived, in `<dir>/archived_sessions`.
fn rollouts(dir: &Path, problems: &mut Vec<String>) -> Vec<Found> {
    let mut files = entries(&dir.join("archived_sessions"), problems);
    for year in entries(&dir.join("sessions"), problems) {
        for month in folders(&year, problems) {
            for day in folders(&month, problems) {
                files.extend(folders(&day, problems));
            }
        }
    }
    files
        .into_iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
            let session = name.get(name.len().checked_sub(UUID_LENGTH)?..)?.to_string();
            Some(Found { path, session, agent: false })
        })
        .collect()
}

/// What `path` holds, nothing for a file.
fn folders(path: &Path, problems: &mut Vec<String>) -> Vec<PathBuf> {
    if path.is_dir() { entries(path, problems) } else { Vec::new() }
}

fn entries(dir: &Path, problems: &mut Vec<String>) -> Vec<PathBuf> {
    match fs::read_dir(dir) {
        Ok(read) => read.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            problems.push(format!("cannot list {}: {error}", dir.display()));
            Vec::new()
        }
    }
}

/// `previous` with what was appended to the file since. A file shorter than what was read was
/// written again: it is read from the start.
fn update(previous: Option<Transcript>, account: &str, found: &Found, read_line: LineReader) -> io::Result<Transcript> {
    let size = fs::metadata(&found.path)?.len();
    let mut transcript = previous.filter(|old| old.read <= size).unwrap_or_else(|| Transcript {
        account: account.to_string(),
        session: found.session.clone(),
        agent: found.agent,
        ..Transcript::default()
    });
    if transcript.read == size {
        return Ok(transcript);
    }
    let mut file = File::open(&found.path)?;
    file.seek(SeekFrom::Start(transcript.read))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let used = feed(&mut transcript, &bytes, read_line);
    transcript.read += used as u64;
    Ok(transcript)
}

/// Counts one line of a transcript.
type LineReader = fn(&mut Transcript, &[u8]);

fn claude_line(transcript: &mut Transcript, line: &[u8]) {
    if let Ok(line) = serde_json::from_slice::<Line>(line) {
        count(transcript, line);
    }
}

/// The tokens of a Codex session since it started, as its last reply said. Those read from the
/// cache are part of `input_tokens`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct CodexTotal {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
}

#[derive(Deserialize)]
struct CodexLine {
    timestamp: Option<String>,
    payload: Option<CodexPayload>,
}

#[derive(Deserialize)]
struct CodexPayload {
    #[serde(rename = "type")]
    kind: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
    info: Option<CodexInfo>,
}

#[derive(Deserialize)]
struct CodexInfo {
    total_token_usage: Option<CodexTotal>,
}

/// A turn starts with a message of the user, written apart in a form that also holds what Codex
/// adds. A reply is what the totals gained since the line before: the same totals may come twice.
fn codex_line(transcript: &mut Transcript, line: &[u8]) {
    let Ok(CodexLine { timestamp, payload: Some(payload) }) = serde_json::from_slice::<CodexLine>(line) else {
        return;
    };
    if transcript.project.is_none() {
        transcript.project = payload.cwd;
    }
    if let Some(model) = payload.model {
        transcript.model = model;
    }
    let Some(time) = timestamp.as_deref().and_then(unix_ms) else {
        return;
    };
    let hour = time.div_euclid(MS_PER_HOUR);
    match (payload.kind.as_deref(), payload.info.and_then(|info| info.total_token_usage)) {
        (Some("task_started"), _) => transcript.hours.entry(hour).or_default().tokens.prompts += 1,
        (Some("token_count"), Some(total)) if total != transcript.counted => {
            // Totals lower than before started again from nothing.
            let before = if total.input_tokens >= transcript.counted.input_tokens { transcript.counted } else { CodexTotal::default() };
            transcript.counted = total;
            let cached = total.cached_input_tokens.saturating_sub(before.cached_input_tokens);
            let figures = transcript.hours.entry(hour).or_default();
            figures.tokens.add(&Tokens {
                input: total.input_tokens.saturating_sub(before.input_tokens).saturating_sub(cached),
                output: total.output_tokens.saturating_sub(before.output_tokens),
                cache_read: cached,
                thinking: total.reasoning_output_tokens.saturating_sub(before.reasoning_output_tokens),
                replies: 1,
                ..Tokens::default()
            });
            *figures.models.entry(transcript.model.clone()).or_default() += 1;
        },
        _ => return,
    }
    extend_spans(&mut transcript.spans, time);
}

/// Counts the whole lines of `bytes`; returns how many bytes they took. A last line without its
/// end is still being written: it waits for the next read.
fn feed(transcript: &mut Transcript, bytes: &[u8], read_line: LineReader) -> usize {
    let Some(last_end) = bytes.iter().rposition(|byte| *byte == b'\n') else {
        return 0;
    };
    for line in bytes[..last_end].split(|byte| *byte == b'\n') {
        read_line(transcript, line);
    }
    last_end + 1
}

fn count(transcript: &mut Transcript, line: Line) {
    if transcript.project.is_none() {
        transcript.project = line.cwd;
    }
    let (Some(kind), Some(time)) = (line.kind.as_deref(), line.timestamp.as_deref().and_then(unix_ms)) else {
        return;
    };
    let hour = time.div_euclid(MS_PER_HOUR);
    match (kind, line.message) {
        ("assistant", Some(message)) => {
            let Some(usage) = message.usage else {
                return;
            };
            if message.id.is_some() && message.id == transcript.last_reply {
                return;
            }
            transcript.last_reply = message.id;
            let model = message.model.unwrap_or_default();
            if model == SYNTHETIC_MODEL {
                return;
            }
            let figures = transcript.hours.entry(hour).or_default();
            figures.tokens.add(&usage.tokens());
            *figures.models.entry(model).or_default() += 1;
        }
        ("user", Some(message)) if !transcript.agent && !line.is_meta && message.content.as_ref().is_some_and(|content| content.typed) => {
            transcript.hours.entry(hour).or_default().tokens.prompts += 1;
        }
        _ => {}
    }
    if !transcript.agent && matches!(kind, "user" | "assistant") {
        extend_spans(&mut transcript.spans, time);
    }
}

fn extend_spans(spans: &mut Vec<Span>, time: i64) {
    match spans.last_mut() {
        Some(last) if time >= last.start && time - last.end <= IDLE_GAP_MS => last.end = last.end.max(time),
        _ => spans.push(Span { start: time, end: time }),
    }
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "isMeta", default)]
    is_meta: bool,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
    content: Option<Content>,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    output_tokens_details: Option<OutputDetails>,
    server_tool_use: Option<ServerTools>,
}

#[derive(Deserialize)]
struct OutputDetails {
    thinking_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct ServerTools {
    web_search_requests: Option<u64>,
}

impl Usage {
    fn tokens(&self) -> Tokens {
        Tokens {
            input: self.input_tokens.unwrap_or(0),
            output: self.output_tokens.unwrap_or(0),
            cache_read: self.cache_read_input_tokens.unwrap_or(0),
            cache_creation: self.cache_creation_input_tokens.unwrap_or(0),
            thinking: self.output_tokens_details.as_ref().and_then(|details| details.thinking_tokens).unwrap_or(0),
            web_searches: self.server_tool_use.as_ref().and_then(|tools| tools.web_search_requests).unwrap_or(0),
            replies: 1,
            prompts: 0,
        }
    }
}

/// Only whether a user line is a message the user typed. Read without keeping the content:
/// a tool result can weigh megabytes.
struct Content {
    typed: bool,
}

/// Lines of the user that are not typed messages start with a tag: `!` commands
/// (`<bash-input>`), notifications (`<task-notification>`), slash commands (`<command-name>`).
fn is_typed(text: &str) -> bool {
    let text = text.trim_start();
    !text.is_empty() && !text.starts_with('<') && !text.starts_with("[Request interrupted")
}

impl<'de> Deserialize<'de> for Content {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ContentVisitor)
    }
}

struct ContentVisitor;

impl<'de> Visitor<'de> for ContentVisitor {
    type Value = Content;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a text or a list of blocks")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Content, E> {
        Ok(Content { typed: is_typed(text) })
    }

    /// Typed when the first text block is, and no block is a tool result.
    fn visit_seq<A: SeqAccess<'de>>(self, mut blocks: A) -> Result<Content, A::Error> {
        let mut first_text = None;
        let mut tool_result = false;
        while let Some(block) = blocks.next_element::<Block>()? {
            tool_result |= block.kind.as_deref() == Some("tool_result");
            if first_text.is_none() && block.kind.as_deref() == Some("text") {
                first_text = block.text.map(|text| is_typed(&text));
            }
        }
        Ok(Content { typed: !tool_result && first_text.unwrap_or(false) })
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Content, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Content { typed: false })
    }
}

#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
}

/// `2026-11-18T15:52:20.069Z` in Unix milliseconds. Claude Code writes UTC: other forms give none.
fn unix_ms(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' || bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let rest = text.get(19..)?.strip_suffix('Z')?;
    let millis = match rest.strip_prefix('.') {
        None if rest.is_empty() => 0,
        Some(fraction) if !fraction.is_empty() && fraction.bytes().all(|byte| byte.is_ascii_digit()) => {
            let digits: String = fraction.chars().chain("00".chars()).take(3).collect();
            digits.parse::<i64>().ok()?
        }
        _ => return None,
    };
    let days = days_from_civil(year, month, day);
    Some(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + millis)
}

/// Days since 1970-01-01 of a date of the proleptic Gregorian calendar (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(id: &str, time: &str, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{time}","cwd":"/work/kindric","message":{{"id":"{id}","model":"claude-opus-5-5","content":[{{"type":"text","text":"hi"}}],"usage":{{"input_tokens":2,"output_tokens":{output},"cache_read_input_tokens":100,"cache_creation_input_tokens":50,"output_tokens_details":{{"thinking_tokens":3}},"server_tool_use":{{"web_search_requests":1}}}}}}}}"#
        )
    }

    fn prompt(time: &str, content: &str) -> String {
        format!(r#"{{"type":"user","timestamp":"{time}","cwd":"/work/kindric","message":{{"role":"user","content":{content}}}}}"#)
    }

    fn lines(lines: &[String]) -> Vec<u8> {
        lines.iter().flat_map(|line| format!("{line}\n").into_bytes()).collect()
    }

    fn fed(text: &[u8]) -> Transcript {
        let mut transcript = Transcript::default();
        let used = feed(&mut transcript, text, claude_line);
        transcript.read = used as u64;
        transcript
    }

    const T0: &str = "2026-11-18T15:52:20.069Z";
    /// Python: `datetime(2026, 11, 18, 15, 52, 20, 69000, tzinfo=timezone.utc).timestamp() * 1000`.
    const T0_MS: i64 = 1_795_017_140_069;

    #[test]
    fn timestamps_are_read_as_utc_milliseconds() {
        assert_eq!(unix_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_ms(T0), Some(T0_MS));
        // Python: datetime(2000, 2, 29, 23, 59, 59, 500000, tzinfo=timezone.utc).timestamp() * 1000
        assert_eq!(unix_ms("2000-02-29T23:59:59.5Z"), Some(951_868_799_500));
        assert_eq!(unix_ms("1969-12-31T23:59:59Z"), Some(-1000));
    }

    #[test]
    fn other_timestamps_give_none() {
        for text in ["", "2026-11-18", "2026-11-18T15:52:20", "2026-11-18T15:52:20+01:00", "2026-13-18T15:52:20Z", "2026-11-18T15:52:20.Z", "x026-11-18T15:52:20Z"] {
            assert_eq!(unix_ms(text), None, "{text}");
        }
    }

    #[test]
    fn a_reply_split_in_blocks_counts_once() {
        let transcript = fed(&lines(&[reply("m1", T0, 10), reply("m1", T0, 10), reply("m2", T0, 7)]));
        let figures = &transcript.hours[&(T0_MS / MS_PER_HOUR)];
        assert_eq!(figures.tokens.replies, 2);
        assert_eq!(figures.tokens.output, 17);
        assert_eq!(figures.tokens.input, 4);
        assert_eq!(figures.tokens.cache_read, 200);
        assert_eq!(figures.tokens.cache_creation, 100);
        assert_eq!(figures.tokens.thinking, 6);
        assert_eq!(figures.tokens.web_searches, 2);
        assert_eq!(figures.models["claude-opus-5-5"], 2);
        assert_eq!(transcript.project.as_deref(), Some("/work/kindric"));
    }

    #[test]
    fn a_reply_repeated_across_two_reads_counts_once() {
        let mut transcript = fed(&lines(&[reply("m1", T0, 10)]));
        feed(&mut transcript, &lines(&[reply("m1", T0, 10), reply("m2", T0, 1)]), claude_line);
        assert_eq!(transcript.hours[&(T0_MS / MS_PER_HOUR)].tokens.replies, 2);
    }

    #[test]
    fn synthetic_replies_and_missing_fields_count_nothing() {
        let synthetic = r#"{"type":"assistant","timestamp":"2026-11-18T15:52:20Z","message":{"id":"s","model":"<synthetic>","usage":{"output_tokens_details":null,"input_tokens":0}}}"#;
        let no_time = r#"{"type":"assistant","message":{"id":"x","model":"m","usage":{"output_tokens":5}}}"#;
        let transcript = fed(&lines(&[synthetic.to_string(), no_time.to_string(), "not json".to_string()]));
        assert!(transcript.hours.values().all(|figures| figures.tokens.replies == 0));
    }

    #[test]
    fn only_typed_messages_are_prompts() {
        let typed = [
            prompt(T0, r#""fix the bug""#),
            prompt(T0, r#"[{"type":"text","text":"[Image #1] look"},{"type":"image","source":{}}]"#),
        ];
        let not_typed = [
            prompt(T0, r#""<bash-input>git status</bash-input>""#),
            prompt(T0, r#""<task-notification>done</task-notification>""#),
            prompt(T0, r#""[Request interrupted by user]""#),
            prompt(T0, r#"[{"type":"tool_result","tool_use_id":"t","content":[{"type":"text","text":"output"}]}]"#),
            r#"{"type":"user","isMeta":true,"timestamp":"2026-11-18T15:52:20Z","message":{"content":"caveat"}}"#.to_string(),
        ];
        let all: Vec<String> = typed.iter().chain(not_typed.iter()).cloned().collect();
        let transcript = fed(&lines(&all));
        assert_eq!(transcript.hours[&(T0_MS / MS_PER_HOUR)].tokens.prompts, 2);
    }

    #[test]
    fn a_subagent_gives_tokens_but_no_prompts_nor_time() {
        let mut transcript = Transcript { agent: true, ..Transcript::default() };
        feed(&mut transcript, &lines(&[prompt(T0, r#""task for the agent""#), reply("a1", T0, 4)]), claude_line);
        let figures = &transcript.hours[&(T0_MS / MS_PER_HOUR)];
        assert_eq!((figures.tokens.prompts, figures.tokens.output), (0, 4));
        assert!(transcript.spans.is_empty());
    }

    #[test]
    fn a_line_still_being_written_waits() {
        let mut text = lines(&[reply("m1", T0, 1)]);
        let complete = text.len();
        text.extend_from_slice(reply("m2", T0, 1).as_bytes());
        let transcript = fed(&text);
        assert_eq!(transcript.read, complete as u64);
        assert_eq!(transcript.hours[&(T0_MS / MS_PER_HOUR)].tokens.replies, 1);
    }

    #[test]
    fn a_pause_longer_than_the_gap_starts_a_span() {
        let mut spans = Vec::new();
        for minutes in [0, 10, 40, 71, 72] {
            extend_spans(&mut spans, minutes * 60_000);
        }
        assert_eq!(spans, vec![Span { start: 0, end: 40 * 60_000 }, Span { start: 71 * 60_000, end: 72 * 60_000 }]);
    }

    struct Setup {
        root: PathBuf,
        source: Source,
    }

    fn setup(label: &str) -> Setup {
        let root = std::env::temp_dir().join(format!("kindric-stats-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = Source::of("louis".into(), root.join("louis"));
        Setup { root, source }
    }

    impl Drop for Setup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn write(path: &Path, text: &[u8]) {
        fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
        fs::write(path, text).expect("write");
    }

    fn append(path: &Path, text: &[u8]) {
        use std::io::Write;
        fs::OpenOptions::new().append(true).open(path).and_then(|mut file| file.write_all(text)).expect("append");
    }

    #[test]
    fn refresh_reads_only_what_was_appended_and_adds_up_sessions_with_their_agents() {
        let setup = setup("stats-refresh");
        let session = setup.source.dir.join("projects/-work-kindric/s1.jsonl");
        let agent = setup.source.dir.join("projects/-work-kindric/s1/subagents/agent-a.jsonl");
        write(&session, &lines(&[prompt(T0, r#""go""#), reply("m1", T0, 10)]));
        write(&agent, &lines(&[reply("a1", T0, 5)]));
        let mut cache = Cache::default();
        assert!(cache.refresh(std::slice::from_ref(&setup.source), &[]).is_empty());

        append(&session, &lines(&[reply("m2", "2026-11-18T17:00:00Z", 1)]));
        assert!(cache.refresh(std::slice::from_ref(&setup.source), &[]).is_empty());
        let summary = cache.summary(&BTreeMap::new());

        assert_eq!(summary.sessions.len(), 1);
        let row = &summary.sessions[0];
        assert_eq!((row.account.as_str(), row.session.as_str(), row.project.as_str()), ("louis", "s1", "/work/kindric"));
        assert_eq!((row.tokens.output, row.tokens.replies, row.tokens.prompts), (16, 3, 1));
        assert_eq!(row.spans.len(), 2, "an hour without activity ends the span");
        assert_eq!(summary.hours.len(), 2);
        assert_eq!(summary.hours[0].figures.tokens.output, 15, "the agent's reply is in its hour");
        assert_eq!(summary.hours[1].figures.tokens.output, 1);
    }

    #[test]
    fn a_file_written_again_shorter_is_read_from_the_start() {
        let setup = setup("stats-rewritten");
        let session = setup.source.dir.join("projects/p/s1.jsonl");
        write(&session, &lines(&[reply("m1", T0, 10), reply("m2", T0, 10)]));
        let mut cache = Cache::default();
        cache.refresh(std::slice::from_ref(&setup.source), &[]);
        write(&session, &lines(&[reply("m3", T0, 1)]));
        cache.refresh(std::slice::from_ref(&setup.source), &[]);
        assert_eq!(cache.summary(&BTreeMap::new()).sessions[0].tokens.output, 1);
    }

    #[test]
    fn accounts_left_out_are_forgotten() {
        let setup = setup("stats-forgotten");
        let session = setup.source.dir.join("projects/p/s1.jsonl");
        write(&session, &lines(&[reply("m1", T0, 10)]));
        let mut cache = Cache::default();
        cache.refresh(std::slice::from_ref(&setup.source), &[]);
        assert_eq!(cache.files.len(), 1);
        fs::remove_file(&session).expect("remove");
        cache.refresh(std::slice::from_ref(&setup.source), &[]);
        assert_eq!((cache.files.len(), cache.gone.len()), (0, 1));
        cache.refresh(&[], &[]);
        assert!(cache.files.is_empty() && cache.gone.is_empty());
    }

    #[test]
    fn the_figures_of_an_account_unseen_for_once_stay() {
        let setup = setup("stats-unseen");
        let file = setup.root.join("kindric").join(CACHE_FILE);
        let account = setup.source.account.clone();
        let path = setup.source.dir.join("projects/p/s1.jsonl");
        write(&path, &lines(&[reply("m1", T0, 10)]));
        write(&setup.source.dir.join("projects/p/s2.jsonl"), &lines(&[reply("m2", T0, 5)]));
        let read = |sources: &[Source], unseen: &[&str]| {
            let mut cache = Cache::load(&file);
            assert_eq!(cache.refresh(sources, unseen), Vec::<String>::new());
            cache.save(&file).expect("save");
            output_of(&cache)
        };
        let sources = std::slice::from_ref(&setup.source);
        read(sources, &[]);
        fs::remove_file(&path).expect("remove");
        assert_eq!(read(sources, &[]), [10, 5]);
        assert_eq!(read(&[], &[&account]), [10, 5], "not asked to read: as it was");
        assert_eq!(read(sources, &[]), [10, 5], "back, counted once");
        assert!(read(&[], &[]).is_empty(), "removed");
    }

    fn output_of(cache: &Cache) -> Vec<u64> {
        cache.summary(&BTreeMap::new()).sessions.iter().map(|row| row.tokens.output).collect()
    }

    #[test]
    fn a_transcript_removed_keeps_its_figures_until_it_comes_back() {
        let setup = setup("stats-gone");
        let session = setup.source.dir.join("projects/p/s1.jsonl");
        let text = lines(&[reply("m1", T0, 10)]);
        write(&session, &text);
        let file = setup.root.join("kindric").join(CACHE_FILE);
        let refreshed = || {
            let mut cache = Cache::load(&file);
            assert_eq!(cache.refresh(std::slice::from_ref(&setup.source), &[]), Vec::<String>::new());
            cache.save(&file).expect("save");
            cache
        };
        assert_eq!(output_of(&refreshed()), [10]);
        assert!(!gone_file(&file).exists(), "nothing gone, nothing written");
        fs::remove_file(&session).expect("remove");
        assert_eq!(output_of(&refreshed()), [10]);
        fs::write(&file, "{broken").expect("write");
        assert_eq!(output_of(&refreshed()), [10], "a cache read again from scratch keeps them");
        write(&session, &text);
        assert_eq!(output_of(&refreshed()), [10], "found again, counted once");
        assert!(Cache::load(&file).gone.is_empty());
    }

    #[test]
    fn a_codex_session_archived_is_counted_once() {
        let setup = setup("stats-codex-moved");
        let source = codex_source(&setup);
        let name = "rollout-2026-11-01T10-00-00-01a20533-7ccd-7873-8297-750f72a13e8b.jsonl";
        let before = source.dir.join("sessions/2026/11/01").join(name);
        write(&before, &lines(&[codex_start(), vec![codex_tokens(T0, 10, 0, 7)]].concat()));
        let mut cache = Cache::default();
        cache.refresh(std::slice::from_ref(&source), &[]);
        fs::create_dir_all(source.dir.join("archived_sessions")).expect("create dir");
        fs::rename(&before, source.dir.join("archived_sessions").join(name)).expect("move");
        cache.refresh(std::slice::from_ref(&source), &[]);
        assert_eq!(output_of(&cache), [7]);
    }

    #[test]
    fn a_file_of_those_gone_that_cannot_be_read_is_left_as_it_is() {
        let setup = setup("stats-gone-broken");
        let session = setup.source.dir.join("projects/p/s1.jsonl");
        write(&session, &lines(&[reply("m1", T0, 10)]));
        let file = setup.root.join("kindric").join(CACHE_FILE);
        write(&gone_file(&file), b"{broken");
        let mut cache = Cache::load(&file);
        cache.refresh(std::slice::from_ref(&setup.source), &[]);
        fs::remove_file(&session).expect("remove");
        let problems = cache.refresh(std::slice::from_ref(&setup.source), &[]);
        assert!(problems.len() == 1 && problems[0].contains(GONE_FILE), "{problems:?}");
        cache.save(&file).expect("save");
        assert_eq!(fs::read(gone_file(&file)).expect("read"), b"{broken");
        assert_eq!(output_of(&Cache::load(&file)), [10], "kept in the cache meanwhile");
    }

    #[test]
    fn figures_kept_by_an_older_kindric_are_still_read() {
        let setup = setup("stats-gone-older");
        let file = setup.root.join("kindric").join(CACHE_FILE);
        let hour = T0_MS / MS_PER_HOUR;
        write(
            &gone_file(&file),
            format!(r#"{{"/x/s9.jsonl":{{"account":"louis","session":"s9","hours":{{"{hour}":{{"tokens":{{"output":4}}}}}}}}}}"#).as_bytes(),
        );
        let mut cache = Cache::load(&file);
        assert_eq!(cache.refresh(std::slice::from_ref(&setup.source), &[]), Vec::<String>::new());
        assert_eq!(output_of(&cache), [4]);
    }

    #[test]
    fn the_cache_survives_a_save_and_an_older_one_is_dropped() {
        let setup = setup("stats-cache");
        write(&setup.source.dir.join("projects/p/s1.jsonl"), &lines(&[reply("m1", T0, 10)]));
        let mut cache = Cache::default();
        cache.refresh(std::slice::from_ref(&setup.source), &[]);
        let file = setup.root.join("kindric").join(CACHE_FILE);
        cache.save(&file).expect("save");
        assert_eq!(Cache::load(&file), cache);

        let older = String::from_utf8(fs::read(&file).expect("read")).expect("utf-8").replacen(&format!("\"version\":{VERSION}"), "\"version\":0", 1);
        fs::write(&file, older).expect("write");
        assert_eq!(Cache::load(&file), Cache::default());
        fs::write(&file, "{broken").expect("write");
        assert_eq!(Cache::load(&file), Cache::default());
        assert_eq!(Cache::load(&setup.root.join("missing.json")), Cache::default());
    }

    fn codex_tokens(time: &str, input: u64, cached: u64, output: u64) -> String {
        format!(
            r#"{{"timestamp":"{time}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"cache_write_input_tokens":0,"output_tokens":{output},"reasoning_output_tokens":3,"total_tokens":1}},"model_context_window":258400}},"rate_limits":null}}}}"#
        )
    }

    /// Lines as codex 0.172.0 writes them, the rest left out.
    fn codex_start() -> Vec<String> {
        vec![
            format!(r#"{{"timestamp":"{T0}","type":"session_meta","payload":{{"id":"01a2","cwd":"/work/shop","git":{{}}}}}}"#),
            format!(r#"{{"timestamp":"{T0}","type":"turn_context","payload":{{"cwd":"/work/shop","model":"gpt-6-luna"}}}}"#),
            format!(r#"{{"timestamp":"{T0}","type":"event_msg","payload":{{"type":"task_started","turn_id":"t1"}}}}"#),
            format!(r#"{{"timestamp":"{T0}","type":"event_msg","payload":{{"type":"token_count","info":null}}}}"#),
        ]
    }

    fn codex_source(setup: &Setup) -> Source {
        Source { agent: Agent::Codex, ..setup.source.clone() }
    }

    #[test]
    fn a_codex_session_counts_what_its_totals_gain() {
        let setup = setup("stats-codex");
        let source = codex_source(&setup);
        let file = source.dir.join("sessions/2026/11/18/rollout-2026-11-18T16-52-20-01a20533-7ccd-7873-8297-750f72a13e8b.jsonl");
        let mut text = codex_start();
        text.extend([codex_tokens(T0, 1000, 400, 50), codex_tokens(T0, 1000, 400, 50)]);
        write(&file, &lines(&text));
        let mut cache = Cache::default();
        assert!(cache.refresh(std::slice::from_ref(&source), &[]).is_empty());
        append(&file, &lines(&[codex_tokens("2026-11-18T17:00:00Z", 1500, 800, 70)]));
        assert!(cache.refresh(std::slice::from_ref(&source), &[]).is_empty());

        let summary = cache.summary(&BTreeMap::new());
        let row = &summary.sessions[0];
        assert_eq!((row.session.as_str(), row.project.as_str()), ("01a20533-7ccd-7873-8297-750f72a13e8b", "/work/shop"));
        assert_eq!((row.tokens.input, row.tokens.cache_read, row.tokens.output, row.tokens.thinking), (700, 800, 70, 3));
        assert_eq!((row.tokens.replies, row.tokens.prompts, row.cost), (2, 1, 0), "the same totals twice are one reply");
        assert_eq!(summary.hours[0].figures.tokens.input, 600);
        assert_eq!(summary.hours[1].figures.models["gpt-6-luna"], 1);
    }

    #[test]
    fn an_archived_codex_session_counts_and_other_files_do_not() {
        let setup = setup("stats-codex-archived");
        let source = codex_source(&setup);
        let text = lines(&[codex_start(), vec![codex_tokens(T0, 10, 0, 1)]].concat());
        write(&source.dir.join("archived_sessions/rollout-2026-11-01T10-00-00-01a20533-7ccd-7873-8297-750f72a13e8b.jsonl"), &text);
        write(&source.dir.join("archived_sessions/notes.jsonl"), &text);
        write(&source.dir.join("sessions/stray.jsonl"), &text);
        let mut cache = Cache::default();
        assert!(cache.refresh(&[source], &[]).is_empty());
        assert_eq!(cache.summary(&BTreeMap::new()).sessions.len(), 1);
    }

    /// The account of an OpenCode service on `port`.
    fn opencode_source(setup: &Setup, port: u16) -> Source {
        let config = kindric_accounts::opencode::config_dir(&setup.source.dir);
        write(&config.join("service.json"), format!(r#"{{"port":{port},"password":"x"}}"#).as_bytes());
        Source { agent: Agent::OpenCode, ..setup.source.clone() }
    }

    /// As OpenCode 2.4.1 answered on the test bench, the rest left out.
    const OPENCODE_SESSIONS: &str = r#"{"data":[
        {"id":"ses_a","title":"cost","location":{"directory":"/work/shop"},"time":{"created":1,"updated":20}},
        {"id":"ses_b","parentID":"ses_a","title":"sub","location":{"directory":"/work/shop"},"time":{"created":1,"updated":21}}],"cursor":{"previous":"p","next":null}}"#;
    const OPENCODE_REPLIES: &str = r#"{"data":[{"id":"msg_1","type":"assistant","time":{"created":1795017140069},"model":{"id":"m","providerID":"fake"},
        "cost":0.0084,"tokens":{"input":600,"output":70,"reasoning":30,"cache":{"read":400,"write":0}}}],"cursor":{"next":null}}"#;

    #[test]
    fn opencode_sessions_are_read_from_the_service_and_kept_when_it_stops() {
        let setup = setup("stats-opencode");
        let (port, requests) = opencode::fake::service(vec![
            ("/api/session/active", r#"{"data":{}}"#),
            ("/api/session?limit=100", OPENCODE_SESSIONS),
            ("/api/session/ses_a/message?type=assistant&limit=100", OPENCODE_REPLIES),
            ("/api/session/ses_b/message?type=assistant&limit=100", OPENCODE_REPLIES),
        ]);
        let source = opencode_source(&setup, port);
        let mut cache = Cache::default();
        assert_eq!(cache.refresh(std::slice::from_ref(&source), &[]), Vec::<String>::new());
        let summary = cache.summary(&BTreeMap::new());
        let row = &summary.sessions[0];
        assert_eq!((summary.sessions.len(), row.session.as_str(), row.project.as_str()), (1, "ses_a", "/work/shop"), "a subagent counts for its session");
        assert_eq!((row.tokens.input, row.tokens.output, row.tokens.thinking, row.tokens.cache_read), (1200, 200, 60, 800));
        assert_eq!((row.cost, row.spans.len()), (16_800, 1));
        assert_eq!(summary.hours[0].figures.cost, 16_800);

        cache.refresh(std::slice::from_ref(&source), &[]);
        let messages = requests.try_iter().filter(|request| request.contains("/message")).count();
        assert_eq!(messages, 2, "sessions that did not change are not read again");

        let source = opencode_source(&setup, opencode::fake::NO_SERVICE);
        assert_eq!(cache.refresh(&[source], &[]), Vec::<String>::new());
        assert_eq!(cache.summary(&BTreeMap::new()).sessions[0].cost, 16_800);
        cache.refresh(&[], &[]);
        assert!(cache.summary(&BTreeMap::new()).sessions.is_empty(), "an account left out is forgotten");
    }

    fn noted(session: &str, total_usd: f64, updated_at: i64) -> Cost {
        Cost { session_id: session.into(), total_usd, updated_at }
    }

    #[test]
    fn a_cost_is_what_its_session_gained_since_its_last_note() {
        let costs = [noted("s1", 0.01, 3600), noted("s2", 1.0, 3600), noted("s1", 0.03, 7200), noted("s1", 0.005, 7300), noted("s1", 0.005, 7400)];
        assert_eq!(cost_steps(&costs), [("s1", 1, 10_000), ("s2", 1, 1_000_000), ("s1", 2, 20_000), ("s1", 2, 5_000)]);
    }

    #[test]
    fn the_cost_of_a_claude_session_goes_to_its_project() {
        let setup = setup("stats-cost");
        write(&setup.source.dir.join("projects/-work-kindric/s1.jsonl"), &lines(&[reply("m1", T0, 10)]));
        let mut cache = Cache::default();
        cache.refresh(std::slice::from_ref(&setup.source), &[]);
        let at = T0_MS / 1000;
        let costs = BTreeMap::from([("louis".to_string(), vec![noted("s1", 0.25, at), noted("unknown", 9.0, at)])]);
        let summary = cache.summary(&costs);
        assert_eq!(summary.sessions.iter().map(|row| row.cost).collect::<Vec<_>>(), [250_000]);
        assert_eq!(summary.hours.iter().map(|row| (row.project.as_str(), row.figures.cost)).collect::<Vec<_>>(), [("/work/kindric", 250_000)]);
    }
}
