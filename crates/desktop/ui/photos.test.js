// Tests of the screen photos (decision 338): `node --test crates/desktop/ui/photos.test.js`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { ScreenPhotos, fits } from "./photos.js";

const alpha = { account: "louis", short_id: "aaaa1111" };
const beta = { account: "louis", short_id: "bbbb2222" };
const gamma = { account: "pro", short_id: "aaaa1111" };

function photo(text, columns = 120) {
  return { text, columns };
}

test("a session never seen has no photo", () => {
  assert.equal(new ScreenPhotos().get(alpha), null);
});

test("the photo kept last for a session is the one given back", () => {
  const photos = new ScreenPhotos();
  photos.keep(alpha, photo("first"));
  photos.keep(alpha, photo("second"));
  assert.deepEqual(photos.get(alpha), photo("second"));
});

test("the same id on two accounts is two sessions", () => {
  const photos = new ScreenPhotos();
  photos.keep(alpha, photo("louis"));
  photos.keep(gamma, photo("pro"));
  assert.equal(photos.get(alpha).text, "louis");
  assert.equal(photos.get(gamma).text, "pro");
});

test("past the limit the oldest photo goes, and one taken again counts as new", () => {
  const photos = new ScreenPhotos(2);
  photos.keep(alpha, photo("a"));
  photos.keep(beta, photo("b"));
  photos.keep(alpha, photo("a again"));
  photos.keep(gamma, photo("g"));
  assert.equal(photos.get(beta), null);
  assert.equal(photos.get(alpha).text, "a again");
  assert.equal(photos.get(gamma).text, "g");
});

test("a photo is shown only at the width it was taken, and never empty", () => {
  assert.equal(fits(photo("screen", 120), 120), true);
  assert.equal(fits(photo("screen", 120), 90), false);
  assert.equal(fits(photo("", 120), 120), false);
  assert.equal(fits(null, 120), false);
});
