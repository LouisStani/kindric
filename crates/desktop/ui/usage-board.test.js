// Tests of the usage window (decision 469): `node --test crates/desktop/ui/usage-board.test.js`.
// Summaries have the shape of core stats.rs's Summary, histories that of usage.rs's Usage.

import { test } from "node:test";
import assert from "node:assert/strict";
import { ROWS_SHOWN, byProject, bySession, cacheColumns, costOf, costText, newTokens, projectName, projectOf, tokensText, total, weekOf, weekText } from "./usage-board.js";

const NOW = 1_795_600_000;
const HOUR = Math.floor(NOW / 3600);
const DAY = 86_400;

function tokens(output, extra = {}) {
  return { input: 0, output, cache_read: 0, cache_creation: 0, thinking: 0, web_searches: 0, replies: 1, prompts: 0, ...extra };
}

function hour(ago, project, figures, cost = 0) {
  return { hour: HOUR - ago, account: "louis", project, tokens: figures, models: {}, cost };
}

test("tokens are short", () => {
  assert.deepEqual([0, 812, 999, 1000, 94_256, 999_400, 999_999, 14_567_429, 1_913_741_616].map(tokensText), ["0", "812", "999", "1k", "94k", "999k", "1.0M", "14.6M", "1.9B"]);
});

test("a worktree counts for its checkout", () => {
  assert.equal(projectOf("/home/u/kindric/.claude/worktrees/leak"), "/home/u/kindric");
  assert.equal(projectOf("C:\\u\\kindric\\.claude\\worktrees\\leak"), "C:\\u\\kindric");
  assert.equal(projectOf("/home/u/kindric"), "/home/u/kindric");
  assert.deepEqual([projectName("/home/u/kindric"), projectName("C:\\u\\kindric"), projectName("")], ["kindric", "kindric", "no folder"]);
});

test("projects over a period, the most new tokens first, cache read apart", () => {
  const hours = [
    hour(1, "/p/small", tokens(5, { cache_read: 9_000 })),
    hour(2, "/p/big", tokens(10, { input: 1, cache_creation: 4 })),
    hour(3, "/p/big/.claude/worktrees/x", tokens(20)),
    hour(7 * 24, "/p/old", tokens(1_000)),
    hour(1, "/p/nothing", tokens(0)),
  ];
  const rows = byProject({ hours, sessions: [] }, 7, NOW);
  assert.deepEqual(rows.map((row) => [row.name, newTokens(row.tokens), row.tokens.cache_read]), [["big", 35, 0], ["small", 5, 9_000]]);
  assert.equal(byProject({ hours, sessions: [] }, 30, NOW)[0].name, "old");
  assert.equal(total({ hours, sessions: [] }, 7, NOW).output, 35);
});

test("past the lines shown, the others take one", () => {
  const hours = Array.from({ length: ROWS_SHOWN + 2 }, (_, index) => hour(1, `/p/${index}`, tokens(100 - index)));
  const rows = byProject({ hours, sessions: [] }, 7, NOW);
  assert.equal(rows.length, ROWS_SHOWN + 1);
  assert.deepEqual([rows.at(-1).name, rows.at(-1).rest, newTokens(rows.at(-1).tokens)], ["2 other projects", true, 100 - ROWS_SHOWN + 100 - ROWS_SHOWN - 1]);
});

test("the sessions active in the period, with their names", () => {
  const session = (id, endedAgo, output) => ({ account: "louis", session: id, project: "/p/kindric", tokens: tokens(output), cost: output * 1000, spans: [{ start: 0, end: (NOW - endedAgo) * 1000 }] });
  const sessions = [session("aaaaaaaa-1", DAY, 5), session("bbbbbbbb-2", 8 * DAY, 50), session("cccccccc-3", 60, 9)];
  const rows = bySession({ hours: [], sessions }, 7, NOW, (account, id) => (id === "cccccccc-3" ? "parser" : null));
  assert.deepEqual(rows.map((row) => [row.name, row.project, row.title]), [["parser", "kindric", "louis"], ["aaaaaaaa", "kindric", "louis"]]);
  assert.equal(bySession({ hours: [], sessions }, 30, NOW, () => null).length, 3);
});

