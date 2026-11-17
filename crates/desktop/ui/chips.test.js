// Tests of the dots of the bar and the tabs (decision 355): `node --test crates/desktop/ui/chips.test.js`.
// Sessions have the shape of view.rs's SessionRow.

import { test } from "node:test";
import assert from "node:assert/strict";
import { chipState, dotSign, stateWords, tabMark } from "./chips.js";

function session(extra = {}) {
  return { short_id: "abc", state: "waiting", finished: false, asks: false, unread: 0, waiting_for: null, ...extra };
}

test("a question shows before a finished turn, with its own sign", () => {
  const asking = session({ asks: true, finished: true, waiting_for: "permission prompt" });
  assert.equal(chipState(asking), "asking");
  assert.equal(dotSign(chipState(asking)), "?");
  assert.equal(dotSign(chipState(session({ finished: true }))), "✓");
  assert.equal(dotSign(chipState(session({ state: "working" }))), "");
});

test("a session of another terminal stays elsewhere, even when it asks", () => {
  assert.equal(chipState(session({ short_id: null, asks: true })), "elsewhere");
});

test("a tab shows the question of its first session before its messages", () => {
  assert.equal(tabMark(session({ asks: true, unread: 2 })), "asking");
  assert.equal(tabMark(session({ unread: 2, state: "working" })), "waiting");
  assert.equal(tabMark(session({ finished: true })), "finished");
  assert.equal(tabMark(session({ state: "working" })), null);
  assert.equal(tabMark(undefined), null);
});

test("the tooltip says what the session waits for", () => {
  assert.equal(stateWords(session({ asks: true, waiting_for: "permission prompt" })), "asks you: permission prompt");
  assert.equal(stateWords(session({ asks: true })), "asks you");
  assert.equal(stateWords(session({ finished: true })), "done");
  assert.equal(stateWords(session({ state: "working" })), "working");
});
