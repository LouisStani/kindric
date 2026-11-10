import { DirectoryField } from "./directories.js";
import { Panes, sameSession } from "./panes.js";
import { applyTheme, nextTheme, THEMES } from "./themes.js";
import { mark, startTrace, traced } from "./trace.js";
import { askTrust } from "./helper.js";
import { checkItems, hasToDo } from "./welcome.js";
import { ArtifactsWindow } from "./artifacts.js";
import { SettingsWindow } from "./settings.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let lastView = { loaded: false, accounts: [], sessions: [], errors: [] };
let currentTheme = "ember";
const picker = { rows: [], selected: 0, query: 0 };
const status = { notice: "", timer: null };
// The welcome list, null until checked; what it was checked for; one check at a time, and
// one more after it when asked meanwhile.
const welcome = { lines: null, checkedFor: "", running: false, again: false };

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function elsewhere(session) {
  return `"${session.label}" runs in another terminal: it cannot be opened here`;
}

// A notice goes after 8 s. Errors of the list come first and stay as long as they last.
function setStatus(text) {
  clearTimeout(status.timer);
  status.notice = text;
  if (text) status.timer = setTimeout(() => setStatus(""), 8000);
  renderStatus();
}

// The right end of the bar: an error, a notice, or the name of the app.
function renderStatus() {
  const node = document.getElementById("status");
  const errors = lastView.errors.join(" · ");
  const text = errors || status.notice;
  node.className = errors ? "error" : text ? "notice" : "";
  node.title = text;
  if (text) node.replaceChildren(element("span", "status-text", text));
  else node.replaceChildren("kindric", element("span", "cursor"));
}

// The state a chip shows: its color and the shape of its dot.
function chipState(session) {
  if (session.finished) return "finished";
  if (!session.short_id) return "elsewhere";
  return session.state;
}

function directoryField(formId) {
  const form = document.getElementById(formId);
  return new DirectoryField(form.elements.directory, form.querySelector(".suggestions"));
}

// The sessions of the bar: those of the visible tab, its project's or, in "other", those of
// no project. Rust sorts them, most urgent first.
function barSessions() {
  if (panes.state === null) return [];
  const tab = panes.activeTab();
  return lastView.sessions.filter((session) => !session.hidden && session.tab === tab.id);
}

// The dot of a tab: its most urgent session, if that one needs you.
function markOf(tabId) {
  const session = lastView.sessions.find((row) => row.tab === tabId && !row.hidden);
  if (!session) return null;
  if (session.unread > 0) return "waiting";
  const state = chipState(session);
  return state === "finished" || state === "waiting" ? state : null;
}

function sessionItem(session) {
  const item = element("li", "session");
  const title = element("div", "title");
  title.append(element("span", "name", session.label));
  if (session.unread > 0) title.append(element("span", "badge", `${session.unread} msg`));
  if (session.finished) title.append(element("span", "done", "done"));
  const detail = [session.account, session.state];
  if (session.isolated) detail.push("isolated");
  const project = panes.projectName(session.tab);
  if (project) detail.push(project);
  if (session.hidden) detail.push("hidden");
  item.append(title, element("div", "detail", detail.join(" · ")));
  if (session.progress) item.append(element("div", "detail progress", session.progress));
  return item;
}

function chip(session, index) {
  const item = element("li", `chip ${chipState(session)}`);
  item.append(element("span", "pos", index < 9 ? `${index + 1}` : ""));
  item.append(element("span", "dot", session.finished ? "✓" : ""));
  item.append(element("span", "chip-title", session.label));
  if (session.unread > 0) {
    const badge = element("span", "badge", `${session.unread}`);
    badge.title = "read the messages";
    badge.addEventListener("click", (event) => {
      event.stopPropagation();
      openMessages(session);
    });
    item.append(badge);
  }
  if (panes.shows(session)) item.classList.add("open");
  item.addEventListener("click", () => traced("chip click", () => openSession(session)));
  item.addEventListener("mouseenter", () => showTip(session, item));
  item.addEventListener("mouseleave", hideTip);
  return item;
}

