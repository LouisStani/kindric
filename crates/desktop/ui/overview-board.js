// What the control center shows (decision 343, mockups C2), without the page: where each session
// goes, what the summary counts, the projects and accounts of the side. Tested by
// `node --test crates/desktop/ui/overview.test.js`.

// The summary's groups, in the order of the summary and of the rows of a project.
export const STANDINGS = [
  { id: "need", label: "need you" },
  { id: "working", label: "working" },
  { id: "done", label: "done, not seen" },
  { id: "rest", label: "at rest" },
  { id: "paused", label: "paused" },
];

// Where a session of the list stands for the user. It needs them while a question of Claude
// Code is on its screen, or while it has requests in the overview (`requests` by session id,
// from `requestsBySession`). "paused": stopped, no process. "elsewhere": shown in a terminal
// outside kindric, whose activity kindric does not know; counted in no group.
export function standing(row, requests = new Map()) {
  if (row.waiting_for || requests.has(row.session_id)) return "need";
  if (row.finished) return "done";
  if (row.state === "working") return "working";
  if (row.state === "waiting") return "rest";
  if (row.state === "paused") return "paused";
  return "elsewhere";
}

const ORDER = new Map([...STANDINGS.map((group, index) => [group.id, index]), ["elsewhere", STANDINGS.length]]);

// `rows` most urgent first; the list's own order within a group.
export function urgentFirst(rows, requests = new Map()) {
  return rows
    .map((row, index) => ({ row, index }))
    .sort((a, b) => ORDER.get(standing(a.row, requests)) - ORDER.get(standing(b.row, requests)) || a.index - b.index)
    .map(({ row }) => row);
}

function needCount(rows, requests) {
  return rows.filter((row) => standing(row, requests) === "need").length;
}

// The side's list: "all", then one entry per tab of `tabs` (`[{ id, name }]`, in the order of the
// bar), each with the standing of its sessions, most urgent first, and how many need you.
export function projectsOf(rows, tabs, requests = new Map()) {
  const entry = (id, name, mine) => ({
    id,
    name,
    standings: urgentFirst(mine, requests).map((row) => standing(row, requests)),
    needs: needCount(mine, requests),
  });
  return [entry("all", "all", rows), ...tabs.map((tab) => entry(tab.id, tab.name, rows.filter((row) => row.tab === tab.id)))];
}

// The view of every project: the sessions that need you first, whatever their project, then a
// group per tab with its other sessions; a tab with none is left out.
export function allView(rows, tabs, requests = new Map()) {
  const needing = (row) => standing(row, requests) === "need";
  const groups = tabs
    .map((tab) => ({ tab, rows: urgentFirst(rows.filter((row) => row.tab === tab.id && !needing(row)), requests) }))
    .filter((group) => group.rows.length > 0);
  return { needs: urgentFirst(rows.filter(needing), requests), groups };
}

// The view of one tab: its sessions most urgent first, the paused ones apart, at the bottom.
export function projectView(rows, tabId, requests = new Map()) {
  const mine = urgentFirst(rows.filter((row) => row.tab === tabId), requests);
  const paused = (row) => standing(row, requests) === "paused";
  return { active: mine.filter((row) => !paused(row)), paused: mine.filter(paused) };
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

// The accounts of the side: those of `accounts` (the registry's order), then any other a session
// runs on; one is kept with sessions running or figures. `usages` as `control_figures` gives them.
export function accountCards(rows, accounts, usages) {
  const names = new Set([...accounts, ...rows.map((row) => row.account)]);
  return [...names]
    .map((name) => {
      const usage = usages.find((candidate) => candidate.name === name) ?? null;
      const sessions = rows.filter((row) => row.account === name && row.state !== "paused").length;
      return { name, sessions, usage, hot: usage !== null && usage.five_hour >= HOT_PERCENT };
    })
    .filter((card) => card.sessions > 0 || card.usage !== null);
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

// What a row says the session does: what it last said, else what it waits for.
export function lineOf(row, requests = new Map()) {
  if (row.progress) return row.progress;
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
