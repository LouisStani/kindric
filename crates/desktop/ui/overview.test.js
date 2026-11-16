// Tests of the control center (decision 343, mockups C2): `node --test crates/desktop/ui/overview.test.js`.
// Rows have the shape of view.rs's SessionRow, requests that of overview.rs's Request, usages that
// of core accounts.rs's AccountUsage.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  HOT_PERCENT,
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
  lineOf,
  nextAsking,
  permissionText,
  projectView,
  projectsOf,
  promptTargets,
  requestsBySession,
  sentText,
  standing,
  summaryOf,
} from "./overview-board.js";

function row(label, tab, extra = {}) {
  return { label, tab, session_id: `s-${label}`, account: "louis", state: "waiting", finished: false, waiting_for: null, progress: null, ...extra };
}

const asking = { waiting_for: "permission prompt" };
const working = { state: "working" };
const done = { finished: true };
const paused = { state: "paused" };

const labels = (rows) => rows.map((each) => each.label);

test("a session stands where the user sees it", () => {
  assert.equal(standing(row("a", 1, asking)), "need");
  assert.equal(standing(row("a", 1, { ...asking, state: "working" })), "need", "a question on screen comes first");
  assert.equal(standing(row("a", 1, done)), "done");
  assert.equal(standing(row("a", 1, working)), "working");
  assert.equal(standing(row("a", 1)), "rest");
  assert.equal(standing(row("a", 1, paused)), "paused");
  assert.equal(standing(row("a", 1, { state: "elsewhere" })), "elsewhere");
});

test("the summary counts each group, paused too, and elsewhere in none", () => {
  const rows = [row("a", 1, asking), row("b", 1, working), row("c", 2, working), row("d", 2, done), row("e", 2), row("f", 2, paused), row("g", 2, { state: "elsewhere" })];
  assert.deepEqual(summaryOf(rows), { need: 1, working: 2, done: 1, rest: 1, paused: 1 });
});

const tabs = [
  { id: 1, name: "other" },
  { id: 2, name: "kindric" },
  { id: 3, name: "merkio" },
  { id: 4, name: "suivi42" },
];

test("the side lists all, then every tab in the bar's order, with its dots most urgent first and its needs", () => {
  const rows = [row("rest", 2), row("stopped", 2, paused), row("asks", 2, asking), row("tests", 3, working), row("notes", 1, done)];
  const entries = projectsOf(rows, tabs);
  assert.deepEqual(entries.map((entry) => entry.name), ["all", "other", "kindric", "merkio", "suivi42"]);
  assert.deepEqual(entries[2].standings, ["need", "rest", "paused"]);
  assert.deepEqual(entries.map((entry) => entry.needs), [1, 0, 1, 0, 0]);
  assert.deepEqual(entries[0].standings, ["need", "working", "done", "rest", "paused"], "all gathers every project");
  assert.deepEqual(entries[4].standings, [], "a project without sessions is still listed");
});

test("all projects: what needs you first, from any project, then a group per project in the bar's order", () => {
  const rows = [row("rest", 2), row("api", 3, asking), row("merge", 2, asking), row("stopped", 2, paused), row("pricing", 3, working)];
  const { needs, groups } = allView(rows, tabs);
  assert.deepEqual(labels(needs), ["api", "merge"], "the list's order among those that need you");
  assert.deepEqual(groups.map((group) => group.tab.name), ["kindric", "merkio"], "a project left with nothing is not shown");
  assert.deepEqual(labels(groups[0].rows), ["rest", "stopped"]);
  assert.deepEqual(labels(groups[1].rows), ["pricing"]);
});

test("one project: its sessions most urgent first, the paused ones apart", () => {
  const rows = [row("rest", 2), row("stopped", 2, paused), row("working", 2, working), row("asks", 2, asking), row("elsewhere", 3, working)];
  const { active, paused: stopped } = projectView(rows, 2);
  assert.deepEqual(labels(active), ["asks", "working", "rest"]);
  assert.deepEqual(labels(stopped), ["stopped"]);
});

test("a row says how many files its session kept and how many are new", () => {
  assert.equal(keptLabel(0, 0), "nothing kept");
  assert.equal(keptLabel(1, 0), "1 kept");
  assert.equal(keptLabel(3, 2), "3 kept · 2 new");
  assert.equal(keptLabel(0, 1), "1 kept · 1 new", "a new file not counted yet is still kept");
});

test("an account is shown with sessions running or figures, framed from 80% of its five hours", () => {
  const rows = [row("a", 1), row("b", 1, { account: "pro" }), row("c", 1, { account: "louis", ...paused }), row("d", 1, { account: "old", ...paused })];
  const usages = [
    { name: "louis", five_hour: 52, seven_day: 19, five_hour_resets_at: null, updated_at: 1 },
    { name: "pro", five_hour: HOT_PERCENT, seven_day: 63, five_hour_resets_at: 2, updated_at: 1 },
    { name: "lystriq", five_hour: 12, seven_day: 40, five_hour_resets_at: null, updated_at: 1 },
  ];
  const cards = accountCards(rows, ["louis", "lystriq", "unused"], usages);
  assert.deepEqual(cards.map((card) => [card.name, card.sessions, card.hot]), [
    ["louis", 1, false],
    ["lystriq", 0, false],
    ["pro", 1, true],
  ], "the registry's order first; a paused session does not count; an account with neither is left out");
});