function showTip(session, item) {
  const tip = document.getElementById("tip");
  const state = [session.finished ? "done" : session.state];
  if (session.isolated) state.push("isolated");
  const head = element("div");
  head.append(element("b", "", session.title), element("span", `tip-state ${chipState(session)}`, state.join(" · ")));
  const unread = session.unread > 0 ? ` · ${session.unread} unread` : "";
  tip.replaceChildren(
    head,
    element("div", "", `${session.account} · ${session.directory}`),
    element("div", "", `${session.short_id ?? "another terminal"}${unread}`),
  );
  if (session.progress) tip.append(element("div", "progress", session.progress));
  // Shown before it is placed: a hidden element has no width.
  tip.hidden = false;
  const box = item.getBoundingClientRect();
  tip.style.left = `${Math.max(Math.min(box.left, window.innerWidth - tip.offsetWidth - 8), 8)}px`;
  tip.style.top = `${box.bottom + 6}px`;
}

function hideTip() {
  document.getElementById("tip").hidden = true;
}

// Chips that do not fit are hidden from the end, where the sessions that need you least
// are; "+N" opens Ctrl+P, which has them all. Chips never shrink, and hiding the last ones
// does not move the first: two measures are enough.
function fitChips() {
  const chips = document.getElementById("chips");
  const more = chips.querySelector(".more");
  if (!more) return;
  const items = [...chips.children].filter((item) => item !== more);
  for (const item of items) item.hidden = false;
  more.hidden = true;
  // Not laid out yet: nothing to measure, every chip stays.
  if (chips.clientWidth === 0) return;
  // Right edges rather than scrollWidth, which engines round differently.
  const edge = chips.getBoundingClientRect().right + 0.5;
  const rights = items.map((item) => item.getBoundingClientRect().right);
  if (rights.length === 0 || rights[rights.length - 1] <= edge) return;
  // "+N" as wide as it can get, after the gap between chips.
  more.textContent = `+${items.length}`;
  more.hidden = false;
  const room = edge - more.getBoundingClientRect().width - parseFloat(getComputedStyle(chips).columnGap || "0");
  const kept = rights.filter((right) => right <= room).length;
  for (const item of items.slice(kept)) item.hidden = true;
  more.textContent = `+${items.length - kept}`;
}

function renderChips(view) {
  const chips = document.getElementById("chips");
  hideTip();
  const sessions = barSessions();
  if (!view.loaded) {
    chips.replaceChildren(element("li", "chip-note", "loading…"));
  } else if (sessions.length === 0) {
    chips.replaceChildren(element("li", "chip-note", `no sessions here · ${keyText("new-session")} starts one`));
  } else {
    const more = element("li", "chip more");
    more.addEventListener("click", openPicker);
    chips.replaceChildren(...sessions.map(chip), more);
  }
  fitChips();
}

function render(view) {
  const start = performance.now();
  lastView = view;
  renderChips(view);
  renderStatus();
  panes.renderBar();
  panes.decorate();
  mark("render sessions", start);
  restoreTab();
  if (document.getElementById("messages").open) loadMessages();
  // An account or a session more or less can tick a line of the welcome list.
  const checkFor = JSON.stringify([view.accounts, view.sessions.length]);
  if (view.loaded && checkFor !== welcome.checkedFor) {
    welcome.checkedFor = checkFor;
    checkSetup();
  }
  placeWelcome();
}

// The checks run two programs: one at a time.
async function checkSetup() {
  if (welcome.running) {
    welcome.again = true;
    return;
  }
  welcome.running = true;
  try {
    welcome.lines = await invoke("welcome_checks");
  } catch (error) {
    setStatus(`${error}`);
  } finally {
    welcome.running = false;
  }
  drawChecks();
  placeWelcome();
  if (welcome.again) {
    welcome.again = false;
    checkSetup();
  }
}

// Drawn again only when checked again, so that a "copied" or a selection stays meanwhile.
function drawChecks() {
  if (welcome.lines === null) return;
  for (const id of ["welcome-list", "help-checks"]) {
    document.getElementById(id).replaceChildren(...checkItems(welcome.lines, { onStart: startFromChecks, onAdd: addMessagesServer }));
  }
}

// The list comes back checked again; sessions already running do not get the server.
async function addMessagesServer() {
  try {
    welcome.lines = await invoke("add_messages_server");
  } catch (error) {
    return `${error}`;
  }
  drawChecks();
  placeWelcome();
  return null;
}

