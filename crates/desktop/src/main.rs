#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod claim;
mod layout;
mod messages;
mod pane;
mod prefs;
mod projects;
mod settings;
mod signal;
mod store;
mod tabs;
mod trace;
mod view;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use kindric_core::attach::{AttachCommand, attach_command, login_command, stop_command, trust_command};
use kindric_core::launcher::{self, Request, resolve_directory};
use kindric_core::mailbox::Mailbox;
use kindric_core::accounts::SessionInfo;
use kindric_core::artifacts::{self, Kind};
use kindric_core::{accounts, changes, checks, images, mailbox, refresher, worktrees};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tauri::ipc::{Channel, InvokeResponseBody, Response};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};

use crate::claim::Claim;
use crate::layout::{PaneId, SessionKey, Shown};
use crate::messages::MessageRow;
use crate::pane::Pane;
use crate::prefs::Prefs;
use crate::projects::Project;
use crate::settings::Settings;
use crate::tabs::{Saved, Tabs};
use crate::trace::Trace;
use crate::view::{ListView, SessionRow};

/// The tabs, their panes and the sessions attached in them. `generation` changes with every attach, so
/// the end of a session that was since replaced in its pane is ignored. The claim keeps other kindric
/// windows from showing the same session.
#[derive(Default)]
struct Workspace {
    tabs: Tabs,
    generation: u64,
    attached: HashMap<PaneId, (u64, Pane, Claim)>,
}

struct Shared {
    view: Mutex<ListView>,
    /// Problems found at startup, shown above every list since a refresh cannot fix them.
    startup_errors: Vec<String>,
    workspace: Mutex<Workspace>,
    /// Where the projects are kept, or why they cannot be: a file that did not load is
    /// never written over.
    projects_file: Result<PathBuf, String>,
    /// The short names given to sessions, kept like the projects.
    prefs: Mutex<Prefs>,
    prefs_file: Result<PathBuf, String>,
    /// The active tab and each tab's session, kept for the next launch like the projects;
    /// `saved` is what the file holds now.
    workspace_file: Result<PathBuf, String>,
    saved: Mutex<Saved>,
    /// The theme chosen, kept like the projects.
    settings: Mutex<Settings>,
    settings_file: Result<PathBuf, String>,
    /// Off unless `KINDRIC_TRACE` is set.
    trace: Trace,
    helper: Mutex<Helper>,
    /// Where the locks of the sessions shown here are taken, or why they cannot be.
    claims_dir: Result<PathBuf, String>,
}

/// The Claude Code shown in a terminal over a dialog: asking whether a folder is trusted, or
/// signing an account in. One at a time. `generation` tells its end apart from that of one closed
/// before.
#[derive(Default)]
struct Helper {
    generation: u64,
    running: Option<Pane>,
}