test("a prompt reaches the project's sessions kindric runs, not the paused ones nor those elsewhere", () => {
  const rows = [
    row("rest", 2, { short_id: "aaaa" }),
    row("stopped", 2, { short_id: "bbbb", ...paused }),
    row("terminal", 2, { short_id: null, ...working }),
    row("working", 2, { short_id: "cccc", ...working }),
    row("other project", 3, { short_id: "dddd" }),
  ];
  assert.deepEqual(labels(promptTargets(rows, 2)), ["working", "rest"], "most urgent first, as shown");
  assert.equal(canPrompt(rows[1]), false);
  assert.equal(canPrompt(rows[2]), false);
});

test("prompt all says how many it reached and why the others failed", () => {
  assert.equal(sentText([{ label: "a", error: null }, { label: "b", error: null }]), "sent to 2 of 2");
  assert.equal(sentText([{ label: "a", error: null }, { label: "pricing page", error: "busy elsewhere" }]), "sent to 1 of 2 · pricing page: busy elsewhere");
  assert.equal(sentText([{ label: "a", error: "x" }]), "sent to 0 of 1 · a: x");
});

test("a project counts the files its sessions changed, each session on its own", () => {
  const rows = [row("a", 2, { changed: 3 }), row("b", 2, { changed: 0 }), row("c", 2, { changed: null }), row("d", 2, { changed: 1 })];
  const { total, sessions } = changedOf(rows);
  assert.equal(total, 4);
  assert.deepEqual(labels(sessions), ["a", "d"], "a session without a count, or with none, is left out");
  assert.deepEqual(changedOf([]), { total: 0, sessions: [] });
});

test("a group of all says what its project changed and kept, only what there is", () => {
  assert.equal(groupNote(4, 2), "4 files changed · 2 new artifacts");
  assert.equal(groupNote(1, 0), "1 file changed");
  assert.equal(groupNote(0, 1), "1 new artifact");
  assert.equal(groupNote(0, 0), "");
});

test("a figure is singular for one only", () => {
  assert.equal(figureText(1, "worktree", "worktrees"), "1 worktree");
  assert.equal(figureText(3, "worktree", "worktrees"), "3 worktrees");
  assert.equal(figureText(0, "worktree", "worktrees"), "0 worktrees");
});

test("the last screen says how long ago the session wrote", () => {
  assert.equal(ageText(1000, 1003), "now");
  assert.equal(ageText(1000, 1020), "20 s ago");
  assert.equal(ageText(1000, 1000 + 4 * 60 + 30), "4 min ago");
  assert.equal(ageText(1000, 1000 + 2 * 3600 + 5), "2 h ago");
  assert.equal(ageText(1000, 1000 + 3 * 86400), "3 d ago");
  assert.equal(ageText(1010, 1000), "now", "a clock a little ahead is not in the future");
});

test("a kept file says when: now, the time today, else the date", () => {
  const at = (...parts) => new Date(...parts).getTime() / 1000;
  const now = at(2026, 10, 16, 21, 30);
  assert.equal(keptTime(now - 20, now), "now");
  assert.equal(keptTime(at(2026, 10, 16, 14, 2), now), "14:02");
  assert.equal(keptTime(at(2026, 10, 15, 23, 59), now), "15/11/2026");
});

test("a kept file shows its extension", () => {
  assert.equal(extensionOf("control-center-c.PNG"), "png");
  assert.equal(extensionOf("deck.html"), "html");
  assert.equal(extensionOf("Makefile"), "file");
  assert.equal(extensionOf(".env"), "file");
  assert.equal(extensionOf("odd."), "file");
});

test("a context bar turns amber from 60%, red from 80%", () => {
  assert.equal(contextLevel(41), "low");
  assert.equal(contextLevel(60), "mid");
  assert.equal(contextLevel(79.9), "mid");
  assert.equal(contextLevel(80), "high");
});

test("n goes to the next session that asks, round, from anywhere", () => {
  const asking = ["s-a", "s-b", "s-c"];
  assert.equal(nextAsking(asking, null), "s-a", "from no row: the first");
  assert.equal(nextAsking(asking, "s-a"), "s-b");
  assert.equal(nextAsking(asking, "s-c"), "s-a");
  assert.equal(nextAsking(asking, "s-quiet"), "s-a", "from a row that asks nothing: the first");
  assert.equal(nextAsking([], "s-a"), null);
});

test("a row says what the session last said, else what it waits for", () => {
  assert.equal(lineOf(row("a", 1, { ...working, progress: "running the tests" })), "running the tests");
  assert.equal(lineOf(row("a", 1, asking)), "waits for you: permission prompt");
  assert.equal(lineOf(row("a", 1)), "at rest");
  assert.equal(lineOf(row("a", 1, paused)), "paused");
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
  assert.equal(lineOf(board, requests), "waits for your answer", "no question of Claude Code on screen to quote");
  const permission = requestsBySession([request("3-1", "s-merge", { kind: "permission", tool: "Bash", input: { command: "git merge main" } })]);
  assert.equal(lineOf(row("merge", 1, { session_id: "s-merge" }), permission), "waits for your permission");
  assert.deepEqual(summaryOf([board, other], requests), { need: 1, working: 0, done: 0, rest: 1, paused: 0 });
  assert.deepEqual(labels(projectView([other, board], 1, requests).active), ["leaderboard", "notes"]);
  assert.deepEqual(labels(allView([other, board], [{ id: 1, name: "arcade" }], requests).needs), ["leaderboard"]);
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

test("the files a session kept and not seen are named, the latest first, three at most", () => {
  const kept = (name) => ({ session_id: "s-logo", project: "/home/u/socials", group: null, name, image: true });
  assert.equal(keptText([kept("logo.png")]), "kept logo.png");
  const four = ["a.png", "b.png", "c.png", "d.png"].map(kept);
  assert.equal(keptText(four), "kept d.png, c.png, b.png +1");
  assert.deepEqual([...requestsBySession(four).keys()], ["s-logo"], "grouped by session like the requests");
});
