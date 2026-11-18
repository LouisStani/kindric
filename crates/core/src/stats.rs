//! Figures over the transcripts of Claude Code: tokens, replies, messages typed, and the spans of
//! time each session was active (decisions 347, 384, 387). There can be gigabytes of transcripts:
//! a file is read once, then only what was appended since, and what each file said is kept in a
//! cache. The transcripts are not documented: a line that cannot be read counts for nothing.
//!
//! Figures are added up per hour in UTC; the page turns hours into its own local days.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use kindric_accounts::Paths;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

/// Bumped when what the cache holds changes: an older cache is then read again from scratch.
const VERSION: u32 = 1;
const CACHE_FILE: &str = "stats.json";
/// A pause longer than this ends a span of activity.
const IDLE_GAP_MS: i64 = 30 * 60 * 1000;
const MS_PER_HOUR: i64 = 3_600_000;
/// The model of the replies Claude Code writes itself (an API error): no tokens, not a reply.
const SYNTHETIC_MODEL: &str = "<synthetic>";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
pub struct Figures {
    pub tokens: Tokens,
    /// Model → replies.
    pub models: BTreeMap<String, u64>,
}

impl Figures {
    fn add(&mut self, other: &Figures) {
        self.tokens.add(&other.tokens);
        for (model, replies) in &other.models {
            *self.models.entry(model.clone()).or_default() += replies;
        }
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
    /// Unix hour (UTC) → figures.
    hours: BTreeMap<i64, Figures>,
    spans: Vec<Span>,
}

/// What every transcript read so far said, by path.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    version: u32,
    files: BTreeMap<String, Transcript>,
}

/// An account whose transcripts count, and where Claude Code keeps its files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub account: String,
    pub dir: PathBuf,
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
    /// The cache in `path`. A missing, broken or older one is empty: everything is read again.
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Cache>(&bytes).ok())
            .filter(|cache| cache.version == VERSION)
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string(self).map_err(io::Error::other)?;
        kindric_accounts::files::write_atomic(path, &text)
            .map_err(|error| io::Error::new(error.kind(), format!("cannot write {}: {error}", path.display())))
    }

    /// Reads what the transcripts of `sources` gained since the last time. Transcripts of other
    /// accounts, and those gone, are forgotten. Returns what could not be read.
    pub fn refresh(&mut self, sources: &[Source]) -> Vec<String> {
        let mut problems = Vec::new();
        let mut kept = BTreeMap::new();
        for source in sources {
            for found in transcripts(&source.dir, &mut problems) {
                let key = found.path.to_string_lossy().into_owned();
                let previous = self.files.remove(&key).filter(|old| old.account == source.account);
                match update(previous, &source.account, &found) {
                    Ok(transcript) => {
                        kept.insert(key, transcript);
                    }
                    Err(error) => problems.push(format!("cannot read {}: {error}", found.path.display())),
                }
            }
        }
        self.version = VERSION;
        self.files = kept;
        problems
    }

    /// Figures added up per hour, account and project, then per session.
    pub fn summary(&self) -> Summary {
        let mut hours: BTreeMap<(i64, &str, &str), Figures> = BTreeMap::new();
        let mut sessions: BTreeMap<(&str, &str), SessionRow> = BTreeMap::new();
        for transcript in self.files.values() {
            let project = transcript.project.as_deref().unwrap_or("");
            let session = sessions.entry((&transcript.account, &transcript.session)).or_insert_with(|| SessionRow {
                account: transcript.account.clone(),
                session: transcript.session.clone(),
                project: project.to_string(),
                tokens: Tokens::default(),
                spans: Vec::new(),
            });
            if !transcript.agent {
                session.project = project.to_string();
                session.spans.extend_from_slice(&transcript.spans);
            }
            for (hour, figures) in &transcript.hours {
                session.tokens.add(&figures.tokens);
                hours.entry((*hour, &transcript.account, project)).or_default().add(figures);
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
fn update(previous: Option<Transcript>, account: &str, found: &Found) -> io::Result<Transcript> {
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
    let used = feed(&mut transcript, &bytes);
    transcript.read += used as u64;
    Ok(transcript)
}

/// Counts the whole lines of `bytes`; returns how many bytes they took. A last line without its
/// end is still being written: it waits for the next read.
fn feed(transcript: &mut Transcript, bytes: &[u8]) -> usize {
    let Some(last_end) = bytes.iter().rposition(|byte| *byte == b'\n') else {
        return 0;
    };
    for line in bytes[..last_end].split(|byte| *byte == b'\n') {
        if let Ok(line) = serde_json::from_slice::<Line>(line) {
            count(transcript, line);
        }
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
        let used = feed(&mut transcript, text);
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
        feed(&mut transcript, &lines(&[reply("m1", T0, 10), reply("m2", T0, 1)]));
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
        feed(&mut transcript, &lines(&[prompt(T0, r#""task for the agent""#), reply("a1", T0, 4)]));
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
        let source = Source { account: "louis".into(), dir: root.join("louis") };
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
        assert!(cache.refresh(std::slice::from_ref(&setup.source)).is_empty());

        append(&session, &lines(&[reply("m2", "2026-11-18T17:00:00Z", 1)]));
        assert!(cache.refresh(std::slice::from_ref(&setup.source)).is_empty());
        let summary = cache.summary();

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
        cache.refresh(std::slice::from_ref(&setup.source));
        write(&session, &lines(&[reply("m3", T0, 1)]));
        cache.refresh(std::slice::from_ref(&setup.source));
        assert_eq!(cache.summary().sessions[0].tokens.output, 1);
    }

    #[test]
    fn accounts_left_out_and_files_gone_are_forgotten() {
        let setup = setup("stats-forgotten");
        let session = setup.source.dir.join("projects/p/s1.jsonl");
        write(&session, &lines(&[reply("m1", T0, 10)]));
        let mut cache = Cache::default();
        cache.refresh(std::slice::from_ref(&setup.source));
        assert_eq!(cache.files.len(), 1);
        cache.refresh(&[]);
        assert!(cache.files.is_empty());
        cache.refresh(std::slice::from_ref(&setup.source));
        fs::remove_file(&session).expect("remove");
        cache.refresh(std::slice::from_ref(&setup.source));
        assert!(cache.files.is_empty());
    }

    #[test]
    fn the_cache_survives_a_save_and_an_older_one_is_dropped() {
        let setup = setup("stats-cache");
        write(&setup.source.dir.join("projects/p/s1.jsonl"), &lines(&[reply("m1", T0, 10)]));
        let mut cache = Cache::default();
        cache.refresh(std::slice::from_ref(&setup.source));
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
}
