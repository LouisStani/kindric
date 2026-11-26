// What the control center shows (decision 343, mockups C2), without the page: where each session
// goes, what the summary counts, the projects and accounts of the side. Tested by
// `node --test crates/desktop/ui/overview.test.js`.

// The summary's groups, in the order of the summary and of the rows of a project.
export const STANDINGS = [
  { id: "need", label: "need you" },
  { id: "working", label: "working" },
  { id: "done", label: "done, not seen" },
  { id: "rest", label: "at rest" },
];

// A session the bar shows, and with it the control center (decision 497): not hidden, and with a
// process. Stopped ones (dozens, from `agents --json --all`) are found with Ctrl+P only.
export function inBar(session) {
  return !session.hidden && session.state !== "paused";
}

// Where a session of the list stands for the user. It needs them while a question of Claude
// Code is on its screen, or while it has requests in the overview (`requests` by session id,
// from `requestsBySession`). "paused": stopped, no process. "elsewhere": shown in a terminal
// outside kindric, whose activity kindric does not know; counted in no group.
export function standing(row, requests = new Map()) {
  if (row.asks || row.waiting_for || requests.has(row.session_id)) return "need";
  if (row.finished) return "done";
  if (row.state === "working") return "working";
  if (row.state === "waiting") return "rest";
  if (row.state === "paused") return "paused";
  return "elsewhere";
}

// Need you, working, done, at rest; paused and elsewhere, which the control center does not show
// (decision 497), last.
const ORDER = new Map([["need", 0], ["working", 1], ["done", 2], ["rest", 3], ["paused", 4], ["elsewhere", 5]]);

// `rows` most urgent first, and within a group the last active first (decisions 349, 350, 354);
// the list's own order among those with the same time.
export function urgentFirst(rows, requests = new Map()) {
  return rows
    .map((row, index) => ({ row, index, rank: ORDER.get(standing(row, requests)) }))
    .sort((a, b) => a.rank - b.rank || (b.row.last_active ?? 0) - (a.row.last_active ?? 0) || a.index - b.index)
    .map(({ row }) => row);
}

function needCount(rows, requests) {
  return rows.filter((row) => standing(row, requests) === "need").length;
}

// Whether the tab shows in "all": a project the user left out of it does not (decision 351);
// "other" always does.
function inAll(tab) {
  return tab.inAll !== false;
}

// The sessions "all" shows: those of the tabs in it, and from the others those that need you.
export function shownInAll(rows, tabs, requests = new Map()) {
  const out = new Set(tabs.filter((tab) => !inAll(tab)).map((tab) => tab.id));
  return rows.filter((row) => !out.has(row.tab) || standing(row, requests) === "need");
}

// The side's list: "all", then one entry per tab of `tabs` (`[{ id, name, inAll }]`, in the
// order of the bar), each with the standing of its sessions, most urgent first, how many need
// you, and whether it shows in "all".
export function projectsOf(rows, tabs, requests = new Map()) {
  const entry = (id, name, mine, shown) => ({
    id,
    name,
    standings: urgentFirst(mine, requests).map((row) => standing(row, requests)),
    needs: needCount(mine, requests),
    inAll: shown,
  });
  return [
    entry("all", "all", shownInAll(rows, tabs, requests), true),
    ...tabs.map((tab) => entry(tab.id, tab.name, rows.filter((row) => row.tab === tab.id), inAll(tab))),
  ];
}

// The view of every project: the sessions that need you first, whatever their project, then a
// group per tab in "all" with its other sessions, most urgent first; a tab with none is left out.
export function allView(rows, tabs, requests = new Map()) {
  const needing = (row) => standing(row, requests) === "need";
  const groups = tabs
    .filter(inAll)
    .map((tab) => ({ tab, rows: urgentFirst(rows.filter((row) => row.tab === tab.id && !needing(row)), requests) }))
    .filter((group) => group.rows.length > 0);
  return { needs: urgentFirst(rows.filter(needing), requests), groups };
}

// The view of one tab: its sessions, most urgent first.
export function projectView(rows, tabId, requests = new Map()) {
  return urgentFirst(rows.filter((row) => row.tab === tabId), requests);
}

// What a row says of the files its session kept for the user: `kept` in all, `fresh` of them not
// seen yet. The new ones are noted apart, so for a moment they can outnumber the others.
export function keptLabel(kept, fresh) {
  const total = Math.max(kept, fresh);
  if (total === 0) return "nothing kept";
  return fresh > 0 ? `${total} kept · ${fresh} new` : `${total} kept`;
}

// From this share of its five hours, an account's card is framed (mockup C2).
export const HOT_PERCENT = 80;

// How long a window lasts, in words: "5h", "7d", "30d".
export function windowText(minutes) {
  if (minutes === null) return "limit";
  if (minutes % 1440 === 0) return `${minutes / 1440}d`;
  return minutes % 60 === 0 ? `${minutes / 60}h` : `${minutes}min`;
}