// The welcome screen covers the panes while something is left to do and no pane of the
// visible tab shows a session.
function placeWelcome() {
  const empty = panes.state !== null && panes.tabEmpty(panes.activeTab());
  document.getElementById("welcome").hidden = !(hasToDo(welcome.lines) && empty);
}

function startFromChecks() {
  document.getElementById("help").close();
  openNewSessionForm();
}

// Back from a terminal where a command of the list was run: check again.
function checkOnReturn() {
  if (!document.getElementById("welcome").hidden || document.getElementById("help").open) checkSetup();
}

// Tabs whose session is being opened again, so that a render meanwhile does not open it twice.
const restoring = new Set();

// The first visit of a tab since launch opens the session it showed last, once the list
// is known, if it is still there and still belongs to this tab; otherwise the tab stops
// waiting for it.
async function restoreTab() {
  const tab = panes.state === null ? null : panes.activeTab();
  if (!tab || !tab.restore || !lastView.loaded || restoring.has(tab.id) || !panes.tabEmpty(tab)) return;
  restoring.add(tab.id);
  try {
    const session = rowOf(tab.restore);
    if (session && session.short_id && !session.hidden && session.tab === tab.id) await openSession(session);
    else panes.render(await invoke("drop_restore"));
  } catch (error) {
    setStatus(`${error}`);
  } finally {
    restoring.delete(tab.id);
  }
}

// Claude finished a turn in this session and it has not been looked at since.
function isFinished(key) {
  return lastView.sessions.some((row) => row.finished && row.account === key.account && row.short_id === key.short_id);
}

function rowOf(key) {
  return lastView.sessions.find((row) => row.account === key.account && row.short_id === key.short_id);
}

// The name of a session shown in a pane, for its head.
function titleOf(key) {
  const session = rowOf(key);
  return session ? session.label : key.short_id;
}

// A session opens in the tab of its project, "other" for no project. A
// session just started is not in the list yet: it opens where it was started.
async function openSession(session) {
  if (!session.short_id) {
    setStatus(elsewhere(session));
    return;
  }
  setStatus("");
  if (session.tab !== undefined) await panes.goTabId(session.tab);
  await panes.open(session);
}

function showNumber(number) {
  const session = barSessions()[number - 1];
  if (!session) {
    setStatus(`no session #${number}`);
    return;
  }
  return openSession(session);
}

// The session `step` places away from `key` among those that can open here, round the
// end; from an empty pane, the first or the last. Null when there is nothing else to open.
export function neighbour(sessions, key, step) {
  const openable = sessions.filter((session) => session.short_id);
  const at = key === null ? -1 : openable.findIndex((session) => sameSession(key, session));
  if (at === -1) return openable.length === 0 ? null : openable[step > 0 ? 0 : openable.length - 1];
  if (openable.length === 1) return null;
  return openable[(at + step + openable.length) % openable.length];
}

// Alt+H / Alt+L: the previous or next chip of the bar, in the active pane.
function stepSession(step) {
  const session = neighbour(barSessions(), panes.activeSession(), step);
  if (!session) {
    setStatus("no other session here");
    return;
  }
  return openSession(session);
}

// The session whose messages are shown, and the last request, as answers can cross.
const messagesView = { session: null, request: 0 };

function age(sentAt) {
  const minutes = Math.floor(Math.max(Date.now() / 1000 - sentAt, 0) / 60);
  if (minutes === 0) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  if (minutes < 48 * 60) return `${Math.floor(minutes / 60)} h ago`;
  return `${Math.floor(minutes / (24 * 60))} d ago`;
}

function messageItem(row) {
  const item = element("li", "message");
  const head = element("div", "message-head");
  head.append(element("span", "message-who", `${row.incoming ? "← from" : "→ to"} ${row.other}`));
  if (row.account) head.append(element("span", "", row.account));
  head.append(element("span", "", age(row.sent_at)));
  if (row.unread) head.append(element("span", "message-unread", "unread"));
  item.append(head, element("p", "message-text", row.text));
  return item;
}

