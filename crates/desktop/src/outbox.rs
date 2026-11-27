//! What Rust has for the page, kept until the page takes it. WebKitGTK 2.54 keeps memory for every
//! script of new text it runs, and Tauri 2.12 sends each channel message and each event as such a
//! script: a busy session's output made the page grow by megabytes a minute. The page is only
//! ever told by the same script (`RING`), once until it takes what waits, and takes it by a
//! command, whose answer comes back as a response, not a script.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// The script that tells the page something waits; always the same text, which WebKitGTK keeps
/// once (`collect` in outbox.js).
pub const RING: &str = "window.kindricOutbox && window.kindricOutbox()";

const OUTPUT: u8 = 0;
const SESSIONS: u8 = 1;

#[derive(Default)]
pub struct Outbox {
    waiting: Mutex<Waiting>,
}

#[derive(Default)]
struct Waiting {
    /// Terminal output by the number the page gave its terminal, in the order written.
    outputs: Vec<(u32, Vec<u8>)>,
    /// The list of sessions as JSON; only the last one matters.
    sessions: Option<String>,
    /// The page was told and has not taken what waits yet.
    rung: bool,
}

impl Outbox {
    /// Keeps output of the page's terminal `output`; true when the page must be told.
    pub fn post_output(&self, output: u32, bytes: &[u8]) -> bool {
        let mut waiting = self.waiting();
        match waiting.outputs.last_mut() {
            Some((last, kept)) if *last == output => kept.extend_from_slice(bytes),
            _ => waiting.outputs.push((output, bytes.to_vec())),
        }
        waiting.ring()
    }

    /// Keeps the list of sessions in place of one not taken yet; true when the page must be told.
    pub fn post_sessions(&self, json: String) -> bool {
        let mut waiting = self.waiting();
        waiting.sessions = Some(json);
        waiting.ring()
    }

    /// Everything waiting, as the page reads it (`letters` in outbox.js): per terminal output a 0,
    /// the terminal's number, a length and the bytes; for the sessions a 1, a length and the JSON.
    /// Numbers on 4 bytes, least significant first. The next post rings again.
    pub fn take(&self) -> Vec<u8> {
        let mut waiting = self.waiting();
        waiting.rung = false;
        let mut taken = Vec::new();
        for (output, bytes) in waiting.outputs.drain(..) {
            taken.push(OUTPUT);
            taken.extend_from_slice(&output.to_le_bytes());
            put_sized(&mut taken, &bytes);
        }
        if let Some(json) = waiting.sessions.take() {
            taken.push(SESSIONS);
            put_sized(&mut taken, json.as_bytes());
        }
        taken
    }

    fn waiting(&self) -> MutexGuard<'_, Waiting> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Waiting {
    fn ring(&mut self) -> bool {
        !std::mem::replace(&mut self.rung, true)
    }
}

/// A chunk of output or a list stays far under 4 GB.
fn put_sized(taken: &mut Vec<u8>, bytes: &[u8]) {
    taken.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    taken.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_is_told_once_until_it_takes_what_waits() {
        let outbox = Outbox::default();
        assert!(outbox.post_output(1, b"a"));
        assert!(!outbox.post_output(2, b"b"));
        assert!(!outbox.post_sessions("{}".into()));
        outbox.take();
        assert!(outbox.post_output(1, b"c"));
    }

    #[test]
    fn output_keeps_its_order_and_only_the_last_list_goes() {
        let outbox = Outbox::default();
        outbox.post_output(7, b"ab");
        outbox.post_output(7, b"c");
        outbox.post_sessions("[1]".into());
        outbox.post_output(300, b"d");
        outbox.post_sessions("[2]".into());
        let mut expected = vec![0, 7, 0, 0, 0, 3, 0, 0, 0];
        expected.extend_from_slice(b"abc");
        expected.extend_from_slice(&[0, 44, 1, 0, 0, 1, 0, 0, 0, b'd']);
        expected.extend_from_slice(&[1, 3, 0, 0, 0]);
        expected.extend_from_slice(b"[2]");
        assert_eq!(outbox.take(), expected);
        assert!(outbox.take().is_empty());
    }
}
