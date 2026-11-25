// Tests of the recap image's figures (decisions 385, 481): `node --test crates/desktop/ui/recap-board.test.js`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { RECAP_DAYS, recapOf } from "./recap-board.js";

// Noon, for the user, of a day without a change of time around it.
const NOW = new Date(2026, 10, 25, 12).getTime() / 1000;
const HOUR = 3600;

function tokens(output, extra = {}) {
  return { input: 0, output, cache_read: 0, cache_creation: 0, thinking: 0, web_searches: 0, replies: 1, prompts: 1, ...extra };
}

// `ago` in hours before NOW.
function hour(ago, output, extra = {}) {
  return { hour: Math.floor(NOW / HOUR) - ago, account: "secret-account", project: "/p/secret-project", tokens: tokens(output, extra), models: { "claude-opus-5-5": 1 }, cost: 0 };
}

function session(id, ...spans) {
  return { account: "secret-account", session: id, project: "/p/secret-project", tokens: tokens(1), cost: 0, spans: spans.map(([start, end]) => ({ start: (NOW - start) * 1000, end: (NOW - end) * 1000 })) };
}

const SUMMARY = {
  hours: [hour(1, 3_000, { cache_read: 57_000, prompts: 2, replies: 5 }), hour(25, 6_000, { replies: 1 }), hour(31 * 24 + 12, 999_999)],
  sessions: [session("secret-session", [5 * HOUR, 2 * HOUR]), session("other", [4 * HOUR, 3 * HOUR])],
};

function number(recap, label) {
  return recap.numbers.find((each) => each.label === label)?.value;
}

test("nothing to show without tokens", () => {
  assert.equal(recapOf({ hours: [], sessions: [] }, NOW), null);
  assert.equal(recapOf({ hours: [hour(40 * 24, 5)], sessions: [] }, NOW), null, "before the period");
});

test("the figures of the last thirty days only", () => {
  const recap = recapOf(SUMMARY, NOW);
  assert.deepEqual([recap.tokens, recap.fresh], ["66k", "9k"]);
  assert.equal(recap.perDay.length, RECAP_DAYS);
  assert.deepEqual(recap.perDay.slice(-2), [6_000, 3_000]);
  assert.deepEqual(recap.best, { day: RECAP_DAYS - 2, tokens: "6k" });
  assert.equal(number(recap, "sessions"), "2");
  assert.equal(number(recap, "replies"), "6");
  assert.equal(number(recap, "agents at once"), "2");
  assert.equal(number(recap, "agents at work"), "4 h", "3 h and 1 h");
  assert.equal(number(recap, "longest run"), "3 h 00");
  assert.equal(number(recap, "days in a row"), "2");
  assert.equal(number(recap, "days lit"), "2/30");
  assert.equal(number(recap, "replies a message"), "×2.0");
});

test("a square holds two hours and the squares hold every new token", () => {
  const recap = recapOf(SUMMARY, NOW);
  assert.deepEqual([recap.squares.length, recap.squares[0].length], [RECAP_DAYS, 12]);
  assert.equal(recap.squares.flat().reduce((sum, each) => sum + each, 0), 9_000);
  assert.equal(recap.most, 6_000);
});

test("the pace is that of the time there are figures of", () => {
  const recap = recapOf({ hours: [hour(10, 600_000)], sessions: [] }, NOW);
  assert.equal(recap.perMinute, "1k", "600k over 10 hours");
});

test("words in the singular", () => {
  const recap = recapOf({ hours: [hour(1, 5)], sessions: [session("a", [HOUR, 60])] }, NOW);
  assert.deepEqual(recap.numbers.slice(0, 3).map((each) => each.label), ["session", "reply", "message typed"]);
  assert.equal(number(recap, "day in a row"), "1");
  assert.equal(number(recap, "agents at work"), "59 min");
});

test("without a message typed there is no ratio, and nothing is divided by zero", () => {
  const recap = recapOf({ hours: [hour(1, 5, { prompts: 0 })], sessions: [] }, NOW);
  assert.equal(number(recap, "replies a message"), undefined);
  assert.equal(number(recap, "longest run"), "0 min");
  assert.ok(!JSON.stringify(recap).includes("NaN") && !JSON.stringify(recap).includes("Infinity"));
});

test("only the cache read again: no day lit, no best day", () => {
  const recap = recapOf({ hours: [hour(1, 0, { cache_read: 50 })], sessions: [] }, NOW);
  assert.equal(recap.best, null);
  assert.equal(number(recap, "days lit"), "0/30");
});

test("no name of an account, a project or a session", () => {
  assert.ok(!JSON.stringify(recapOf(SUMMARY, NOW)).includes("secret"));
});
