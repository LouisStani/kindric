// Tests of the overview's board (decision 331, page B): `node --test crates/desktop/ui/overview.test.js`.
// Rows have the shape of view.rs's SessionRow, requests that of overview.rs's Request.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  answersInOneClick,
  choicesAnswer,
  columnsOf,
  lineOf,
  permissionText,
  requestsBySession,
  standing,
  summaryOf,
} from "./overview-board.js";

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

function request(id, session_id, body) {
  return { id, session_id, cwd: "/home/u/arcade", asked_at: 1, ...body };
}

test("a session with a request needs you, even one that keeps working on a kindric ask", () => {
  const requests = requestsBySession([
    request("1-1", "s-board", { kind: "ask", question: "UTC?", context: "Scores store UTC." }),
    request("2-1", "s-board", { kind: "ask", question: "Keep the old board?", context: "Reset on Monday." }),
  ]);
  assert.deepEqual(requests.get("s-board").map((r) => r.id), ["1-1", "2-1"], "oldest first");
  const board = row("leaderboard", 1, { ...working, session_id: "s-board" });
  const other = row("notes", 1, { session_id: "s-notes" });
  assert.equal(standing(board, requests), "need");
  assert.equal(standing(other, requests), "rest");
  assert.deepEqual(summaryOf([board, other], requests), { need: 1, working: 0, done: 0, rest: 1 });
  const columns = columnsOf([other, board], [{ id: 1, name: "arcade" }], 4, requests);
  assert.deepEqual(columns[0].cards.map((card) => card.row.label), ["leaderboard", "notes"]);
});

test("a permission shows the command alone, another tool what it touches", () => {
  assert.deepEqual(permissionText({ tool: "Bash", input: { command: "cargo test", description: "Run the tests" } }), { prompt: "$", text: "cargo test" });
  assert.deepEqual(permissionText({ tool: "Write", input: { file_path: "/w/a.txt", content: "x" } }), { prompt: "Write", text: "/w/a.txt" });
  assert.deepEqual(permissionText({ tool: "mcp__x__y", input: { n: 1 } }), { prompt: "mcp__x__y", text: '{"n":1}' });
});

test("the answer to AskUserQuestion waits for every question, labels joined, Other typed", () => {
  const questions = [
    { question: "Red or blue?", header: "Color", options: [{ label: "Red" }, { label: "Blue" }], multiSelect: false },
    { question: "Which toppings?", header: "Toppings", options: [{ label: "cheese" }, { label: "olives" }], multiSelect: true },
  ];
  const chosen = new Map([["Red or blue?", ["Blue"]]]);
  assert.equal(choicesAnswer(questions, chosen), null);
  chosen.set("Which toppings?", ["cheese", "olives"]);
  assert.deepEqual(choicesAnswer(questions, chosen), { "Red or blue?": "Blue", "Which toppings?": "cheese, olives" });
  chosen.set("Red or blue?", "  teal  ");
  assert.equal(choicesAnswer(questions, chosen)["Red or blue?"], "teal");
  chosen.set("Red or blue?", " ");
  assert.equal(choicesAnswer(questions, chosen), null, "an empty Other is no answer");
  assert.equal(answersInOneClick(questions), false);
  assert.equal(answersInOneClick(questions.slice(0, 1)), true);
  assert.equal(answersInOneClick(questions.slice(1)), false);
});
