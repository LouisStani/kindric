#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod claim;
mod layout;
mod messages;
mod pane;
#[cfg(test)]
mod page;
mod prefs;
mod projects;
mod purge;
mod settings;
mod signal;
mod store;
mod tabs;
mod trace;
mod typing;
mod view;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use kindric_core::attach::{AttachCommand, attach_command, login_command, remove_command, stop_command, trust_command};
use kindric_core::launcher::{self, Request, resolve_directory};
use kindric_core::mailbox::Mailbox;
use kindric_core::accounts::SessionInfo;
use kindric_core::artifacts::{self, Kind};
use kindric_core::{accounts, activity, atlas, away, changes, checks, claude_theme, images, mailbox, origins, overview, refresher, steps, worktrees};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tauri::ipc::{Channel, InvokeResponseBody, Response};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};
use tauri_plugin_dialog::DialogExt;

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
    /// What `claude --version` and `--help` said, kept while the same claude is found.
    claude_facts: Mutex<Option<checks::ClaudeFacts>>,
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

fn start_session(account: String, directory: &str, name: &str, projects: &[Project]) -> Result<Created, String> {
    let home = std::env::home_dir().ok_or("HOME is not set")?;
    let directory = resolve_directory(directory, &home)?;
    let request = Request { account, directory, name: name.trim().to_string() };
    // The state of the folder before the session, to tell later what the session changed.
    let recorded = changes::record(&request.directory);
    // Claude Code draws with the theme kindric keeps in step with the colours of the session's
    // project, else with its own (claude_theme).
    let theme = projects::claude_theme_of(&request.directory, projects);
    let launched = launcher::launch(&request, Some(&theme))?;
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
    let projects = shared.workspace().tabs.projects();
    let mut created = tauri::async_runtime::spawn_blocking(move || start_session(account, &directory, &name, &projects))
        .await
        .map_err(|error| error.to_string())??;
    if let Err(error) = remember_account(&app, &shared, &created) {
        created.warnings.push(error);
    }
    Ok(created)
}

/// The list of the welcome screen for the accounts and sessions shown now, after `first` (which
/// may change what it finds). It runs programs, so off the window's thread; claude only when it
/// is not the one already asked.
async fn check_setup(
    shared: &Shared,
    first: impl FnOnce(&[String]) -> Result<(), String> + Send + 'static,
) -> Result<Vec<checks::Line>, String> {
    let (accounts, sessions) = shown_now(shared);
    let previous = shared.claude_facts.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let (lines, facts) = tauri::async_runtime::spawn_blocking(move || {
        first(&accounts)?;
        let facts = checks::ClaudeFacts::current(previous.as_ref());
        Ok::<_, String>((checks::run(accounts, sessions, &facts)?, facts))
    })
    .await
    .map_err(|error| error.to_string())??;
    *shared.claude_facts.lock().unwrap_or_else(PoisonError::into_inner) = Some(facts);
    Ok(lines)
}

#[tauri::command]
async fn welcome_checks(shared: State<'_, Shared>) -> Result<Vec<checks::Line>, String> {
    let _span = shared.trace.span("welcome_checks");
    check_setup(&shared, |_| Ok(())).await
}

/// The "add" button of the welcome list: adds the messages server where it is missing, then
/// returns the list checked again.
#[tauri::command]
async fn add_messages_server(shared: State<'_, Shared>) -> Result<Vec<checks::Line>, String> {
    let _span = shared.trace.span("add_messages_server");
    check_setup(&shared, checks::add_messages_server).await
}

/// How much of its limits each of `accounts` used, for the account choice of a new session. Reads
/// a small file per account.
#[tauri::command]
fn account_usage(accounts: Vec<String>) -> Result<Vec<accounts::AccountUsage>, String> {
    let paths = accounts::paths().map_err(|error| error.to_string())?;
    Ok(accounts::usages(&paths, &accounts, unix_now()?))
}

