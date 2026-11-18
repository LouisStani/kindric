// Tests of the "show in stats" box of the settings (stats.js): `node --test crates/desktop/ui/stats.test.js`.
// Accounts have the shape of account_statuses, `isolated` included.

import { test } from "node:test";
import assert from "node:assert/strict";
import { statsBox } from "./stats.js";

test("an ordinary account has no box", () => {
  assert.equal(statsBox({ name: "louis", isolated: false }, ["louis"]), null);
  assert.equal(statsBox({ name: "louis" }, []), null);
});

test("an isolated account has a box, unticked unless its name is listed", () => {
  const work = { name: "work", isolated: true };
  assert.deepEqual(statsBox(work, []), { shown: false });
  assert.deepEqual(statsBox(work, undefined), { shown: false });
  assert.deepEqual(statsBox(work, ["lystriq"]), { shown: false });
  assert.deepEqual(statsBox(work, ["work"]), { shown: true });
  assert.deepEqual(statsBox(work, ["lystriq", "work"]), { shown: true });
});

test("the name is matched exactly", () => {
  assert.deepEqual(statsBox({ name: "work", isolated: true }, ["Work", "work2"]), { shown: false });
});