test("what was paid by use", () => {
  assert.deepEqual([0, 1, 4_999, 12_000, 420_000, 99_990_000, 123_400_000].map(costText), ["", "<$0.01", "<$0.01", "$0.01", "$0.42", "$99.99", "$123"]);
  const hours = [hour(1, "/p/shop", tokens(5), 16_800), hour(2, "/p/shop/.claude/worktrees/x", tokens(5), 3_200), hour(1, "/p/free", tokens(50))];
  const rows = byProject({ hours, sessions: [] }, 7, NOW);
  assert.deepEqual(rows.map((row) => [row.name, row.cost]), [["free", 0], ["shop", 20_000]]);
  assert.equal(costOf(rows), 20_000);
});

test("the cache written, an hour a column over a week, a day over a month", () => {
  const hours = [hour(0, "/p", tokens(0, { cache_creation: 7 })), hour(0, "/q", tokens(0, { cache_creation: 1 })), hour(2, "/p", tokens(0, { cache_creation: 5 }))];
  const week = cacheColumns({ hours, sessions: [] }, 7, NOW);
  assert.equal(week.length, 168);
  assert.deepEqual(week.slice(-3).map((column) => column.tokens), [5, 0, 8]);
  assert.equal(week.at(-1).at, HOUR * 3600);
  const month = cacheColumns({ hours, sessions: [] }, 30, NOW);
  assert.ok(month.length === 30 || month.length === 31, `${month.length} days`);
  assert.equal(month.reduce((all, column) => all + column.tokens, 0), 13);
});

// A week that started `startedAgo` seconds ago, read `readAgo` seconds ago.
function history(percent, startedAgo, readAgo = 0) {
  return [{ five_hour: null, seven_day: { used_percentage: percent, resets_at: NOW - startedAgo + 7 * DAY }, updated_at: NOW - readAgo }];
}

test("a week is projected at its pace, from a day on", () => {
  assert.equal(weekOf([], NOW), null);
  assert.equal(weekOf([{ five_hour: null, seven_day: null, updated_at: NOW }], NOW), null);
  assert.equal(weekOf(history(50, DAY - 1), NOW).kind, "early");
  const holds = weekOf(history(10, DAY), NOW);
  assert.deepEqual([holds.kind, holds.atReset], ["holds", 70]);
  const full = weekOf(history(50, 2 * DAY), NOW);
  assert.deepEqual([full.kind, full.fullAt], ["full-at", NOW + 2 * DAY]);
  assert.equal(weekOf(history(100, 2 * DAY), NOW).kind, "full");
});

test("old figures say so, and project nothing already past", () => {
  const old = weekOf(history(50, 3 * DAY, 2 * DAY), NOW);
  assert.deepEqual([old.kind, old.old], ["old", true], "read after 1 day at 50%: full a day ago at that pace");
  assert.equal(weekOf(history(5, 3 * DAY, 2 * DAY), NOW).kind, "holds");
  assert.equal(weekOf(history(5, 8 * DAY, 2 * DAY), NOW).kind, "over");
  assert.equal(weekOf(history(5, 2 * DAY, 600), NOW).old, false);
});

test("a week in words", () => {
  const at = (seconds) => `@${seconds - NOW}`;
  const said = (...args) => weekText(weekOf(history(...args), NOW), at);
  assert.equal(said(10, DAY), `at this pace, 70% when it starts again @${6 * DAY}`);
  assert.equal(said(50, 2 * DAY), `at this pace, full @${2 * DAY} · starts again @${5 * DAY}`);
  assert.equal(said(100, 2 * DAY), `full, starts again @${5 * DAY}`);
  assert.equal(said(3, 3600), `too early to project · starts again @${7 * DAY - 3600}`);
  assert.equal(said(5, 8 * DAY, 2 * DAY), `a new week started @${-DAY}, no figures since`);
});
