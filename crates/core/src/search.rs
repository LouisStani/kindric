use crate::accounts::SessionInfo;

/// Every word of `query` must appear, case-insensitively, in the session's title, account or directory.
pub fn matches(session: &SessionInfo, query: &str) -> bool {
    matches_with(session, "", query)
}

/// Like `matches`, `extra` (a name given in an interface) counting as one more field.
pub fn matches_with(session: &SessionInfo, extra: &str, query: &str) -> bool {
    let haystack = format!("{} {} {} {extra}", session.title(), session.account, session.cwd).to_lowercase();
    query.to_lowercase().split_whitespace().all(|word| haystack.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::SessionState;

    fn session(account: &str, name: &str, cwd: &str) -> SessionInfo {
        SessionInfo {
            account: account.into(),
            short_id: Some("abc".into()),
            session_id: format!("{account}-{name}"),
            name: name.into(),
            cwd: cwd.into(),
            state: SessionState::Waiting,
            isolated: false,
            progress: None,
            waiting_for: None,
        }
    }

    #[test]
    fn every_word_must_match_somewhere() {
        assert!(matches(&session("louis", "boss-fight", "/home/u/Roblox/frieren"), "ROB louis"));
        assert!(matches(&session("louis", "", "/home/u/Merkio"), "merkio"));
        assert!(!matches(&session("lystriq", "api", "/home/u/Pro/api"), "api louis"));
    }

    #[test]
    fn an_extra_name_counts_with_the_other_fields() {
        let api = session("lystriq", "Router CEO et flux", "/home/u/Pro/api");
        assert!(matches_with(&api, "router", "ROUTER lystriq"));
        assert!(matches_with(&api, "short", "short api"));
        assert!(!matches(&api, "short"));
    }
}
