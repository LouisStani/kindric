// Tests of the words of the purge in the settings (decisions 361, 362): `node --test crates/desktop/ui/purge.test.js`.
// Counts and reports have the shape of purge.rs's Count and Report.

import { test } from "node:test";
import assert from "node:assert/strict";
import { purgeQuestion, purgeResult, refusalLine } from "./purge.js";

function target(label, account = "louis") {
  return { account, short_id: "abc", label };
}

test("the question counts the sessions, by account when there are several", () => {
  const counts = [{ account: "louis", sessions: 20 }, { account: "lystriq", sessions: 4 }];
  assert.equal(
    purgeQuestion(counts, 7),
    "24 sessions (louis 20, lystriq 4), idle for more than 7 days, will leave kindric.",
  );
  assert.equal(purgeQuestion([{ account: "pro", sessions: 1 }], 31).split(",")[0], "1 session of pro");
});

test("nothing to purge asks nothing", () => {
  assert.equal(purgeQuestion([], 31), null);
});

test("the result says what went and what Claude Code kept", () => {
  assert.equal(purgeResult({ removed: [target("a"), target("b")], refused: [] }), "2 sessions removed.");
  const refused = { target: target("api", "pro"), reason: "worktree has unpushed commits" };
  assert.equal(purgeResult({ removed: [], refused: [refused] }), "no session removed; 1 refused by Claude Code, kept:");
  assert.equal(refusalLine(refused), "api (pro): worktree has unpushed commits");
});
