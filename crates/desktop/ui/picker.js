// Ctrl+P by day (decision 376): Rust gives the rows the most recently active first, hidden ones
// last (view.rs, filter); this module puts the head of a day before the first row of each, and
// says when each session was last active. No browser here, so `node --test` checks it
// (picker.test.js).

const DAY = 24 * 3600 * 1000;

function startOfDay(date) {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

// How many days before `now` the day of `then` is, counted on the calendar: a day of 23 or 25
// hours, when the clocks change, is still one day.
function daysBefore(then, now) {
  return Math.round((startOfDay(now) - startOfDay(then)) / DAY);
}

// "today", "yesterday", else the date, with its year when it is not this one. A session whose
// last activity is unknown has not been used yet: today.
export function dayLabel(seconds, now, locale = []) {
  if (seconds === null || seconds === undefined) return "today";
  const then = new Date(seconds * 1000);
  const days = daysBefore(then, now);
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  const options = { month: "short", day: "numeric" };
  if (then.getFullYear() !== now.getFullYear()) options.year = "numeric";
  return then.toLocaleDateString(locale, options);
}

function timeOf(seconds, locale) {
  return new Date(seconds * 1000).toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" });
}

// The rows of Ctrl+P with their heads: { head } before the first row of each day, and one head
// "hidden" before the hidden ones, which mix days; then { index, when } for each row, `index`
// in `rows`. Under a day, `when` is the time; under "hidden", the day and the time.
export function pickerItems(rows, now, locale = []) {
  const items = [];
  let head = null;
  rows.forEach((row, index) => {
    const known = row.last_active !== null && row.last_active !== undefined;
    const day = dayLabel(row.last_active, now, locale);
    const group = row.hidden ? "hidden" : day;
    if (group !== head) {
      items.push({ head: group });
      head = group;
    }
    const time = known ? timeOf(row.last_active, locale) : "";
    items.push({ index, when: row.hidden && known ? `${day} ${time}` : time });
  });
  return items;
}