// The limits of a Claude subscription, as a card shows them.
function claudeBars(usage) {
  return [
    { label: "5h", percent: usage.five_hour, resetsAt: usage.five_hour_resets_at },
    { label: "7d", percent: usage.seven_day, resetsAt: null },
  ];
}

// The accounts of the side: those of `accounts` (the registry's order), then any other a session
// runs on; one is kept with sessions running, figures, or a model that is not a Claude login.
// `usages`, `providers` and `limits` as `control_figures` gives them; `provider` is "claude" for a
// Claude login, else the provider and model ("openrouter qwen3"), whose account has no five-hour or
// weekly limits (decision 417), or the agent ("codex"), whose limits are its own (decision 464).
// `bars` are the limits, shortest window first (the first one frames the card); `note` says why
// there are none.
export function accountCards(rows, accounts, usages, providers = {}, limits = {}) {
  const names = new Set([...accounts, ...rows.map((row) => row.account)]);
  return [...names]
    .map((name) => {
      const usage = usages.find((candidate) => candidate.name === name) ?? null;
      const sessions = rows.filter((row) => row.account === name && row.state !== "paused").length;
      const provider = providers[name] ?? null;
      const ofAgent = name in limits;
      let bars = [];
      if (usage) bars = claudeBars(usage);
      else if (ofAgent) bars = (limits[name] ?? []).map((limit) => ({ label: windowText(limit.minutes), percent: limit.percent, resetsAt: limit.resets_at }));
      let note = null;
      if (bars.length === 0) {
        if (ofAgent) note = limits[name] === null ? "no figures yet" : "no limits";
        else note = provider === null ? "no figures yet" : "no 5h or 7d limits";
      }
      return { name, sessions, provider: provider ?? "claude", bars, note, hot: bars.length > 0 && bars[0].percent >= HOT_PERCENT };
    })
    .filter((card) => card.sessions > 0 || card.bars.length > 0 || card.provider !== "claude");
}

// Whether a prompt can be typed into a session from here: one kindric runs in the background
// (`short_id`), with a process.
export function canPrompt(row) {
  return Boolean(row.short_id) && row.state !== "paused";
}

// The sessions "prompt all" types into: those of the tab a prompt can reach, in the order shown.
export function promptTargets(rows, tabId, requests = new Map()) {
  return projectView(rows, tabId, requests).filter(canPrompt);
}

// What "prompt all" says once done: `results` are `{ label, error }`, `error` null when sent.
export function sentText(results) {
  const failed = results.filter((result) => result.error !== null);
  const head = `sent to ${results.length - failed.length} of ${results.length}`;
  return [head, ...failed.map((result) => `${result.label}: ${result.error}`)].join(" · ");
}

// "1 worktree", "3 worktrees", "0 worktrees".
export function figureText(count, singular, plural) {
  return `${count} ${count === 1 ? singular : plural}`;
}

// The files the sessions of `rows` changed, each session counted since it started: the sum, and
// the sessions that changed some. A file two sessions touched counts twice.
export function changedOf(rows) {
  const changing = rows.filter((row) => (row.changed ?? 0) > 0);
  return { total: changing.reduce((sum, row) => sum + row.changed, 0), sessions: changing };
}

// What a project's group says on the right in "all": files changed and new artifacts, each only
// when there are some.
export function groupNote(changed, fresh) {
  const parts = [];
  if (changed > 0) parts.push(figureText(changed, "file changed", "files changed"));
  if (fresh > 0) parts.push(figureText(fresh, "new artifact", "new artifacts"));
  return parts.join(" · ");
}

// How long ago `seconds` (Unix) was, at `now` (Unix seconds): "now", "20 s ago", "4 min ago",
// "2 h ago", "3 d ago".
export function ageText(seconds, now) {
  const age = Math.floor(now - seconds);
  // A clock a little ahead gives a negative age: "now" too.
  if (age < 5) return "now";
  if (age < 60) return `${age} s ago`;
  if (age < 3600) return `${Math.floor(age / 60)} min ago`;
  if (age < 86400) return `${Math.floor(age / 3600)} h ago`;
  return `${Math.floor(age / 86400)} d ago`;
}

const twoDigits = (number) => `${number}`.padStart(2, "0");

// When a file was kept, at `now` (both Unix seconds), in the user's time: "now" within a minute,
// "14:02" the same day, else "16/11/2026" as the artifacts window writes dates.
export function keptTime(seconds, now) {
  if (now - seconds < 60) return "now";
  const kept = new Date(seconds * 1000);
  const today = new Date(now * 1000);
  if (kept.toDateString() === today.toDateString()) return `${twoDigits(kept.getHours())}:${twoDigits(kept.getMinutes())}`;
  return `${twoDigits(kept.getDate())}/${twoDigits(kept.getMonth() + 1)}/${kept.getFullYear()}`;
}

