// Tests of the welcome list's counts: `node --test crates/desktop/ui/welcome.test.js`.
// Lines have the shape of core checks.rs's Line.

import { test } from "node:test";
import assert from "node:assert/strict";
import { coversOverview, toDoCount } from "./welcome.js";

const line = (state) => ({ state, text: "", copy: [], add: false, start: false });

test("only what is left to do counts, not a warning or an option", () => {
  assert.equal(toDoCount([line("ok"), line("to do"), line("warning"), line("optional"), line("to do")]), 2);
  assert.equal(toDoCount(null), 0);
});

test("the welcome screen covers the overview only while there is no session", () => {
  const lines = [line("ok"), line("to do")];
  assert.equal(coversOverview(lines, 0), true);
  assert.equal(coversOverview(lines, 243), false);
  assert.equal(coversOverview([line("ok"), line("warning")], 0), false);
});
