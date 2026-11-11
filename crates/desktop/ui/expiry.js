// When the login of an account must be renewed (`login_expires` of kindric-accounts status),
// counted like Claude Code's own warning: "expires in 3 days".

const HOUR = 3600 * 1000;
const DAY = 24 * HOUR;

// Milliseconds left before the login of `account` expires; null without a date.
export function timeLeft(account, now = Date.now()) {
  const at = account.login_expires ? Date.parse(account.login_expires) : NaN;
  return Number.isNaN(at) ? null : at - now;
}

// Whether `account` is logged in and must renew its login within `days` days.
export function expiresWithin(account, days, now = Date.now()) {
  const left = timeLeft(account, now);
  return account.state === "logged-in" && left !== null && left <= days * DAY;
}

// "in 3 days", "in 5 hours", "now": rounded up, as Claude Code does.
export function inHowLong(left) {
  if (left <= 0) return "now";
  if (left < DAY) return plural(Math.ceil(left / HOUR), "hour");
  return plural(Math.ceil(left / DAY), "day");
}

// "3d", "5h", "now": the short form of the account choice.
export function shortly(left) {
  if (left <= 0) return "now";
  return left < DAY ? `${Math.ceil(left / HOUR)}h` : `${Math.ceil(left / DAY)}d`;
}

function plural(count, unit) {
  return `in ${count} ${unit}${count > 1 ? "s" : ""}`;
}
