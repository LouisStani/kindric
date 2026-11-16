// What the overview shows, without the page: where each session goes and what the summary
// counts. Tested by `node --test crates/desktop/ui/overview.test.js`.

// The summary's groups, in the order of the summary and of the cards in a column.
export const STANDINGS = [
  { id: "need", label: "need you" },
  { id: "working", label: "working" },
  { id: "done", label: "done, not seen" },
  { id: "rest", label: "at rest" },
];

// Where a session of the list stands for the user. "elsewhere": shown in a terminal outside
// kindric, whose activity kindric does not know; counted in no group.
export function standing(row) {
  if (row.waiting_for) return "need";
  if (row.finished) return "done";
  if (row.state === "working") return "working";
  if (row.state === "waiting") return "rest";
  return "elsewhere";
}

const ORDER = new Map([...STANDINGS.map((group, index) => [group.id, index]), ["elsewhere", STANDINGS.length]]);

// `cards` most urgent first; the list's own order within a group.
function sorted(cards) {
  return cards
    .map((card, index) => ({ card, index }))
    .sort((a, b) => ORDER.get(standing(a.card.row)) - ORDER.get(standing(b.card.row)) || a.index - b.index)
    .map(({ card }) => card);
}

function needs(column) {
  return column.cards.filter((card) => standing(card.row) === "need").length;
}

// The columns of the board: one per tab with sessions, `tabs` being `[{ id, name }]` in the
// order of the bar. Those that need the user most come first, then the busiest. Past `most`
// columns, the last one gathers the others, the smallest, and its cards then name their project.
export function columnsOf(rows, tabs, most) {
  const columns = tabs
    .map((tab) => ({ names: [tab.name], cards: rows.filter((row) => row.tab === tab.id).map((row) => ({ row, project: null })) }))
    .filter((column) => column.cards.length > 0)
    .map((column) => ({ ...column, cards: sorted(column.cards) }))
    .sort((a, b) => needs(b) - needs(a) || b.cards.length - a.cards.length);
  if (columns.length <= Math.max(most, 1)) return columns;
  const kept = columns.slice(0, Math.max(most, 1) - 1);
  const gathered = columns.slice(kept.length);
  const cards = gathered.flatMap((column) => column.cards.map((card) => ({ ...card, project: column.names[0] })));
  return [...kept, { names: gathered.flatMap((column) => column.names), cards: sorted(cards) }];
}

// How many sessions are in each group of the summary.
export function summaryOf(rows) {
  const counts = Object.fromEntries(STANDINGS.map((group) => [group.id, 0]));
  for (const row of rows) {
    const id = standing(row);
    if (id in counts) counts[id] += 1;
  }
  return counts;
}

// The line under a card's name: what the session last said it does, else what it waits for.
export function lineOf(row) {
  if (row.progress) return row.progress;
  switch (standing(row)) {
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
