// Tests of the outbox the page takes from Rust: `node --test crates/desktop/ui/outbox.test.js`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { letters, Outbox } from "./outbox.js";

// What `output_keeps_its_order_and_only_the_last_list_goes` in outbox.rs expects Rust to write.
function fromRust() {
  const text = (words) => [...new TextEncoder().encode(words)];
  return new Uint8Array([0, 7, 0, 0, 0, 3, 0, 0, 0, ...text("abc"), 0, 44, 1, 0, 0, 1, 0, 0, 0, ...text("d"), 1, 3, 0, 0, 0, ...text("[2]")]).buffer;
}

const decoded = (bytes) => new TextDecoder().decode(bytes);

test("the letters are read as Rust writes them, in order", () => {
  const found = letters(fromRust());
  assert.deepEqual(
    found.map((letter) => ("sessions" in letter ? letter.sessions : [letter.output, decoded(letter.bytes)])),
    [[7, "abc"], [300, "d"], [2]],
  );
  assert.deepEqual(letters(new ArrayBuffer(0)), []);
  assert.deepEqual(letters([...new Uint8Array(fromRust())]).length, 3);
  assert.throws(() => letters(new Uint8Array([9]).buffer), /unknown letter 9/);
});

test("each terminal gets its own output, a closed one nothing", async () => {
  const seen = [];
  const outbox = new Outbox(async () => fromRust(), (sessions) => seen.push(["sessions", sessions]));
  for (let number = 1; number < 7; number += 1) outbox.open(() => seen.push(["unexpected", number]));
  const seven = outbox.open((bytes) => seen.push(["seven", decoded(bytes)]));
  assert.equal(seven, 7);
  outbox.close(seven);
  await outbox.collect();
  assert.deepEqual(seen, [["sessions", [2]]]);
});

test("a ring while taking takes once more after", async () => {
  let release;
  let takes = 0;
  const outbox = new Outbox(
    () => {
      takes += 1;
      return takes === 1 ? new Promise((resolve) => (release = resolve)) : Promise.resolve(new ArrayBuffer(0));
    },
    () => {},
  );
  const first = outbox.collect();
  await outbox.collect();
  release(new ArrayBuffer(0));
  await first;
  assert.equal(takes, 2);
});

test("a failed take is tried again", async () => {
  let takes = 0;
  const original = console.error;
  console.error = () => {};
  const outbox = new Outbox(
    async () => {
      takes += 1;
      if (takes === 1) throw new Error("page not ready");
      return new ArrayBuffer(0);
    },
    () => {},
    1,
  );
  await outbox.collect();
  await new Promise((resolve) => setTimeout(resolve, 20));
  console.error = original;
  assert.equal(takes, 2);
});
