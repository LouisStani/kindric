use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use kindric_accounts::Paths;
use kindric_accounts::files::create_private_dir;
use serde::{Deserialize, Serialize};

const READ_DIR: &str = "read";

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub from_session: String,
    pub from_title: String,
    pub from_account: String,
    pub sent_at: u64,
    pub text: String,
}

pub struct Mailbox {
    root: PathBuf,
}

/// A message as the app shows it: received by a session when `to_session` is that session,
/// sent by it otherwise. `unread` until the receiving session calls `read_messages`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub message: Message,
    pub to_session: String,
    pub unread: bool,
}

/// The message files of `dir`, in name order (the order they were sent); none if `dir` does
/// not exist.
fn message_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.is_file() && is_message_file(&path) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Where kindric keeps its files, next to the claude-accounts data: `$XDG_DATA_HOME`
/// (else `~/.local/share`) on Unix, the user's AppData on Windows, then `kindric`.
pub fn data_dir() -> io::Result<PathBuf> {
    data_dir_for(&Paths::from_env().map_err(io::Error::other)?)
}

fn data_dir_for(paths: &Paths) -> io::Result<PathBuf> {
    paths.kindric_data_dir().ok_or_else(|| io::Error::other("claude-accounts data has no parent"))
}

/// `mailbox` in the data directory.
pub fn default_root() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("mailbox"))
}

#[cfg(test)]
fn root_for(paths: &Paths) -> io::Result<PathBuf> {
    Ok(data_dir_for(paths)?.join("mailbox"))
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

/// Session ids become directory names: only UUID-like ids are accepted.
fn is_valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// 700 on Unix; Windows keeps the profile's own access rules.
fn private_dir(path: &Path) -> io::Result<()> {
    create_private_dir(path)
}

fn is_message_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "json")
        && path.file_name().is_some_and(|name| !name.to_string_lossy().starts_with('.'))
}

impl Mailbox {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Creates the root (700) so it can be watched before the first message.
    pub fn create_root(&self) -> io::Result<()> {
        private_dir(&self.root)
    }