impl Shared {
    fn workspace(&self) -> MutexGuard<'_, Workspace> {
        self.workspace.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn helper(&self) -> MutexGuard<'_, Helper> {
        self.helper.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn prefs(&self) -> Prefs {
        self.prefs.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

#[tauri::command]
fn current_list(shared: State<'_, Shared>) -> ListView {
    let _span = shared.trace.span("current_list");
    shared.view.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

#[tauri::command(async)]
fn filter_sessions(shared: State<'_, Shared>, tab: PaneId, query: String) -> Vec<SessionRow> {
    let _span = shared.trace.span("filter_sessions");
    view::filter(&shared.view.lock().unwrap_or_else(PoisonError::into_inner), tab, &query)
}

#[derive(Serialize)]
struct Created {
    account: String,
    short_id: String,
    warnings: Vec<String>,
    /// Where it runs, so its project remembers the account.
    #[serde(skip)]
    directory: PathBuf,
}

fn start_session(account: String, directory: &str, name: &str) -> Result<Created, String> {
    let home = std::env::home_dir().ok_or("HOME is not set")?;
    let directory = resolve_directory(directory, &home)?;
    let request = Request { account, directory, name: name.trim().to_string() };
    // The state of the folder before the session, to tell later what the session changed.
    let recorded = changes::record(&request.directory);
    let launched = launcher::launch(&request)?;
    let mut warnings = launched.warnings;
    if let Err(error) = keep_recorded(recorded, &request.account, &launched.short_id) {
        warnings.push(error);
    }
    Ok(Created { account: request.account, short_id: launched.short_id, warnings, directory: request.directory })
}

/// Writes down the state taken before a session's launch; a folder outside git has none.
fn keep_recorded(recorded: Result<Option<changes::Recorded>, String>, account: &str, short_id: &str) -> Result<(), String> {
    let Some(recorded) = recorded? else {
        return Ok(());
    };
    let data_dir = mailbox::data_dir().map_err(|error| error.to_string())?;
    changes::save_recorded(&data_dir, account, short_id, &recorded)
}

/// Why a session did not start, and whether Claude Code can be asked to trust its folder.
#[derive(Serialize)]
struct StartError {
    message: String,
    trust: bool,
}

impl From<String> for StartError {
    fn from(message: String) -> Self {
        let trust = launcher::needs_trust(&message);
        Self { message, trust }
    }
}

/// Starting a session takes a few seconds: it runs off the window's thread.
#[tauri::command]
async fn new_session(app: AppHandle, shared: State<'_, Shared>, account: String, directory: String, name: String) -> Result<Created, StartError> {
    let _span = shared.trace.span("new_session");
    let mut created = tauri::async_runtime::spawn_blocking(move || start_session(account, &directory, &name))
        .await
        .map_err(|error| error.to_string())??;
    if let Err(error) = remember_account(&app, &shared, &created) {
        created.warnings.push(error);
    }
    Ok(created)
}

/// The list of the welcome screen, for the accounts and sessions shown now. It runs two
/// programs, so off the window's thread.
#[tauri::command]
async fn welcome_checks(shared: State<'_, Shared>) -> Result<Vec<checks::Line>, String> {
    let _span = shared.trace.span("welcome_checks");
    let (accounts, sessions) = shown_now(&shared);
    tauri::async_runtime::spawn_blocking(move || checks::run(accounts, sessions)).await.map_err(|error| error.to_string())?
}

/// The "add" button of the welcome list: adds the messages server where it is missing, then
/// returns the list checked again.
#[tauri::command]
async fn add_messages_server(shared: State<'_, Shared>) -> Result<Vec<checks::Line>, String> {
    let _span = shared.trace.span("add_messages_server");
    let (accounts, sessions) = shown_now(&shared);
    tauri::async_runtime::spawn_blocking(move || {
        checks::add_messages_server(&accounts)?;
        checks::run(accounts, sessions)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// How much of its limits each of `accounts` used, for the account choice of a new session. Reads
/// a small file per account.
#[tauri::command]
fn account_usage(accounts: Vec<String>) -> Result<Vec<accounts::AccountUsage>, String> {
    let paths = accounts::paths().map_err(|error| error.to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .map_err(|error| error.to_string())?;
    Ok(accounts::usages(&paths, &accounts, now))
}

/// The accounts of the settings: login state, plan, sessions, isolation. Asks Claude Code about
/// each, so off the window's thread.
#[tauri::command]
async fn account_statuses(shared: State<'_, Shared>) -> Result<Vec<accounts::AccountStatus>, String> {
    let _span = shared.trace.span("account_statuses");
    tauri::async_runtime::spawn_blocking(|| accounts::paths().and_then(|paths| accounts::statuses(&paths)))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("cannot read the accounts: {error}"))
}

/// Creates an account of kindric-accounts; returns its warnings, if any.
#[tauri::command]
fn add_account(name: String) -> Result<Vec<String>, String> {
    let paths = accounts::paths().map_err(|error| error.to_string())?;
    accounts::add_account(&paths, name.trim()).map_err(|error| error.to_string())
}

/// Takes an account off the list of kindric-accounts; its directory stays.
#[tauri::command]
fn forget_account(name: String) -> Result<(), String> {
    let paths = accounts::paths().map_err(|error| error.to_string())?;
    accounts::forget_account(&paths, &name).map_err(|error| error.to_string())
}

/// The memory warning of the settings, in KB.
#[tauri::command]
fn memory_warning() -> Result<u64, String> {
    let paths = accounts::paths().map_err(|error| error.to_string())?;
    Ok(accounts::memory_warning_kb(&paths))
}

#[tauri::command]
fn set_memory_warning(kb: u64) -> Result<(), String> {
    let paths = accounts::paths().map_err(|error| error.to_string())?;
    accounts::set_memory_warning_kb(&paths, kb).map_err(|error| error.to_string())
}

/// The accounts listed and the number of sessions, which the welcome list depends on.
fn shown_now(shared: &Shared) -> (Vec<String>, usize) {
    let view = shared.view.lock().unwrap_or_else(PoisonError::into_inner);
    (view.accounts.clone(), view.sessions.len())
}

/// The project the new session runs in offers its account first at the next Ctrl+N.
fn remember_account(app: &AppHandle, shared: &Shared, created: &Created) -> Result<(), String> {
    let mut workspace = shared.workspace();
    let mut next = workspace.tabs.clone();
    if !next.remember_account(&created.directory, &created.account) {
        return Ok(());
    }
    save_projects(shared, &next)?;
    workspace.tabs = next;
    let tabs = workspace.tabs.clone();
    drop(workspace);
    let _ = app.emit("tabs", &tabs);
    Ok(())
}

/// The memory line of a project tab's tooltip, read at each hover: a zone declared since shows at
/// once.
#[tauri::command(async)]
fn memory_zone(shared: State<'_, Shared>, directory: String) -> String {
    let _span = shared.trace.span("memory_zone");
    projects::memory_label(launcher::memory_zone(std::path::Path::new(&directory)))
}

/// The folders a directory field offers before anything is typed.
#[tauri::command(async)]
fn known_directories(shared: State<'_, Shared>) -> Vec<String> {
    let _span = shared.trace.span("known_directories");
    let Some(home) = std::env::home_dir() else {
        return Vec::new();
    };
    let projects = shared.workspace().tabs.projects();
    view::known_directories(&projects, &shared.view.lock().unwrap_or_else(PoisonError::into_inner), &home)
}

/// The folders that can follow what is typed in a directory field.
#[tauri::command(async)]
fn complete_directory(shared: State<'_, Shared>, text: String) -> Vec<String> {
    let _span = shared.trace.span("complete_directory");
    std::env::home_dir().map(|home| launcher::complete_directory(&text, &home)).unwrap_or_default()
}

#[derive(Clone, Serialize)]
struct Ended {
    pane: PaneId,
    generation: u64,
}

/// What `open_session` did. `generation` is None when the session was already shown: the
/// pane showing it now has the focus.
#[derive(Serialize)]
struct Opened {
    tabs: Tabs,
    generation: Option<u64>,
}

#[tauri::command]
fn tabs(shared: State<'_, Shared>) -> Tabs {
    let _span = shared.trace.span("tabs");
    shared.workspace().tabs.clone()
}

/// Takes the session out of `pane`, without waiting for its attaching process to end.
fn detach(workspace: &mut Workspace, pane: PaneId) {
    if let Some((_, attached, _)) = workspace.attached.remove(&pane) {
        pane::detach(attached);
    }
}

/// Writes `workspace.json` when the active tab or a tab's session changed. A failed write is
/// tried again at the next change.
fn remember(shared: &Shared, tabs: &Tabs) {
    let Ok(path) = &shared.workspace_file else {
        return;
    };
    let next = tabs.saved();
    let mut saved = shared.saved.lock().unwrap_or_else(PoisonError::into_inner);
    if *saved == next {
        return;
    }
    match store::save(path, &next) {
        Ok(()) => *saved = next,
        Err(error) => shared.trace.record(&format!("cannot save {}: {error}", path.display()), Duration::ZERO),
    }
}

/// Ends every command that may move the focus: the session now looked at loses its
/// "finished" mark. The two locks are never held together.
fn respond(app: &AppHandle, shared: &Shared, workspace: MutexGuard<'_, Workspace>) -> Tabs {
    let tabs = workspace.tabs.clone();
    drop(workspace);
    remember(shared, &tabs);
    if let Some(key) = tabs.watched() {
        let changed = {
            let mut view = shared.view.lock().unwrap_or_else(PoisonError::into_inner);
            view::seen(&mut view, key).then(|| view.clone())
        };
        if let Some(view) = changed {
            let _ = app.emit("sessions", &view);
        }
    }
    tabs
}

/// `pointer` is where the mouse is, from the top left corner of the active pane, if known: the new
/// pane goes under it.
#[tauri::command]
fn split_pane(app: AppHandle, shared: State<'_, Shared>, width: f64, height: f64, pointer: Option<(f64, f64)>) -> Tabs {
    let _span = shared.trace.span("split_pane");
    let mut workspace = shared.workspace();
    workspace.tabs.split(width, height, layout::side_under(width, height, pointer));
    respond(&app, &shared, workspace)
}

#[tauri::command]
fn close_pane(app: AppHandle, shared: State<'_, Shared>) -> Tabs {
    let _span = shared.trace.span("close_pane");
    let mut workspace = shared.workspace();
    if let Some(closed) = workspace.tabs.current_mut().close() {
        // Detaching only: the background session keeps running in Claude Code.
        detach(&mut workspace, closed);
    }
    respond(&app, &shared, workspace)
}

#[tauri::command]
fn focus_pane(app: AppHandle, shared: State<'_, Shared>, pane: PaneId) -> Tabs {
    let _span = shared.trace.span("focus_pane");
    let mut workspace = shared.workspace();
    workspace.tabs.current_mut().focus(pane);
    respond(&app, &shared, workspace)
}

#[tauri::command]
fn focus_step(app: AppHandle, shared: State<'_, Shared>, step: i64) -> Tabs {
    let _span = shared.trace.span("focus_step");
    let mut workspace = shared.workspace();
    workspace.tabs.current_mut().focus_step(step);
    respond(&app, &shared, workspace)
}

#[tauri::command]
fn resize_split(shared: State<'_, Shared>, split: PaneId, ratio: f32) -> Tabs {
    let _span = shared.trace.span("resize_split");
    let mut workspace = shared.workspace();
    workspace.tabs.current_mut().set_ratio(split, ratio);
    workspace.tabs.clone()
}

fn save_projects(shared: &Shared, tabs: &Tabs) -> Result<(), String> {
    let path = shared.projects_file.as_ref()?;
    store::save(path, &tabs.projects()).map_err(|error| format!("cannot save {}: {error}", path.display()))
}

/// The projects or the names changed: every session gets its tab and name again.
fn reassign(app: &AppHandle, shared: &Shared, tabs: &Tabs) {
    let prefs = shared.prefs();
    let view = {
        let mut view = shared.view.lock().unwrap_or_else(PoisonError::into_inner);
        view::assign(&mut view, tabs, &prefs);
        view.clone()
    };
    let _ = app.emit("sessions", &view);
}

/// Whether `account` is Claude Code's own configuration rather than an account of
/// kindric-accounts: its sessions run with `claude` alone.
fn implicit_default(account: &str) -> Result<bool, String> {
    accounts::paths().and_then(|paths| accounts::is_implicit(&paths, account)).map_err(|error| error.to_string())
}

/// The full id of the background session `short_id` of `account`.
fn session_id_of(shared: &Shared, account: &str, short_id: &str) -> Result<String, String> {
    let view = shared.view.lock().unwrap_or_else(PoisonError::into_inner);
    let row = view.sessions.iter().find(|row| row.account == account && row.short_id.as_deref() == Some(short_id));
    row.map(|row| row.info.session_id.clone()).ok_or_else(|| format!("no session {short_id}"))
}

/// Changes what is kept about the sessions, saved in `sessions.json` before it counts, then
/// gives every session its name and visibility again.
fn update_prefs(app: &AppHandle, shared: &Shared, change: impl FnOnce(&mut Prefs)) -> Result<(), String> {
    let path = shared.prefs_file.as_ref()?;
    {
        let mut prefs = shared.prefs.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = prefs.clone();
        change(&mut next);
        store::save(path, &next).map_err(|error| format!("cannot save {}: {error}", path.display()))?;
        *prefs = next;
    }
    let tabs = shared.workspace().tabs.clone();
    reassign(app, shared, &tabs);
    Ok(())
}

/// Gives the session a short name, kept in `sessions.json`; an empty one removes it.
#[tauri::command]
fn rename_session(app: AppHandle, shared: State<'_, Shared>, account: String, short_id: String, name: String) -> Result<(), String> {
    let _span = shared.trace.span("rename_session");
    let session_id = session_id_of(&shared, &account, &short_id)?;
    update_prefs(&app, &shared, |prefs| prefs::set_label(prefs, &session_id, &name))
}

/// Stops the session (its conversation is kept) and hides it from the bar; its panes empty.
#[tauri::command]
async fn hide_session(app: AppHandle, shared: State<'_, Shared>, account: String, short_id: String) -> Result<Tabs, String> {
    let _span = shared.trace.span("hide_session");
    let session_id = session_id_of(&shared, &account, &short_id)?;
    let implicit = implicit_default(&account)?;
    let (stop_account, stop_id) = (account.clone(), short_id.clone());
    let stopped =
        tauri::async_runtime::spawn_blocking(move || stop_command(&stop_account, &stop_id, implicit).output())
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| format!("cannot stop {short_id}: {error}"))?;
    if !stopped.status.success() {
        let reason = String::from_utf8_lossy(&stopped.stderr).trim().to_string();
        return Err(format!("cannot stop {short_id}: {reason}"));
    }
    update_prefs(&app, &shared, |prefs| prefs::set_hidden(prefs, &session_id, true))?;
    let mut workspace = shared.workspace();
    // Detaching: the session is stopped already.
    for pane in workspace.tabs.clear_session(&SessionKey { account, short_id }) {
        detach(&mut workspace, pane);
    }
    Ok(respond(&app, &shared, workspace))
}

/// What the session received and sent, oldest first, read or not. Nothing is marked read.
/// By full id: a session running in another terminal has messages too.
#[tauri::command(async)]
fn messages(shared: State<'_, Shared>, session_id: String) -> Result<Vec<MessageRow>, String> {
    let _span = shared.trace.span("messages");
    let known: HashMap<String, (String, String)> = {
        let view = shared.view.lock().unwrap_or_else(PoisonError::into_inner);
        let row = view.sessions.iter().find(|row| row.session_id == session_id).ok_or("this session is gone")?;
        if row.isolated {
            return Err("isolated account: no messages".into());
        }
        view.sessions.iter().map(|row| (row.session_id.clone(), (row.label.clone(), row.account.clone()))).collect()
    };
    let mailbox = Mailbox::new(mailbox::default_root().map_err(|error| error.to_string())?);
    let entries = mailbox.conversation(&session_id).map_err(|error| format!("cannot read messages: {error}"))?;
    Ok(messages::rows(entries, &session_id, |id| known.get(id).cloned()))
}

/// A hidden session comes back to the bar.
#[tauri::command]
fn show_session(app: AppHandle, shared: State<'_, Shared>, account: String, short_id: String) -> Result<(), String> {
    let _span = shared.trace.span("show_session");
    let session_id = session_id_of(&shared, &account, &short_id)?;
    update_prefs(&app, &shared, |prefs| prefs::set_hidden(prefs, &session_id, false))
}

/// A tab for the folder typed in the page, kept in `projects.json`.
#[tauri::command]
fn add_project(app: AppHandle, shared: State<'_, Shared>, directory: String) -> Result<Tabs, String> {
    let _span = shared.trace.span("add_project");
    let home = std::env::home_dir().ok_or("HOME is not set")?;
    let directory = resolve_directory(&directory, &home)?;
    let mut workspace = shared.workspace();
    let mut next = workspace.tabs.clone();
    next.add_project(Project::new(directory))?;
    save_projects(&shared, &next)?;
    workspace.tabs = next;
    let tabs = respond(&app, &shared, workspace);
    reassign(&app, &shared, &tabs);
    Ok(tabs)
}

/// Forgets the project of the active tab. Detaching only: its sessions keep running.
#[tauri::command]
fn close_tab(app: AppHandle, shared: State<'_, Shared>) -> Result<Tabs, String> {
    let _span = shared.trace.span("close_tab");
    let mut workspace = shared.workspace();
    let mut next = workspace.tabs.clone();
    let Some(panes) = next.close_tab() else {
        return Err("the other tab stays".into());
    };
    save_projects(&shared, &next)?;
    workspace.tabs = next;
    for pane in panes {
        detach(&mut workspace, pane);
    }
    let tabs = respond(&app, &shared, workspace);
    reassign(&app, &shared, &tabs);
    Ok(tabs)
}

/// A project tab dragged to another place; the new order is kept in `projects.json`.
#[tauri::command]
fn move_tab(app: AppHandle, shared: State<'_, Shared>, from: usize, to: usize) -> Result<Tabs, String> {
    let _span = shared.trace.span("move_tab");
    let mut workspace = shared.workspace();
    let mut next = workspace.tabs.clone();
    next.move_tab(from, to)?;
    save_projects(&shared, &next)?;
    workspace.tabs = next;
    Ok(respond(&app, &shared, workspace))
}

#[tauri::command]
fn tab_step(app: AppHandle, shared: State<'_, Shared>, step: i64) -> Tabs {
    let _span = shared.trace.span("tab_step");
    let mut workspace = shared.workspace();
    workspace.tabs.step(step);
    respond(&app, &shared, workspace)
}

#[tauri::command]
fn go_tab(app: AppHandle, shared: State<'_, Shared>, number: usize) -> Tabs {
    let _span = shared.trace.span("go_tab");
    let mut workspace = shared.workspace();
    workspace.tabs.go(number);
    respond(&app, &shared, workspace)
}

/// A session ended by itself: its pane is emptied, unless another session was opened there since.
fn end_session(app: &AppHandle, pane: PaneId, generation: u64) {
    let shared = app.state::<Shared>();
    let tabs = {
        let mut workspace = shared.workspace();
        if workspace.attached.get(&pane).map(|(current, ..)| *current) != Some(generation) {
            return;
        }
        workspace.attached.remove(&pane);
        workspace.tabs.clear(pane);
        workspace.tabs.clone()
    };
    remember(&shared, &tabs);
    let _ = app.emit("pane-ended", Ended { pane, generation });
    let _ = app.emit("tabs", &tabs);
}

/// Shows a session in `pane`, unless another pane of the same tab already shows it; shown in
/// another tab, it moves here. The sessions it replaces or leaves are detached, not stopped.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn open_session(
    app: AppHandle,
    shared: State<'_, Shared>,
    pane: PaneId,
    account: String,
    short_id: String,
    columns: u16,
    rows: u16,
    output: Channel<InvokeResponseBody>,
) -> Result<Opened, String> {
    let _span = shared.trace.span("open_session");
    shared.trace.record(&format!("attach {short_id} in pane {pane} at {columns}x{rows}"), Duration::ZERO);
    let implicit = implicit_default(&account)?;
    let mut workspace = shared.workspace();
    let key = SessionKey { account: account.clone(), short_id: short_id.clone() };
    let left = match workspace.tabs.show(pane, key.clone()) {
        None => return Err(format!("no pane {pane}")),
        Some((Shown::Already(_), _)) => return Ok(Opened { tabs: respond(&app, &shared, workspace), generation: None }),
        Some((Shown::Attach, left)) => left,
    };
    {
        let _span = shared.trace.span("detach previous");
        // The session shown here before, and this one where it was shown in another tab.
        for pane in left.into_iter().chain([pane]) {
            detach(&mut workspace, pane);
        }
    }
    // Taken once the panes of this window that showed the session have let it go.
    let claim = match shared.claims_dir.as_ref().map_err(Clone::clone).and_then(|directory| claim::claim(directory, &key)) {
        Ok(claim) => claim,
        Err(error) => {
            workspace.tabs.clear(pane);
            return Err(error);
        },
    };
    workspace.generation += 1;
    let generation = workspace.generation;
    let ended = app.clone();
    let traced = app.clone();
    let mut first_output = Some((Instant::now(), format!("attach {short_id} first output")));
    let opened = Pane::open(
        &attach_command(&account, &short_id, implicit),
        columns,
        rows,
        move |bytes| {
            if let Some((start, what)) = first_output.take() {
                traced.state::<Shared>().trace.record(&what, start.elapsed());
            }
            // Fails only once the page has dropped this channel for a newer session.
            let _ = output.send(InvokeResponseBody::Raw(bytes.to_vec()));
        },
        // Waits for the lock held here, so it always sees this attach recorded.
        move || end_session(&ended, pane, generation),
    );
    match opened {
        Ok(attached) => {
            workspace.attached.insert(pane, (generation, attached, claim));
            Ok(Opened { tabs: respond(&app, &shared, workspace), generation: Some(generation) })
        },
        Err(error) => {
            workspace.tabs.clear(pane);
            Err(format!("cannot open {short_id}: {error}"))
        },
    }
}

/// The session the active tab waited for is gone, or belongs to another tab now.
#[tauri::command]
fn drop_restore(app: AppHandle, shared: State<'_, Shared>) -> Tabs {
    let _span = shared.trace.span("drop_restore");
    let mut workspace = shared.workspace();
    workspace.tabs.drop_restore();
    respond(&app, &shared, workspace)
}

#[tauri::command]
fn settings(shared: State<'_, Shared>) -> Settings {
    shared.settings.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The theme chosen with Ctrl+Shift+E, saved before it counts.
#[tauri::command]
fn set_theme(shared: State<'_, Shared>, name: String) -> Result<(), String> {
    let path = shared.settings_file.as_ref()?;
    let mut settings = shared.settings.lock().unwrap_or_else(PoisonError::into_inner);
    let mut next = settings.clone();
    next.theme = Some(name);
    store::save(path, &next).map_err(|error| format!("cannot save {}: {error}", path.display()))?;
    *settings = next;
    Ok(())
}

/// A shortcut changed in the settings, saved before it counts; None gives it its own keys back.
#[tauri::command]
fn set_key(shared: State<'_, Shared>, id: String, keys: Option<String>) -> Result<(), String> {
    if !settings::is_shortcut_id(&id) {
        return Err(format!("unknown shortcut {id:?}"));
    }
    if let Some(keys) = &keys
        && !settings::is_key_name(keys)
    {
        return Err(format!("{keys:?} is not a key the app can use"));
    }
    let path = shared.settings_file.as_ref()?;
    let mut saved = shared.settings.lock().unwrap_or_else(PoisonError::into_inner);
    let mut next = saved.clone();
    match keys {
        Some(keys) => next.keys.insert(id, keys),
        None => next.keys.remove(&id),
    };
    store::save(path, &next).map_err(|error| format!("cannot save {}: {error}", path.display()))?;
    *saved = next;
    Ok(())
}

/// The colors of `theme.json`, read again each time it is chosen; None without the file.
#[tauri::command(async)]
fn custom_theme() -> Result<Option<std::collections::BTreeMap<String, String>>, String> {
    let path = mailbox::data_dir().map_err(|error| error.to_string())?.join("theme.json");
    match std::fs::read_to_string(&path) {
        Ok(json) => settings::custom_theme(&json).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

/// Whether the page measures too: it asks once, when it loads.
#[tauri::command]
fn trace_enabled(shared: State<'_, Shared>) -> bool {
    shared.trace.enabled()
}

/// A timing measured by the page.
#[tauri::command]
fn trace(shared: State<'_, Shared>, what: String, ms: f64) {
    shared.trace.record(&what, Duration::from_secs_f64(ms.max(0.0) / 1000.0));
}

#[tauri::command]
fn write_input(shared: State<'_, Shared>, pane: PaneId, data: String) -> Result<(), String> {
    let mut workspace = shared.workspace();
    let (_, attached, _) = workspace.attached.get_mut(&pane).ok_or("no session in this pane")?;
    attached.write(data.as_bytes()).map_err(|error| error.to_string())
}

#[tauri::command]
fn resize_pane(shared: State<'_, Shared>, pane: PaneId, columns: u16, rows: u16) -> Result<(), String> {
    let _span = shared.trace.span("resize_pane");
    shared.trace.record(&format!("pane {pane} resized to {columns}x{rows}"), Duration::ZERO);
    let workspace = shared.workspace();
    let (_, attached, _) = workspace.attached.get(&pane).ok_or("no session in this pane")?;
    attached.resize(columns, rows).map_err(|error| error.to_string())
}

/// Larger image files are not shown.
const LARGEST_IMAGE: u64 = 32 * 1024 * 1024;

/// The bytes of an image a session proposed; the page can read no other file.
#[tauri::command(async)]
fn image_bytes(shared: State<'_, Shared>, path: String) -> Result<Response, String> {
    let _span = shared.trace.span("image_bytes");
    let proposed = shared.view.lock().unwrap_or_else(PoisonError::into_inner).sessions.iter().any(|row| row.images.contains(&path));
    if !proposed {
        return Err(format!("{path}: no session proposes this image now"));
    }
    let size = std::fs::metadata(&path).map_err(|error| format!("cannot read {path}: {error}"))?.len();
    if size > LARGEST_IMAGE {
        return Err(format!("{path}: {} MiB, too large to show (32 MiB at most)", size / (1024 * 1024)));
    }
    std::fs::read(&path).map(Response::new).map_err(|error| format!("cannot read {path}: {error}"))
}

/// The user's artifacts of `project` (a tab's folder; None for "other").
fn artifacts_root(project: Option<&str>) -> Result<PathBuf, String> {
    let data = mailbox::data_dir().map_err(|error| error.to_string())?;
    Ok(artifacts::root_of_project(&data, project.map(Path::new)))
}

#[derive(Serialize)]
struct ArtifactFile {
    name: String,
    /// Seconds since 1970.
    modified: u64,
    image: bool,
}

/// A block of the artifacts window: the unsorted files (no name), then each group.
#[derive(Serialize)]
struct ArtifactGroup {
    name: Option<String>,
    files: Vec<ArtifactFile>,
}

/// The unsorted files first, then every group by name, empty ones included.
#[tauri::command(async)]
fn artifacts_view(shared: State<'_, Shared>, project: Option<String>) -> Result<Vec<ArtifactGroup>, String> {
    let _span = shared.trace.span("artifacts_view");
    let root = artifacts_root(project.as_deref())?;
    let listed = artifacts::entries(&root, Kind::User)?;
    let names = std::iter::once(None).chain(artifacts::groups(&root, Kind::User)?.into_iter().map(Some));
    Ok(names
        .map(|name| {
            let files = listed
                .iter()
                .filter(|file| file.group == name)
                .map(|file| ArtifactFile { name: file.name.clone(), modified: file.modified, image: images::is_image_name(&file.name) })
                .collect();
            ArtifactGroup { name, files }
        })
        .collect())
}

#[tauri::command(async)]
fn move_artifact(project: Option<String>, group: Option<String>, name: String, to: Option<String>) -> Result<(), String> {
    artifacts::move_to(&artifacts_root(project.as_deref())?, Kind::User, group.as_deref(), &name, to.as_deref()).map(drop)
}

#[tauri::command(async)]
fn create_artifact_group(project: Option<String>, group: String) -> Result<(), String> {
    artifacts::create_group(&artifacts_root(project.as_deref())?, Kind::User, group.trim())
}

#[tauri::command(async)]
fn rename_artifact_group(project: Option<String>, from: String, to: String) -> Result<(), String> {
    artifacts::rename_group(&artifacts_root(project.as_deref())?, Kind::User, &from, to.trim())
}

/// Its files go back to the unsorted ones.
#[tauri::command(async)]
fn remove_artifact_group(project: Option<String>, group: String) -> Result<(), String> {
    artifacts::remove_group(&artifacts_root(project.as_deref())?, Kind::User, &group)
}

#[tauri::command(async)]
fn delete_artifact(project: Option<String>, group: Option<String>, name: String) -> Result<(), String> {
    artifacts::delete(&artifacts_root(project.as_deref())?, Kind::User, group.as_deref(), &name)
}

/// The bytes of a kept image, for its thumbnail and its full view.
#[tauri::command(async)]
fn artifact_bytes(project: Option<String>, group: Option<String>, name: String) -> Result<Response, String> {
    let path = artifacts::file_path(&artifacts_root(project.as_deref())?, Kind::User, group.as_deref(), &name)?;
    let size = std::fs::metadata(&path).map_err(|error| format!("cannot read {name}: {error}"))?.len();
    if size > LARGEST_IMAGE {
        return Err(format!("{name}: {} MiB, too large to show (32 MiB at most)", size / (1024 * 1024)));
    }
    std::fs::read(&path).map(Response::new).map_err(|error| format!("cannot read {name}: {error}"))
}

/// The program that opens a file with its usual app, on each system.
fn system_opener(path: &Path) -> std::process::Command {
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]).arg(path);
        command
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = std::process::Command::new("open");
        command.arg(path);
        command
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(path);
        command
    }
}

/// Opens a kept file with the app the system gives it, without waiting for it.
#[tauri::command(async)]
fn open_artifact(project: Option<String>, group: Option<String>, name: String) -> Result<(), String> {
    let path = artifacts::file_path(&artifacts_root(project.as_deref())?, Kind::User, group.as_deref(), &name)?;
    let mut command = system_opener(&path);
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let mut child = command.spawn().map_err(|error| format!("cannot open {name}: {error}"))?;
    // Reaped in the background: the opener ends once it has handed the file over.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// The images the last turn of `session` proposed; none when the files cannot be found.
fn proposed_images(session: &SessionInfo, implicit_default: bool) -> Vec<String> {
    let Ok(paths) = accounts::paths() else {
        return Vec::new();
    };
    let account_dir = accounts::dir_of(&paths, &session.account, implicit_default);
    images::proposed(&account_dir, &session.session_id, Path::new(&session.cwd), std::env::home_dir().as_deref())
}

fn publish(app: &AppHandle, update: refresher::Update) -> bool {
    let shared = app.state::<Shared>();
    let start = Instant::now();
    let implicit_default = update.as_ref().is_ok_and(|(snapshot, _)| snapshot.implicit_default);
    if let Ok((snapshot, _)) = &update {
        for (account, took) in &snapshot.timings {
            shared.trace.record(&format!("agents {account}"), *took);
        }
    }
    let tabs = shared.workspace().tabs.clone();
    let prefs = shared.prefs();
    let (view, to_count) = {
        let mut current = shared.view.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = view::apply(&current, update, tabs.watched());
        view::propose_images(&mut next, &current, |session| proposed_images(session, implicit_default));
        let to_count = view::to_count(&next, &current);
        view::carry_changed(&mut next, &current, &HashMap::new());
        view::assign(&mut next, &tabs, &prefs);
        next.errors.splice(0..0, shared.startup_errors.iter().cloned());
        // The same list again (the 30-second refresh, mostly): the page has nothing to draw.
        if next == *current {
            (None, to_count)
        } else {
            *current = next.clone();
            (Some(next), to_count)
        }
    };
    let emitted = match view {
        Some(view) => {
            let emitted = app.emit("sessions", &view).is_ok();
            shared.trace.record(&format!("list published ({} sessions)", view.sessions.len()), start.elapsed());
            emitted
        },
        None => {
            shared.trace.record("list unchanged", start.elapsed());
            true
        },
    };
    count_changes(app, &to_count, implicit_default);
    emitted
}

/// Counts the files the sessions of `to_count` changed, outside the lock of the list (git takes
/// tens to hundreds of milliseconds a session), then shows the counts.
fn count_changes(app: &AppHandle, to_count: &[SessionInfo], implicit_default: bool) {
    if to_count.is_empty() {
        return;
    }
    let shared = app.state::<Shared>();
    let start = Instant::now();
    let counted: HashMap<String, Option<usize>> =
        to_count.iter().map(|session| (session.session_id.clone(), changed_count(session, implicit_default))).collect();
    shared.trace.record(&format!("changes counted ({} sessions)", to_count.len()), start.elapsed());
    let view = {
        let mut current = shared.view.lock().unwrap_or_else(PoisonError::into_inner);
        view::set_changed(&mut current, &counted).then(|| current.clone())
    };
    if let Some(view) = view {
        let _ = app.emit("sessions", &view);
    }
}

/// How many files `session` changed in its repositories; `None` when it cannot be told (no
/// conversation, git missing).
fn changed_count(session: &SessionInfo, implicit_default: bool) -> Option<usize> {
    blocks_of(session, implicit_default).ok().map(|blocks| changes::count(&blocks))
}

/// What `session` changed, one block per working copy, compared with the state kindric wrote
/// down when it launched it, if it did.
fn blocks_of(session: &SessionInfo, implicit_default: bool) -> Result<Vec<changes::Block>, String> {
    let paths = accounts::paths().map_err(|error| error.to_string())?;
    let account_dir = accounts::dir_of(&paths, &session.account, implicit_default);
    let recorded = session.short_id.as_deref().and_then(|short_id| {
        let data_dir = mailbox::data_dir().ok()?;
        changes::load_recorded(&data_dir, &account_dir, &session.account, short_id, &session.session_id)
    });
    changes::session_changes(&account_dir, &session.session_id, recorded.as_ref())
}

/// The files the background session `short_id` of `account` changed, for the window of
/// Ctrl+Shift+G.
#[tauri::command]
async fn session_changes(shared: State<'_, Shared>, account: String, short_id: String) -> Result<Vec<changes::Block>, String> {
    let _span = shared.trace.span("session_changes");
    let session = {
        let view = shared.view.lock().unwrap_or_else(PoisonError::into_inner);
        let row = view.sessions.iter().find(|row| row.account == account && row.short_id.as_deref() == Some(short_id.as_str()));
        row.map(|row| row.info.clone()).ok_or_else(|| format!("no session {short_id}"))?
    };
    let implicit = implicit_default(&account)?;
    tauri::async_runtime::spawn_blocking(move || blocks_of(&session, implicit))
        .await
        .map_err(|error| error.to_string())?
}

/// The diff of one file of a block of `session_changes`, against the block's `base`.
#[tauri::command]
async fn file_diff(shared: State<'_, Shared>, copy: PathBuf, base: String, path: String, from: Option<String>) -> Result<String, String> {
    let _span = shared.trace.span("file_diff");
    tauri::async_runtime::spawn_blocking(move || changes::file_diff(&copy, &base, &path, from.as_deref()))
        .await
        .map_err(|error| error.to_string())?
}

/// The folders of the live sessions with their names, for the worktrees a session works in.
fn session_folders(shared: &Shared) -> Vec<(PathBuf, String)> {
    let view = shared.view.lock().unwrap_or_else(PoisonError::into_inner);
    view.sessions.iter().filter(|row| !row.directory.is_empty()).map(|row| (PathBuf::from(&row.directory), row.label.clone())).collect()
}

/// The worktrees of the repository of `project` (a tab's folder), main checkout first.
#[tauri::command]
async fn project_worktrees(shared: State<'_, Shared>, project: PathBuf) -> Result<Vec<worktrees::Worktree>, String> {
    let _span = shared.trace.span("project_worktrees");
    let sessions = session_folders(&shared);
    tauri::async_runtime::spawn_blocking(move || worktrees::list(&project, &sessions))
        .await
        .map_err(|error| error.to_string())?
}

/// Removes a worktree of `project` and its branch if git finds it merged; never forced.
#[tauri::command]
async fn remove_worktree(shared: State<'_, Shared>, project: PathBuf, path: PathBuf) -> Result<worktrees::BranchOutcome, String> {
    let _span = shared.trace.span("remove_worktree");
    let sessions = session_folders(&shared);
    tauri::async_runtime::spawn_blocking(move || worktrees::remove(&project, &path, &sessions))
        .await
        .map_err(|error| error.to_string())?
}

/// Runs `command` in the helper window, closing the one running before. Returns the generation
/// that `helper-ended` will carry.
fn start_helper(
    app: &AppHandle,
    shared: &Shared,
    command: &AttachCommand,
    columns: u16,
    rows: u16,
    output: Channel<InvokeResponseBody>,
) -> std::io::Result<u64> {
    let mut helper = shared.helper();
    if let Some(previous) = helper.running.take() {
        pane::detach(previous);
    }
    helper.generation += 1;
    let generation = helper.generation;
    let ended = app.clone();
    let running = Pane::open(
        command,
        columns,
        rows,
        move |bytes| {
            let _ = output.send(InvokeResponseBody::Raw(bytes.to_vec()));
        },
        // Waits for the lock held here, so it always sees this one recorded.
        move || end_helper(&ended, generation),
    )?;
    helper.running = Some(running);
    Ok(generation)
}

/// Starts Claude Code in `directory` on `account`, not in the background, so that it asks whether
/// the folder is trusted; Claude Code keeps the answer.
#[tauri::command]
fn open_trust(
    app: AppHandle,
    shared: State<'_, Shared>,
    account: String,
    directory: String,
    columns: u16,
    rows: u16,
    output: Channel<InvokeResponseBody>,
) -> Result<u64, String> {
    let home = std::env::home_dir().ok_or("HOME is not set")?;
    let directory = resolve_directory(&directory, &home)?;
    let implicit = implicit_default(&account)?;
    start_helper(&app, &shared, &trust_command(&account, &directory, implicit), columns, rows, output)
        .map_err(|error| format!("cannot start Claude Code in {}: {error}", directory.display()))
}

/// Starts the login of `account`, an account of kindric-accounts; Claude Code keeps the credentials.
#[tauri::command]
fn open_login(
    app: AppHandle,
    shared: State<'_, Shared>,
    account: String,
    columns: u16,
    rows: u16,
    output: Channel<InvokeResponseBody>,
) -> Result<u64, String> {
    start_helper(&app, &shared, &login_command(&account), columns, rows, output)
        .map_err(|error| format!("cannot start the login of {account}: {error}"))
}

/// Claude Code ended by itself in the helper window: answered, or quit.
fn end_helper(app: &AppHandle, generation: u64) {
    let shared = app.state::<Shared>();
    {
        let mut helper = shared.helper();
        if helper.generation != generation || helper.running.is_none() {
            return;
        }
        helper.running = None;
    }
    let _ = app.emit("helper-ended", generation);
}

#[tauri::command]
fn helper_input(shared: State<'_, Shared>, data: String) -> Result<(), String> {
    let mut helper = shared.helper();
    let running = helper.running.as_mut().ok_or("Claude Code is not running in this window")?;
    running.write(data.as_bytes()).map_err(|error| error.to_string())
}

#[tauri::command]
fn helper_resize(shared: State<'_, Shared>, columns: u16, rows: u16) -> Result<(), String> {
    let helper = shared.helper();
    let running = helper.running.as_ref().ok_or("Claude Code is not running in this window")?;
    running.resize(columns, rows).map_err(|error| error.to_string())
}

/// The helper window was closed: its Claude Code ends, what it was asked already kept.
#[tauri::command]
fn close_helper(shared: State<'_, Shared>) {
    if let Some(running) = shared.helper().running.take() {
        pane::detach(running);
    }
}

/// Leaves no `claude attach`, and no Claude Code in the helper window, behind when the window closes.
fn detach_on_exit(app: &AppHandle, event: RunEvent) {
    if let RunEvent::Exit = event {
        let shared = app.state::<Shared>();
        shared.workspace().attached.clear();
        shared.helper().running = None;
    }
}

/// `name` in the data directory, read at startup, and where to save it: a file that did not
/// load is reported and never written over.
fn load_file<T: DeserializeOwned + Default>(name: &str, errors: &mut Vec<String>) -> (T, Result<PathBuf, String>) {
    let path = match mailbox::data_dir() {
        Ok(directory) => directory.join(name),
        Err(error) => {
            errors.push(format!("{name} unavailable: {error}"));
            return (T::default(), Err(format!("{name} unavailable: {error}")));
        },
    };
    match store::load(&path) {
        Ok(value) => (value, Ok(path)),
        Err(error) => {
            errors.push(format!("{name} not loaded: {error}"));
            (T::default(), Err(format!("{} did not load: fix or delete it", path.display())))
        },
    }
}

/// `trace.log` in the data directory when `KINDRIC_TRACE` is set; a trace that cannot
/// be opened is reported, and the app runs without it.
fn open_trace(errors: &mut Vec<String>) -> Trace {
    if std::env::var_os(trace::VARIABLE).is_none_or(|value| value.is_empty()) {
        return Trace::off();
    }
    match mailbox::data_dir().and_then(|directory| Trace::open(&directory.join("trace.log"))) {
        Ok(trace) => trace,
        Err(error) => {
            errors.push(format!("trace.log unavailable: {error}"));
            Trace::off()
        },
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(action) = signal::parse(&args) {
        return signal::run(action);
    }
    let mut startup_errors = Vec::new();
    let timings = open_trace(&mut startup_errors);
    startup_errors.extend(accounts::check_launcher_version().err().map(|error| error.to_string()));
    let (projects, projects_file) = load_file::<Vec<Project>>("projects.json", &mut startup_errors);
    let (prefs, prefs_file) = load_file::<Prefs>("sessions.json", &mut startup_errors);
    let (saved, workspace_file) = load_file::<Saved>("workspace.json", &mut startup_errors);
    let (chosen, settings_file) = load_file::<Settings>("settings.json", &mut startup_errors);
    let mut restored = Tabs::with_projects(projects);
    restored.restore(&saved);
    let workspace = Workspace { tabs: restored, ..Workspace::default() };
    let shared = Shared {
        view: Mutex::new(ListView::default()),
        startup_errors,
        workspace: Mutex::new(workspace),
        projects_file,
        prefs: Mutex::new(prefs),
        prefs_file,
        workspace_file,
        saved: Mutex::new(saved),
        settings: Mutex::new(chosen),
        settings_file,
        trace: timings,
        helper: Mutex::new(Helper::default()),
        claims_dir: mailbox::data_dir()
            .map(|directory| directory.join("attached"))
            .map_err(|error| format!("cannot tell whether another kindric window shows a session: {error}")),
    };
    let built = tauri::Builder::default()
        .manage(shared)
        .invoke_handler(tauri::generate_handler![
            current_list,
            filter_sessions,
            new_session,
            welcome_checks,
            add_messages_server,
            known_directories,
            complete_directory,
            memory_zone,
            tabs,
            split_pane,
            close_pane,
            focus_pane,
            focus_step,
            resize_split,
            add_project,
            rename_session,
            hide_session,
            show_session,
            messages,
            close_tab,
            move_tab,
            tab_step,
            go_tab,
            open_session,
            drop_restore,
            settings,
            set_theme,
            custom_theme,
            trace_enabled,
            trace,
            write_input,
            resize_pane,
            image_bytes,
            artifacts_view,
            move_artifact,
            create_artifact_group,
            rename_artifact_group,
            remove_artifact_group,
            delete_artifact,
            artifact_bytes,
            open_artifact,
            open_trust,
            open_login,
            account_statuses,
            account_usage,
            session_changes,
            file_diff,
            project_worktrees,
            remove_worktree,
            add_account,
            forget_account,
            memory_warning,
            set_memory_warning,
            set_key,
            helper_input,
            helper_resize,
            close_helper
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            // Stopped sessions too: Ctrl+P finds them and opening one starts it again.
            refresher::spawn(true, move |update| publish(&handle, update));
            Ok(())
        })
        .build(tauri::generate_context!());
    match built {
        Ok(app) => {
            app.run(detach_on_exit);
            ExitCode::SUCCESS
        },
        Err(error) => {
            eprintln!("kindric-desktop: {error}");
            ExitCode::FAILURE
        },
    }
}
