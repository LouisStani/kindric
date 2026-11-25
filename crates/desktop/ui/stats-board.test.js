// Tests of the stats window (decisions 385, 467): `node --test crates/desktop/ui/stats-board.test.js`.
// Summaries have the shape of core stats.rs's Summary.

import { test } from "node:test";
import assert from "node:assert/strict";
import { ALL, activity, counters, daysOf, durationText, novelsText, parallel, perDay, periodsOf, profile, records, streak, strength } from "./stats-board.js";

// Noon, for the user, of a day without a change of time around it.
const NOW = new Date(2026, 10, 25, 12).getTime() / 1000;
const DAY = 86_400;
// The hour of the day of the figures of NOW: 11 where a time zone is half an hour off.
const NOON = new Date(Math.floor(NOW / 3600) * 3_600_000).getHours();

function tokens(output, extra = {}) {
  return { input: 0, output, cache_read: 0, cache_creation: 0, thinking: 0, web_searches: 0, replies: 1, prompts: 1, ...extra };
}

// `ago` in hours before NOW.
function hour(ago, output, { account = "louis", project = "/p/kindric", extra = {}, models = { "claude-opus-5-5": 1 } } = {}) {
  return { hour: Math.floor(NOW / 3600) - ago, account, project, tokens: tokens(output, extra), models, cost: 0 };
}

function session(id, output, ...spans) {
  return { account: "louis", session: id, project: "/p/kindric", tokens: tokens(output), cost: 0, spans: spans.map(([start, end]) => ({ start: (NOW - start) * 1000, end: (NOW - end) * 1000 })) };
}

test("all is a choice past a month of figures", () => {
  assert.deepEqual(periodsOf({ hours: [hour(24, 1)], sessions: [] }, NOW), [7, 30]);
  assert.deepEqual(periodsOf({ hours: [hour(31 * 24, 1)], sessions: [] }, NOW), [7, 30, ALL]);
});

test("the big numbers of a period", () => {
  const summary = {
    hours: [
      hour(1, 1_000, { extra: { cache_read: 9_000, thinking: 30, web_searches: 2, prompts: 4 } }),
      hour(8 * 24, 500, { extra: { prompts: 9 } }),
    ],
    sessions: [session("a", 1, [3600, 60]), session("b", 1, [9 * DAY, 8 * DAY])],
  };
  const told = counters(summary, 7, NOW);
  assert.deepEqual([told.tokens, told.fresh, told.written, told.thinking, told.prompts, told.searches, told.sessions], [10_000, 1_000, 1_000, 30, 4, 2, 1]);
  assert.equal(told.change, null, "the figures do not go back to the two weeks");
  assert.equal(counters(summary, 30, NOW).sessions, 2);
  const longer = { ...summary, hours: [...summary.hours, hour(15 * 24, 5)] };
  assert.equal(counters({ ...longer, hours: [...longer.hours, hour(7 * 24 - 2, 9_999)] }, 7, NOW).fresh, 1_000, "before midnight of the first day");
  assert.equal(counters(longer, 7, NOW).change, 100, "1000 against 500 the week before");
  assert.equal(counters(longer, ALL, NOW).fresh, 1_505);
});

test("what was written, in novels", () => {
  assert.equal(novelsText(0), "");
  assert.equal(novelsText(0.4), "≈ 40% of The Lord of the Rings");
  assert.equal(novelsText(2.34), "≈ 2.3 times The Lord of the Rings");
  assert.equal(novelsText(23.4), "≈ 23 times The Lord of the Rings");
  // 481,103 words at 0.55 word a token are 874,733 tokens.
  assert.equal(Math.round(counters({ hours: [hour(1, 8_747_327)], sessions: [] }, 7, NOW).novels), 10);
});

test("the columns add up to the new tokens of the period", () => {
  const hours = Array.from({ length: 24 * 9 }, (_, ago) => hour(ago, 1));
  for (const days of [7, 30, ALL]) {
    const drawn = perDay({ hours, sessions: [] }, days, NOW).flatMap(({ columns }) => columns).reduce((sum, column) => sum + column.tokens, 0);
    assert.equal(drawn, counters({ hours, sessions: [] }, days, NOW).fresh, `${days} days`);
  }
});

