//! A session at rest wakes when a message asks it to (`Message::wake`). `kindric wait-message` is
//! a Stop and SessionStart hook Claude Code runs in the background (`asyncRewake`): it waits for
//! such a message, then exits with code 2, and its error output goes to the session, which starts
//! a turn. The hook starts again at the end of every turn and each time the session starts:
//! `wait/<session id>` in kindric's data holds the token of the latest wait, and an older one ends
//! without a word. It also wakes a session that the user is away from and that stopped to wait for
//! work it started in the background, once that wait is long (`away::background_wake`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kindric_accounts::files::{create_private_dir, write_atomic};

use crate::away;
use crate::mailbox::{self, Mailbox, data_dir, is_valid_session_id};

/// How often a wait looks at the inbox.
const POLL: Duration = Duration::from_secs(2);

/// `wait` in the data directory.
pub fn default_root() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("wait"))
}

/// What ended a wait.
#[derive(Debug, PartialEq, Eq)]
pub enum Woken {
    /// A message asks the session to act: the notice to give it.
    Message(String),
    /// A newer wait of the same session took over.
    Replaced,
}

fn claim_file(root: &Path, session: &str) -> io::Result<PathBuf> {
    if !is_valid_session_id(session) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid session id: {session:?}")));
    }
    Ok(root.join(session))
}

/// Waits for a message that wakes `session`: claims the wait with `token`, then looks at the
/// claim, the inbox and `due` (another reason to wake it, with its notice), with a `pause`
/// between two looks.
pub fn wait(mailbox: &Mailbox, root: &Path, session: &str, token: &str, mut due: impl FnMut() -> Option<String>, mut pause: impl FnMut()) -> io::Result<Woken> {
    let claim = claim_file(root, session)?;
    create_private_dir(root)?;
    write_atomic(&claim, token)?;
    loop {
        if fs::read_to_string(&claim).ok().as_deref() != Some(token) {
            return Ok(Woken::Replaced);
        }
        if let Some(notice) = mailbox.wake_notice(session)?.or_else(&mut due) {
            let _ = fs::remove_file(&claim);
            return Ok(Woken::Message(notice));
        }
        pause();
    }
}

/// The session of `input`, the JSON Claude Code gives a hook, when a wait starts there: the end of
/// a turn, or the start of the session (a new one, or one started again, whose wait ended with
/// its former process).
pub fn waiting_session(input: &str) -> Option<String> {
    let input: serde_json::Value = serde_json::from_str(input).ok()?;
    if !matches!(input.get("hook_event_name")?.as_str()?, "Stop" | "SessionStart") {
        return None;
    }
    Some(input.get("session_id")?.as_str()?.to_owned())
}

