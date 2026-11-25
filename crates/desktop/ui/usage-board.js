// What the usage window shows, worked out from what the `stats` command returns: no page here, so
// that `node --test` can check it. Hours are Unix hours (UTC), dates Unix seconds.

const DAY = 86_400;
const WEEK = 7 * DAY;
// Before this much of a week, its pace says nothing: a busy first day would announce a limit that
// never comes (decision 469).
const PROJECTED_FROM = DAY;
const OLD_FIGURES = 3600;
export const ROWS_SHOWN = 8;

export const PERIODS = [7, 30];

// "14.6M", "94k", "812".
export function tokensText(count) {
  for (const [size, letter] of [[1e9, "B"], [1e6, "M"], [1e3, "k"]]) {
    if (count < size * 0.9995) continue;
    const value = count / size;
    return `${value < 99.95 && letter !== "k" ? value.toFixed(1) : Math.round(value)}${letter}`;
  }
  return `${count}`;
}

// "$0.42", "$123": what was paid by use, in millionths of a dollar.
export function costText(micros) {
  const dollars = micros / 1e6;
  if (micros === 0) return "";
  if (dollars < 0.005) return "<$0.01";
  return dollars < 99.995 ? `$${dollars.toFixed(2)}` : `$${Math.round(dollars)}`;
}

// What a reply costs anew. Cache read again is counted apart: far larger, it would hide the rest.
export function newTokens(tokens) {
  return tokens.input + tokens.output + tokens.cache_creation;
}

// The project of a folder: a worktree of Claude Code counts for the checkout it belongs to.
export function projectOf(directory) {
  return directory.split(/[\\/]\.claude[\\/]worktrees[\\/]/)[0];
}

export function projectName(directory) {
  return directory.split(/[\\/]/).filter(Boolean).pop() ?? "no folder";
}

// The hours of the last `days` days.
function within(hours, days, now) {
  const first = Math.floor(now / 3600) - days * 24;
  return hours.filter((row) => row.hour > first);
}

const KINDS = ["input", "output", "cache_creation", "cache_read"];

function added(total, tokens) {
  for (const kind of KINDS) total[kind] += tokens[kind];
}

function none() {
  return { input: 0, output: 0, cache_creation: 0, cache_read: 0 };
}

// What `rows` cost together.
export function costOf(rows) {
  return rows.reduce((sum, row) => sum + row.cost, 0);
}

// The most first, without those that used nothing, then one line for all the others.
function most(all, others) {
  const rows = all.filter((row) => newTokens(row.tokens) + row.tokens.cache_read + row.cost > 0);
  rows.sort((a, b) => newTokens(b.tokens) - newTokens(a.tokens));
  if (rows.length <= ROWS_SHOWN) return rows;
  const rest = rows.slice(ROWS_SHOWN);
  const total = none();
  for (const row of rest) added(total, row.tokens);
  return [...rows.slice(0, ROWS_SHOWN), { name: others(rest.length), title: "", tokens: total, cost: costOf(rest), rest: true }];
}

// The tokens of each project over the last `days` days.
export function byProject(summary, days, now) {
  const projects = new Map();
  for (const row of within(summary.hours, days, now)) {
    const project = projectOf(row.project);
    if (!projects.has(project)) projects.set(project, { tokens: none(), cost: 0 });
    const sum = projects.get(project);
    added(sum.tokens, row.tokens);
    sum.cost += row.cost;
  }
  const rows = [...projects].map(([project, sum]) => ({ name: projectName(project), title: project, ...sum, rest: false }));
  return most(rows, (count) => `${count} other projects`);
}

// The sessions active in the last `days` days, with all the tokens of their lives. `labelOf` gives
// the name of a session kindric lists, null for the others.
export function bySession(summary, days, now, labelOf) {
  const since = (now - days * DAY) * 1000;
  const rows = summary.sessions
    .filter((session) => session.spans.some((span) => span.end >= since))
    .map((session) => ({
      name: labelOf(session.account, session.session) ?? session.session.slice(0, 8),
      project: projectName(projectOf(session.project)),
      title: session.account,
      tokens: session.tokens,
      cost: session.cost,
      rest: false,
    }));
  return most(rows, (count) => `${count} other sessions`);
}

export function total(summary, days, now) {
  const tokens = none();
  for (const row of within(summary.hours, days, now)) added(tokens, row.tokens);
  return tokens;
}

// The tokens written to the cache: an hour a column over 7 days, a local day over 30. `[{ at, tokens }]`,
// oldest first, the empty ones included.
export function cacheColumns(summary, days, now) {
  const perHour = new Map();
  for (const row of within(summary.hours, days, now)) perHour.set(row.hour, (perHour.get(row.hour) ?? 0) + row.tokens.cache_creation);
  const last = Math.floor(now / 3600);
  const columns = [];
  for (let hour = last - days * 24 + 1; hour <= last; hour += 1) {
    const tokens = perHour.get(hour) ?? 0;
    const at = hour * 3600;
    const previous = columns.at(-1);
    if (days > 7 && previous && new Date(previous.at * 1000).toDateString() === new Date(at * 1000).toDateString()) previous.tokens += tokens;
    else columns.push({ at, tokens });
  }
  return columns;
}

// How the week of an account goes, from the last figures of its history: `null` without figures.
// `kind`: "over" (a new week started since), "full", "early" (too soon to project), "old" (figures
// too old to project), "full-at" (before its end, at the pace so far) or "holds".
export function weekOf(history, now) {
  const last = history.at(-1);
  const week = last?.seven_day;
  if (!week) return null;
  const seen = { percent: week.used_percentage, resetsAt: week.resets_at ?? null, readAt: last.updated_at, old: now - last.updated_at > OLD_FIGURES };
  if (seen.resetsAt === null) return { ...seen, kind: "early" };
  if (seen.resetsAt <= now) return { ...seen, kind: "over" };
  if (seen.percent >= 100) return { ...seen, kind: "full" };
  const start = seen.resetsAt - WEEK;
  const elapsed = seen.readAt - start;
  if (elapsed < PROJECTED_FROM) return { ...seen, kind: "early" };
  const atReset = (seen.percent * WEEK) / elapsed;
  if (atReset < 100) return { ...seen, kind: "holds", atReset: Math.round(atReset) };
  const fullAt = Math.round(start + (elapsed * 100) / seen.percent);
  return fullAt <= now ? { ...seen, kind: "old" } : { ...seen, kind: "full-at", fullAt };
}

// `weekOf` in words; `at` writes a date.
export function weekText(week, at) {
  switch (week.kind) {
    case "over":
      return `a new week started ${at(week.resetsAt)}, no figures since`;
    case "full":
      return `full, starts again ${at(week.resetsAt)}`;
    case "early":
      return week.resetsAt === null ? "" : `too early to project · starts again ${at(week.resetsAt)}`;
    case "old":
      return `figures too old to project · starts again ${at(week.resetsAt)}`;
    case "full-at":
      return `at this pace, full ${at(week.fullAt)} · starts again ${at(week.resetsAt)}`;
    default:
      return `at this pace, ${week.atReset}% when it starts again ${at(week.resetsAt)}`;
  }
}