test("a column per day and per account, the empty days included", () => {
  const summary = { hours: [hour(1, 5), hour(25, 7), hour(26, 1), hour(1, 90, { account: "pro" })], sessions: [] };
  assert.equal(daysOf(summary, 7, NOW).length, 7);
  const [first, second] = perDay(summary, 7, NOW);
  assert.equal(first.account, "pro", "the most first");
  assert.deepEqual(second.columns.map((column) => column.tokens), [0, 0, 0, 0, 0, 8, 5]);
});

test("days follow each other across a change of time", () => {
  const now = new Date(2026, 9, 31, 12).getTime() / 1000;
  const days = daysOf({ hours: [], sessions: [] }, 14, now);
  assert.deepEqual(days.map((day) => new Date(day * 1000).getDate()), [18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31]);
});

test("the activity of each hour of each day", () => {
  const summary = { hours: [hour(0, 4), hour(0, 6, { account: "pro" }), hour(24 + 2, 3)], sessions: [] };
  const { days, cells, most } = activity(summary, 7, NOW);
  assert.equal(days.length, 7);
  assert.deepEqual([cells[6][NOON], cells[5][NOON - 2], most], [10, 3, 10]);
  assert.deepEqual([0, 1, 100, 400, 900, 1600].map((each) => strength(each, 1600)), [0, 1, 1, 2, 3, 4]);
  assert.equal(strength(0, 0), 0);
});

test("the records", () => {
  const summary = {
    hours: [hour(1, 5), hour(25, 40), hour(26, 2)],
    sessions: [session("long", 10, [3 * 3600, 60]), session("heavy", 90, [600, 300], [DAY, DAY - 60])],
  };
  const best = records(summary, 7, NOW);
  assert.deepEqual([best.day.count, best.longest.session.session, best.heaviest.session], [42, "long", "heavy"]);
  assert.equal(best.longest.milliseconds, (3 * 3600 - 60) * 1000);
  assert.equal(best.parallel, 2);
  assert.deepEqual(records({ hours: [], sessions: [] }, 7, NOW), { day: null, longest: null, heaviest: null, streak: 0, parallel: 0 });
});

test("a run started before the period counts for the part inside it", () => {
  const best = records({ hours: [], sessions: [session("a", 1, [8 * DAY, 6 * DAY])] }, 7, NOW);
  assert.equal(best.longest.milliseconds, (DAY / 2) * 1000);
});

test("sessions at once, one ending as another starts not counted together", () => {
  const span = (start, end) => ({ start, end });
  assert.equal(parallel([]), 0);
  assert.equal(parallel([span(0, 10), span(10, 20), span(5, 15)]), 2);
  assert.equal(parallel([span(0, 10), span(1, 9), span(2, 8)]), 3);
});

test("days in a row, broken by a day without anything", () => {
  const at = (days) => hour(days * 24, 1);
  assert.equal(streak({ hours: [at(0), at(1), at(3), at(4), at(5)], sessions: [] }, 7, NOW), 3);
});

test("the habits", () => {
  const hours = [hour(0, 5), hour(24, 9, { project: "/p/shop" }), hour(2, 8, { project: "/p/shop/.claude/worktrees/x", models: { "gpt-6-luna": 3 } })];
  assert.deepEqual(profile({ hours, sessions: [] }, 7, NOW), { hour: NOON, project: "shop", model: { name: "gpt-6-luna", percent: 60 } });
  assert.deepEqual(profile({ hours: [], sessions: [] }, 7, NOW), { hour: null, project: null, model: null });
});

test("durations are short", () => {
  assert.deepEqual([0, 42 * 60_000, 3_600_000, 28_535_000].map(durationText), ["0 min", "42 min", "1 h 00", "7 h 56"]);
});