// Read only: the session still gets its messages from read_messages.
async function loadMessages() {
  const request = ++messagesView.request;
  let rows;
  try {
    rows = await invoke("messages", { sessionId: messagesView.session.session_id });
  } catch (error) {
    rows = `${error}`;
  }
  if (request !== messagesView.request) return;
  const list = document.getElementById("messages-list");
  if (typeof rows === "string") list.replaceChildren(element("li", "muted", rows));
  else if (rows.length === 0) list.replaceChildren(element("li", "muted", "no messages"));
  else list.replaceChildren(...rows.map(messageItem));
  list.scrollTop = list.scrollHeight;
}

function openMessages(session) {
  messagesView.session = session;
  document.getElementById("messages-title").textContent = `messages · ${session.label}`;
  document.getElementById("messages-list").replaceChildren();
  document.getElementById("messages").showModal();
  return loadMessages();
}

// Ctrl+Shift+A: what sessions kept for the user in the project of the visible tab.
function openArtifacts() {
  const project = panes.activeTab().project;
  return artifactsWindow.open(project ? project.directory : null, project ? project.name : "other");
}

// Ctrl+Shift+M: the messages of the session of the active pane.
function openActiveMessages() {
  const key = panes.activeSession();
  const session = key && rowOf(key);
  if (!session) {
    setStatus("no session in this pane to read messages of");
    return;
  }
  return openMessages(session);
}

// The colors of theme `name`; custom ones are read from theme.json each time.
async function themeColors(name) {
  if (name !== "custom") return THEMES[name] ?? THEMES.ember;
  const colors = await invoke("custom_theme");
  if (!colors) throw new Error("no theme.json");
  return { ...THEMES.ember, ...colors };
}

async function showTheme(name) {
  applyTheme(name, await themeColors(name));
  panes.retheme();
}

// Ctrl+Shift+E: the next theme, kept for the next launch. A theme.json that does not load
// is reported and skipped at the next press.
async function cycleTheme() {
  let custom;
  try {
    custom = await invoke("custom_theme");
  } catch (error) {
    custom = error;
  }
  const name = nextTheme(currentTheme, custom !== null);
  currentTheme = name;
  try {
    await showTheme(name);
  } catch (error) {
    setStatus(`${error}`);
    return;
  }
  try {
    await invoke("set_theme", { name });
  } catch (error) {
    setStatus(`${error}`);
    return;
  }
  setStatus(`theme: ${name}`);
}

function openNewSessionForm() {
  const form = document.getElementById("new-form");
  const fields = form.elements;
  fields.account.replaceChildren(...lastView.accounts.map((account) => element("option", "", account)));
  const project = panes.activeTab().project;
  // The account of the last session started in this project, while it still exists.
  if (project && project.account && lastView.accounts.includes(project.account)) fields.account.value = project.account;
  fields.directory.value = project ? project.directory : "~";
  sessionDirectory.reset();
  fields.name.value = "";
  document.getElementById("new-message").textContent = "";
  form.querySelector("button").disabled = false;
  document.getElementById("new-session").showModal();
  fields.account.focus();
}

// Claude Code refused a folder the account has not trusted yet: it asks, in its own window, and
// the form is sent again once that window is closed.
function offerTrust(message, account, directory) {
  const trust = document.createElement("button");
  trust.type = "button";
  trust.textContent = "trust it";
  trust.addEventListener("click", () => askTrust(account, directory, () => document.getElementById("new-form").requestSubmit()));
  message.replaceChildren(`${account} has not trusted this folder yet.`, trust);
}

async function startNewSession(event) {
  event.preventDefault();
  const form = event.target;
  const fields = form.elements;
  const dialog = document.getElementById("new-session");
  const message = document.getElementById("new-message");
  const account = fields.account.value;
  if (!account) {
    message.textContent = "no accounts: kindric-accounts add <name>";
    return;
  }
  const button = form.querySelector("button");
  button.disabled = true;
  message.textContent = `starting a session on ${account}…`;
  let created;
  try {
    created = await invoke("new_session", { account, directory: fields.directory.value, name: fields.name.value });
  } catch (error) {
    const text = error.message ?? `${error}`;
    // Escape may have closed the form while the session was starting.
    if (!dialog.open) setStatus(text);
    else if (error.trust) offerTrust(message, account, fields.directory.value);
    else message.textContent = text;
    button.disabled = false;
    return;
  }
  dialog.close();
  await openSession(created);
  if (created.warnings.length > 0) setStatus(created.warnings.join(" · "));
}

