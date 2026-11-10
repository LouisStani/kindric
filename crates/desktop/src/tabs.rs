//! The tabs of the window, each with its own tree of panes: "other" first, then one per
//! project. Hidden tabs keep their sessions attached; pane ids are unique across tabs. The
//! active tab and the session of each come back at the next launch (`Saved`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::layout::{Ids, Layout, PaneId, SessionKey, Shown, Side};
use crate::projects::{self, Project};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Tab {
    id: PaneId,
    layout: Layout,
    /// None for "other", whose bar shows the sessions of no project.
    project: Option<Project>,
    /// The session this tab showed when the app last closed: the page opens it at the first
    /// visit, unless a session was opened here before.
    #[serde(skip_serializing_if = "Option::is_none")]
    restore: Option<SessionKey>,
}

/// What `workspace.json` keeps: the active tab and the session each tab showed, tabs named
/// by their project's folder (None for "other").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    active: Option<PathBuf>,
    shown: Vec<SavedTab>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SavedTab {
    project: Option<PathBuf>,
    session: SessionKey,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Tabs {
    tabs: Vec<Tab>,
    active: PaneId,
    #[serde(skip)]
    ids: Ids,
}

impl Default for Tabs {
    fn default() -> Self {
        let mut ids = Ids::default();
        let all = Tab { id: ids.take(), layout: Layout::new(&mut ids), project: None, restore: None };
        Self { active: all.id, tabs: vec![all], ids }
    }
}

impl Tabs {
    /// "other", then a tab per project, "other" active.
    pub fn with_projects(projects: Vec<Project>) -> Self {
        let mut tabs = Self::default();
        for project in projects {
            tabs.push(project);
        }
        tabs.active = tabs.tabs[0].id;
        tabs
    }

    fn push(&mut self, project: Project) {
        let tab = Tab { id: self.ids.take(), layout: Layout::new(&mut self.ids), project: Some(project), restore: None };
        self.active = tab.id;
        self.tabs.push(tab);
    }

    fn index(&self) -> usize {
        self.tabs.iter().position(|tab| tab.id == self.active).unwrap_or(0)
    }

    #[cfg(test)]
    pub fn active_tab(&self) -> PaneId {
        self.active
    }

    /// The tree of the active tab, which the pane commands act on.
    pub fn current_mut(&mut self) -> &mut Layout {
        let index = self.index();
        &mut self.tabs[index].layout
    }

    /// The session being looked at: the one in the active pane of the visible tab.
    pub fn watched(&self) -> Option<&SessionKey> {
        self.tabs[self.index()].layout.active_session()
    }

    /// Splits the active pane of the active tab; the new pane goes on `side`.
    pub fn split(&mut self, width: f64, height: f64, side: Side) {
        let index = self.index();
        self.tabs[index].layout.split(&mut self.ids, width, height, side);
    }

    /// A tab for `project`, with one empty pane, after the others so their numbers do not
    /// change. A folder is one project only.
    pub fn add_project(&mut self, project: Project) -> Result<(), String> {
        if self.projects().iter().any(|known| known.directory == project.directory) {
            return Err(format!("{} is already a project", project.directory.display()));
        }
        self.push(project);
        Ok(())
    }

    /// The projects, in tab order: what `projects.json` keeps.
    pub fn projects(&self) -> Vec<Project> {
        self.tabs.iter().filter_map(|tab| tab.project.clone()).collect()
    }

    /// The project holding `directory` keeps `account` for its next Ctrl+N. Returns whether
    /// that changed something, so `projects.json` is written only then.
    pub fn remember_account(&mut self, directory: &Path, account: &str) -> bool {
        let projects = self.tabs.iter().enumerate().filter_map(|(index, tab)| tab.project.as_ref().map(|project| (index, project)));
        let Some(index) = projects::owner(directory, projects) else {
            return false;
        };
        let Some(project) = self.tabs[index].project.as_mut() else {
            return false;
        };
        if project.account.as_deref() == Some(account) {
            return false;
        }
        project.account = Some(account.to_string());
        true
    }

    /// The tab a session working in `directory` belongs to: its project's, else "other".
    pub fn owner_tab(&self, directory: &Path) -> PaneId {
        let projects = self.tabs.iter().filter_map(|tab| tab.project.as_ref().map(|project| (tab.id, project)));
        projects::owner(directory, projects).unwrap_or(self.tabs[0].id)
    }

    /// Closes the active project tab and gives the focus to the one before it. "other" stays:
    /// returns the panes to detach, or None.
    pub fn close_tab(&mut self) -> Option<Vec<PaneId>> {
        let index = self.index();
        self.tabs[index].project.as_ref()?;
        let closed = self.tabs.remove(index);
        self.active = self.tabs[index.saturating_sub(1)].id;
        Some(closed.layout.leaves())
    }

    /// Moves to the tab `step` places forward (or back if negative), wrapping around.
    pub fn step(&mut self, step: i64) {
        let length = self.tabs.len() as i64;
        self.active = self.tabs[(self.index() as i64 + step).rem_euclid(length) as usize].id;
    }

    /// Moves tab `from` to place `to`, both counted from 1. Only project tabs move, and only
    /// among themselves: "other" stays first. The active tab stays the same.
    pub fn move_tab(&mut self, from: usize, to: usize) -> Result<(), String> {
        let projects = 2..=self.tabs.len();
        if !projects.contains(&from) || !projects.contains(&to) {
            return Err(format!("cannot move tab {from} to {to}: other stays first"));
        }
        let tab = self.tabs.remove(from - 1);
        self.tabs.insert(to - 1, tab);
        Ok(())
    }

    /// Moves to tab `number`, counted from 1; nothing happens if there is no such tab.
    pub fn go(&mut self, number: usize) {
        if let Some(tab) = number.checked_sub(1).and_then(|index| self.tabs.get(index)) {
            self.active = tab.id;
        }
    }

    /// Like `Layout::show`, in the active tab: a session opens where it is asked for. Shown in
    /// another tab, it leaves that tab's pane, returned with the answer to be detached; a tab
    /// that waited for it waits no more.
    pub fn show(&mut self, pane: PaneId, session: SessionKey) -> Option<(Shown, Vec<PaneId>)> {
        let index = self.index();
        if !self.tabs[index].layout.leaves().contains(&pane) {
            return None;
        }
        let mut left = Vec::new();
        for (_, tab) in self.tabs.iter_mut().enumerate().filter(|(other, _)| *other != index) {
            if tab.restore.as_ref() == Some(&session) {
                tab.restore = None;
            }
            if let Some(shown_in) = tab.layout.shown_in(&session) {
                tab.layout.clear(shown_in);
                left.push(shown_in);
            }
        }
        self.tabs[index].restore = None;
        let shown = self.tabs[index].layout.show(pane, session)?;
        Some((shown, left))
    }

    /// The active tab no longer waits for its session: it is gone or belongs elsewhere now.
    pub fn drop_restore(&mut self) {
        let index = self.index();
        self.tabs[index].restore = None;
    }

    /// The active tab, and the session in the active pane of each tab, or the one it still
    /// waits for.
    pub fn saved(&self) -> Saved {
        let folder = |tab: &Tab| tab.project.as_ref().map(|project| project.directory.clone());
        let shown = self
            .tabs
            .iter()
            .filter_map(|tab| {
                let session = tab.layout.active_session().or(tab.restore.as_ref())?;
                Some(SavedTab { project: folder(tab), session: session.clone() })
            })
            .collect();
        Saved { active: folder(&self.tabs[self.index()]), shown }
    }

    /// Back to the tab and sessions of `saved`; tabs of projects since removed are ignored.
    pub fn restore(&mut self, saved: &Saved) {
        let folder = |tab: &Tab| tab.project.as_ref().map(|project| project.directory.clone());
        if let Some(tab) = self.tabs.iter().find(|tab| folder(tab) == saved.active) {
            self.active = tab.id;
        }
        for kept in &saved.shown {
            if let Some(tab) = self.tabs.iter_mut().find(|tab| folder(tab) == kept.project) {
                tab.restore = Some(kept.session.clone());
            }
        }
    }

    /// Every pane showing `session`, in any tab, shows nothing now: returns them.
    pub fn clear_session(&mut self, session: &SessionKey) -> Vec<PaneId> {
        let mut cleared = Vec::new();
        for tab in &mut self.tabs {
            if let Some(pane) = tab.layout.shown_in(session) {
                tab.layout.clear(pane);
                cleared.push(pane);
            }
        }
        cleared
    }

    /// The pane, in whichever tab, no longer shows a session.
    pub fn clear(&mut self, pane: PaneId) {
        for tab in &mut self.tabs {
            tab.layout.clear(pane);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(short_id: &str) -> SessionKey {
        SessionKey { account: "lystriq".into(), short_id: short_id.into() }
    }

    /// A project tab for a new folder.
    fn add(tabs: &mut Tabs) {
        let folder = format!("/home/u/project{}", tabs.tabs.len());
        tabs.add_project(Project::new(folder.into())).unwrap();
    }

    fn active_tab(tabs: &Tabs) -> &Tab {
        &tabs.tabs[tabs.index()]
    }

    fn panes(tabs: &Tabs) -> Vec<PaneId> {
        tabs.tabs.iter().flat_map(|tab| tab.layout.leaves()).collect()
    }

    #[test]
    fn a_new_tab_comes_last_with_one_empty_active_pane() {
        let mut tabs = Tabs::default();
        add(&mut tabs);
        assert_eq!(tabs.tabs.len(), 2);
        assert_eq!(tabs.index(), 1);
        let layout = &active_tab(&tabs).layout;
        assert_eq!(layout.leaves().len(), 1);
        assert_eq!(layout.leaves(), [layout.active()]);
        assert_eq!(layout.shown_in(&key("aaa")), None);
    }

    #[test]
    fn pane_ids_are_unique_across_tabs() {
        let mut tabs = Tabs::default();
        tabs.split(1000.0, 500.0, Side::After);
        add(&mut tabs);
        tabs.split(1000.0, 500.0, Side::After);
        add(&mut tabs);
        let mut all = panes(&tabs);
        assert_eq!(all.len(), 5);
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn the_other_tab_cannot_be_closed() {
        let mut tabs = Tabs::default();
        assert_eq!(tabs.close_tab(), None);
        add(&mut tabs);
        tabs.go(1);
        assert_eq!(tabs.close_tab(), None);
        assert_eq!(tabs.tabs.len(), 2);
    }

    #[test]
    fn closing_a_tab_returns_its_panes_and_focuses_the_previous_tab() {
        let mut tabs = Tabs::default();
        add(&mut tabs);
        tabs.split(1000.0, 500.0, Side::After);
        let middle = panes(&tabs)[1..].to_vec();
        add(&mut tabs);
        tabs.go(2);
        assert_eq!(tabs.close_tab(), Some(middle));
        assert_eq!(tabs.tabs.len(), 2);
        assert_eq!(tabs.index(), 0);
    }

    #[test]
    fn closing_a_project_tab_forgets_the_project() {
        let mut tabs = Tabs::default();
        add(&mut tabs);
        add(&mut tabs);
        tabs.go(2);
        assert!(tabs.close_tab().is_some());
        assert_eq!(tabs.projects(), [Project::new("/home/u/project2".into())]);
    }

    #[test]
    fn projects_come_back_after_other_in_order_with_other_active() {
        let projects = vec![Project::new("/home/u/blog".into()), Project::new("/home/u/api".into())];
        let tabs = Tabs::with_projects(projects.clone());
        assert_eq!(tabs.tabs[0].project, None);
        assert_eq!(tabs.projects(), projects);
        assert_eq!(tabs.index(), 0);
    }

    #[test]
    fn a_folder_is_one_project_only() {
        let mut tabs = Tabs::default();
        tabs.add_project(Project::new("/home/u/api".into())).unwrap();
        assert!(tabs.add_project(Project::new("/home/u/api".into())).is_err());
        assert_eq!(tabs.projects().len(), 1);
    }

    #[test]
    fn a_session_belongs_to_its_project_tab_else_to_other() {
        let mut tabs = Tabs::default();
        let all = tabs.active;
        tabs.add_project(Project::new("/home/u/api".into())).unwrap();
        let api = tabs.active;
        assert_eq!(tabs.owner_tab(Path::new("/home/u/api/src")), api);
        assert_eq!(tabs.owner_tab(Path::new("/home/u")), all);
    }

    #[test]
    fn steps_wrap_around() {
        let mut tabs = Tabs::default();
        add(&mut tabs);
        add(&mut tabs);
        assert_eq!(tabs.index(), 2);
        tabs.step(1);
        assert_eq!(tabs.index(), 0);
        tabs.step(-1);
        assert_eq!(tabs.index(), 2);
        tabs.step(-1);
        assert_eq!(tabs.index(), 1);
    }

    fn folders(tabs: &Tabs) -> Vec<String> {
        tabs.projects().iter().map(|project| project.directory.display().to_string()).collect()
    }

    #[test]
    fn a_project_tab_moves_forward_or_back_and_the_projects_follow() {
        let mut tabs = Tabs::default();
        add(&mut tabs);
        add(&mut tabs);
        add(&mut tabs);
        tabs.move_tab(2, 4).unwrap();
        assert_eq!(folders(&tabs), ["/home/u/project2", "/home/u/project3", "/home/u/project1"]);
        tabs.move_tab(4, 2).unwrap();
        assert_eq!(folders(&tabs), ["/home/u/project1", "/home/u/project2", "/home/u/project3"]);
    }

    #[test]
    fn moving_tabs_keeps_the_active_one() {
        let mut tabs = Tabs::default();
        add(&mut tabs);
        add(&mut tabs);
        let active = tabs.active;
        tabs.move_tab(3, 2).unwrap();
        assert_eq!(tabs.active, active);
        assert_eq!(tabs.index(), 1);
    }

    #[test]
    fn other_stays_first_and_missing_tabs_do_not_move() {
        let mut tabs = Tabs::default();
        add(&mut tabs);
        add(&mut tabs);
        let before = folders(&tabs);
        assert!(tabs.move_tab(1, 2).is_err());
        assert!(tabs.move_tab(2, 1).is_err());
        assert!(tabs.move_tab(2, 4).is_err());
        assert!(tabs.move_tab(0, 2).is_err());
        assert_eq!(folders(&tabs), before);
        assert_eq!(tabs.tabs[0].project, None);
    }

    #[test]
    fn go_ignores_missing_numbers() {
        let mut tabs = Tabs::default();
        add(&mut tabs);
        tabs.go(1);
        assert_eq!(tabs.index(), 0);
        tabs.go(0);
        tabs.go(3);
        assert_eq!(tabs.index(), 0);
        tabs.go(2);
        assert_eq!(tabs.index(), 1);
    }

    #[test]
    fn a_session_shown_in_another_tab_moves_to_the_pane_asked_for() {
        let mut tabs = Tabs::default();
        let first = active_tab(&tabs).layout.active();
        tabs.show(first, key("aaa"));
        add(&mut tabs);
        let project_tab = tabs.active;
        let pane = active_tab(&tabs).layout.active();
        assert_eq!(tabs.show(pane, key("aaa")), Some((Shown::Attach, vec![first])));
        assert_eq!(tabs.active, project_tab);
        assert_eq!(tabs.tabs[0].layout.shown_in(&key("aaa")), None);
        assert_eq!(active_tab(&tabs).layout.shown_in(&key("aaa")), Some(pane));
    }

    #[test]
    fn a_session_shown_in_the_same_tab_gets_the_focus_and_leaves_nothing() {
        let mut tabs = Tabs::default();
        tabs.split(1000.0, 500.0, Side::After);
        let shown = active_tab(&tabs).layout.leaves()[0];
        tabs.show(shown, key("aaa"));
        tabs.current_mut().focus_step(1);
        let pane = active_tab(&tabs).layout.active();
        assert_eq!(tabs.show(pane, key("aaa")), Some((Shown::Already(shown), vec![])));
        assert_eq!(active_tab(&tabs).layout.active(), shown);
    }

    #[test]
    fn a_pane_of_another_tab_is_refused_and_nothing_moves() {
        let mut tabs = Tabs::default();
        let first = active_tab(&tabs).layout.active();
        tabs.show(first, key("aaa"));
        add(&mut tabs);
        assert_eq!(tabs.show(first, key("aaa")), None);
        assert_eq!(tabs.tabs[0].layout.shown_in(&key("aaa")), Some(first));
    }

    #[test]
    fn a_tab_waiting_for_a_session_opened_elsewhere_waits_no_more() {
        let mut tabs = worked_in();
        tabs.go(1);
        let pane = tabs.current_mut().active();
        assert_eq!(tabs.show(pane, key("bbb")), Some((Shown::Attach, vec![])));
        assert_eq!(restore_of(&tabs, 3), None);
    }

    #[test]
    fn a_pane_in_a_hidden_tab_can_be_cleared() {
        let mut tabs = Tabs::default();
        let pane = active_tab(&tabs).layout.active();
        tabs.show(pane, key("aaa"));
        add(&mut tabs);
        tabs.clear(pane);
        let other = active_tab(&tabs).layout.active();
        assert_eq!(tabs.show(other, key("aaa")), Some((Shown::Attach, vec![])));
    }

    #[test]
    fn clearing_a_session_empties_its_pane_in_any_tab() {
        let mut tabs = Tabs::default();
        let pane = active_tab(&tabs).layout.active();
        tabs.show(pane, key("aaa"));
        add(&mut tabs);
        assert_eq!(tabs.clear_session(&key("aaa")), [pane]);
        assert!(tabs.clear_session(&key("aaa")).is_empty());
        let other = active_tab(&tabs).layout.active();
        assert_eq!(tabs.show(other, key("aaa")), Some((Shown::Attach, vec![])));
    }

    #[test]
    fn the_page_gets_the_tabs_and_the_active_one() {
        let mut tabs = Tabs::default();
        add(&mut tabs);
        let json = serde_json::to_value(&tabs).unwrap_or_default();
        assert_eq!(json["tabs"].as_array().map(Vec::len), Some(2));
        assert_eq!(json["active"], json["tabs"][1]["id"]);
        assert_eq!(json["tabs"][1]["layout"]["root"]["kind"], "leaf");
        assert!(json.get("ids").is_none());
    }

    #[test]
    fn the_watched_session_is_the_one_of_the_visible_tab() {
        let mut tabs = Tabs::default();
        let pane = active_tab(&tabs).layout.active();
        tabs.show(pane, key("aaa"));
        assert_eq!(tabs.watched(), Some(&key("aaa")));
        add(&mut tabs);
        assert_eq!(tabs.watched(), None);
        tabs.go(1);
        assert_eq!(tabs.watched(), Some(&key("aaa")));
    }

    /// "other", then api and blog; api shows "aaa" in its active pane, blog was never
    /// visited and waits for "bbb", and api is active.
    fn worked_in() -> Tabs {
        let mut tabs = Tabs::with_projects(vec![Project::new("/home/u/api".into()), Project::new("/home/u/blog".into())]);
        tabs.tabs[2].restore = Some(key("bbb"));
        tabs.go(2);
        let pane = tabs.current_mut().active();
        tabs.show(pane, key("aaa"));
        tabs
    }

    fn restore_of(tabs: &Tabs, number: usize) -> Option<&SessionKey> {
        tabs.tabs[number - 1].restore.as_ref()
    }

    #[test]
    fn saved_holds_the_active_tab_and_what_each_tab_shows_or_waits_for() {
        let saved = worked_in().saved();
        assert_eq!(saved.active, Some(PathBuf::from("/home/u/api")));
        assert_eq!(
            saved.shown,
            [
                SavedTab { project: Some("/home/u/api".into()), session: key("aaa") },
                SavedTab { project: Some("/home/u/blog".into()), session: key("bbb") },
            ]
        );
    }

    #[test]
    fn restoring_brings_back_the_tab_and_what_each_tab_waits_for() {
        let json = serde_json::to_string(&worked_in().saved()).unwrap();
        let mut tabs = Tabs::with_projects(vec![Project::new("/home/u/api".into()), Project::new("/home/u/blog".into())]);
        tabs.restore(&serde_json::from_str(&json).unwrap());
        assert_eq!(tabs.index(), 1);
        assert_eq!(restore_of(&tabs, 1), None);
        assert_eq!(restore_of(&tabs, 2), Some(&key("aaa")));
        assert_eq!(restore_of(&tabs, 3), Some(&key("bbb")));
        assert!(tabs.tabs.iter().all(|tab| tab.layout.active_session().is_none()), "nothing is attached before a visit");
    }

    #[test]
    fn a_removed_project_is_ignored_and_other_comes_first() {
        let saved = worked_in().saved();
        let mut tabs = Tabs::with_projects(vec![Project::new("/home/u/blog".into())]);
        tabs.restore(&saved);
        assert_eq!(tabs.index(), 0, "api is gone: other is active");
        assert_eq!(restore_of(&tabs, 2), Some(&key("bbb")));
        assert_eq!(tabs.tabs.len(), 2);
    }

    #[test]
    fn opening_a_session_in_a_tab_ends_its_wait() {
        let mut tabs = worked_in();
        tabs.go(3);
        let pane = tabs.current_mut().active();
        tabs.show(pane, key("ccc"));
        assert_eq!(restore_of(&tabs, 3), None);
        tabs.tabs[1].restore = Some(key("zzz"));
        tabs.go(1);
        let pane = tabs.current_mut().active();
        tabs.show(pane, key("aaa"));
        assert_eq!(tabs.index(), 0, "aaa moved to other");
        assert_eq!(restore_of(&tabs, 2), Some(&key("zzz")), "api was not visited");
        tabs.tabs[0].restore = Some(key("zzz"));
        tabs.go(1);
        tabs.drop_restore();
        assert_eq!(restore_of(&tabs, 1), None);
    }

    #[test]
    fn the_deepest_project_remembers_the_account_once() {
        let mut tabs = Tabs::with_projects(vec![Project::new("/home/u/Dev".into()), Project::new("/home/u/Dev/api".into())]);
        assert!(tabs.remember_account(Path::new("/home/u/Dev/api/src"), "pro"));
        assert!(!tabs.remember_account(Path::new("/home/u/Dev/api"), "pro"), "the same account changes nothing");
        assert!(!tabs.remember_account(Path::new("/tmp/elsewhere"), "louis"), "no project, nothing kept");
        let accounts: Vec<_> = tabs.projects().into_iter().map(|project| project.account).collect();
        assert_eq!(accounts, [None, Some("pro".to_string())]);
        assert!(tabs.remember_account(Path::new("/home/u/Dev/api"), "louis"));
        assert_eq!(tabs.projects()[1].account.as_deref(), Some("louis"));
    }
}
