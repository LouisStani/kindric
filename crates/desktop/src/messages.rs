//! The messages of a session as the page shows them (Ctrl+Shift+M): read from the mailbox
//! without marking anything read.

use kindric_core::mailbox::Entry;
use serde::Serialize;

/// An unknown session is named by the start of its id.
const SHORT_ID: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MessageRow {
    /// Received by the session, else sent by it.
    pub incoming: bool,
    /// The other session: who sent it, or who it was sent to.
    pub other: String,
    pub account: String,
    pub sent_at: u64,
    pub text: String,
    /// Received and not read by the session yet.
    pub unread: bool,
}

/// The rows of `entries`, a conversation of session `me`. `known` gives the name and account
/// of a session still listed, by full id, so a session is named as in the bar.
pub fn rows(entries: Vec<Entry>, me: &str, known: impl Fn(&str) -> Option<(String, String)>) -> Vec<MessageRow> {
    entries
        .into_iter()
        .map(|entry| {
            let incoming = entry.to_session == me;
            let other_id = if incoming { &entry.message.from_session } else { &entry.to_session };
            let (other, account) = known(other_id).unwrap_or_else(|| {
                if incoming {
                    (entry.message.from_title.clone(), entry.message.from_account.clone())
                } else {
                    (other_id.chars().take(SHORT_ID).collect(), String::new())
                }
            });
            MessageRow { incoming, other, account, sent_at: entry.message.sent_at, text: entry.message.text, unread: incoming && entry.unread }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use kindric_core::mailbox::Message;

    use super::*;

    fn entry(from: &str, to: &str, unread: bool) -> Entry {
        let message = Message {
            from_session: from.into(),
            from_title: format!("{from} title"),
            from_account: "louis".into(),
            sent_at: 10,
            text: format!("{from} to {to}"),
            wake: false,
        };
        Entry { message, to_session: to.into(), unread }
    }

    fn known(id: &str) -> Option<(String, String)> {
        (id == "bbbb-2222").then(|| ("router".to_string(), "lystriq".to_string()))
    }

    #[test]
    fn received_and_sent_messages_name_the_other_session() {
        let rows = rows(
            vec![
                entry("bbbb-2222", "aaaa-1111", true),
                entry("aaaa-1111", "bbbb-2222", true),
                entry("cccc-3333", "aaaa-1111", false),
                entry("aaaa-1111", "dddd-4444-5555", false),
            ],
            "aaaa-1111",
            known,
        );
        let seen: Vec<_> = rows.iter().map(|row| (row.incoming, row.other.as_str(), row.account.as_str(), row.unread)).collect();
        assert_eq!(
            seen,
            [
                (true, "router", "lystriq", true),
                (false, "router", "lystriq", false),
                (true, "cccc-3333 title", "louis", false),
                (false, "dddd-444", "", false),
            ]
        );
        assert_eq!(rows[1].text, "aaaa-1111 to bbbb-2222");
    }
}