function openProjectForm() {
  const form = document.getElementById("project-form");
  form.elements.directory.value = "~";
  projectDirectory.reset();
  document.getElementById("project-message").textContent = "";
  document.getElementById("new-project").showModal();
  form.elements.directory.focus();
}

async function addProject(event) {
  event.preventDefault();
  const dialog = document.getElementById("new-project");
  try {
    await panes.addProject(event.target.elements.directory.value);
  } catch (error) {
    document.getElementById("project-message").textContent = `${error}`;
    return;
  }
  dialog.close();
}

async function closeTab() {
  try {
    await panes.closeTab();
  } catch (error) {
    setStatus(`${error}`);
  }
}

function openRenameForm() {
  const key = panes.activeSession();
  const session = key && rowOf(key);
  if (!session) {
    setStatus("no session in this pane to name");
    return;
  }
  const form = document.getElementById("rename-form");
  form.dataset.account = session.account;
  form.dataset.shortId = session.short_id;
  form.elements.label.value = session.given ?? "";
  document.getElementById("rename-message").textContent = "";
  document.getElementById("rename").showModal();
  form.elements.label.focus();
}

async function renameSession(event) {
  event.preventDefault();
  const form = event.target;
  try {
    await invoke("rename_session", { account: form.dataset.account, shortId: form.dataset.shortId, name: form.elements.label.value });
  } catch (error) {
    document.getElementById("rename-message").textContent = `${error}`;
    return;
  }
  document.getElementById("rename").close();
}

function renderPicker() {
  const results = document.getElementById("picker-results");
  if (picker.rows.length === 0) {
    results.replaceChildren(element("li", "muted", "no match"));
    return;
  }
  results.replaceChildren(
    ...picker.rows.map((session, index) => {
      const item = sessionItem(session);
      if (session.short_id) item.classList.add("openable");
      if (index === picker.selected) item.classList.add("selected");
      item.addEventListener("click", () => choose(session));
      return item;
    }),
  );
  results.children[picker.selected].scrollIntoView({ block: "nearest" });
}

async function filterPicker() {
  // Answers can come back out of order: only the last query counts.
  const query = ++picker.query;
  const rows = await invoke("filter_sessions", { tab: panes.state.active, query: document.getElementById("picker-query").value });
  if (query !== picker.query) return;
  picker.rows = rows;
  picker.selected = 0;
  renderPicker();
}

function openPicker() {
  document.getElementById("picker-query").value = "";
  document.getElementById("picker").showModal();
  filterPicker();
}

// A hidden session chosen here comes back to the bar first.
async function choose(session) {
  document.getElementById("picker").close();
  if (session.hidden) {
    try {
      await invoke("show_session", { account: session.account, shortId: session.short_id });
    } catch (error) {
      setStatus(`${error}`);
      return;
    }
  }
  openSession(session);
}

// Asks before stopping and hiding the session of the active pane.
function confirmHide() {
  const key = panes.activeSession();
  const session = key && rowOf(key);
  if (!session) {
    setStatus("no session in this pane to hide");
    return;
  }
  const form = document.getElementById("hide-form");
  form.dataset.account = session.account;
  form.dataset.shortId = session.short_id;
  document.getElementById("hide-question").textContent =
    `Stop "${session.label}" and hide it from the bar? Its conversation is kept; ${keyText("find-session")} brings it back.`;
  document.getElementById("confirm-hide").showModal();
  form.querySelector("button[type=submit]").focus();
}

// Stops the session and hides it; Ctrl+P brings it back.
async function hideSession(event) {
  event.preventDefault();
  document.getElementById("confirm-hide").close();
  const form = event.target;
  const session = rowOf({ account: form.dataset.account, short_id: form.dataset.shortId });
  if (!session) return;
  setStatus(`stopping ${session.label}…`);
  try {
    panes.render(await invoke("hide_session", { account: session.account, shortId: session.short_id }));
  } catch (error) {
    setStatus(`${error}`);
    return;
  }
  setStatus(`${session.label} hidden · ${keyText("find-session")} brings it back`);
}