fn unix_now() -> Result<i64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .map_err(|error| error.to_string())
}

/// The figures of the overview that no refresh brings: the limits of every account, the context
/// of each session, and how many files each session kept for the user.
#[derive(Serialize)]
struct ControlFigures {
    usages: Vec<accounts::AccountUsage>,
    /// Session id → percentage of its context window used.
    contexts: std::collections::BTreeMap<String, f64>,
    /// Session id → files it kept for the user, in every project.
    kept: std::collections::BTreeMap<String, usize>,
}

/// What the control center shows of the session picked: its last steps, when its transcript was
/// last written, and the files it kept for the user.
#[derive(Serialize)]
struct SessionDetail {
    steps: Vec<steps::Step>,
    /// Unix seconds; None when the transcript cannot be found.
    written_at: Option<u64>,
    kept: Vec<KeptByDetail>,
}

#[derive(Serialize)]
struct KeptByDetail {
    #[serde(flatten)]
    file: origins::KeptFile,
    image: bool,
}

/// How many steps the detail shows.
const DETAIL_STEPS: usize = 8;

/// Reads the end of the session's transcript and the origins of every project: off the window's
/// thread.
#[tauri::command(async)]
fn session_detail(shared: State<'_, Shared>, account: String, session_id: String) -> Result<SessionDetail, String> {
    let _span = shared.trace.span("session_detail");
    let paths = accounts::paths().map_err(|error| error.to_string())?;
    let account_dir = accounts::dir_of(&paths, &account, implicit_default(&account)?);
    let (steps, written_at) = steps::of_session(&account_dir, &session_id, DETAIL_STEPS);
    let data = mailbox::data_dir().map_err(|error| error.to_string())?;
    let projects = artifacts::read_projects(&data)?;
    let roots: Vec<(Option<String>, PathBuf)> = projects
        .iter()
        .map(|project| Some(project.as_path()))
        .chain(std::iter::once(None))
        .map(|project| (project.map(|project| project.to_string_lossy().into_owned()), artifacts::root_of_project(&data, project)))
        .collect();
    let kept = origins::kept_by(&roots, &session_id)
        .into_iter()
        .map(|file| KeptByDetail { image: images::is_image_name(&file.name), file })
        .collect();
    Ok(SessionDetail { steps, written_at, kept })
}

