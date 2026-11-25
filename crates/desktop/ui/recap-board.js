// The figures of the recap image, worked out from what the `stats` command returns: no page here, so
// that `node --test` can check it. The image is made to be posted: no name of an account, a project
// or a session may be in it.

import { activity, counters, daysOf, durationText, novelsText, perDay, profile, records, timeAtWork } from "./stats-board.js";
import { tokensText } from "./usage-board.js";

export const RECAP_DAYS = 30;
// A square of the map: this many hours of a day.
export const HOURS_A_SQUARE = 2;
const SQUARES_A_DAY = 24 / HOURS_A_SQUARE;

function named(count, one, many) {
  return { value: count.toLocaleString("en-US"), label: count === 1 ? one : many };
}

// `null` without tokens. `numbers`: `[{ value, label, hot }]`, twelve at most, the hot ones being what
// the user will want to beat.
export function recapOf(summary, now) {
  const told = counters(summary, RECAP_DAYS, now);
  if (told.tokens === 0) return null;
  const days = daysOf(summary, RECAP_DAYS, now);
  const accounts = perDay(summary, RECAP_DAYS, now);
  const perDayTokens = days.map((_, day) => accounts.reduce((sum, account) => sum + account.columns[day].tokens, 0));
  const best = Math.max(...perDayTokens);
  const squares = activity(summary, RECAP_DAYS, now).cells.map((hours) =>
    Array.from({ length: SQUARES_A_DAY }, (_, square) => hours.slice(square * HOURS_A_SQUARE, (square + 1) * HOURS_A_SQUARE).reduce((sum, tokens) => sum + tokens, 0)),
  );
  const oldest = summary.hours.reduce((first, row) => Math.min(first, row.hour), Infinity) * 3600;
  const minutes = Math.max(1, (now - Math.max(days[0], oldest)) / 60);
  const bests = records(summary, RECAP_DAYS, now);
  const { hour } = profile(summary, RECAP_DAYS, now);
  const atWork = timeAtWork(summary, RECAP_DAYS, now);
  const numbers = [
    named(told.sessions, "session", "sessions"),
    named(told.replies, "reply", "replies"),
    named(told.prompts, "message typed", "messages typed"),
    { value: tokensText(told.written), label: "tokens written" },
    { value: atWork < 3_600_000 ? durationText(atWork) : `${Math.round(atWork / 3_600_000)} h`, label: "agents at work", hot: true },
    { ...named(bests.parallel, "agent at once", "agents at once"), hot: true },
    { value: durationText(bests.longest?.milliseconds ?? 0), label: "longest run", hot: true },
    { ...named(bests.streak, "day in a row", "days in a row"), hot: true },
    { value: tokensText(told.thinking), label: "tokens of thinking" },
    { value: `${perDayTokens.filter((tokens) => tokens > 0).length}/${RECAP_DAYS}`, label: "days lit" },
  ];
  if (hour !== null) numbers.push({ value: `${hour}h`, label: "peak hour" });
  if (told.prompts > 0) numbers.push({ value: `×${(told.replies / told.prompts).toFixed(1)}`, label: "replies a message" });
  const novels = novelsText(told.novels);
  return {
    from: days[0],
    to: days.at(-1),
    tokens: tokensText(told.tokens),
    fresh: tokensText(told.fresh),
    perMinute: tokensText(Math.round(told.tokens / minutes)),
    perDay: perDayTokens,
    best: best > 0 ? { day: perDayTokens.indexOf(best), tokens: tokensText(best) } : null,
    squares,
    most: Math.max(...squares.flat()),
    numbers,
    novels: novels && `${novels}, written`,
  };
}