function onPickerKey(event) {
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    const step = event.key === "ArrowDown" ? 1 : -1;
    picker.selected = Math.min(Math.max(picker.selected + step, 0), Math.max(picker.rows.length - 1, 0));
    renderPicker();
  } else if (event.key === "Enter") {
    event.preventDefault();
    const session = picker.rows[picker.selected];
    if (session) choose(session);
  }
}

// Ctrl+Shift+C: the selection of the active pane, to the clipboard. Ctrl+C stays with
// Claude Code, which interrupts with it.
async function copySelection() {
  const text = panes.active().terminal.getSelection();
  if (!text) {
    setStatus("nothing selected in this pane");
    return;
  }
  try {
    await navigator.clipboard.writeText(text);
  } catch (error) {
    setStatus(`cannot copy: ${error}`);
  }
}

// Ctrl+Shift+V: text from the clipboard, as a paste into the session of the active pane.
// Ctrl+V stays with Claude Code, which reads images from the clipboard itself.
async function pasteText() {
  const view = panes.active();
  if (!view.current || view.current.ended) {
    setStatus("no session in this pane to paste into");
    return;
  }
  let text;
  try {
    text = await navigator.clipboard.readText();
  } catch (error) {
    setStatus(`cannot paste: ${error}`);
    return;
  }
  if (text) view.terminal.paste(text);
}

// A Mac takes Cmd for the app and leaves Ctrl to the terminal.
const MAC =/Mac/.test(navigator.platform || navigator.userAgent);

// Keys the app keeps for itself, like `command_for` in the terminal interface; F1 lists
// them from here, so the help cannot drift. "ctrl" is Cmd on a Mac, "#" a digit from 1 to 9,
// which `run` gets; `mac` replaces keys a Mac keeps for itself (Cmd+H hides the app).
// `pane` keys act only from a pane: elsewhere, in a form, the field gets them. `id` names a
// shortcut in `settings.json`, where the keys changed in the settings are kept.
const SHORTCUTS = [
  { group: "sessions", id: "new-session", keys: "ctrl+n", help: "start a new session", run: openNewSessionForm },
  { group: "sessions", id: "find-session", keys: "ctrl+p", help: "find a session of this tab", run: openPicker },
  { group: "sessions", id: "open-chip", keys: "alt+#", help: "open chip n of the bar", run: (digit) => showNumber(digit) },
  { group: "sessions", id: "previous-chip", keys: "alt+h", help: "open the previous chip", run: () => stepSession(-1) },
  { group: "sessions", id: "next-chip", keys: "alt+l", help: "open the next chip", run: () => stepSession(1) },
  { group: "sessions", id: "rename", keys: "f2", help: "give a short name", run: openRenameForm },
  { group: "sessions", id: "hide", keys: "ctrl+shift+k", help: () => `stop and hide (${keyText("find-session")} brings it back)`, run: confirmHide },
  { group: "sessions", id: "messages", keys: "ctrl+shift+m", help: "read its messages", run: openActiveMessages },
  { group: "panes", id: "split", keys: "ctrl+shift+d", help: "split the pane", run: () => panes.split() },
  { group: "panes", id: "close-pane", keys: "ctrl+shift+w", help: "close the pane (the session keeps running)", run: () => panes.close() },
  { group: "panes", id: "previous-pane", keys: "ctrl+h", mac: "ctrl+bracketleft", help: "previous pane", run: () => panes.step(-1) },
  { group: "panes", id: "next-pane", keys: "ctrl+l", mac: "ctrl+bracketright", help: "next pane", run: () => panes.step(1) },
  { group: "panes", id: "copy", keys: "ctrl+shift+c", mac: "ctrl+c", pane: true, help: "copy the selection", run: copySelection },
  { group: "panes", id: "paste", keys: "ctrl+shift+v", mac: "ctrl+v", pane: true, help: "paste text (ctrl+v: an image, for Claude Code)", run: pasteText },
  { group: "tabs", id: "add-project", keys: "ctrl+shift+t", help: "add a project", run: openProjectForm },
  { group: "tabs", id: "remove-project", keys: "ctrl+shift+x", help: "remove this project (its sessions keep running)", run: closeTab },
  { group: "tabs", id: "previous-tab", keys: "ctrl+shift+h", help: "previous tab", run: () => panes.tabStep(-1) },
  { group: "tabs", id: "next-tab", keys: "ctrl+shift+l", help: "next tab", run: () => panes.tabStep(1) },
  { group: "tabs", id: "tab-n", keys: "ctrl+shift+#", help: "tab n", run: (digit) => panes.goTab(digit) },
  { group: "app", id: "artifacts", keys: "ctrl+shift+a", help: "artifacts of this project", run: openArtifacts },
  { group: "app", id: "settings", keys: "ctrl+comma", help: "settings: accounts, memory, shortcuts", run: () => settingsWindow.open() },
  { group: "app", id: "next-theme", keys: "ctrl+shift+e", help: "next theme", run: cycleTheme },
  { group: "app", id: "help", keys: "f1", help: "what is left to set up, and these shortcuts", run: openHelp },
];
// Shortcut id → keys, for those changed in the settings.
let changedKeys = {};

