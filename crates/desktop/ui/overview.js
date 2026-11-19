// The control center (decisions 324 to 343, mockups C2): number 0 in the bar, the page the user
// lives on. On the side, every project with the standing of its sessions, and every account with
// its limits. In the middle, every project ("all", what needs you first) or one of them: a row per
// session, with what it asks, answered from here (325).

import {
  STANDINGS,
  accountCards,
  ageText,
  allView,
  answersInOneClick,
  canPrompt,
  changedOf,
  choicesAnswer,
  contextLevel,
  extensionOf,
  figureText,
  groupNote,
  keptLabel,
  keptText,
  keptTime,
  leftText,
  lineOf,
  nextAsking,
  permissionText,
  projectView,
  projectsOf,
  promptTargets,
  requestsBySession,
  sentText,
  shownInAll,
  standing,
  showsTasks,
  summaryOf,
  taskLine,
  tasksBadge,
} from "./overview-board.js";

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function button(className, text, onClick) {
  const node = element("button", className, text);
  node.type = "button";
  node.onclick = onClick;
  return node;
}

// "18:40" for Unix seconds, in the user's time.
function clockOf(seconds) {
  return new Date(seconds * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

// A bar filled to `percent`, set through the CSSOM: the page's policy refuses style attributes.
function bar(className, percent) {
  const track = element("span", `bar ${className}`);
  const fill = element("span", "bar-fill");
  fill.style.setProperty("width", `${Math.max(0, Math.min(100, percent))}%`);
  track.append(fill);
  return track;
}

// At most this many dots for a project on the side.
const MOST_DOTS = 12;

// How often the detail of the session picked is read again, in ms.
const DETAIL_EVERY = 3000;

// How often the head of the project shown counts again, in ms: git is asked about each worktree.
const PROJECT_EVERY = 30000;

// How each step of the last screen starts, as Claude Code draws them.
const STEP_MARKS = { prompt: "›", text: "●", tool: "●", result: "⎿", error: "⎿" };
const TASK_MARKS = { completed: "✓", in_progress: "▸", pending: "○" };

export class Overview {
  // `onOpen(row)` opens the session of a row. `onAnswer(request, answer)` answers a permission or
  // a question of AskUserQuestion; `onType(row, request, text)` types the answer to a `kindric ask`
  // into the session. Both return promises that fail with what to tell the user.
  // `onArtifacts(row, project)` opens the artifacts window of a project (a folder, null for
  // "other"); `thumbnailOf(artifact)` gives the address of an image kept, or null.
  // `onNewSession(project)` starts a session in a project (null: the form); `onAddProject()`.
  // `onPrompt(row, text)` types a prompt into a session, a promise that fails with what to say.
  // `onDetail(row)` reads the detail of a session (`session_detail`); `onOpenFile(file)` opens a
  // file it kept with the system's app. `onProjectFigures(tab)` reads what a project's head
  // counts (`project_figures`); `onChanges(row)` and `onWorktrees(tab)` open their windows.
  constructor(section, { onOpen, onAnswer, onType, onPrompt, onDetail, onOpenFile, onProjectFigures, onChanges, onWorktrees, onArtifacts, thumbnailOf, onNewSession, onAddProject, onSetInAll }) {
    this.section = section;
    this.onProjectFigures = onProjectFigures;
    this.onChanges = onChanges;
    this.onWorktrees = onWorktrees;
    // What the head of the project shown counts: its tab id, then what `project_figures` gave.
    this.projectFigures = null;
    this.projectTimer = null;
    this.onOpen = onOpen;
    this.onAnswer = onAnswer;
    this.onType = onType;
    this.onPrompt = onPrompt;
    this.onDetail = onDetail;
    this.onOpenFile = onOpenFile;
    // The detail of the session picked: its id, then what `session_detail` gave or the error.
    this.detail = null;
    this.detailTimer = null;
    this.onArtifacts = onArtifacts;
    this.thumbnailOf = thumbnailOf;
    this.onNewSession = onNewSession;
    this.onAddProject = onAddProject;
    this.onSetInAll = onSetInAll;
    // Session id → the files it kept for the user, not yet seen, oldest first (346).
    this.artifacts = new Map();
    this.seenArtifacts = "[]";
    // What `control_figures` read last: limits by account, context and files kept by session.
    this.figures = { usages: [], contexts: {}, kept: {} };
    this.seenFigures = "";
    this.side = element("aside", "control-side");
    this.main = element("div", "control-main");
    section.append(this.side, this.main);
    section.addEventListener("keydown", (event) => this.onKey(event));
    // "all", or the id of the tab shown; kept in memory only, "all" at start.
    this.shown = "all";
    // The session picked with the arrows, by id.
    this.selected = null;
    // Session id → its requests, oldest first.
    this.requests = new Map();
    this.seen = "[]";
    // Kept across renders, which come several times a second while a session works: what is
    // typed (request id or "id/question" → text), what is picked (request id → question →
    // labels, or the words of "Other…"), the questions whose "Other…" is open, and what an
    // answer being sent says (request id → "sending…" or the error).
    this.drafts = new Map();
    this.chosen = new Map();
    this.others = new Set();
    this.sending = new Map();
    // Prompts, apart from the requests whose keys go with them: session id (or "all") → the
    // words being typed, and → what sending said ("sending…", "sent", or the error).
    this.prompts = new Map();
    this.promptNotes = new Map();
    // "prompt all" of the project shown: null when closed, else whether it asks to confirm.
    this.promptAll = null;
    this.last = null;
  }

  // The files of `new_artifacts`. Returns whether they changed.
  setArtifacts(artifacts) {
    const seen = JSON.stringify(artifacts);
    if (seen === this.seenArtifacts) return false;
    this.seenArtifacts = seen;
    this.artifacts = requestsBySession(artifacts);
    return true;
  }

  artifactsOf(sessionId) {
    return this.artifacts.get(sessionId) ?? [];
  }

  // The files the detail shows, whose thumbnails must stay.
  keptShown() {
    return this.detail?.data?.kept ?? [];
  }

  // What `control_figures` returned. Returns whether it changed.
  setFigures(figures) {
    const seen = JSON.stringify(figures);
    if (seen === this.seenFigures) return false;
    this.seenFigures = seen;
    this.figures = figures;
    return true;
  }

  // The requests of `overview_requests`. Returns whether they changed. What was kept for a
  // request that is gone (answered, or settled in the terminal) goes with it.
  setRequests(requests) {
    const seen = JSON.stringify(requests);
    if (seen === this.seen) return false;
    this.seen = seen;
    this.requests = requestsBySession(requests);
    const ids = new Set(requests.map((request) => request.id));
    const kept = (key) => ids.has(key.split("/")[0]);
    for (const map of [this.drafts, this.chosen, this.sending]) {
      for (const key of [...map.keys()]) if (!kept(key)) map.delete(key);
    }
    for (const key of [...this.others]) if (!kept(key)) this.others.delete(key);
    return true;
  }

  // `rows` are the sessions not hidden, paused ones included; `tabs` `[{ id, name, directory }]` in
  // the order of the bar (directory null for "other"); `accounts` those of the registry.
  render(view, rows, tabs, accounts, newSessionKey) {
    this.last = { view, rows, tabs, accounts, newSessionKey };
    if (this.shown !== "all" && !tabs.some((tab) => tab.id === this.shown)) this.shown = "all";
    const focused = document.activeElement instanceof HTMLElement && this.section.contains(document.activeElement) ? document.activeElement : null;
    const focusKey = focused?.dataset.key ?? null;
    const caret = focused instanceof HTMLInputElement ? focused.selectionStart : null;
    const scroll = this.main.scrollTop;
    this.renderSide(rows, tabs, accounts);
    this.renderMain(view, rows, tabs, newSessionKey);
    this.main.scrollTop = scroll;
    // The field being typed in keeps the keyboard and its caret.
    const again = focusKey && this.section.querySelector(`[data-key="${CSS.escape(focusKey)}"]`);
    if (again) {
      again.focus();
      if (caret !== null && again instanceof HTMLInputElement) again.setSelectionRange(caret, caret);
    }
  }

  redraw() {
    if (this.last) {
      const { view, rows, tabs, accounts, newSessionKey } = this.last;
      this.render(view, rows, tabs, accounts, newSessionKey);
    }
  }

  show(shown) {
    this.shown = shown;
    this.selected = null;
    this.followDetail();
    this.promptAll = null;
    this.promptNotes.delete("all");
    this.redraw();
    this.main.scrollTop = 0;
    this.section.focus();
  }

  renderSide(rows, tabs, accounts) {
    const projects = element("nav", "control-projects");
    for (const entry of projectsOf(rows, tabs, this.requests)) {
      const item = button(entry.id === this.shown ? "control-project on" : "control-project", "", () => this.show(entry.id));
      if (!entry.inAll) item.classList.add("out");
      const dots = element("span", "control-dots");
      for (const id of entry.standings.slice(0, MOST_DOTS)) dots.append(element("span", `dot is-${id}`));
      item.append(element("span", "control-project-name", entry.name), dots);
      if (entry.needs > 0) item.append(element("span", "control-needs", `${entry.needs}`));
      const line = element("div", "control-project-line");
      line.append(item);
      const tab = tabs.find((each) => each.id === entry.id);
      if (tab?.directory) line.append(this.inAllToggle(entry, tab));
      projects.append(line);
    }
    projects.append(button("control-add", "+ add a project", () => this.onAddProject()));
    const cards = accountCards(rows, accounts, this.figures.usages ?? []);
    const title = element("h3", "control-side-title", "accounts");
    this.side.replaceChildren(projects, title, ...cards.map((card) => this.accountCard(card)));
  }

  // The dot left of a project's name: shown in "all" or left out of it (decision 351). "all" and
  // "other" have none.
  inAllToggle(entry, tab) {
    const toggle = button(entry.inAll ? "in-all-toggle on" : "in-all-toggle", "", () => this.onSetInAll(tab, !entry.inAll));
    toggle.setAttribute("aria-pressed", `${entry.inAll}`);
    toggle.setAttribute("aria-label", `${entry.name} in all projects`);
    toggle.title = entry.inAll ? "shown in all projects · click to leave it out" : "left out of all projects, except what needs you · click to show it";
    return toggle;
  }

  accountCard(card) {
    const box = element("section", card.hot ? "account-card hot" : "account-card");
    const head = element("div", "account-head");
    head.append(element("strong", "", card.name), element("span", "", card.sessions === 1 ? "1 session" : `${card.sessions} sessions`));
    box.append(head);
    if (!card.usage) {
      box.append(element("p", "account-note", "no figures yet"));
      return box;
    }
    for (const [label, percent] of [["5h", card.usage.five_hour], ["7d", card.usage.seven_day]]) {
      const line = element("div", "account-line");
      line.append(element("span", "", label), bar(label === "5h" && card.hot ? "hot" : "", percent), element("span", "account-percent", `${Math.round(percent)}%`));
      box.append(line);
    }
    if (card.usage.five_hour_resets_at) box.append(element("p", "account-note", `5h resets at ${clockOf(card.usage.five_hour_resets_at)}`));
    return box;
  }

  renderMain(view, rows, tabs, newSessionKey) {
    const tab = tabs.find((each) => each.id === this.shown) ?? null;
    const mine = tab ? rows.filter((row) => row.tab === tab.id) : shownInAll(rows, tabs, this.requests);
    const head = element("header", "control-head");
    const title = element("div", "control-title");
    title.append(element("h1", "", tab ? tab.name : "all projects"));
    if (tab?.directory) title.append(element("span", "control-directory", tab.directory));
    const actions = element("div", "control-actions");
    actions.append(button("primary", "+ new session", () => this.onNewSession(tab?.project ?? null)));
    if (tab && this.projectFigures?.tabId !== tab.id) this.followProject(tab);
    if (!tab && this.projectFigures) this.followProject(null);
    const targets = tab ? promptTargets(rows, tab.id, this.requests) : [];
    if (targets.length > 0) {
      actions.append(
        button(this.promptAll ? "on" : "", `prompt all ${targets.length}`, () => {
          this.promptAll = this.promptAll ? null : { confirm: false };
          this.redraw();
          this.section.querySelector('[data-key="prompt/all"]')?.focus();
        }),
      );
    }
    if (tab) actions.append(this.headFigures(tab, mine));
    head.append(title, this.summary(mine), actions);
    if (this.promptAll && targets.length > 0) head.append(this.promptAllBox(targets));
    const body = element("div", "control-body");
    if (!view.loaded) {
      body.append(element("p", "overview-note", "loading…"));
    } else if (mine.length === 0 && !tab && rows.length > 0) {
      body.append(element("p", "overview-note", "the projects with sessions are left out of all · the dot left of a project's name shows it here"));
    } else if (mine.length === 0) {
      body.append(element("p", "overview-note", `no sessions yet · ${newSessionKey} starts one`));
    } else if (tab) {
      this.projectBody(body, mine, tab);
    } else {
      this.allBody(body, rows, tabs);
    }
    const hint = element("footer", "overview-hint", "↑ ↓ picks a session · ⏎ types in its field · o opens it in a pane · a allows · d denies · n next request in any project · esc back");
    this.main.replaceChildren(head, body, hint);
  }

  // What a project's head counts is read on entering it, then every 30 s while it shows; leaving
  // it for "all" stops that.
  followProject(tab) {
    clearInterval(this.projectTimer);
    this.projectTimer = null;
    if (!tab) {
      this.projectFigures = null;
      return;
    }
    this.projectFigures = { tabId: tab.id, data: null };
    const read = async () => {
      if (this.section.hidden || this.projectFigures?.tabId !== tab.id) return;
      let data = null;
      try {
        data = await this.onProjectFigures(tab);
      } catch {
        // The head then shows no worktrees nor artifacts: nothing else depends on them.
        data = null;
      }
      if (this.projectFigures?.tabId !== tab.id) return;
      const changed = JSON.stringify(data) !== JSON.stringify(this.projectFigures.data);
      this.projectFigures = { tabId: tab.id, data };
      if (changed) this.redraw();
    };
    read();
    this.projectTimer = setInterval(read, PROJECT_EVERY);
  }

  // "4 files changed", "3 worktrees", "12 artifacts [2 new]" on the right of the project's head;
  // each opens its window, none when its count is 0.
  headFigures(tab, rows) {
    const box = element("div", "control-figures");
    const { total, sessions } = changedOf(rows);
    const changes = button("control-figure", figureText(total, "file changed", "files changed"), () => {
      if (sessions.length === 1) this.onChanges(sessions[0]);
    });
    changes.title = sessions.length > 1 ? "changed by its sessions, each counted since it started: the N files of a row open its changes" : "changed by its sessions, each counted since it started";
    changes.disabled = total === 0;
    box.append(changes);
    const data = this.projectFigures?.tabId === tab.id ? this.projectFigures.data : null;
    if (data?.worktrees !== null && data?.worktrees !== undefined) {
      const worktrees = button("control-figure", figureText(data.worktrees, "worktree", "worktrees"), () => this.onWorktrees(tab));
      worktrees.disabled = data.worktrees === 0;
      box.append(worktrees);
    }
    if (data) {
      const fresh = rows.reduce((sum, row) => sum + this.artifactsOf(row.session_id).length, 0);
      const artifacts = button("control-figure", figureText(data.artifacts, "artifact", "artifacts"), () => this.onArtifacts({ tab: tab.id }, tab.directory));
      if (fresh > 0) artifacts.append(element("span", "figure-new", `${fresh} new`));
      artifacts.disabled = data.artifacts === 0 && fresh === 0;
      box.append(artifacts);
    }
    return box;
  }

  summary(rows) {
    const counts = summaryOf(rows, this.requests);
    const line = element("p", "control-summary");
    for (const group of STANDINGS) {
      if (counts[group.id] === 0 && group.id !== "need") continue;
      const item = element("span", `overview-count is-${group.id}`);
      item.append(element("strong", "", `${counts[group.id]}`), document.createTextNode(` ${group.label}`));
      line.append(item);
    }
    const fresh = rows.reduce((sum, row) => sum + this.artifactsOf(row.session_id).length, 0);
    if (fresh > 0) {
      const item = element("span", "overview-count is-kept");
      item.append(element("strong", "", `${fresh}`), document.createTextNode(fresh === 1 ? " new artifact" : " new artifacts"));
      line.append(item);
    }
    return line;
  }

  allBody(body, rows, tabs) {
    const { needs, groups } = allView(rows, tabs, this.requests);
    const nameOf = new Map(tabs.map((tab) => [tab.id, tab]));
    if (needs.length > 0) {
      body.append(element("h2", "control-group is-need", `needs you · ${needs.length}`));
      for (const row of needs) body.append(this.row(row, nameOf.get(row.tab) ?? null, true));
    }
    for (const group of groups) {
      const heading = element("h2", "control-group");
      heading.append(button("control-group-name", `${group.tab.name} ›`, () => this.show(group.tab.id)));
      // The whole project, those that need you included.
      const mine = rows.filter((row) => row.tab === group.tab.id);
      const fresh = mine.reduce((sum, row) => sum + this.artifactsOf(row.session_id).length, 0);
      const note = groupNote(changedOf(mine).total, fresh);
      if (note) heading.append(element("span", "control-group-note", note));
      body.append(heading, ...group.rows.map((row) => this.row(row, group.tab, false)));
    }
  }

  projectBody(body, rows, tab) {
    const { active, paused } = projectView(rows, tab.id, this.requests);
    body.append(...active.map((row) => this.row(row, tab, false)));
    if (paused.length > 0) body.append(element("h2", "control-group", "paused"), ...paused.map((row) => this.row(row, tab, false)));
  }

  // The rows in the order shown, by session id.
  shownIds() {
    return [...this.main.querySelectorAll(".control-row")].map((row) => row.dataset.session);
  }

  // A session: its name, what it does, what it kept, its context and account, then what it asks.
  // `withProject`: in "needs you", where it comes from.
  row(row, tab, withProject) {
    const id = standing(row, this.requests);
    const article = element("article", `control-row is-${id}`);
    if (row.session_id === this.selected) article.classList.add("selected");
    article.dataset.session = row.session_id;
    article.dataset.key = `row/${row.session_id}`;
    article.tabIndex = -1;
    article.title = `${row.title}\n${row.directory}`;
    const line = element("div", "row-line");
    const doing = withProject ? `${tab?.name ?? "other"} · ${lineOf(row, this.requests)}` : lineOf(row, this.requests);
    // The badges go with what it does: the columns after it stay aligned from row to row.
    const what = element("span", "row-doing");
    what.append(element("span", "row-doing-text", doing));
    if (showsTasks(row, this.requests)) what.append(element("span", "row-badge tasks", tasksBadge(row.tasks)));
    if (row.unread > 0) what.append(element("span", "row-badge", `${row.unread} msg`));
    if ((row.changed ?? 0) > 0 && row.short_id) {
      what.append(
        button("row-badge files", figureText(row.changed, "file", "files"), (event) => {
          event.stopPropagation();
          this.onChanges(row);
        }),
      );
    }
    if ((this.requests.get(row.session_id) ?? []).some((request) => request.kind === "ask")) what.append(element("span", "row-badge keeps", "keeps working meanwhile"));
    line.append(element("span", "dot"), element("span", "row-name", row.label), what, this.kept(row, tab), this.context(row), element("span", "row-account", row.account));
    if (id === "paused") {
      line.append(
        button("row-resume", "resume", (event) => {
          event.stopPropagation();
          this.onOpen(row);
        }),
      );
    }
    article.append(line);
    if (showsTasks(row, this.requests)) article.append(element("p", "row-task", `▸ ${taskLine(row.tasks)}`));
    for (const request of this.requests.get(row.session_id) ?? []) article.append(this.request(row, request));
    if (row.session_id === this.selected) article.append(this.detailBox(row));
    if (row.session_id === this.selected && canPrompt(row)) article.append(this.promptBox(row));
    // A click picks the row, which opens its field.
    article.onclick = () => {
      if (this.selected !== row.session_id) this.pick(row.session_id, false);
    };
    return article;
  }

  // The detail of the session picked is read at once, then every 3 s while it stays picked and
  // the page shows; picking another session or none stops it.
  followDetail() {
    clearInterval(this.detailTimer);
    this.detailTimer = null;
    if (!this.selected) {
      this.detail = null;
      return;
    }
    if (this.detail?.sessionId !== this.selected) this.detail = { sessionId: this.selected, data: null, error: null };
    this.readDetail();
    this.detailTimer = setInterval(() => this.readDetail(), DETAIL_EVERY);
  }

  async readDetail() {
    const sessionId = this.selected;
    const row = this.last?.rows.find((each) => each.session_id === sessionId);
    if (!row || this.section.hidden) return;
    let data = null;
    let error = null;
    try {
      data = await this.onDetail(row);
    } catch (failure) {
      error = `${failure}`;
    }
    // Picked another one meanwhile: this answer is no longer wanted.
    if (this.selected !== sessionId) return;
    const before = JSON.stringify(this.detail);
    this.detail = { sessionId, data, error };
    if (JSON.stringify(this.detail) !== before) this.redraw();
  }

  // Where the session is (its last steps) and what it made (the files it kept for the user).
  detailBox(row) {
    const box = element("div", "row-detail");
    box.onclick = (event) => event.stopPropagation();
    const detail = this.detail?.sessionId === row.session_id ? this.detail : null;
    const now = Date.now() / 1000;
    const screen = element("section", "detail-screen");
    const written = detail?.data?.written_at;
    screen.append(element("h3", "detail-title", written ? `last screen · ${ageText(written, now)}` : "last screen"));
    const lines = element("ol", "detail-steps");
    if (detail?.error) lines.append(element("li", "step is-error", detail.error));
    else if (!detail?.data) lines.append(element("li", "step is-note", "reading…"));
    else if (detail.data.steps.length === 0) lines.append(element("li", "step is-note", "nothing to show yet"));
    else for (const step of detail.data.steps) lines.append(element("li", `step is-${step.kind}`, `${STEP_MARKS[step.kind] ?? ""} ${step.text}`));
    screen.append(lines);
    const kept = element("section", "detail-kept");
    kept.append(element("h3", "detail-title", "kept by this session"));
    const files = detail?.data?.kept ?? [];
    if (files.length === 0) kept.append(element("p", "step is-note", detail?.data ? "nothing kept yet" : ""));
    const fresh = this.artifactsOf(row.session_id);
    const isFresh = (file) => fresh.some((each) => each.name === file.name && (each.group ?? null) === (file.group ?? null) && (each.project ?? null) === (file.project ?? null));
    for (const file of files) kept.append(this.keptFile(file, isFresh(file), now));
    if (row.tasks) box.append(this.taskList(row.tasks));
    box.append(screen, kept);
    return box;
  }

  // The session's task list (decision 368): each task with its state, what the one in progress is
  // about under it.
  taskList(tasks) {
    const section = element("section", "detail-tasks");
    const left = tasks.left_seconds != null ? ` · ${leftText(tasks.left_seconds)} left` : "";
    section.append(element("h3", "detail-title", `tasks · ${tasks.done}/${tasks.total}${left}`));
    const list = element("ol", "task-list");
    for (const task of tasks.tasks) {
      list.append(element("li", `task is-${task.status}`, `${TASK_MARKS[task.status] ?? "○"} ${task.subject}`));
      if (task.status === "in_progress" && tasks.about) list.append(element("li", "task-about", tasks.about));
    }
    section.append(list);
    return section;
  }

  keptFile(file, fresh, now) {
    const item = button(fresh ? "detail-file fresh" : "detail-file", "", () => this.onOpenFile(file));
    item.title = `open ${file.name}`;
    const badge = element("span", "detail-badge", extensionOf(file.name));
    if (file.image) {
      const thumb = element("img", "detail-thumb");
      thumb.alt = "";
      this.thumbnailOf(file).then((url) => {
        if (!url) return;
        thumb.src = url;
        badge.replaceChildren(thumb);
      });
    }
    item.append(badge, element("span", "detail-name", file.name), element("span", "detail-time", keptTime(file.kept_at, now)));
    return item;
  }

  // The field of the row picked: what is typed goes to the session as the user's message.
  promptBox(row) {
    const box = element("div", "row-prompt");
    box.onclick = (event) => event.stopPropagation();
    const key = row.session_id;
    const send = () => this.sendPrompt(row);
    const actions = element("div", "request-actions answer");
    actions.append(
      this.field(`prompt/${key}`, "› prompt…", () => this.promptNotes.delete(key), send, this.prompts, key),
      button("primary", "send ⏎", send),
      button("", "open in a pane", () => this.onOpen(row)),
    );
    const note = this.promptNotes.get(key);
    const said = note ?? "typed into the session while it works · it reads it at the end of its step";
    box.append(actions, element("p", note && note !== "sending…" && note !== "sent" ? "request-note error" : "request-note", said));
    return box;
  }

  async sendPrompt(row) {
    const key = row.session_id;
    const text = (this.prompts.get(key) ?? "").trim();
    if (!text || this.promptNotes.get(key) === "sending…") return;
    this.promptNotes.set(key, "sending…");
    this.redraw();
    try {
      await this.onPrompt(row, text);
      this.prompts.delete(key);
      this.promptNotes.set(key, "sent");
    } catch (error) {
      this.promptNotes.set(key, `${error}`);
    }
    this.redraw();
  }

  // "prompt all": one field for every session of the project a prompt reaches. The first send
  // asks to confirm, the second types it into each, one after the other.
  promptAllBox(targets) {
    const box = element("div", "prompt-all");
    const send = () => this.sendToAll(targets);
    const actions = element("div", "request-actions answer");
    const label = this.promptAll.confirm ? `send to ${targets.length} sessions?` : "send ⏎";
    actions.append(
      this.field("prompt/all", `› prompt for ${targets.length} sessions…`, () => {
        this.promptAll.confirm = false;
        this.promptNotes.delete("all");
      }, send, this.prompts, "all"),
      button("primary", label, send),
    );
    box.append(actions);
    const note = this.promptNotes.get("all");
    if (note) box.append(element("p", note.includes(": ") ? "request-note error" : "request-note", note));
    return box;
  }

  async sendToAll(targets) {
    const text = (this.prompts.get("all") ?? "").trim();
    if (!text || this.promptNotes.get("all") === "sending…") return;
    if (!this.promptAll.confirm) {
      this.promptAll.confirm = true;
      this.redraw();
      return;
    }
    this.promptNotes.set("all", "sending…");
    this.redraw();
    const results = [];
    for (const row of targets) {
      try {
        await this.onPrompt(row, text);
        results.push({ label: row.label, error: null });
      } catch (error) {
        results.push({ label: row.label, error: `${error}` });
      }
    }
    this.prompts.delete("all");
    if (this.promptAll) this.promptAll.confirm = false;
    this.promptNotes.set("all", sentText(results));
    this.redraw();
  }

  // "3 kept · 2 new": a click opens the artifacts window of its project; the latest new image
  // beside it (346).
  kept(row, tab) {
    const fresh = this.artifactsOf(row.session_id);
    const total = this.figures.kept?.[row.session_id] ?? 0;
    const label = keptLabel(total, fresh.length);
    const box = element("span", fresh.length > 0 ? "row-kept fresh" : total > 0 ? "row-kept" : "row-kept none", label);
    if (fresh.length === 0 && total === 0) return box;
    if (fresh.length > 0) box.title = keptText(fresh);
    const image = [...fresh].reverse().find((artifact) => artifact.image);
    if (image) {
      const thumb = element("img", "row-thumb");
      thumb.alt = image.name;
      thumb.hidden = true;
      this.thumbnailOf(image).then((url) => {
        if (!url) return;
        thumb.src = url;
        thumb.hidden = false;
      });
      box.prepend(thumb);
    }
    box.onclick = (event) => {
      event.stopPropagation();
      this.onArtifacts(row, fresh[0]?.project ?? tab?.directory ?? null);
    };
    return box;
  }

  context(row) {
    const box = element("span", "row-ctx");
    const percent = this.figures.contexts?.[row.session_id];
    box.append(element("span", "row-ctx-label", "ctx"));
    if (percent === undefined) {
      box.append(element("span", "row-ctx-none", "—"));
      return box;
    }
    box.append(bar(contextLevel(percent), percent), element("span", "row-ctx-percent", `${Math.round(percent)}%`));
    return box;
  }

  // ↑ ↓ pick a row, Enter goes into its field, o opens it in a pane, a / d answer its permission,
  // n goes to the next session that asks, Escape goes back to every project. Fields keep their
  // keys; Escape in a field comes back here.
  onKey(event) {
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) return;
    if (event.target instanceof HTMLButtonElement && (event.key === "Enter" || event.key === " ")) return;
    const ids = this.shownIds();
    const row = () => this.last?.rows.find((each) => each.session_id === this.selected) ?? null;
    switch (event.key) {
      case "ArrowDown":
      case "ArrowUp": {
        if (ids.length === 0) return;
        const at = ids.indexOf(this.selected);
        const step = event.key === "ArrowDown" ? 1 : -1;
        this.pick(ids[at < 0 ? (step > 0 ? 0 : ids.length - 1) : Math.max(0, Math.min(ids.length - 1, at + step))]);
        break;
      }
      case "Enter": {
        const field = this.selected && this.section.querySelector(`[data-key="${CSS.escape(`prompt/${this.selected}`)}"]`);
        if (!field) return;
        field.focus();
        break;
      }
      case "o":
        if (!row()?.short_id) return;
        this.onOpen(row());
        break;
      case "a":
      case "d": {
        const permission = (this.requests.get(this.selected) ?? []).find((request) => request.kind === "permission");
        if (!permission) return;
        this.answer(permission, { kind: event.key === "a" ? "allow" : "deny" });
        break;
      }
      case "n": {
        const asking = this.askingInOrder();
        const next = nextAsking(asking, this.selected);
        if (next === null) return;
        const target = this.last.rows.find((each) => each.session_id === next);
        if (this.shown !== "all" && target && target.tab !== this.shown) this.shown = target.tab;
        this.pick(next);
        break;
      }
      case "Escape":
        if (this.shown === "all") return;
        this.show("all");
        break;
      default:
        return;
    }
    event.preventDefault();
    event.stopPropagation();
  }

  // The sessions with a request, as "all" orders them.
  askingInOrder() {
    if (!this.last) return [];
    const { rows, tabs } = this.last;
    return allView(rows, tabs, this.requests)
      .needs.filter((row) => this.requests.has(row.session_id))
      .map((row) => row.session_id);
  }

  // `scroll`: the arrows bring the row into view; a click leaves the page where it is.
  pick(sessionId, scroll = true) {
    this.selected = sessionId;
    this.followDetail();
    this.redraw();
    const picked = this.main.querySelector(`[data-session="${CSS.escape(sessionId)}"]`);
    if (scroll) picked?.scrollIntoView({ block: "nearest" });
    this.section.focus();
  }

  request(row, request) {
    const block = element("div", `request is-${request.kind}`);
    // Answering is not opening the session.
    block.onclick = (event) => event.stopPropagation();
    if (request.kind === "permission") this.permission(block, request);
    else if (request.kind === "question") this.question(block, request);
    else this.ask(block, row, request);
    const sending = this.sending.get(request.id);
    if (sending) block.append(element("p", sending === "sending…" ? "request-note" : "request-note error", sending));
    return block;
  }

  permission(block, request) {
    const { prompt, text } = permissionText(request);
    const command = element("pre", "request-command");
    command.append(element("span", "prompt", prompt), document.createTextNode(` ${text}`));
    const actions = element("div", "request-actions");
    actions.append(
      button("primary", "Allow", () => this.answer(request, { kind: "allow" })),
      button("", "Deny", () => this.answer(request, { kind: "deny" })),
    );
    block.append(command, actions);
  }

  question(block, request) {
    const questions = Array.isArray(request.questions) ? request.questions : [];
    if (!this.chosen.has(request.id)) this.chosen.set(request.id, new Map());
    const chosen = this.chosen.get(request.id);
    const oneClick = answersInOneClick(questions);
    questions.forEach((question, index) => {
      block.append(element("p", "request-question", question.question));
      const options = element("div", "request-options");
      (question.options ?? []).forEach((option, number) => {
        const picked = chosen.get(question.question);
        const on = Array.isArray(picked) && picked.includes(option.label);
        const choice = button(on ? "option on" : "option", "", () => {
          if (oneClick) {
            this.answer(request, { kind: "choices", answers: { [question.question]: option.label } });
            return;
          }
          const before = Array.isArray(chosen.get(question.question)) ? chosen.get(question.question) : [];
          const after = question.multiSelect
            ? before.includes(option.label) ? before.filter((label) => label !== option.label) : [...before, option.label]
            : [option.label];
          chosen.set(question.question, after);
          this.others.delete(`${request.id}/${index}`);
          this.redraw();
        });
        choice.append(element("span", "option-number", `${number + 1}`), document.createTextNode(` ${option.label}`));
        if (option.description) choice.title = option.description;
        options.append(choice);
      });
      const otherKey = `${request.id}/${index}`;
      options.append(
        button(this.others.has(otherKey) ? "option other on" : "option other", "Other…", () => {
          this.others.add(otherKey);
          chosen.set(question.question, this.drafts.get(otherKey) ?? "");
          this.redraw();
          this.section.querySelector(`[data-key="${CSS.escape(otherKey)}"]`)?.focus();
        }),
      );
      block.append(options);
      if (this.others.has(otherKey)) {
        const field = this.field(otherKey, "your answer…", (text) => chosen.set(question.question, text), () => this.sendChoices(request, questions, chosen));
        block.append(field);
      }
    });
    if (!oneClick) {
      const actions = element("div", "request-actions");
      const send = button("primary", "Send", () => this.sendChoices(request, questions, chosen));
      send.disabled = choicesAnswer(questions, chosen) === null;
      actions.append(send);
      block.append(actions);
    }
  }

  sendChoices(request, questions, chosen) {
    const answers = choicesAnswer(questions, chosen);
    if (answers) this.answer(request, { kind: "choices", answers });
  }

  ask(block, row, request) {
    block.append(element("p", "request-question", request.question), element("p", "request-context", request.context));
    const send = () => {
      const text = (this.drafts.get(request.id) ?? "").trim();
      if (text) this.send(request, () => this.onType(row, request, text));
    };
    const field = this.field(request.id, "your answer…", () => {}, send);
    const actions = element("div", "request-actions answer");
    actions.append(field, button("primary", "Send", send));
    block.append(actions);
  }

  // A text field whose words survive the renders, kept in `drafts` under `draft`; Enter sends,
  // Escape gives the keyboard back to the page.
  field(key, placeholder, onText, onEnter, drafts = this.drafts, draft = key) {
    const input = element("input", "request-field");
    input.dataset.key = key;
    input.placeholder = placeholder;
    input.spellcheck = false;
    input.value = drafts.get(draft) ?? "";
    input.oninput = () => {
      drafts.set(draft, input.value);
      onText(input.value);
    };
    input.onkeydown = (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        onEnter();
      } else if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        this.section.focus();
      }
    };
    return input;
  }

  answer(request, answer) {
    this.send(request, () => this.onAnswer(request, answer));
  }

  // "sending…" until the request goes from the list; the error if it fails.
  async send(request, sent) {
    if (this.sending.get(request.id) === "sending…") return;
    this.sending.set(request.id, "sending…");
    this.redraw();
    try {
      await sent();
    } catch (error) {
      this.sending.set(request.id, `${error}`);
      this.redraw();
    }
  }
}
