import { test } from "node:test";
import assert from "node:assert/strict";
import { countText, highlight, whenOf, wordsOf } from "./history.js";

test("the words are cut on spaces, as Rust cuts them", () => {
  assert.deepEqual(wordsOf("  logo   SHOP "), ["logo", "SHOP"]);
  assert.deepEqual(wordsOf("   "), []);
});

test("each word is marked in any case, the rest left as it is", () => {
  assert.deepEqual(highlight("The Logo of the shop", ["logo", "SHOP"]), [
    { text: "The ", mark: false },
    { text: "Logo", mark: true },
    { text: " of the ", mark: false },
    { text: "shop", mark: true },
  ]);
  assert.deepEqual(highlight("Réglage de l'ÉCRAN", ["écran"]), [
    { text: "Réglage de l'", mark: false },
    { text: "ÉCRAN", mark: true },
  ]);
});

test("a word with signs of a pattern is looked for as it is", () => {
  assert.deepEqual(highlight("is a.b here? (yes)", ["a.b", "(yes)"]), [
    { text: "is ", mark: false },
    { text: "a.b", mark: true },
    { text: " here? ", mark: false },
    { text: "(yes)", mark: true },
  ]);
  assert.deepEqual(highlight("nothing", []), [{ text: "nothing", mark: false }]);
});

test("a match of today shows its time, an older one its day", () => {
  const now = new Date(2026, 10, 28, 12, 0);
  assert.equal(whenOf(new Date(2026, 10, 28, 9, 5).toISOString(), now, "en-GB"), "09:05");
  assert.equal(whenOf(new Date(2026, 10, 20, 9, 5).toISOString(), now, "en-GB"), "20 Nov");
  assert.equal(whenOf(new Date(2025, 10, 20, 9, 5).toISOString(), now, "en-GB"), "20 Nov 2025");
  assert.equal(whenOf(null, now), "");
  assert.equal(whenOf("not a date", now), "");
});

test("how many messages hold the words", () => {
  assert.equal(countText(1), "1 message");
  assert.equal(countText(3), "3 messages");
});
