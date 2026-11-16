// Tests of the overview's board (decision 331, page B): `node --test crates/desktop/ui/overview.test.js`.
// Rows have the shape of view.rs's SessionRow.

import { test } from "node:test";
import assert from "node:assert/strict";
import { columnsOf, lineOf, standing, summaryOf } from "./overview-board.js";

function row(label, tab, extra = {}) {
  return { label, tab, state: "waiting", finished: false, waiting_for: null, progress: null, ...extra };
}

const asking = { waiting_for: "permission prompt" };
const working = { state: "working" };
const done = { finished: true };

test("a session stands where the user sees it", () => {
  assert.equal(standing(row("a", 1, asking)), "need");
  assert.equal(standing(row("a", 1, { ...asking, state: "working" })), "need", "a question on screen comes first");
  assert.equal(standing(row("a", 1, done)), "done");
  assert.equal(standing(row("a", 1, working)), "working");
  assert.equal(standing(row("a", 1)), "rest");
  assert.equal(standing(row("a", 1, { state: "elsewhere" })), "elsewhere");
});

test("the summary counts each group, and elsewhere in none", () => {
  const rows = [row("a", 1, asking), row("b", 1, working), row("c", 2, working), row("d", 2, done), row("e", 2), row("f", 2, { state: "elsewhere" })];
  assert.deepEqual(summaryOf(rows), { need: 1, working: 2, done: 1, rest: 1 });
});

test("a column per tab with sessions, those that need you first, then the busiest, urgent cards on top", () => {
  const tabs = [
    { id: 1, name: "other" },
    { id: 2, name: "kindric" },
    { id: 3, name: "arcade" },
    { id: 4, name: "empty" },
  ];
  const rows = [row("rest", 2), row("working", 2, working), row("leaderboard", 3, asking), row("notes", 1), row("server", 3, asking)];
  const columns = columnsOf(rows, tabs, 4);
  assert.deepEqual(columns.map((column) => column.names), [["arcade"], ["kindric"], ["other"]]);
  assert.deepEqual(columns[1].cards.map((card) => card.row.label), ["working", "rest"]);
  assert.deepEqual(columns[0].cards.map((card) => card.row.label), ["leaderboard", "server"], "the list's order within a group");
  assert.ok(columns.every((column) => column.cards.every((card) => card.project === null)));
});

test("past the room, the last column gathers the smallest and names their project", () => {
  const tabs = ["other", "pro", "kindric", "app", "socials"].map((name, index) => ({ id: index + 1, name }));
  const rows = [row("a", 1), row("b", 2, asking), row("c", 3), row("c2", 3, working), row("d", 4, done), row("e", 5)];
  const columns = columnsOf(rows, tabs, 3);
  assert.deepEqual(columns.map((column) => column.names), [["pro"], ["kindric"], ["other", "app", "socials"]]);
  assert.deepEqual(
    columns[2].cards.map((card) => [card.row.label, card.project]),
    [["d", "app"], ["a", "other"], ["e", "socials"]],
  );
  assert.equal(columnsOf(rows, tabs, 0).length, 1, "at least one column");
});

test("a card says what the session last said, else what it waits for", () => {
  assert.equal(lineOf(row("a", 1, { ...working, progress: "running the tests" })), "running the tests");
  assert.equal(lineOf(row("a", 1, asking)), "waits for you: permission prompt");
  assert.equal(lineOf(row("a", 1)), "at rest");
});
