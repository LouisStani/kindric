// What the stats window shows, worked out from what the `stats` command returns: no page here, so
// that `node --test` can check it. Hours are Unix hours (UTC), dates Unix seconds, days and hours of
// the day those of the user.

import { newTokens, projectName, projectOf } from "./usage-board.js";

const DAY = 86_400;
// "all" is a choice only when there is more than a month of it.
export const ALL = 0;
const PERIODS = [7, 30];

// A novel everyone knows, to size what the models wrote: 481,103 words (widely quoted for the three
// volumes), at 0.55 word a token, the estimate for the models of 2026. Only ever shown with "≈".
const WORDS_PER_TOKEN = 0.55;
const NOVEL_WORDS = 481_103;

function oldestHour(summary) {
  return summary.hours.reduce((oldest, row) => Math.min(oldest, row.hour), Infinity);
}

export function periodsOf(summary, now) {
  const oldest = oldestHour(summary) * 3600;
  return now - oldest > 30 * DAY ? [...PERIODS, ALL] : PERIODS;
}

export function periodText(days) {
  return days === ALL ? "all" : `${days} days`;
}

// Midnight of the day `at` is in, for the user.
function dayOf(at) {
  const day = new Date(at * 1000);
  day.setHours(0, 0, 0, 0);
  return day.getTime() / 1000;
}

// Midnight of the first day of the period: today is its last one.
function startOf(summary, days, now) {
  return days === ALL ? dayOf(oldestHour(summary) * 3600) : dayOf(dayOf(now) - (days - 1) * DAY + DAY / 2);
}

// The hours of the period.
function hoursOf(summary, days, now) {
  const start = startOf(summary, days, now);
  return summary.hours.filter((row) => row.hour * 3600 >= start);
}

// The hours of the same number of days before the period, up to the time of day it is now, and
// whether the figures go back to them.
function hoursBefore(summary, days, now) {
  const start = startOf(summary, days, now);
  const before = dayOf(start - days * DAY + DAY / 2);
  const end = before + (now - start);
  return { covered: oldestHour(summary) * 3600 <= before, hours: summary.hours.filter((row) => row.hour * 3600 >= before && row.hour * 3600 < end) };
}

// The spans of activity of the period, cut to it: `[{ start, end, session }]` in milliseconds.
function spansOf(summary, days, now) {
  const since = startOf(summary, days, now) * 1000;
  return summary.sessions.flatMap((session) =>
    session.spans.filter((span) => span.end >= since).map((span) => ({ start: Math.max(span.start, since), end: span.end, session })),
  );
}

function sumOf(hours, read) {
  return hours.reduce((sum, row) => sum + read(row.tokens), 0);
}

const everyToken = (tokens) => newTokens(tokens) + tokens.cache_read;

// The big numbers. `change`: the new tokens compared with the same time before, in percent; null
// when the figures do not go back that far.
export function counters(summary, days, now) {
  const hours = hoursOf(summary, days, now);
  const fresh = sumOf(hours, newTokens);
  const previous = days === ALL ? { covered: false, hours: [] } : hoursBefore(summary, days, now);
  const before = sumOf(previous.hours, newTokens);
  const covered = previous.covered && before > 0;
  const sessions = new Set(spansOf(summary, days, now).map((span) => `${span.session.account}/${span.session.session}`));
  const written = sumOf(hours, (tokens) => tokens.output);
  return {
    tokens: sumOf(hours, everyToken),
    fresh,
    change: covered ? Math.round(((fresh - before) / before) * 100) : null,
    written,
    novels: (written * WORDS_PER_TOKEN) / NOVEL_WORDS,
    thinking: sumOf(hours, (tokens) => tokens.thinking),
    replies: sumOf(hours, (tokens) => tokens.replies),
    prompts: sumOf(hours, (tokens) => tokens.prompts),
    searches: sumOf(hours, (tokens) => tokens.web_searches),
    sessions: sessions.size,
  };
}

// The days from the first of the period to today, oldest first. A day can last 23 or 25 hours.
export function daysOf(summary, days, now) {
  const first = startOf(summary, days, now);
  const list = [];
  for (let day = first; day <= now; day = dayOf(day + 26 * 3600)) list.push(day);
  return list;
}

// The new tokens of each account, a day a column: `[{ account, columns: [{ at, tokens }] }]`, those
// that used the most first.
export function perDay(summary, days, now) {
  const list = daysOf(summary, days, now);
  const accounts = new Map();
  for (const row of hoursOf(summary, days, now)) {
    const tokens = newTokens(row.tokens);
    if (tokens === 0) continue;
    if (!accounts.has(row.account)) accounts.set(row.account, new Map());
    const byDay = accounts.get(row.account);
    const day = dayOf(row.hour * 3600);
    byDay.set(day, (byDay.get(day) ?? 0) + tokens);
  }
  const sum = (entry) => entry.columns.reduce((total, column) => total + column.tokens, 0);
  return [...accounts]
    .map(([account, byDay]) => ({ account, columns: list.map((at) => ({ at, tokens: byDay.get(at) ?? 0 })) }))
    .sort((a, b) => sum(b) - sum(a));
}

