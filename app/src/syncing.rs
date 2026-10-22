//! The window's own syncing: when the schedule of the core says a sync is due, through the folder or relay the settings name.

use serde::Serialize;
use youarel_core::sync_rules::{self, Schedule};
use youarel_store::settings::SyncChoice;

/// Where the window's syncing stands, as the sidebar and the settings show it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncState {
    /// The window does not sync on its own.
    Off,
    /// The device belongs to no account, so there is nothing to sync with.
    NoAccount,
    /// The window syncs on its own and has not synced yet.
    Waiting,
    Syncing,
    Synced,
    Failed,
}

/// The window's syncing as the page draws it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncView {
    pub enabled: bool,
    pub target: String,
    pub state: SyncState,
    /// When the last sync succeeded, in milliseconds since the Unix epoch.
    pub last: Option<u64>,
    /// Why the last sync failed, until one succeeds.
    pub error: Option<String>,
}

/// What the window knows about its syncs beyond its settings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Syncing {
    schedule: Schedule,
    running: bool,
    error: Option<String>,
}

impl Syncing {
    /// Notes an edit, which the other devices then need soon.
    pub const fn edited(&mut self) {
        self.schedule.edited = true;
    }

    /// Makes the next sync due at once, as after a change of the settings or a person's request.
    pub fn now(&mut self) {
        self.schedule = Schedule {
            edited: self.schedule.edited,
            ..Schedule::default()
        };
    }

    /// Whether a sync should begin at `now` through `choice`, for a device that `has_account` says belongs to one; marks it begun when it should.
    pub fn begin(&mut self, choice: &SyncChoice, has_account: bool, now: u64) -> bool {
        let ready = choice.enabled
            && !choice.target.is_empty()
            && has_account
            && !self.running
            && sync_rules::due(self.schedule, now);
        if ready {
            self.running = true;
            self.schedule.last_attempt = Some(now);
            self.schedule.edited = false;
        }
        ready
    }

    /// Records how a sync ended, and answers whether it succeeded.
    pub fn end(&mut self, outcome: Result<(), String>) -> bool {
        self.running = false;
        match outcome {
            Ok(()) => {
                self.schedule.failures = 0;
                self.error = None;
                true
            }
            Err(error) => {
                self.schedule.failures = self.schedule.failures.saturating_add(1);
                self.error = Some(error);
                false
            }
        }
    }

    /// The syncing as the page draws it.
    #[must_use]
    pub fn view(&self, choice: &SyncChoice, has_account: bool) -> SyncView {
        let state = if !choice.enabled {
            SyncState::Off
        } else if !has_account {
            SyncState::NoAccount
        } else if self.running {
            SyncState::Syncing
        } else if self.error.is_some() {
            SyncState::Failed
        } else if choice.last.is_some() {
            SyncState::Synced
        } else {
            SyncState::Waiting
        };
        SyncView {
            enabled: choice.enabled,
            target: choice.target.clone(),
            state,
            last: choice.last,
            error: self.error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SyncState, Syncing};
    use youarel_core::sync_rules::{BACKOFF, INTERVAL, SETTLE};
    use youarel_store::settings::SyncChoice;

    const NOW: u64 = 1_700_000_000_000;

    fn chosen() -> SyncChoice {
        SyncChoice {
            enabled: true,
            target: "/shared".to_owned(),
            last: None,
        }
    }

    #[test]
    fn a_sync_begins_only_when_chosen_due_and_free() {
        let mut syncing = Syncing::default();
        assert!(!syncing.begin(&SyncChoice::default(), true, NOW), "off");
        assert!(!syncing.begin(&chosen(), false, NOW), "no account");
        assert_eq!(syncing.view(&chosen(), true).state, SyncState::Waiting);
        assert!(syncing.begin(&chosen(), true, NOW));
        assert_eq!(syncing.view(&chosen(), true).state, SyncState::Syncing);
        assert!(
            !syncing.begin(&chosen(), true, NOW + INTERVAL),
            "one sync at a time"
        );
        assert!(syncing.end(Ok(())));
        assert!(!syncing.begin(&chosen(), true, NOW + 1));
        syncing.edited();
        assert!(
            syncing.begin(&chosen(), true, NOW + SETTLE),
            "an edit syncs soon"
        );
        assert!(!syncing.end(Err("unreachable".to_owned())));
        let failed = syncing.view(&chosen(), true);
        assert_eq!(
            (failed.state, failed.error.as_deref()),
            (SyncState::Failed, Some("unreachable"))
        );
        assert!(!syncing.begin(&chosen(), true, NOW + SETTLE + 1));
        assert!(
            syncing.begin(&chosen(), true, NOW + SETTLE + BACKOFF),
            "the backoff passes"
        );
        syncing.end(Ok(()));
        syncing.now();
        assert!(
            syncing.begin(&chosen(), true, NOW + SETTLE + BACKOFF + 1),
            "a request syncs at once"
        );
    }

    #[test]
    fn the_view_names_each_state() {
        let syncing = Syncing::default();
        assert_eq!(
            syncing.view(&SyncChoice::default(), true).state,
            SyncState::Off
        );
        assert_eq!(syncing.view(&chosen(), false).state, SyncState::NoAccount);
        let synced = SyncChoice {
            last: Some(NOW),
            ..chosen()
        };
        let shown = syncing.view(&synced, true);
        assert_eq!(
            (shown.state, shown.last, shown.target.as_str()),
            (SyncState::Synced, Some(NOW), "/shared")
        );
    }
}
