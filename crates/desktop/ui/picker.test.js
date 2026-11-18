// Tests of Ctrl+P by day (decision 376): `node --test crates/desktop/ui/picker.test.js`.
// Rows have the shape of view.rs's SessionRow; times are local, as the page shows them.

import { test } from "node:test";
import assert from "node:assert/strict";

import { dayLabel, pickerItems } from "./picker.js";

const NOW = new Date(2026, 10, 18, 15, 0);
const at = (year, month, day, hour, minute = 0) => new Date(year, month, day, hour, minute).getTime() / 1000;

test("days are named from the calendar, not from 24 hours", () => {
  assert.equal(dayLabel(at(2026, 10, 18, 0, 5), NOW, "en-US"), "today");
  assert.equal(dayLabel(at(2026, 10, 17, 23, 55), NOW, "en-US"), "yesterday", "five minutes before midnight is yesterday");
  assert.equal(dayLabel(at(2026, 10, 17, 0, 5), NOW, "en-US"), "yesterday");
  assert.equal(dayLabel(at(2026, 10, 12, 9), NOW, "en-US"), "Nov 12");
  assert.equal(dayLabel(at(2025, 11, 30, 9), NOW, "en-US"), "Dec 30, 2025", "another year says it");
});

test("a session never used counts as today", () => {
  assert.equal(dayLabel(null, NOW, "en-US"), "today");
  assert.equal(dayLabel(undefined, NOW, "en-US"), "today");
});

test("a head before the first row of each day, the time on each row", () => {
  const rows = [
    { last_active: null, hidden: false },
    { last_active: at(2026, 10, 18, 14, 5), hidden: false },
    { last_active: at(2026, 10, 17, 9, 30), hidden: false },
    { last_active: at(2026, 10, 12, 20, 0), hidden: false },
    { last_active: at(2026, 10, 10, 21, 55), hidden: false },
  ];
  const items = pickerItems(rows, NOW, "en-GB");
  assert.deepEqual(items, [
    { head: "today" },
    { index: 0, when: "" },
    { index: 1, when: "14:05" },
    { head: "yesterday" },
    { index: 2, when: "09:30" },
    { head: "12 Nov" },
    { index: 3, when: "20:00" },
    { head: "10 Nov" },
    { index: 4, when: "21:55" },
  ]);
});

test("the hidden ones come under one head, each with its day", () => {
  const rows = [
    { last_active: at(2026, 10, 18, 10, 0), hidden: false },
    { last_active: at(2026, 10, 18, 11, 0), hidden: true },
    { last_active: at(2026, 10, 12, 8, 0), hidden: true },
    { last_active: null, hidden: true },
  ];
  assert.deepEqual(pickerItems(rows, NOW, "en-GB"), [
    { head: "today" },
    { index: 0, when: "10:00" },
    { head: "hidden" },
    { index: 1, when: "today 11:00" },
    { index: 2, when: "12 Nov 08:00" },
    { index: 3, when: "" },
  ]);
});
