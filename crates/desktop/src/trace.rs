//! Timings of what the app does, written to `trace.log` when `KINDRIC_TRACE` is set, to
//! find where a delay comes from before fixing it. Off, it writes nothing.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

pub const VARIABLE: &str = "KINDRIC_TRACE";

pub struct Trace {
    file: Option<Mutex<File>>,
    start: Instant,
}

/// Records how long it lived once dropped: the time a command took.
pub struct Span<'a> {
    trace: &'a Trace,
    what: &'static str,
    start: Instant,
}

impl Drop for Span<'_> {
    fn drop(&mut self) {
        self.trace.record(self.what, self.start.elapsed());
    }
}

/// "   1840 ms  agents pro  612.4 ms": when, since the app started, then what and how long.
fn line(at: Duration, what: &str, took: Duration) -> String {
    format!("{:>8} ms  {what}  {:.1} ms\n", at.as_millis(), took.as_secs_f64() * 1000.0)
}

impl Trace {
    pub fn off() -> Self {
        Self { file: None, start: Instant::now() }
    }

    /// Starts `path` over: a trace holds one run of the app.
    pub fn open(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self { file: Some(Mutex::new(File::create(path)?)), start: Instant::now() })
    }

    pub fn enabled(&self) -> bool {
        self.file.is_some()
    }

    /// A trace that cannot be written stays silent: it must never disturb the app.
    pub fn record(&self, what: &str, took: Duration) {
        let Some(file) = &self.file else {
            return;
        };
        let text = line(self.start.elapsed(), what, took);
        let _ = file.lock().unwrap_or_else(PoisonError::into_inner).write_all(text.as_bytes());
    }

    pub fn span(&self, what: &'static str) -> Span<'_> {
        Span { trace: self, what, start: Instant::now() }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-trace-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn a_line_says_when_what_and_how_long() {
        let text = line(Duration::from_millis(1840), "agents pro", Duration::from_micros(612_449));
        assert_eq!(text, "    1840 ms  agents pro  612.4 ms\n");
    }

    #[test]
    fn off_is_disabled_and_never_fails() {
        let trace = Trace::off();
        assert!(!trace.enabled());
        trace.record("open_session", Duration::from_millis(3));
        drop(trace.span("go_tab"));
    }

    #[test]
    fn a_span_writes_its_line_once_dropped() {
        let root = scratch("span");
        let path = root.join("kindric/trace.log");
        let trace = Trace::open(&path).unwrap();
        assert!(trace.enabled());
        let span = trace.span("go_tab");
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        drop(span);
        trace.record("agents lystriq", Duration::from_millis(450));
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].contains("  go_tab  "), "{text}");
        assert!(lines[1].ends_with("  agents lystriq  450.0 ms"), "{text}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn each_run_starts_the_file_over() {
        let root = scratch("over");
        let path = root.join("trace.log");
        Trace::open(&path).unwrap().record("first run", Duration::ZERO);
        Trace::open(&path).unwrap().record("second run", Duration::ZERO);
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("first run") && text.contains("second run"), "{text}");
        fs::remove_dir_all(root).unwrap();
    }
}
