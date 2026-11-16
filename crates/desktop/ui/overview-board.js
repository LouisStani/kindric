// What the overview shows, without the page: where each session goes and what the summary
// counts. Tested by `node --test crates/desktop/ui/overview.test.js`.

// The summary's groups, in the order of the summary and of the cards in a column.
export const STANDINGS = [
  { id: "need", label: "need you" },
  { id: "working", label: "working" },
  { id: "done", label: "done, not seen" },
  { id: "rest", label: "at rest" },
];

// Where a session of the list stands for the user. It needs them while a question of Claude
// Code is on its screen, or while it has requests in the overview (`requests` by session id,
// from `requestsBySession`). "elsewhere": shown in a terminal outside kindric, whose activity
// kindric does not know; counted in no group.
export function standing(row, requests = new Map()) {
  if (row.waiting_for || requests.has(row.session_id)) return "need";
  if (row.finished) return "done";
  if (row.state === "working") return "working";
  if (row.state === "waiting") return "rest";
  return "elsewhere";
}

const ORDER = new Map([...STANDINGS.map((group, index) => [group.id, index]), ["elsewhere", STANDINGS.length]]);

// `cards` most urgent first; the list's own order within a group.
function sorted(cards, requests) {
  return cards
    .map((card, index) => ({ card, index }))
    .sort((a, b) => ORDER.get(standing(a.card.row, requests)) - ORDER.get(standing(b.card.row, requests)) || a.index - b.index)
    .map(({ card }) => card);
}

function needs(column, requests) {
  return column.cards.filter((card) => standing(card.row, requests) === "need").length;
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

// The columns of the board: one per tab with sessions, `tabs` being `[{ id, name }]` in the
// order of the bar. Those that need the user most come first, then the busiest. Past `most`
// columns, the last one gathers the others, the smallest, and its cards then name their project.
export function columnsOf(rows, tabs, most, requests = new Map()) {
  const columns = tabs
    .map((tab) => ({ names: [tab.name], cards: rows.filter((row) => row.tab === tab.id).map((row) => ({ row, project: null })) }))
    .filter((column) => column.cards.length > 0)
    .map((column) => ({ ...column, cards: sorted(column.cards, requests) }))
    .sort((a, b) => needs(b, requests) - needs(a, requests) || b.cards.length - a.cards.length);
  if (columns.length <= Math.max(most, 1)) return columns;
  const kept = columns.slice(0, Math.max(most, 1) - 1);
  const gathered = columns.slice(kept.length);
  const cards = gathered.flatMap((column) => column.cards.map((card) => ({ ...card, project: column.names[0] })));
  return [...kept, { names: gathered.flatMap((column) => column.names), cards: sorted(cards, requests) }];
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

// The line under a card's name: what the session last said it does, else what it waits for.
export function lineOf(row, requests = new Map()) {
  if (row.progress) return row.progress;
  switch (standing(row, requests)) {
    case "need":
      return `waits for you: ${row.waiting_for}`;
    case "working":
      return "working";
    case "done":
      return "finished a turn";
    case "rest":
      return "at rest";
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