// The new tokens of each hour of each day: `{ days, cells: cells[day][hour], most }`.
export function activity(summary, days, now) {
  const list = daysOf(summary, days, now);
  const index = new Map(list.map((day, position) => [day, position]));
  const cells = list.map(() => new Array(24).fill(0));
  for (const row of hoursOf(summary, days, now)) {
    const position = index.get(dayOf(row.hour * 3600));
    if (position !== undefined) cells[position][new Date(row.hour * 3_600_000).getHours()] += newTokens(row.tokens);
  }
  return { days: list, cells, most: Math.max(0, ...cells.flat()) };
}

// How strong a cell is drawn, 0 to 4: a step for each quarter of the busiest hour's square root, so
// that ordinary hours show next to a huge one.
export function strength(tokens, most) {
  if (tokens === 0 || most === 0) return 0;
  return Math.max(1, Math.ceil(Math.sqrt(tokens / most) * 4));
}

function largest(map) {
  let best = null;
  for (const [key, count] of map) if (best === null || count > best.count) best = { key, count };
  return best;
}

function count(map, key, more) {
  map.set(key, (map.get(key) ?? 0) + more);
}

// The longest run of days in a row with a message typed or a reply.
export function streak(summary, days, now) {
  const active = new Set(hoursOf(summary, days, now).filter((row) => row.tokens.replies + row.tokens.prompts > 0).map((row) => dayOf(row.hour * 3600)));
  let run = 0;
  let longest = 0;
  for (const day of daysOf(summary, days, now)) {
    run = active.has(day) ? run + 1 : 0;
    longest = Math.max(longest, run);
  }
  return longest;
}

// How many sessions were active at once, at most. One that ends as another starts is not with it.
export function parallel(spans) {
  const edges = spans.flatMap((span) => [[span.start, 1], [span.end, -1]]).sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  let open = 0;
  let most = 0;
  for (const [, step] of edges) {
    open += step;
    most = Math.max(most, open);
  }
  return most;
}

// The bests of the period; a part is null without figures.
export function records(summary, days, now) {
  const byDay = new Map();
  for (const row of hoursOf(summary, days, now)) count(byDay, dayOf(row.hour * 3600), newTokens(row.tokens));
  const spans = spansOf(summary, days, now);
  const longest = spans.reduce((best, span) => (best === null || span.end - span.start > best.end - best.start ? span : best), null);
  const active = new Set(spans.map((span) => span.session));
  const heaviest = [...active].reduce((best, session) => (best === null || newTokens(session.tokens) > newTokens(best.tokens) ? session : best), null);
  return {
    day: largest(byDay),
    longest: longest && { session: longest.session, milliseconds: longest.end - longest.start },
    heaviest,
    streak: streak(summary, days, now),
    parallel: parallel(spans),
  };
}

// The habits: the hour of the day, project and model with the most. Null without figures.
export function profile(summary, days, now) {
  const hours = new Map();
  const projects = new Map();
  const models = new Map();
  for (const row of hoursOf(summary, days, now)) {
    const tokens = newTokens(row.tokens);
    count(hours, new Date(row.hour * 3_600_000).getHours(), tokens);
    count(projects, projectOf(row.project), tokens);
    for (const [model, replies] of Object.entries(row.models)) count(models, model, replies);
  }
  const project = largest(projects);
  const replies = [...models.values()].reduce((sum, each) => sum + each, 0);
  const model = largest(models);
  return {
    hour: largest(hours)?.key ?? null,
    project: project && projectName(project.key),
    model: model && { name: model.key, percent: Math.round((model.count / replies) * 100) },
  };
}

// "7 h 55", "42 min".
export function durationText(milliseconds) {
  const minutes = Math.round(milliseconds / 60_000);
  return minutes < 60 ? `${minutes} min` : `${Math.floor(minutes / 60)} h ${`${minutes % 60}`.padStart(2, "0")}`;
}

// "≈ 23 times The Lord of the Rings", "≈ 40% of The Lord of the Rings".
export function novelsText(novels) {
  if (novels < 0.01) return "";
  if (novels < 1) return `≈ ${Math.round(novels * 100)}% of The Lord of the Rings`;
  return `≈ ${novels < 10 ? novels.toFixed(1) : Math.round(novels)} times The Lord of the Rings`;
}