// The keys a shortcut has before any change, on this system.
function defaultKeysOf(shortcut) {
  return MAC ? (shortcut.mac ?? shortcut.keys) : shortcut.keys;
}

// The keys of `shortcut` now.
function keysOf(shortcut) {
  return changedKeys[shortcut.id] ?? defaultKeysOf(shortcut);
}

function shortcutById(id) {
  return SHORTCUTS.find((shortcut) => shortcut.id === id);
}

// How the keys of the shortcut `id` are shown now: "Ctrl+P".
function keyText(id) {
  return shownKeys(keysOf(shortcutById(id)));
}

function helpOf(shortcut) {
  return typeof shortcut.help === "function" ? shortcut.help() : shortcut.help;
}

let SHORTCUT_KEYS = new Map();

function indexShortcuts() {
  SHORTCUT_KEYS = new Map(SHORTCUTS.map((shortcut) => [keysOf(shortcut), shortcut]));
}
// The default keys until settings.json is read.
indexShortcuts();

// What the settings window needs to show and change the shortcuts.
const shortcutSettings = {
  list: () =>
    SHORTCUTS.map((shortcut) => ({
      id: shortcut.id,
      group: shortcut.group,
      help: helpOf(shortcut),
      keys: keysOf(shortcut),
      shown: shownKeys(keysOf(shortcut)),
      changed: shortcut.id in changedKeys,
      fixed: keysOf(shortcut).includes("#"),
      isDefault: (keys) => keys === defaultKeysOf(shortcut),
    })),
  nameOf: (event) => shortcutName(event),
  shown: (keys) => shownKeys(keys),
  mac: MAC,
  // Saved first: a key that cannot be saved does not change.
  async set(id, keys) {
    await invoke("set_key", { id, keys });
    if (keys === null) delete changedKeys[id];
    else changedKeys[id] = keys;
    indexShortcuts();
  },
};

// How `event` is named in SHORTCUTS, Cmd playing Ctrl on a Mac; null when it holds the
// modifier left to the terminal (Ctrl on a Mac, Super elsewhere).
function shortcutName(event) {
  if (MAC ? event.ctrlKey : event.metaKey) return null;
  const control = MAC ? event.metaKey : event.ctrlKey;
  const modifiers = [control && "ctrl", event.shiftKey && "shift", event.altKey && "alt"].filter(Boolean);
  return [...modifiers, event.code.replace(/^(Key|Digit)/, "").toLowerCase()].join("+");
}

// The shortcut of `event`, its digit given to `run`; null for keys the terminal gets.
export function shortcutFor(event) {
  const name = shortcutName(event);
  if (name === null) return null;
  const digit = /\+([1-9])$/.exec(name);
  const shortcut = SHORTCUT_KEYS.get(digit ? name.replace(/[1-9]$/, "#") : name);
  if (!shortcut) return null;
  return { ...shortcut, run: () => shortcut.run(digit ? Number(digit[1]) : undefined) };
}

const SHOWN = { "#": "1..9", ctrl: MAC ? "Cmd" : "Ctrl", bracketleft: "[", bracketright: "]", comma: "," };

// "ctrl+shift+#" → "Ctrl+Shift+1..9", "Cmd+Shift+1..9" on a Mac.
function shownKeys(keys) {
  return keys
    .split("+")
    .map((part) => SHOWN[part] ?? (part.length === 1 ? part.toUpperCase() : part[0].toUpperCase() + part.slice(1)))
    .join("+");
}