    fn inbox(&self, session: &str) -> io::Result<PathBuf> {
        if !is_valid_session_id(session) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid session id: {session:?}")));
        }
        Ok(self.root.join(session))
    }

    /// Writes to a hidden temporary file first, then renames: a reader never sees half a message.
    pub fn send(&self, to_session: &str, message: &Message) -> io::Result<()> {
        let inbox = self.inbox(to_session)?;
        private_dir(&inbox)?;
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let stem = format!("{:020}-{}-{sequence}", message.sent_at, std::process::id());
        let temporary = inbox.join(format!(".{stem}.tmp"));
        fs::write(&temporary, serde_json::to_vec(message).map_err(io::Error::other)?)?;
        fs::rename(&temporary, inbox.join(format!("{stem}.json")))
    }

    fn unread_files(&self, session: &str) -> io::Result<Vec<PathBuf>> {
        message_files(&self.inbox(session)?)
    }

    /// The messages of an inbox, read or not, with the name of their file.
    fn inbox_entries(&self, to_session: &str) -> io::Result<Vec<(String, Entry)>> {
        let inbox = self.inbox(to_session)?;
        let mut found = Vec::new();
        for (dir, unread) in [(inbox.clone(), true), (inbox.join(READ_DIR), false)] {
            for file in message_files(&dir)? {
                // A damaged file must not hide the other messages.
                let Ok(message) = fs::read(&file).map_err(io::Error::other).and_then(|bytes| serde_json::from_slice(&bytes).map_err(io::Error::other)) else {
                    continue;
                };
                let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                found.push((name, Entry { message, to_session: to_session.to_string(), unread }));
            }
        }
        Ok(found)
    }

    /// What `session` received and sent, read or not, oldest first. Nothing is marked read:
    /// the session still gets its messages from `read_messages`.
    pub fn conversation(&self, session: &str) -> io::Result<Vec<Entry>> {
        let mut found = self.inbox_entries(session)?;
        let inboxes = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        for inbox in inboxes {
            let inbox = inbox?;
            let other = inbox.file_name().to_string_lossy().into_owned();
            if other == session || !is_valid_session_id(&other) || !inbox.path().is_dir() {
                continue;
            }
            let sent = self.inbox_entries(&other)?.into_iter().filter(|(_, entry)| entry.message.from_session == session);
            found.extend(sent);
        }
        found.sort_by(|(a_name, a), (b_name, b)| (a.message.sent_at, a_name).cmp(&(b.message.sent_at, b_name)));
        Ok(found.into_iter().map(|(_, entry)| entry).collect())
    }

    pub fn unread_count(&self, session: &str) -> io::Result<usize> {
        Ok(self.unread_files(session)?.len())
    }

    /// Returns unread messages, oldest first, and moves them to `read/`.
    pub fn take_unread(&self, session: &str) -> io::Result<Vec<Message>> {
        let files = self.unread_files(session)?;
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let read_dir = self.inbox(session)?.join(READ_DIR);
        private_dir(&read_dir)?;
        let mut messages = Vec::with_capacity(files.len());
        for file in files {
            let message = serde_json::from_slice(&fs::read(&file)?).map_err(io::Error::other)?;
            let name = file.file_name().ok_or_else(|| io::Error::other("message file without name"))?;
            fs::rename(&file, read_dir.join(name))?;
            messages.push(message);
        }
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("kindric-test-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn message(text: &str, sent_at: u64) -> Message {
        Message {
            from_session: "aaaa-1111".into(),
            from_title: "roblox".into(),
            from_account: "louis".into(),
            sent_at,
            text: text.into(),
        }
    }

    #[test]
    fn send_then_take_returns_messages_in_order_once() {
        let root = temp_root("order");
        let mailbox = Mailbox::new(root.clone());
        mailbox.send("bbbb-2222", &message("second", 20)).unwrap();
        mailbox.send("bbbb-2222", &message("first", 10)).unwrap();
        assert_eq!(mailbox.unread_count("bbbb-2222").unwrap(), 2);
        let texts: Vec<_> = mailbox.take_unread("bbbb-2222").unwrap().into_iter().map(|m| m.text).collect();
        assert_eq!(texts, ["first", "second"]);
        assert_eq!(mailbox.unread_count("bbbb-2222").unwrap(), 0);
        assert!(mailbox.take_unread("bbbb-2222").unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    fn from(session: &str, text: &str, sent_at: u64) -> Message {
        Message { from_session: session.into(), from_title: session.into(), from_account: "louis".into(), sent_at, text: text.into() }
    }

    #[test]
    fn a_conversation_holds_what_was_received_and_sent_and_marks_nothing_read() {
        let root = temp_root("conversation");
        let mailbox = Mailbox::new(root.clone());
        mailbox.send("bbbb", &from("aaaa", "a to b", 10)).unwrap();
        mailbox.send("aaaa", &from("cccc", "c to a", 20)).unwrap();
        mailbox.send("aaaa", &from("bbbb", "b to a", 5)).unwrap();
        mailbox.send("cccc", &from("bbbb", "b to c", 7)).unwrap();
        assert_eq!(mailbox.take_unread("aaaa").unwrap().len(), 2);
        mailbox.send("aaaa", &from("bbbb", "b to a again", 30)).unwrap();
        fs::write(root.join("aaaa/00000000000000000040-1-0.json"), b"{broken").unwrap();
        let unread_before = (mailbox.unread_count("aaaa").unwrap(), mailbox.unread_count("bbbb").unwrap());
        let seen: Vec<_> = mailbox
            .conversation("aaaa")
            .unwrap()
            .into_iter()
            .map(|entry| (entry.message.text, entry.to_session, entry.unread))
            .collect();
        assert_eq!(
            seen,
            [
                ("b to a".to_string(), "aaaa".to_string(), false),
                ("a to b".to_string(), "bbbb".to_string(), true),
                ("c to a".to_string(), "aaaa".to_string(), false),
                ("b to a again".to_string(), "aaaa".to_string(), true),
            ]
        );
        assert_eq!((mailbox.unread_count("aaaa").unwrap(), mailbox.unread_count("bbbb").unwrap()), unread_before);
        assert!(mailbox.conversation("never-written").unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_mailbox_yet_means_no_conversation() {
        assert!(Mailbox::new(temp_root("none")).conversation("aaaa").unwrap().is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn inbox_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp_root("private");
        let mailbox = Mailbox::new(root.clone());
        mailbox.send("cccc", &message("hi", 1)).unwrap();
        let mode = fs::metadata(root.join("cccc")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn temporary_and_hidden_files_are_ignored() {
        let root = temp_root("ignored");
        let mailbox = Mailbox::new(root.clone());
        mailbox.send("cccc", &message("hi", 1)).unwrap();
        fs::write(root.join("cccc/.half.tmp"), b"{").unwrap();
        fs::write(root.join("cccc/.hidden.json"), b"{").unwrap();
        fs::write(root.join("cccc/stray.txt"), b"{").unwrap();
        assert_eq!(mailbox.unread_count("cccc").unwrap(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unknown_session_has_no_messages_and_bad_ids_are_refused() {
        let mailbox = Mailbox::new(temp_root("ids"));
        assert_eq!(mailbox.unread_count("never-written").unwrap(), 0);
        assert!(mailbox.send("../escape", &message("x", 1)).is_err());
        assert!(mailbox.unread_count("a/b").is_err());
        assert!(mailbox.send("", &message("x", 1)).is_err());
    }

    #[test]
    fn the_mailbox_sits_next_to_the_claude_accounts_data() {
        let home = PathBuf::from("/home/u");
        let paths = Paths::from_roots(home.join(".config"), home.join(".local/share"), home.clone());
        assert_eq!(root_for(&paths).unwrap(), home.join(".local/share").join("kindric").join("mailbox"));
    }

    /// The path of versions up to 0.3.0, so existing messages are still found.
    #[test]
    #[cfg(unix)]
    fn the_unix_path_is_unchanged() {
        let data = match std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
            Some(data) => PathBuf::from(data),
            None => std::env::home_dir().unwrap().join(".local/share"),
        };
        assert_eq!(default_root().unwrap(), data.join("kindric/mailbox"));
    }
}
