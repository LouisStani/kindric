//! The limits of the accounts that are not Claude Code's, for their cards in the control center.
//! Codex accounts only: their daemon gives them, asking OpenAI each time, so each account is asked
//! at most once a minute, off the caller's thread, and the last figures stay meanwhile and once the
//! daemon stopped. An OpenCode account has none.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use kindric_accounts::agent::{self, Agent};
use kindric_accounts::{Paths, codex};
use serde::Serialize;

const ASKED_EVERY: Duration = Duration::from_secs(60);

/// A limit as a card shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CardLimit {
    /// How long the window lasts; `None` when unknown.
    pub minutes: Option<i64>,
    pub percent: i64,
    /// When the window starts again, Unix seconds; `None` when unknown or already past.
    pub resets_at: Option<i64>,
}

/// `limit` at `now`: nothing used any more once its window has started again.
fn card_limit(limit: &codex::Limit, now: i64) -> CardLimit {
    let past = limit.resets_at.is_some_and(|resets_at| resets_at <= now);
    CardLimit {
        minutes: limit.minutes,
        percent: if past { 0 } else { limit.used_percent },
        resets_at: limit.resets_at.filter(|_| !past),
    }
}

struct Asked {
    at: Instant,
    /// `None` until the daemon has given some.
    limits: Option<Vec<codex::Limit>>,
}

#[derive(Default)]
struct Known(HashMap<String, Asked>);

impl Known {
    /// The accounts of `names` to ask at `now`, marked as asked: a slow daemon is not asked twice.
    fn due(&mut self, names: &[String], now: Instant) -> Vec<String> {
        let mut due = Vec::new();
        for name in names {
            match self.0.get_mut(name) {
                Some(asked) if now.duration_since(asked.at) < ASKED_EVERY => continue,
                Some(asked) => asked.at = now,
                None => {
                    self.0.insert(name.clone(), Asked { at: now, limits: None });
                },
            }
            due.push(name.clone());
        }
        due
    }

    /// Keeps what the daemon of `name` gave; without figures, the last ones stay.
    fn keep(&mut self, name: &str, read: Option<Vec<codex::Limit>>) {
        if let (Some(asked), Some(limits)) = (self.0.get_mut(name), read) {
            asked.limits = Some(limits);
        }
    }

    fn shown(&self, name: &str, now: i64) -> Option<Vec<CardLimit>> {
        let limits = self.0.get(name)?.limits.as_ref()?;
        Some(limits.iter().map(|limit| card_limit(limit, now)).collect())
    }
}

/// The limits read so far.
#[derive(Clone, Default)]
pub struct AccountLimits {
    known: Arc<Mutex<Known>>,
}

impl AccountLimits {
    /// The limits of each Codex or OpenCode account of `names` at `now`, Unix seconds: none for an
    /// account without limits, `None` while they are not known. Asks the daemons it is time to
    /// ask, for a later call; `failed` is told of those that answered with an error.
    pub fn read(
        &self,
        paths: &Paths,
        names: &[String],
        now: i64,
        failed: impl Fn(&str, &io::Error) + Send + 'static,
    ) -> BTreeMap<String, Option<Vec<CardLimit>>> {
        let of_agent = |wanted: Agent| -> Vec<String> {
            names.iter().filter(|name| agent::read(&paths.account_dir(name)).is_ok_and(|agent| agent == wanted)).cloned().collect()
        };
        let codex_accounts = of_agent(Agent::Codex);
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        let due: Vec<_> = known.due(&codex_accounts, Instant::now()).into_iter().map(|name| (paths.account_dir(&name), name)).collect();
        let mut shown: BTreeMap<_, _> = codex_accounts.into_iter().map(|name| (name.clone(), known.shown(&name, now))).collect();
        drop(known);
        shown.extend(of_agent(Agent::OpenCode).into_iter().map(|name| (name, Some(Vec::new()))));
        if !due.is_empty() {
            let known = Arc::clone(&self.known);
            thread::spawn(move || {
                for (dir, name) in due {
                    match codex::limits(&dir) {
                        Ok(read) => known.lock().unwrap_or_else(PoisonError::into_inner).keep(&name, read),
                        Err(error) => failed(&name, &error),
                    }
                }
            });
        }
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(used_percent: i64, resets_at: Option<i64>) -> codex::Limit {
        codex::Limit { minutes: Some(300), used_percent, resets_at }
    }

    #[test]
    fn an_account_is_asked_once_a_minute() {
        let start = Instant::now();
        let names = ["work".to_string(), "home".to_string()];
        let mut known = Known::default();
        assert_eq!(known.due(&names[..1], start), ["work"]);
        assert_eq!(known.due(&names, start + Duration::from_secs(59)), ["home"], "asked 59 s ago: not again");
        assert_eq!(known.due(&names, start + Duration::from_secs(60)), ["work"]);
        assert_eq!(known.due(&names, start + Duration::from_secs(61)), Vec::<String>::new());
    }

    #[test]
    fn the_last_figures_stay_when_the_daemon_gives_none() {
        let mut known = Known::default();
        known.due(&["work".to_string()], Instant::now());
        assert_eq!(known.shown("work", 1_000), None, "not known yet");
        known.keep("work", Some(vec![limit(42, Some(2_000))]));
        known.keep("work", None);
        assert_eq!(known.shown("work", 1_000), Some(vec![CardLimit { minutes: Some(300), percent: 42, resets_at: Some(2_000) }]));
        known.keep("work", Some(Vec::new()));
        assert_eq!(known.shown("work", 1_000), Some(Vec::new()), "an account without limits is known to have none");
    }

    #[test]
    fn a_window_that_started_again_has_nothing_used() {
        assert_eq!(card_limit(&limit(42, Some(2_000)), 2_000), CardLimit { minutes: Some(300), percent: 0, resets_at: None });
        assert_eq!(card_limit(&limit(42, None), 2_000), CardLimit { minutes: Some(300), percent: 42, resets_at: None });
    }
}