/// `kindric wait-message`: reads Claude Code's input and waits. Returns the notice that wakes the
/// session, to print on the error output with exit code 2; nothing when the wait ends otherwise
/// or something goes wrong, which must not disturb the session.
pub fn run_wait_hook() -> Option<String> {
    let mut input = String::new();
    io::Read::read_to_string(&mut io::stdin(), &mut input).ok()?;
    let session = waiting_session(&input)?;
    let mailbox = Mailbox::new(mailbox::default_root().ok()?);
    let root = default_root().ok()?;
    let started = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_nanos());
    let token = format!("{}-{started}", std::process::id());
    let away_root = away::default_root().ok()?;
    let due = || away::background_wake(&away_root, &session, away::now());
    match wait(&mailbox, &root, &session, &token, due, || std::thread::sleep(POLL)) {
        Ok(Woken::Message(notice)) => Some(notice),
        Ok(Woken::Replaced) | Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailbox::Message;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-wake-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn message(text: &str, wake: bool) -> Message {
        Message { from_session: "aaaa-1111".into(), from_title: "roblox".into(), from_account: "louis".into(), sent_at: 100, text: text.into(), wake }
    }

    #[test]
    fn a_message_that_asks_to_wake_ends_the_wait_with_the_notice() {
        let root = temp_root("woken");
        let mailbox = Mailbox::new(root.join("mailbox"));
        let mut pauses = 0;
        let woken = wait(&mailbox, &root.join("wait"), "bbbb-2222", "t1", || None, || {
            pauses += 1;
            if pauses == 2 {
                mailbox.send("bbbb-2222", &message("run the tests", true)).unwrap();
            }
        })
        .unwrap();
        let Woken::Message(notice) = woken else { panic!("not woken: {woken:?}") };
        assert!(notice.starts_with("1 unread message from another Claude session: roblox (louis)."), "{notice}");
        assert_eq!(pauses, 2, "woken at the first look after the message");
        assert!(!root.join("wait").join("bbbb-2222").exists(), "the claim goes with the wait");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_message_that_does_not_ask_to_wake_leaves_the_session_at_rest() {
        let root = temp_root("rest");
        let mailbox = Mailbox::new(root.join("mailbox"));
        let claim = root.join("wait").join("bbbb-2222");
        mailbox.send("bbbb-2222", &message("Combat.deal takes 3 args now", false)).unwrap();
        let mut pauses = 0;
        let woken = wait(&mailbox, &root.join("wait"), "bbbb-2222", "t1", || None, || {
            pauses += 1;
            if pauses == 3 {
                fs::write(&claim, "t2").unwrap();
            }
        })
        .unwrap();
        assert_eq!(woken, Woken::Replaced, "still waiting after 3 looks");
        assert!(mailbox.notice("bbbb-2222").unwrap().is_some(), "the message is still told at the next tool");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn another_reason_to_wake_ends_the_wait_with_its_notice() {
        let root = temp_root("due");
        let mailbox = Mailbox::new(root.join("mailbox"));
        let mut looks = 0;
        let due = || {
            looks += 1;
            (looks == 3).then(|| "look at your background work".to_string())
        };
        let woken = wait(&mailbox, &root.join("wait"), "bbbb-2222", "t1", due, || {}).unwrap();
        assert_eq!(woken, Woken::Message("look at your background work".into()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_newer_wait_of_the_session_takes_over_and_the_older_ends() {
        let root = temp_root("replaced");
        let mailbox = Mailbox::new(root.join("mailbox"));
        let waits = root.join("wait");
        let woken = wait(&mailbox, &waits, "bbbb-2222", "older", || None, || write_atomic(&waits.join("bbbb-2222"), "newer").unwrap()).unwrap();
        assert_eq!(woken, Woken::Replaced);
        assert_eq!(fs::read_to_string(waits.join("bbbb-2222")).unwrap(), "newer", "the newer claim stays");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_end_of_a_turn_and_the_start_of_the_session_start_a_wait() {
        assert_eq!(waiting_session(r#"{"session_id":"bbbb-2222","hook_event_name":"Stop"}"#).as_deref(), Some("bbbb-2222"));
        let resumed = r#"{"session_id":"bbbb-2222","hook_event_name":"SessionStart","source":"resume"}"#;
        assert_eq!(waiting_session(resumed).as_deref(), Some("bbbb-2222"), "a session started again waits at once");
        assert_eq!(waiting_session(r#"{"session_id":"bbbb-2222","hook_event_name":"SubagentStop"}"#), None);
        assert_eq!(waiting_session(r#"{"session_id":"bbbb-2222","hook_event_name":"UserPromptSubmit"}"#), None);
        assert_eq!(waiting_session(""), None, "the probe of kindric-accounts gives no input");
        assert_eq!(waiting_session("{broken"), None);
    }

    #[test]
    fn a_malformed_id_is_refused() {
        let root = temp_root("malformed");
        let mailbox = Mailbox::new(root.join("mailbox"));
        assert!(wait(&mailbox, &root.join("wait"), "../x", "t1", || None, || {}).is_err());
        assert!(!root.exists());
    }
}
