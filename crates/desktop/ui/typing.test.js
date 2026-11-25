// Tests of the messages typed without a pane: `node --test crates/desktop/ui/typing.test.js`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { Typing } from "./typing.js";

const alpha = { account: "work", short_id: "aaaa" };
const beta = { account: "work", short_id: "bbbb" };

// A message whose end the test chooses.
function message() {
  const controls = {};
  controls.typed = new Promise((resolve, reject) => Object.assign(controls, { resolve, reject }));
  return controls;
}

test("a session is busy while a message is typed into it, the others are not", async () => {
  const typing = new Typing();
  const first = message();
  const sent = typing.during(alpha, () => first.typed);
  assert.deepEqual([typing.busy(alpha), typing.busy(beta)], [true, false]);
  let open = false;
  const opened = typing.over(alpha).then(() => (open = true));
  await Promise.resolve();
  assert.equal(open, false, "opened while the message is typed");
  first.resolve("done");
  assert.equal(await sent, "done");
  await opened;
  assert.equal(typing.busy(alpha), false);
});

test("a message that fails says so to its sender only, and the session can open", async () => {
  const typing = new Typing();
  const first = message();
  const sent = typing.during(alpha, () => first.typed);
  const opened = typing.over(alpha);
  first.reject(new Error("no prompt"));
  await assert.rejects(sent, /no prompt/);
  await opened;
  assert.equal(typing.busy(alpha), false);
});

test("a second message waits for the first, and the opening for both", async () => {
  const typing = new Typing();
  const [first, second] = [message(), message()];
  const order = [];
  const one = typing.during(alpha, () => first.typed.then(() => order.push("first")));
  const two = typing.during(alpha, () => {
    order.push("second starts");
    return second.typed;
  });
  const opened = typing.over(alpha).then(() => order.push("open"));
  first.resolve();
  await one;
  assert.equal(typing.busy(alpha), true);
  second.resolve();
  await Promise.all([two, opened]);
  assert.deepEqual(order, ["first", "second starts", "open"]);
});

test("nothing typed, nothing to wait for", async () => {
  await new Typing().over(alpha);
});