function openHelp() {
  const sections = [...new Set(SHORTCUTS.map((shortcut) => shortcut.group))].map((group) => {
    const section = element("section", "help-group");
    section.append(element("h2", "", group));
    for (const shortcut of SHORTCUTS.filter((entry) => entry.group === group)) {
      section.append(element("kbd", "", shownKeys(keysOf(shortcut))), element("span", "", helpOf(shortcut)));
    }
    return section;
  });
  document.getElementById("help-list").replaceChildren(...sections);
  if (welcome.lines === null) document.getElementById("help-checks").replaceChildren(element("li", "muted", "checking…"));
  document.getElementById("help").showModal();
  checkSetup();
}

// "ctrl+shift+t", "alt+3", "f2": how a shortcut is named in the trace.
function keyName(event) {
  const modifiers = [event.ctrlKey && "ctrl", event.metaKey && "meta", event.shiftKey && "shift", event.altKey && "alt"].filter(Boolean);
  const key = event.code.replace(/^(Key|Digit)/, "").toLowerCase();
  return [...modifiers, key].join("+");
}

function onKey(event) {
  const shortcut = shortcutFor(event);
  if (!shortcut) return;
  if (shortcut.pane && !document.activeElement?.closest("#panes")) return;
  event.preventDefault();
  if (document.querySelector("dialog[open]")) return;
  traced(`key ${keyName(event)}`, shortcut.run);
}

const panes = new Panes(document.getElementById("panes"), document.getElementById("tabs"), {
  keyFilter: (event) => shortcutFor(event) === null,
  onChange: () => render(lastView),
  onError: setStatus,
  titleOf,
  isFinished,
  markOf,
  progressOf: (key) => rowOf(key)?.progress ?? null,
  imagesOf: (key) => rowOf(key)?.images ?? [],
});

const artifactsWindow = new ArtifactsWindow(document.getElementById("artifacts"), { onError: setStatus });
const settingsWindow = new SettingsWindow(document.getElementById("settings"), { shortcuts: shortcutSettings });

const sessionDirectory = directoryField("new-form");
const projectDirectory = directoryField("project-form");

document.addEventListener("keydown", onKey);
window.addEventListener("focus", checkOnReturn);
new ResizeObserver(fitChips).observe(document.getElementById("chips"));
document.getElementById("new-form").addEventListener("submit", startNewSession);
document.getElementById("project-form").addEventListener("submit", addProject);
document.getElementById("rename-form").addEventListener("submit", renameSession);
document.getElementById("hide-form").addEventListener("submit", hideSession);
document.getElementById("hide-cancel").addEventListener("click", () => document.getElementById("confirm-hide").close());
document.getElementById("helper-done").addEventListener("click", () => document.getElementById("helper").close());
document.getElementById("picker-query").addEventListener("input", filterPicker);
document.getElementById("picker-query").addEventListener("keydown", onPickerKey);
for (const dialog of document.querySelectorAll("dialog")) {
  dialog.addEventListener("close", () => panes.focus());
}

// xterm measures its cells with the font it is given: it must be loaded first.
await Promise.all([document.fonts.load('400 13px "JetBrains Mono"'), document.fonts.load('700 13px "JetBrains Mono"')]);
await startTrace();
// Before the first terminal, which takes its colors when created.
const savedSettings = await invoke("settings");
currentTheme = savedSettings.theme ?? "ember";
changedKeys = savedSettings.keys ?? {};
indexShortcuts();
try {
  await showTheme(currentTheme);
} catch (error) {
  setStatus(`theme ${currentTheme}: ${error}`);
}
await listen("pane-ended", (event) => panes.ended(event.payload));
await listen("tabs", (event) => panes.render(event.payload));
await listen("sessions", (event) => render(event.payload));
// Tauri takes files dropped on the window itself and gives their paths here, at a place in
// physical pixels.
await listen("tauri://drag-drop", ({ payload }) => {
  if (document.querySelector("dialog[open]") || !payload.paths?.length) return;
  const ratio = window.devicePixelRatio || 1;
  panes.dropFiles(payload.paths, payload.position.x / ratio, payload.position.y / ratio);
});
await panes.load();
render(await invoke("current_list"));
// Since the page started loading.
mark("page ready", 0);
