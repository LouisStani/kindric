//! What the page shows: the session list, built from each refresh.

use std::collections::HashMap;
use std::path::Path;

use kindric_core::accounts::{SessionInfo, SessionState};
use kindric_core::launcher::tilde;
use kindric_core::refresher::Update;
use kindric_core::search::matches_with;
use serde::Serialize;

use crate::layout::{PaneId, SessionKey};
use crate::prefs::{Prefs, short_title};
use crate::projects::Project;
use crate::tabs::Tabs;

/// A directory field offers at most this many known folders.
const KNOWN: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionRow {
    /// Claude Code's name, whole.
    pub title: String,
    /// What the bar shows: the short name given with F2, else `title` cut. Set by `assign`.
    pub label: String,
    /// The short name given with F2, if any, which the F2 form starts from.
    pub given: Option<String>,
    /// Hidden with Ctrl+Shift+K: out of the bar, last in Ctrl+P. Set by `assign`.
    pub hidden: bool,
    pub account: String,
    pub directory: String,
    /// The full session id, which names its mailbox.
    pub session_id: String,
    /// What the session last said it is doing, if Claude Code wrote it.
    pub progress: Option<String>,
    /// Only background sessions have one, and only they can be opened here.
    pub short_id: Option<String>,
    pub state: &'static str,
    pub isolated: bool,
    /// The tab the session belongs to: its project's, else "other". Set by `assign`.
    pub tab: PaneId,
    pub unread: usize,
    /// Claude finished a turn here and it has not been looked at since.
    pub finished: bool,
    /// The image files Claude's last turn proposed, shown in the pane until the next prompt.
    pub images: Vec<String>,
    /// How many files the session changed in its repositories (see `changes`), counted when its
    /// state last changed; `None` when it could not be counted.
    pub changed: Option<usize>,
    /// The user said they are away until then (Unix seconds): the session keeps working.
    pub away_until: Option<i64>,
    /// Kept for Ctrl+P, which searches the same fields as the terminal picker.
    #[serde(skip)]
    pub info: SessionInfo,
    /// Place in the snapshot, which orders sessions of the same rank.
    #[serde(skip)]
    position: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ListView {
    /// False until the first refresh, so the page can say "loading…" instead of "no sessions".
    pub loaded: bool,
    /// Offered by the Ctrl+N form.
    pub accounts: Vec<String>,
    pub sessions: Vec<SessionRow>,
    pub errors: Vec<String>,
}

fn shows(key: &SessionKey, session: &SessionInfo) -> bool {
    session.account == key.account && session.short_id.as_deref() == Some(key.short_id.as_str())
}

/// Waiting now after working at the last refresh, or already marked; never the session
/// being looked at.
fn finished(previous: &ListView, session: &SessionInfo, watched: Option<&SessionKey>) -> bool {
    if session.state != SessionState::Waiting || watched.is_some_and(|key| shows(key, session)) {
        return false;
    }
    previous
        .sessions
        .iter()
        .find(|row| row.info.session_id == session.session_id)
        .is_some_and(|row| row.finished || row.info.state == SessionState::Working)
}

fn row(position: usize, session: &SessionInfo, unread: usize, finished: bool) -> SessionRow {
    SessionRow {
        title: session.title(),
        label: short_title(None, &session.title()),
        given: None,
        hidden: false,
        account: session.account.clone(),
        directory: session.cwd.clone(),
        session_id: session.session_id.clone(),
        progress: session.progress.clone(),
        short_id: session.short_id.clone(),
        state: session.state.label(),
        isolated: session.isolated,
        tab: 0,
        unread,
        finished,
        images: Vec::new(),
        changed: None,
        away_until: None,
        info: session.clone(),
        position,
    }
}

/// Where a session goes in the bar: what needs you first, then what runs, then what sleeps.
/// Sessions running in another terminal cannot be opened here: after those. Hidden ones come
/// last of all, for Ctrl+P.
fn rank(row: &SessionRow) -> u8 {
    if row.hidden {
        return 6;
    }
    if row.short_id.is_none() {
        return 5;
    }
    if row.unread > 0 {
        return 0;
    }
    if row.finished {
        return 1;
    }
    match row.info.state {
        SessionState::Waiting => 2,
        SessionState::Working => 3,
        SessionState::Paused => 4,
        SessionState::Elsewhere => 5,
    }
}

/// Sorts the rows by `rank`, in snapshot order within a rank.
fn order(sessions: &mut [SessionRow]) {
    sessions.sort_by_key(|row| (rank(row), row.position));
}

/// Gives every row the tab of its project and its short name, after a refresh or a change of
/// projects or names.
pub fn assign(view: &mut ListView, tabs: &Tabs, prefs: &Prefs) {
    for row in &mut view.sessions {
        row.tab = tabs.owner_tab(Path::new(&row.directory));
        let kept = prefs.get(&row.info.session_id);
        row.given = kept.and_then(|kept| kept.label.clone());
        row.hidden = kept.is_some_and(|kept| kept.hidden);
        row.label = short_title(row.given.as_deref(), &row.title);
    }
    order(&mut view.sessions);
}

/// A failed refresh keeps the previous list on screen and says why it is stale.
/// `watched` is the session in the active pane of the visible tab.
pub fn apply(previous: &ListView, update: Update, watched: Option<&SessionKey>) -> ListView {
    match update {
        Ok((snapshot, unread)) => {
            let mut sessions: Vec<SessionRow> = snapshot
                .sessions
                .iter()
                .enumerate()
                .map(|(position, session)| {
                    let unread = unread.get(&session.session_id).copied().unwrap_or(0);
                    row(position, session, unread, finished(previous, session, watched))
                })
                .collect();
            order(&mut sessions);
            ListView { loaded: true, accounts: snapshot.accounts, sessions, errors: snapshot.errors }
        }
        Err(error) => ListView {
            loaded: previous.loaded,
            accounts: previous.accounts.clone(),
            sessions: previous.sessions.clone(),
            errors: vec![format!("session list unavailable: {error}")],
        },
    }
}

/// The images of a session in `next`: looked up with `lookup` when its turn just ended, kept
/// while it waits, gone once it works again (a prompt) or after a pause. The first list has none.
pub fn propose_images(next: &mut ListView, previous: &ListView, lookup: impl Fn(&SessionInfo) -> Vec<String>) {
    for row in &mut next.sessions {
        let before = previous.sessions.iter().find(|before| before.info.session_id == row.info.session_id);
        row.images = match (row.info.state, before.map(|before| before.info.state)) {
            (SessionState::Waiting, Some(SessionState::Working)) => lookup(&row.info),
            (SessionState::Waiting, Some(SessionState::Waiting)) => before.map(|before| before.images.clone()).unwrap_or_default(),
            _ => Vec::new(),
        };
    }
}

/// Until when each session keeps working while the user is away, as `until` reads it.
pub fn mark_away(view: &mut ListView, until: impl Fn(&str) -> Option<i64>) {
    for row in &mut view.sessions {
        row.away_until = until(&row.session_id);
    }
}

/// The sessions of `next` whose changed files must be counted again: those that just appeared
/// and those whose state changed since `previous` (decision 234).
pub fn to_count(next: &ListView, previous: &ListView) -> Vec<SessionInfo> {
    next.sessions
        .iter()
        .filter(|row| {
            let before = previous.sessions.iter().find(|before| before.info.session_id == row.info.session_id);
            before.is_none_or(|before| before.info.state != row.info.state)
        })
        .map(|row| row.info.clone())
        .collect()
}

/// The counts of `counted` (session id → count) where there is one, else the count the session
/// had before.
pub fn carry_changed(next: &mut ListView, previous: &ListView, counted: &HashMap<String, Option<usize>>) {
    for row in &mut next.sessions {
        row.changed = match counted.get(&row.info.session_id) {
            Some(count) => *count,
            None => previous.sessions.iter().find(|before| before.info.session_id == row.info.session_id).and_then(|before| before.changed),
        };
    }
}

/// Puts the counts of `counted` on their rows, once they are known. Says whether anything changed.
pub fn set_changed(view: &mut ListView, counted: &HashMap<String, Option<usize>>) -> bool {
    let mut changed = false;
    for row in &mut view.sessions {
        if let Some(count) = counted.get(&row.info.session_id)
            && row.changed != *count
        {
            row.changed = *count;
            changed = true;
        }
    }
    changed
}

/// `key` is being looked at: its mark goes. Says whether anything changed.
pub fn seen(view: &mut ListView, key: &SessionKey) -> bool {
    let mut changed = false;
    for row in view.sessions.iter_mut().filter(|row| row.finished && shows(key, &row.info)) {
        row.finished = false;
        changed = true;
    }
    if changed {
        order(&mut view.sessions);
    }
    changed
}

/// The folders a directory field offers before anything is typed: the projects', then those
/// of the sessions in bar order, each once.
pub fn known_directories(projects: &[Project], view: &ListView, home: &Path) -> Vec<String> {
    let folders = projects.iter().map(|project| project.directory.as_path());
    let sessions = view.sessions.iter().filter(|row| !row.hidden && !row.directory.is_empty()).map(|row| Path::new(&row.directory));
    let mut known: Vec<String> = Vec::new();
    for folder in folders.chain(sessions).map(|folder| tilde(folder, home)) {
        if known.len() == KNOWN {
            break;
        }
        if !known.contains(&folder) {
            known.push(folder);
        }
    }
    known
}

/// The rows Ctrl+P offers for `query`: those of `tab` only, hidden ones last, in bar order.
pub fn filter(view: &ListView, tab: PaneId, query: &str) -> Vec<SessionRow> {
    view.sessions.iter().filter(|row| row.tab == tab && matches_with(&row.info, &row.label, query)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use kindric_core::accounts::{SessionState, Snapshot};

    use super::*;

    fn session(account: &str, name: &str, isolated: bool) -> SessionInfo {
        SessionInfo {
            account: account.into(),
            short_id: Some("abc".into()),
            session_id: format!("{account}-{name}"),
            name: name.into(),
            cwd: "/home/u/Dev".into(),
            state: SessionState::Waiting,
            isolated,
            progress: None,
        }
    }

    fn snapshot(sessions: Vec<SessionInfo>) -> Snapshot {
        Snapshot { sessions, ..Snapshot::default() }
    }

    fn unread_of(view: &ListView, title: &str) -> usize {
        view.sessions.iter().find(|row| row.title == title).map(|row| row.unread).unwrap()
    }

    fn titles(rows: &[SessionRow]) -> Vec<&str> {
        rows.iter().map(|row| row.title.as_str()).collect()
    }

    #[test]
    fn rows_of_the_same_state_keep_the_snapshot_order() {
        let sessions = (0..10).map(|i| session("louis", &format!("s{i}"), false)).collect();
        let view = apply(&ListView::default(), Ok((snapshot(sessions), HashMap::new())), None);
        assert!(view.loaded);
        assert_eq!(view.sessions[0].title, "s0");
        assert_eq!(view.sessions[9].title, "s9");
    }

    #[test]
    fn unread_counts_are_matched_by_session_id() {
        let sessions = vec![session("louis", "a", false), session("lystriq", "b", false)];
        let unread = HashMap::from([("lystriq-b".to_string(), 3)]);
        let view = apply(&ListView::default(), Ok((snapshot(sessions), unread)), None);
        assert_eq!(unread_of(&view, "a"), 0);
        assert_eq!(unread_of(&view, "b"), 3);
    }

    #[test]
    fn rows_carry_account_directory_id_state_and_isolation() {
        let sessions = vec![session("pro", "x", true)];
        let view = apply(&ListView::default(), Ok((snapshot(sessions), HashMap::new())), None);
        assert_eq!(view.sessions[0].account, "pro");
        assert_eq!(view.sessions[0].directory, "/home/u/Dev");
        assert_eq!(view.sessions[0].short_id.as_deref(), Some("abc"));
        assert_eq!(view.sessions[0].state, "waiting");
        assert!(view.sessions[0].isolated);
    }

    #[test]
    fn a_session_the_user_is_away_from_carries_the_end() {
        let sessions = vec![session("pro", "x", false), session("louis", "y", false)];
        let mut view = apply(&ListView::default(), Ok((snapshot(sessions), HashMap::new())), None);
        let away_from = view.sessions[0].session_id.clone();
        mark_away(&mut view, |session| (session == away_from).then_some(1_800));
        assert_eq!(view.sessions[0].away_until, Some(1_800));
        assert_eq!(view.sessions[1].away_until, None);
        mark_away(&mut view, |_| None);
        assert_eq!(view.sessions[0].away_until, None, "back");
    }

    #[test]
    fn snapshot_errors_are_shown() {
        let mut broken = snapshot(Vec::new());
        broken.errors.push("louis: agents failed".into());
        let view = apply(&ListView::default(), Ok((broken, HashMap::new())), None);
        assert_eq!(view.errors, ["louis: agents failed"]);
    }

    #[test]
    fn a_failed_refresh_keeps_the_list_and_says_why() {
        let sessions = vec![session("louis", "a", false)];
        let previous = apply(&ListView::default(), Ok((snapshot(sessions), HashMap::new())), None);
        let view = apply(&previous, Err("HOME is not set".into()), None);
        assert_eq!(view.sessions, previous.sessions);
        assert!(view.loaded);
        assert_eq!(view.errors, ["session list unavailable: HOME is not set"]);
    }

    #[test]
    fn accounts_come_from_the_snapshot_and_survive_a_failed_refresh() {
        let listed = Snapshot { accounts: vec!["louis".into(), "lystriq".into()], ..Snapshot::default() };
        let view = apply(&ListView::default(), Ok((listed, HashMap::new())), None);
        assert_eq!(view.accounts, ["louis", "lystriq"]);
        let failed = apply(&view, Err("HOME is not set".into()), None);
        assert_eq!(failed.accounts, ["louis", "lystriq"]);
    }

    #[test]
    fn filter_keeps_matching_rows_in_list_order() {
        let mut sessions = vec![session("louis", "boss-fight", false), session("lystriq", "api", false)];
        sessions.push(session("louis", "api", false));
        let view = apply(&ListView::default(), Ok((snapshot(sessions), HashMap::new())), None);
        assert_eq!(titles(&filter(&view, 0, "")), ["boss-fight", "api", "api"]);
        assert_eq!(titles(&filter(&view, 0, "API")), ["api", "api"]);
        let both = filter(&view, 0, "api louis");
        assert_eq!(both.len(), 1);
        assert_eq!(both[0].account, "louis");
        assert!(filter(&view, 0, "zzz").is_empty());
    }

    fn in_state(account: &str, name: &str, state: SessionState) -> SessionInfo {
        SessionInfo { state, short_id: Some(name.into()), ..session(account, name, false) }
    }

    fn refresh(previous: &ListView, sessions: Vec<SessionInfo>, watched: Option<&SessionKey>) -> ListView {
        apply(previous, Ok((snapshot(sessions), HashMap::new())), watched)
    }

    fn marks(view: &ListView) -> Vec<bool> {
        view.sessions.iter().map(|row| row.finished).collect()
    }

    fn key(account: &str, short_id: &str) -> SessionKey {
        SessionKey { account: account.into(), short_id: short_id.into() }
    }

    #[test]
    fn a_session_that_stops_working_is_marked_until_it_works_again() {
        let working = refresh(&ListView::default(), vec![in_state("louis", "a", SessionState::Working)], None);
        assert_eq!(marks(&working), [false]);
        let done = refresh(&working, vec![in_state("louis", "a", SessionState::Waiting)], None);
        assert_eq!(marks(&done), [true]);
        let still = refresh(&done, vec![in_state("louis", "a", SessionState::Waiting)], None);
        assert_eq!(marks(&still), [true]);
        let again = refresh(&still, vec![in_state("louis", "a", SessionState::Working)], None);
        assert_eq!(marks(&again), [false]);
    }

    #[test]
    fn resumed_sessions_and_the_first_list_are_not_marked() {
        let first = refresh(&ListView::default(), vec![in_state("louis", "a", SessionState::Waiting)], None);
        assert_eq!(marks(&first), [false]);
        let paused = refresh(&first, vec![in_state("louis", "a", SessionState::Paused)], None);
        let resumed = refresh(&paused, vec![in_state("louis", "a", SessionState::Waiting)], None);
        assert_eq!(marks(&resumed), [false]);
    }

    #[test]
    fn the_watched_session_is_never_marked() {
        let working = vec![in_state("louis", "a", SessionState::Working), in_state("louis", "b", SessionState::Working)];
        let working = refresh(&ListView::default(), working, None);
        let waiting = vec![in_state("louis", "a", SessionState::Waiting), in_state("louis", "b", SessionState::Waiting)];
        let done = refresh(&working, waiting, Some(&key("louis", "a")));
        assert_eq!(titles(&done.sessions), ["b", "a"]);
        assert_eq!(marks(&done), [true, false]);
    }

    #[test]
    fn looking_at_a_session_removes_its_mark() {
        let working = refresh(&ListView::default(), vec![in_state("louis", "a", SessionState::Working)], None);
        let mut done = refresh(&working, vec![in_state("louis", "a", SessionState::Waiting)], None);
        assert!(!seen(&mut done, &key("lystriq", "a")), "another account");
        assert_eq!(marks(&done), [true]);
        assert!(seen(&mut done, &key("louis", "a")));
        assert_eq!(marks(&done), [false]);
        assert!(!seen(&mut done, &key("louis", "a")), "nothing left to remove");
    }

    fn elsewhere(account: &str, name: &str) -> SessionInfo {
        SessionInfo { short_id: None, ..in_state(account, name, SessionState::Elsewhere) }
    }

    #[test]
    fn the_bar_puts_what_needs_you_first() {
        let before = vec![in_state("louis", "done", SessionState::Working)];
        let previous = refresh(&ListView::default(), before, None);
        let sessions = vec![
            elsewhere("louis", "other-terminal"),
            in_state("louis", "asleep", SessionState::Paused),
            in_state("louis", "busy", SessionState::Working),
            in_state("louis", "waits", SessionState::Waiting),
            in_state("louis", "done", SessionState::Waiting),
            in_state("louis", "mail", SessionState::Working),
        ];
        let unread = HashMap::from([("louis-mail".to_string(), 1)]);
        let view = apply(&previous, Ok((snapshot(sessions), unread)), None);
        assert_eq!(titles(&view.sessions), ["mail", "done", "waits", "busy", "asleep", "other-terminal"]);
    }

    #[test]
    fn sessions_of_another_terminal_stay_last_even_with_messages() {
        let sessions = vec![elsewhere("louis", "other-terminal"), in_state("louis", "asleep", SessionState::Paused)];
        let unread = HashMap::from([("louis-other-terminal".to_string(), 2)]);
        let view = apply(&ListView::default(), Ok((snapshot(sessions), unread)), None);
        assert_eq!(titles(&view.sessions), ["asleep", "other-terminal"]);
    }

    #[test]
    fn sessions_of_the_same_rank_keep_the_snapshot_order() {
        let states = [SessionState::Paused, SessionState::Waiting];
        let sessions = (0..40).map(|i| in_state("louis", &format!("s{i:02}"), states[i % 2])).collect();
        let view = refresh(&ListView::default(), sessions, None);
        let waiting: Vec<String> = (0..40).filter(|i| i % 2 == 1).map(|i| format!("s{i:02}")).collect();
        let paused: Vec<String> = (0..40).filter(|i| i % 2 == 0).map(|i| format!("s{i:02}")).collect();
        assert_eq!(titles(&view.sessions[..20]), waiting);
        assert_eq!(titles(&view.sessions[20..]), paused);
    }

    #[test]
    fn a_session_looked_at_moves_down_with_its_mark() {
        let working = vec![in_state("louis", "a", SessionState::Waiting), in_state("louis", "b", SessionState::Working)];
        let working = refresh(&ListView::default(), working, None);
        let waiting = vec![in_state("louis", "a", SessionState::Waiting), in_state("louis", "b", SessionState::Waiting)];
        let mut done = refresh(&working, waiting, None);
        assert_eq!(titles(&done.sessions), ["b", "a"]);
        assert!(seen(&mut done, &key("louis", "b")));
        assert_eq!(titles(&done.sessions), ["a", "b"]);
    }

    #[test]
    fn rows_get_the_tab_of_their_project() {
        let mut tabs = Tabs::default();
        let all = tabs.active_tab();
        tabs.add_project(Project::new("/home/u/Dev/api".into())).unwrap();
        let api = tabs.active_tab();
        let sessions = vec![
            SessionInfo { cwd: "/home/u/Dev/api/src".into(), ..session("louis", "in", false) },
            SessionInfo { cwd: "/home/u".into(), ..session("louis", "out", false) },
        ];
        let mut view = refresh(&ListView::default(), sessions, None);
        assign(&mut view, &tabs, &Prefs::new());
        let tab_of = |title: &str| view.sessions.iter().find(|row| row.title == title).map(|row| row.tab);
        assert_eq!(tab_of("in"), Some(api));
        assert_eq!(tab_of("out"), Some(all));
    }

    #[test]
    fn a_given_short_name_is_shown_and_found() {
        let sessions = vec![session("louis", "Router CEO et flux de données Overview", false), session("louis", "api", false)];
        let mut view = refresh(&ListView::default(), sessions, None);
        let mut prefs = Prefs::new();
        crate::prefs::set_label(&mut prefs, "louis-api", "backend");
        let tabs = Tabs::default();
        assign(&mut view, &tabs, &prefs);
        let label_of = |title: &str| view.sessions.iter().find(|row| row.title == title).map(|row| row.label.clone());
        assert_eq!(label_of("api").as_deref(), Some("backend"));
        let given_of = |title: &str| view.sessions.iter().find(|row| row.title == title).and_then(|row| row.given.clone());
        assert_eq!(given_of("api").as_deref(), Some("backend"));
        assert_eq!(given_of("Router CEO et flux de données Overview"), None);
        assert_eq!(label_of("Router CEO et flux de données Overview").as_deref(), Some("Router CEO et flux de…"));
        assert_eq!(titles(&filter(&view, tabs.active_tab(), "backend")), ["api"]);
    }

    #[test]
    fn ctrl_p_only_offers_the_sessions_of_the_visible_tab() {
        let mut tabs = Tabs::default();
        let other = tabs.active_tab();
        tabs.add_project(Project::new("/home/u/Pro".into())).unwrap();
        let pro = tabs.active_tab();
        let sessions = vec![
            SessionInfo { cwd: "/home/u/Pro/api".into(), ..session("louis", "api-pro", false) },
            SessionInfo { cwd: "/home/u/perso".into(), ..session("louis", "api-perso", false) },
        ];
        let mut view = refresh(&ListView::default(), sessions, None);
        assign(&mut view, &tabs, &Prefs::new());
        assert_eq!(titles(&filter(&view, pro, "api")), ["api-pro"]);
        assert_eq!(titles(&filter(&view, other, "api")), ["api-perso"]);
    }

    #[test]
    fn hidden_sessions_go_last_and_stay_findable() {
        let sessions = vec![
            SessionInfo { cwd: "/home/u/old-work".into(), ..in_state("louis", "old", SessionState::Waiting) },
            in_state("louis", "asleep", SessionState::Paused),
            elsewhere("louis", "other"),
        ];
        let mut view = refresh(&ListView::default(), sessions, None);
        let mut prefs = Prefs::new();
        crate::prefs::set_hidden(&mut prefs, "louis-old", true);
        let tabs = Tabs::default();
        assign(&mut view, &tabs, &prefs);
        assert_eq!(titles(&view.sessions), ["asleep", "other", "old"]);
        assert!(view.sessions[2].hidden);
        assert_eq!(titles(&filter(&view, tabs.active_tab(), "old")), ["old"]);
        let home = Path::new("/home/u");
        assert!(!known_directories(&[], &view, home).contains(&tilde(Path::new("/home/u/old-work"), home)));
    }

    #[test]
    fn known_folders_are_the_projects_then_the_sessions_each_once() {
        let home = Path::new("/home/u");
        let projects = [Project::new("/home/u/Dev/api".into()), Project::new("/home/u/Dev/blog".into())];
        let sessions = vec![
            SessionInfo { cwd: "/home/u/Dev/blog".into(), ..session("louis", "a", false) },
            SessionInfo { cwd: "/tmp/x".into(), ..session("louis", "b", false) },
            SessionInfo { cwd: String::new(), ..session("louis", "c", false) },
        ];
        let view = refresh(&ListView::default(), sessions, None);
        let known = known_directories(&projects, &view, home);
        let expected: Vec<String> =
            [Path::new("/home/u/Dev/api"), Path::new("/home/u/Dev/blog"), Path::new("/tmp/x")].iter().map(|folder| tilde(folder, home)).collect();
        assert_eq!(known, expected);
    }

    #[test]
    fn known_folders_stop_at_ten() {
        let projects: Vec<Project> = (0..12).map(|i| Project::new(format!("/home/u/p{i}").into())).collect();
        let known = known_directories(&projects, &ListView::default(), Path::new("/home/u"));
        assert_eq!(known.len(), 10);
        assert_eq!(known[9], tilde(Path::new("/home/u/p9"), Path::new("/home/u")));
    }

    fn with_images(previous: &ListView, sessions: Vec<SessionInfo>) -> ListView {
        let mut next = refresh(previous, sessions, None);
        propose_images(&mut next, previous, |session| vec![format!("{}.png", session.name)]);
        next
    }

    fn images(view: &ListView) -> Vec<Vec<String>> {
        view.sessions.iter().map(|row| row.images.clone()).collect()
    }

    #[test]
    fn images_come_at_the_end_of_a_turn_and_go_at_the_next_prompt() {
        let first = with_images(&ListView::default(), vec![in_state("louis", "a", SessionState::Waiting)]);
        assert_eq!(images(&first), [Vec::<String>::new()], "the first list proposes nothing");
        let working = with_images(&first, vec![in_state("louis", "a", SessionState::Working)]);
        assert_eq!(images(&working), [Vec::<String>::new()]);
        let done = with_images(&working, vec![in_state("louis", "a", SessionState::Waiting)]);
        assert_eq!(images(&done), [vec!["a.png".to_string()]]);
        let mut kept = refresh(&done, vec![in_state("louis", "a", SessionState::Waiting)], None);
        propose_images(&mut kept, &done, |_| panic!("only looked up when a turn ends"));
        assert_eq!(images(&kept), [vec!["a.png".to_string()]]);
        let prompted = with_images(&kept, vec![in_state("louis", "a", SessionState::Working)]);
        assert_eq!(images(&prompted), [Vec::<String>::new()]);
        let paused = with_images(&done, vec![in_state("louis", "a", SessionState::Paused)]);
        assert_eq!(images(&paused), [Vec::<String>::new()]);
    }

    #[test]
    fn a_failed_refresh_keeps_the_images() {
        let working = with_images(&ListView::default(), vec![in_state("louis", "a", SessionState::Working)]);
        let done = with_images(&working, vec![in_state("louis", "a", SessionState::Waiting)]);
        let mut failed = apply(&done, Err("HOME is not set".into()), None);
        propose_images(&mut failed, &done, |_| Vec::new());
        assert_eq!(images(&failed), [vec!["a.png".to_string()]]);
    }

    /// Counts every session `to_count` asks for: its name's length, so each gets its own number.
    fn with_counts(previous: &ListView, sessions: Vec<SessionInfo>) -> (ListView, Vec<String>) {
        let mut next = refresh(previous, sessions, None);
        let asked = to_count(&next, previous);
        let counted: HashMap<String, Option<usize>> = asked.iter().map(|session| (session.session_id.clone(), Some(session.name.len()))).collect();
        carry_changed(&mut next, previous, &counted);
        (next, asked.into_iter().map(|session| session.name).collect())
    }

    #[test]
    fn changed_files_are_counted_when_a_session_appears_or_changes_state() {
        let (first, asked) = with_counts(&ListView::default(), vec![in_state("louis", "a", SessionState::Working)]);
        assert_eq!((asked, first.sessions[0].changed), (vec!["a".to_string()], Some(1)), "a session just seen");
        let (same, asked) = with_counts(&first, vec![in_state("louis", "a", SessionState::Working)]);
        assert_eq!((asked.len(), same.sessions[0].changed), (0, Some(1)), "same state: the count stays");
        let (done, asked) = with_counts(&same, vec![in_state("louis", "a", SessionState::Waiting), in_state("louis", "bb", SessionState::Waiting)]);
        assert_eq!(asked, ["a", "bb"]);
        assert_eq!(done.sessions.iter().map(|row| row.changed).collect::<Vec<_>>(), [Some(1), Some(2)]);
        let mut failed = refresh(&done, vec![in_state("louis", "a", SessionState::Working)], None);
        carry_changed(&mut failed, &done, &HashMap::from([(done.sessions[0].info.session_id.clone(), None)]));
        assert_eq!(failed.sessions[0].changed, None, "a count that failed shows nothing");
    }

    #[test]
    fn a_failed_refresh_keeps_the_marks() {
        let working = refresh(&ListView::default(), vec![in_state("louis", "a", SessionState::Working)], None);
        let done = refresh(&working, vec![in_state("louis", "a", SessionState::Waiting)], None);
        let failed = apply(&done, Err("HOME is not set".into()), None);
        assert_eq!(marks(&failed), [true]);
    }
}
