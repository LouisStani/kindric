// The words of the purge of old sessions in the settings (decisions 361, 362), from what Rust
// gives: the counts of `purge_preview`, the report of `purge_sessions`.

function plural(count, word) {
  return `${count} ${word}${count === 1 ? "" : "s"}`;
}

// The question asked before a purge; null when no session goes.
export function purgeQuestion(counts, days) {
  const total = counts.reduce((sum, count) => sum + count.sessions, 0);
  if (total === 0) return null;
  const accounts = counts.length > 1 ? ` (${counts.map((count) => `${count.account} ${count.sessions}`).join(", ")})` : ` of ${counts[0].account}`;
  return `${plural(total, "session")}${accounts}, idle for more than ${days} days, will leave kindric.`;
}

// What a purge did; the sessions refused follow, one line each (`refusalLine`).
export function purgeResult(report) {
  const removed = report.removed.length === 0 ? "no session removed" : `${plural(report.removed.length, "session")} removed`;
  const refused = report.refused.length === 0 ? "." : `; ${report.refused.length} refused by Claude Code, kept:`;
  return `${removed}${refused}`;
}

export function refusalLine(refused) {
  return `${refused.target.label} (${refused.target.account}): ${refused.reason}`;
}
