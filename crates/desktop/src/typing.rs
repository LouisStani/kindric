//! Types a message into a background session that no pane shows, as if the user typed it: the
//! answer to a `kindric ask` from the overview. A hidden attach, the same command as a pane's,
//! then detach: the session goes on.

use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use kindric_core::attach::AttachCommand;

use crate::pane::Pane;

/// Claude Code turns bracketed paste on once its prompt reads input.
const BRACKETED_PASTE_ON: &[u8] = b"\x1b[?2004h";

/// How long each step may take.
pub struct Timings {
    /// Longest wait for the prompt.
    pub ready: Duration,
    /// Between the text and Enter: Enter apart sends the message instead of adding a line.
    pub enter: Duration,
    /// Before detaching, so that the session has read everything.
    pub settle: Duration,
}

pub const TIMINGS: Timings =
    Timings { ready: Duration::from_secs(5), enter: Duration::from_millis(500), settle: Duration::from_secs(2) };

/// `text` as typed: pasted as one block when the program asked for bracketed paste.
fn pasted(text: &str, bracketed: bool) -> Vec<u8> {
    if !bracketed {
        return text.as_bytes().to_vec();
    }
    [b"\x1b[200~".as_slice(), text.as_bytes(), b"\x1b[201~"].concat()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

/// Waits until the output turns bracketed paste on, at most `longest`. Says whether it did.
fn wait_for_prompt(output: &Mutex<Vec<u8>>, longest: Duration) -> bool {
    let start = Instant::now();
    loop {
        if contains(&output.lock().unwrap_or_else(PoisonError::into_inner), BRACKETED_PASTE_ON) {
            return true;
        }
        if start.elapsed() >= longest {
            return false;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Attaches with `command`, types `text` then Enter, and detaches. Blocks for a few seconds.
pub fn type_into(command: &AttachCommand, text: &str, timings: &Timings) -> io::Result<()> {
    let output = Arc::new(Mutex::new(Vec::new()));
    let mut pane = {
        let output = Arc::clone(&output);
        Pane::open(
            command,
            120,
            40,
            move |bytes| output.lock().unwrap_or_else(PoisonError::into_inner).extend_from_slice(bytes),
            || {},
        )?
    };
    let bracketed = wait_for_prompt(&output, timings.ready);
    pane.write(&pasted(text, bracketed))?;
    thread::sleep(timings.enter);
    pane.write(b"\r")?;
    thread::sleep(timings.settle);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paste_is_bracketed_only_when_the_program_asked() {
        assert_eq!(pasted("blue", false), b"blue".to_vec());
        assert_eq!(pasted("a\nb", true), b"\x1b[200~a\nb\x1b[201~".to_vec());
    }

    #[test]
    fn the_prompt_is_ready_once_bracketed_paste_is_on() {
        let output = Mutex::new(b"\x1b[?25l drawing".to_vec());
        assert!(!wait_for_prompt(&output, Duration::ZERO));
        output.lock().unwrap().extend_from_slice(b"\x1b[?2004h");
        assert!(wait_for_prompt(&output, Duration::ZERO));
    }

    /// A program that turns bracketed paste on, then keeps what it reads in a file: the text
    /// comes pasted, and Enter after it.
    #[cfg(unix)]
    #[test]
    fn the_text_reaches_the_program_pasted_then_enter() {
        let file = std::env::temp_dir().join(format!("kindric-typing-{}", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let script = format!("stty raw -echo; printf '\\033[?2004h'; head -c 18 > '{}'", file.display());
        let command = AttachCommand {
            program: "sh".into(),
            args: vec!["-c".into(), script],
            env: Vec::new(),
            env_remove: Vec::new(),
            directory: None,
        };
        let fast = Timings { ready: Duration::from_secs(5), enter: Duration::from_millis(50), settle: Duration::from_millis(500) };
        type_into(&command, "green", &fast).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"\x1b[200~green\x1b[201~\r".to_vec());
        std::fs::remove_file(file).unwrap();
    }
}