// "png" for "logo.png"; "file" without an extension.
export function extensionOf(name) {
  const dot = name.lastIndexOf(".");
  return dot > 0 && dot < name.length - 1 ? name.slice(dot + 1).toLowerCase() : "file";
}

// How a context bar is coloured.
export function contextLevel(percent) {
  if (percent >= 80) return "high";
  if (percent >= 60) return "mid";
  return "low";
}

// The session `n` goes to: the one after `current` among `asking` (the ids of the sessions with a
// request, in the order shown), round; null when none asks.
export function nextAsking(asking, current) {
  if (asking.length === 0) return null;
  return asking[(asking.indexOf(current) + 1) % asking.length];
}

// The requests of `overview_requests` (oldest first) by session id, each list oldest first.
export function requestsBySession(requests) {
  const bySession = new Map();
  for (const request of requests) {
    if (!bySession.has(request.session_id)) bySession.set(request.session_id, []);
    bySession.get(request.session_id).push(request);
  }
  return bySession;
}

// The line of a card for the files its session kept for the user, not yet seen: the latest
// first, `most` names at most, then how many more.
export function keptText(artifacts, most = 3) {
  const names = artifacts.map((artifact) => artifact.name).reverse();
  const shown = names.slice(0, most).join(", ");
  return names.length > most ? `kept ${shown} +${names.length - most}` : `kept ${shown}`;
}

// How many sessions are in each group of the summary.
export function summaryOf(rows, requests = new Map()) {
  const counts = Object.fromEntries(STANDINGS.map((group) => [group.id, 0]));
  for (const row of rows) {
    const id = standing(row, requests);
    if (id in counts) counts[id] += 1;
  }
  return counts;
}

// The time left, read at a glance: "<1 min", "~4 min", "~1 h 05".
export function leftText(seconds) {
  if (seconds < 60) return "<1 min";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `~${minutes} min`;
  return `~${Math.floor(minutes / 60)} h ${`${minutes % 60}`.padStart(2, "0")}`;
}

// How far a session is in its task list (decision 368): how many are done, and the time left once
// one is done. A badge of its own on the row, so that a long task name never hides it.
export function tasksBadge(tasks) {
  const count = `${tasks.done}/${tasks.total}`;
  return tasks.left_seconds != null ? `${count} · ${leftText(tasks.left_seconds)} left` : count;
}

// Whether a row shows its task list: not while it waits for you, which comes first.
export function showsTasks(row, requests = new Map()) {
  return Boolean(row.tasks) && standing(row, requests) !== "need";
}

// The line under a row with a task list: the task in progress and what it is about, as the
// session wrote them.
export function taskLine(tasks) {
  const doing = tasks.doing ?? "between tasks";
  return tasks.about ? `${doing} · ${tasks.about}` : doing;
}

// What a row says the session does: while it works, what it does now as kindric found it
// (decision 499: the last sentence Claude wrote in the turn, else the prompt); else what it last
// said, else what it waits for. A question asked under the row is not said twice.
export function lineOf(row, requests = new Map()) {
  if (row.doing && standing(row, requests) === "working") return row.doing;
  const asked = (requests.get(row.session_id) ?? []).some((request) => request.kind === "ask" && request.question === row.progress);
  if (row.progress && !asked) return row.progress;
  switch (standing(row, requests)) {
    case "need":
      if (row.waiting_for) return `waits for you: ${row.waiting_for}`;
      return (requests.get(row.session_id) ?? []).some((request) => request.kind === "permission") ? "waits for your permission" : "waits for your answer";
    case "working":
      return "working";
    case "done":
      return "finished a turn";
    case "rest":
      return "at rest";
    case "paused":
      return "paused";
    default:
      return "open in a terminal outside kindric";
  }
}

// The fields that say what a tool will touch, for the tools of Claude Code that have one.
const SUBJECTS = ["command", "file_path", "notebook_path", "path", "url", "pattern", "query", "prompt"];

// What a permission shows: the command alone for Bash (decision 327), else the tool and what it
// touches.
export function permissionText(body) {
  const input = body.input ?? {};
  if (body.tool === "Bash" && typeof input.command === "string") return { prompt: "$", text: input.command };
  const subject = SUBJECTS.map((field) => input[field]).find((value) => typeof value === "string");
  return { prompt: body.tool, text: subject ?? JSON.stringify(input) };
}

// The answer to AskUserQuestion's `questions`, from `chosen`: question text → the labels picked,
// or the words typed for "Other…". Null while a question has no answer. Several labels are
// joined by commas, as the session reads them (trial of 2026-11-16).
export function choicesAnswer(questions, chosen) {
  const answers = {};
  for (const question of questions) {
    const picked = chosen.get(question.question);
    const words = typeof picked === "string" ? picked.trim() : (picked ?? []).join(", ");
    if (!words) return null;
    answers[question.question] = words;
  }
  return answers;
}

// Whether one click answers everything: a single question with a single choice.
export function answersInOneClick(questions) {
  return questions.length === 1 && !questions[0].multiSelect;
}