/// Reads a few small files per account and per project, so off the window's thread.
#[tauri::command(async)]
fn control_figures(shared: State<'_, Shared>, accounts: Vec<String>, sessions: Vec<accounts::SessionRef>) -> Result<ControlFigures, String> {
    let _span = shared.trace.span("control_figures");
    let paths = accounts::paths().map_err(|error| error.to_string())?;
    let data = mailbox::data_dir().map_err(|error| error.to_string())?;
    let projects = artifacts::read_projects(&data)?;
    let roots: Vec<PathBuf> = projects
        .iter()
        .map(|project| Some(project.as_path()))
        .chain(std::iter::once(None))
        .map(|project| artifacts::root_of_project(&data, project))
        .collect();
    Ok(ControlFigures {
        usages: accounts::usages(&paths, &accounts, unix_now()?),
        contexts: accounts::contexts(&paths, &sessions),
        kept: origins::kept_for_user(&roots),
    })
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

/// The memory grouped for the atlas (Settings › memory), read again at each opening.
#[tauri::command(async)]
fn memory_atlas(shared: State<'_, Shared>) -> Result<atlas::Atlas, String> {
    let _span = shared.trace.span("memory_atlas");
    Ok(atlas::read(&atlas::default_root()?))
}

/// The text of one note of the memory, for the atlas; nothing outside the memory.
#[tauri::command(async)]
fn memory_note(path: String) -> Result<String, String> {
    atlas::read_note(&atlas::default_root()?, &path)
}

/// The notes of the memory that hold every word of `query`.
#[tauri::command(async)]
fn memory_search(shared: State<'_, Shared>, query: String) -> Result<Vec<String>, String> {
    let _span = shared.trace.span("memory_search");
    Ok(atlas::search(&atlas::default_root()?, &query))
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

/// The sessions a purge of those idle for more than `days` would remove, now.
fn purge_targets(shared: &Shared, days: u32) -> Result<Vec<purge::Target>, String> {
    if days == 0 {
        return Err("a purge keeps at least the sessions of the last day".into());
    }
    let now = unix_now()?;
    let view = shared.view.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(view::purgeable(&view, now, days).into_iter().filter_map(purge::Target::of).collect())
}

/// How many sessions of each account a purge of those idle for more than `days` would remove.
#[tauri::command]
fn purge_preview(shared: State<'_, Shared>, days: u32) -> Result<Vec<purge::Count>, String> {
    let _span = shared.trace.span("purge_preview");
    Ok(purge::counts(&purge_targets(&shared, days)?))
}

#[derive(Serialize)]
struct Purged {
    report: purge::Report,
    tabs: Tabs,
}

/// Removes the sessions idle for more than `days` (decision 361) with `claude rm`, which stops one
/// still running; its conversation stays for `claude --resume`. The panes that showed one empty.
#[tauri::command]
async fn purge_sessions(app: AppHandle, shared: State<'_, Shared>, days: u32) -> Result<Purged, String> {
    let _span = shared.trace.span("purge_sessions");
    let targets = purge_targets(&shared, days)?;
    let report = tauri::async_runtime::spawn_blocking(move || {
        purge::purge(targets, |target| {
            let implicit = implicit_default(&target.account)?;
            let output = remove_command(&target.account, &target.short_id, implicit)
                .output()
                .map_err(|error| format!("cannot run claude rm: {error}"))?;
            if output.status.success() {
                return Ok(());
            }
            Err(purge::failure_words(&output.stderr, &output.stdout, output.status.code()))
        })
    })
    .await
    .map_err(|error| error.to_string())?;
    let mut workspace = shared.workspace();
    for target in &report.removed {
        let key = SessionKey { account: target.account.clone(), short_id: target.short_id.clone() };
        for pane in workspace.tabs.clear_session(&key) {
            detach(&mut workspace, pane);
        }
    }
    Ok(Purged { report, tabs: respond(&app, &shared, workspace) })
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

/// The terminals are on `background`, those of each project on its own: Claude Code's themes
/// follow, so the sessions that use them draw for that background (see `claude_theme`).
#[tauri::command]
fn follow_claude_theme(shared: State<'_, Shared>, background: String, projects: Vec<claude_theme::ProjectBackground>) -> Result<(), String> {
    let _span = shared.trace.span("follow_claude_theme");
    let paths = accounts::paths().map_err(|error| format!("Claude Code's theme not changed: {error}"))?;
    claude_theme::follow(&paths.shared_root, &background, &projects)
}

/// The project of the active tab takes the theme `theme`, or kindric's with None; kept in
/// `projects.json`.
#[tauri::command]
fn set_project_theme(app: AppHandle, shared: State<'_, Shared>, theme: Option<String>) -> Result<Tabs, String> {
    let _span = shared.trace.span("set_project_theme");
    let mut workspace = shared.workspace();
    let mut next = workspace.tabs.clone();
    next.set_project_theme(theme)?;
    save_projects(&shared, &next)?;
    workspace.tabs = next;
    Ok(respond(&app, &shared, workspace))
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
    /// Kept by a session since the window was last opened.
    new: bool,
}

/// A file a session kept for the user and the user has not seen yet.
#[derive(Serialize)]
struct NewArtifact {
    session_id: String,
    /// The project's folder; None for "other".
    project: Option<String>,
    group: Option<String>,
    name: String,
    image: bool,
}

/// The files sessions kept for the user in every project since its artifacts window was last
/// opened, oldest first in each project.
#[tauri::command(async)]
fn new_artifacts(shared: State<'_, Shared>) -> Result<Vec<NewArtifact>, String> {
    let _span = shared.trace.span("new_artifacts");
    let data = mailbox::data_dir().map_err(|error| error.to_string())?;
    let projects = artifacts::read_projects(&data)?;
    let places = projects.iter().map(|project| Some(project.as_path())).chain(std::iter::once(None));
    Ok(places
        .flat_map(|project| {
            artifacts::unseen(&artifacts::root_of_project(&data, project)).into_iter().map(move |kept| NewArtifact {
                image: images::is_image_name(&kept.name),
                session_id: kept.session_id,
                project: project.map(|project| project.to_string_lossy().into_owned()),
                group: kept.group,
                name: kept.name,
            })
        })
        .collect())
}

/// The artifacts window of `project` was opened: what it showed as new no longer is.
#[tauri::command(async)]
fn artifacts_seen(project: Option<String>) -> Result<(), String> {
    artifacts::mark_seen(&artifacts_root(project.as_deref())?)
}

/// A block of the artifacts window: the unsorted files (no name), then each group.
#[derive(Serialize)]
struct ArtifactGroup {
    name: Option<String>,
    files: Vec<ArtifactFile>,
}

/// The unsorted files first, then every group by name, empty ones included; the newest file first
/// in each.
#[tauri::command(async)]
fn artifacts_view(shared: State<'_, Shared>, project: Option<String>) -> Result<Vec<ArtifactGroup>, String> {
    let _span = shared.trace.span("artifacts_view");
    let root = artifacts_root(project.as_deref())?;
    let listed = artifacts::entries(&root, Kind::User)?;
    let unseen = artifacts::unseen(&root);
    let names = std::iter::once(None).chain(artifacts::groups(&root, Kind::User)?.into_iter().map(Some));
    Ok(names
        .map(|name| {
            let files = listed
                .iter()
                .filter(|file| file.group == name)
                .map(|file| ArtifactFile {
                    name: file.name.clone(),
                    modified: file.modified,
                    image: images::is_image_name(&file.name),
                    new: unseen.iter().any(|kept| kept.group == file.group && kept.name == file.name),
                })
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

/// What the user added: the names the copies got, then one message per file left out.
#[derive(Serialize, Default)]
struct AddedArtifacts {
    names: Vec<String>,
    errors: Vec<String>,
}

/// Asks the system's file picker for files, then keeps a copy of each in `group` (None:
/// unsorted). Nothing is written over. Closing the picker adds nothing.
#[tauri::command(async)]
fn add_artifacts(window: tauri::WebviewWindow, project: Option<String>, group: Option<String>) -> Result<AddedArtifacts, String> {
    let root = artifacts_root(project.as_deref())?;
    let Some(picked) = window.dialog().file().set_title("add artifacts").set_parent(&window).blocking_pick_files() else {
        return Ok(AddedArtifacts::default());
    };
    let mut errors = Vec::new();
    let sources: Vec<PathBuf> = picked
        .into_iter()
        .filter_map(|file| {
            let shown = file.to_string();
            file.into_path().map_err(|error| errors.push(format!("{shown}: not a file on this computer ({error})"))).ok()
        })
        .collect();
    let kept = artifacts::keep_each(&root, Kind::User, group.as_deref(), &sources);
    errors.extend(kept.errors);
    Ok(AddedArtifacts { names: kept.names, errors })
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
        mark_asking(&mut next, implicit_default);
        view::assign(&mut next, &tabs, &prefs);
        mark_away(&mut next);
        mark_activity(&mut next, implicit_default);
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

/// Until when each session of `view` keeps working while the user is away; none when kindric's
/// data cannot be found.
fn mark_away(view: &mut view::ListView) {
    let root = away::default_root().ok();
    view::mark_away(view, |session| root.as_deref().and_then(|root| away::until(root, session)));
}

/// Which sessions of `view` wait for an answer, with the requests they left in the overview; a
/// folder that cannot be found leaves only the questions of Claude Code. A `kindric ask` the user
/// answered by typing into its session goes first (decision 363).
fn mark_asking(view: &mut view::ListView, implicit_default: bool) {
    let paths = accounts::paths().ok();
    let account_dir = |session: &str| {
        let row = view.sessions.iter().find(|row| row.session_id == session)?;
        Some(accounts::dir_of(paths.as_ref()?, &row.account, implicit_default))
    };
    let requested: HashSet<String> = overview::default_root()
        .map(|root| overview::pending_after_typed_answers(&root, account_dir).into_iter().map(|request| request.session_id).collect())
        .unwrap_or_default();
    view::mark_asking(view, &requested);
}

/// When each session of `view` was last active, from its transcript; none when the accounts'
/// folders cannot be found.
fn mark_activity(view: &mut view::ListView, implicit_default: bool) {
    let Ok(paths) = accounts::paths() else {
        return;
    };
    view::mark_activity(view, |account, sessions| activity::last_written(&accounts::dir_of(&paths, account, implicit_default), sessions));
}

/// What `set_away` changed: the end, none when back, and what to tell the session in the user's
/// name.
#[derive(Serialize)]
struct Away {
    until: Option<i64>,
    words: String,
}

/// The user is away from `session_id` for `minutes`, or back with none: the session keeps working
/// until then (see `away`). The end shows at once in the list.
#[tauri::command]
fn set_away(app: AppHandle, shared: State<'_, Shared>, session_id: String, minutes: Option<u32>) -> Result<Away, String> {
    let _span = shared.trace.span("set_away");
    let root = away::default_root().map_err(|error| format!("cannot find kindric's data: {error}"))?;
    let until = match minutes.filter(|minutes| *minutes > 0) {
        Some(minutes) => {
            let until = away::now() + i64::from(minutes) * 60;
            away::set(&root, &session_id, until).map_err(|error| format!("cannot write {}: {error}", root.display()))?;
            Some(until)
        },
        None => {
            away::clear(&root, &session_id).map_err(|error| format!("cannot clear {}: {error}", root.display()))?;
            None
        },
    };
    let view = {
        let mut view = shared.view.lock().unwrap_or_else(PoisonError::into_inner);
        mark_away(&mut view);
        view.clone()
    };
    let _ = app.emit("sessions", &view);
    Ok(Away { until, words: away::user_words(&session_id, minutes) })
}

fn overview_root() -> Result<PathBuf, String> {
    overview::default_root().map_err(|error| format!("cannot find kindric's data: {error}"))
}

/// What the sessions ask the user, oldest first.
#[tauri::command]
fn overview_requests(shared: State<'_, Shared>) -> Result<Vec<overview::Request>, String> {
    let _span = shared.trace.span("overview_requests");
    Ok(overview::pending(&overview_root()?))
}

/// The user answers a permission or a question of AskUserQuestion: the hook waiting in the
/// session reads the answer and gives it to Claude Code.
#[tauri::command]
fn answer_request(shared: State<'_, Shared>, session_id: String, id: String, answer: overview::Answer) -> Result<(), String> {
    let _span = shared.trace.span("answer_request");
    if matches!(answer, overview::Answer::Text { .. }) {
        return Err("the answer to kindric ask is typed into the session, not written for a hook".into());
    }
    overview::answer(&overview_root()?, &session_id, &id, &answer).map_err(|error| error.to_string())
}

/// What the page types into the pane that shows the session for `text`, the user's answer to
/// its `kindric ask`; `settle_request` once typed.
#[tauri::command]
fn answer_words(session_id: String, id: String, text: String) -> Result<String, String> {
    overview::typed_answer(&overview_root()?, &session_id, &id, &text)
}

/// The page typed the answer to a `kindric ask` in the pane that shows the session.
#[tauri::command]
fn settle_request(shared: State<'_, Shared>, session_id: String, id: String) -> Result<(), String> {
    let _span = shared.trace.span("settle_request");
    overview::settle(&overview_root()?, &session_id, &id).map_err(|error| format!("cannot remove the request: {error}"))
}

/// Types the answer to a `kindric ask` into a session no pane shows, as the user's message with
/// its question (`overview::answer_words`), then removes the request. Holds the session's claim
/// meanwhile, as a pane does.
#[tauri::command]
async fn type_answer(
    shared: State<'_, Shared>,
    account: String,
    short_id: String,
    session_id: String,
    id: String,
    text: String,
) -> Result<(), String> {
    let _span = shared.trace.span("type_answer");
    let implicit = implicit_default(&account)?;
    let claims = shared.claims_dir.clone()?;
    let root = overview_root()?;
    let typed = move || {
        let words = overview::typed_answer(&root, &session_id, &id, &text)?;
        type_as_user(&claims, &account, &short_id, implicit, &words)?;
        overview::settle(&root, &session_id, &id).map_err(|error| format!("cannot remove the request: {error}"))
    };
    tauri::async_runtime::spawn_blocking(typed).await.map_err(|error| error.to_string())?
}

/// Types `words` into a session no pane shows, then Enter, holding its claim meanwhile as a pane
/// does. Blocks for a few seconds.
fn type_as_user(claims: &Path, account: &str, short_id: &str, implicit: bool, words: &str) -> Result<(), String> {
    let key = SessionKey { account: account.to_string(), short_id: short_id.to_string() };
    let _claim = claim::claim(claims, &key)?;
    typing::type_into(&attach_command(account, short_id, implicit), words, &typing::TIMINGS).map_err(|error| format!("cannot type into {short_id}: {error}"))
}

/// Types a prompt from the control center into a session no pane shows, as the user's message.
/// Claude Code reads it even in the middle of a turn.
#[tauri::command]
async fn type_prompt(shared: State<'_, Shared>, account: String, short_id: String, text: String) -> Result<(), String> {
    let _span = shared.trace.span("type_prompt");
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("nothing to send".into());
    }
    let implicit = implicit_default(&account)?;
    let claims = shared.claims_dir.clone()?;
    tauri::async_runtime::spawn_blocking(move || type_as_user(&claims, &account, &short_id, implicit, &text)).await.map_err(|error| error.to_string())?
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

/// What the head of a project in the control center counts.
#[derive(Serialize)]
struct ProjectFigures {
    /// The worktrees besides the main checkout; None outside a git repository or for "other".
    worktrees: Option<usize>,
    /// The files kept for the user in the project.
    artifacts: usize,
}

/// Asks git about each worktree and lists the project's artifacts, so off the window's thread.
#[tauri::command]
async fn project_figures(shared: State<'_, Shared>, project: Option<String>) -> Result<ProjectFigures, String> {
    let _span = shared.trace.span("project_figures");
    let sessions = session_folders(&shared);
    let root = artifacts_root(project.as_deref())?;
    let counted = move || -> Result<ProjectFigures, String> {
        let worktrees = project.and_then(|project| worktrees::list(Path::new(&project), &sessions).ok()).map(|all| all.len().saturating_sub(1));
        Ok(ProjectFigures { worktrees, artifacts: artifacts::list(&root, Kind::User)?.len() })
    };
    tauri::async_runtime::spawn_blocking(counted).await.map_err(|error| error.to_string())?
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
        claude_facts: Mutex::new(None),
    };
    let built = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
            purge_preview,
            purge_sessions,
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
            follow_claude_theme,
            set_project_theme,
            custom_theme,
            trace_enabled,
            trace,
            write_input,
            resize_pane,
            image_bytes,
            memory_atlas,
            memory_note,
            memory_search,
            artifacts_view,
            new_artifacts,
            artifacts_seen,
            add_artifacts,
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
            control_figures,
            session_detail,
            session_changes,
            file_diff,
            set_away,
            overview_requests,
            answer_request,
            answer_words,
            settle_request,
            type_answer,
            type_prompt,
            project_worktrees,
            project_figures,
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
