// The "show in stats" box of an isolated account in the settings (accounts section). An isolated
// account is left out of the statistics unless its name is in `stats_isolated` of the settings.
// Ordinary accounts have no box: they always count.

// The box of `account`, or null when it has none: `{ shown }`, ticked when its name is in `statsIsolated`.
export function statsBox(account, statsIsolated) {
  if (!account.isolated) return null;
  return { shown: (statsIsolated ?? []).includes(account.name) };
}
