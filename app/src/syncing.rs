//! The window's own syncing: when the schedule of the core says a sync is due, through the folder or relay the settings name.

use serde::Serialize;
use youarel_core::folder_rules::{self, Seen};
use youarel_core::sync_rules::{self, Outcome, Schedule, Sync};
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
    /// Whether the window watches the shared folder, so a change from another device arrives within seconds.
    pub watched: bool,
}

/// What the window knows about its syncs beyond its settings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Syncing {
    schedule: Schedule,
    /// The sync that runs, or `None`.
    running: Option<Sync>,
    error: Option<String>,
    /// The name of this device's own bundle in the shared folder, once a sync learned it.
    own: Option<String>,
    /// Whether the window watches the shared folder.
    watched: bool,
    /// Whether a sync failed since the watch last started, which may mean the watch lost its folder unannounced.
    rewatch: bool,
}

impl Syncing {
    /// Notes an edit at `now`, which the other devices then need once a burst of edits settles.
    pub const fn edited(&mut self, now: u64) {
        self.schedule.pending = true;
        self.schedule.edited = Some(now);
    }

    /// Notes that this device holds what no publish carried, as the store says at the window's start.
    pub const fn unpublished(&mut self) {
        self.schedule.pending = true;
    }

    /// Makes the next sync due at once, as after a change of the settings or a person's request: a publish when something waits for one, and a merge otherwise.
    pub fn now(&mut self) {
        self.schedule = Schedule {
            pending: self.schedule.pending,
            ..Schedule::default()
        };
    }

    /// The sync that should begin at `now` through `choice`, for a device that `has_account` says belongs to one, which this call marks begun; `None` when none should.
    pub fn begin(&mut self, choice: &SyncChoice, has_account: bool, now: u64) -> Option<Sync> {
        if !choice.enabled || choice.target.is_empty() || !has_account || self.running.is_some() {
            return None;
        }
        let sync = sync_rules::due(self.schedule, now)?;
        self.running = Some(sync);
        self.schedule = sync_rules::begun(self.schedule, sync, now);
        Some(sync)
    }

    /// Records how the running sync ended: whether this device still has something new to say at the target, or why it failed; and answers whether it succeeded.
    pub fn end(&mut self, outcome: Result<bool, String>) -> bool {
        // An end with no sync begun counts as a publish's, which keeps whatever waits for one.
        let sync = self.running.take().unwrap_or(Sync::Publish);
        match outcome {
            Ok(pending) => {
                self.schedule = sync_rules::ended(self.schedule, sync, Outcome::Done { pending });
                self.error = None;
                true
            }
            Err(error) => {
                self.schedule = sync_rules::ended(self.schedule, sync, Outcome::Failed);
                self.error = Some(error);
                self.rewatch = true;
                false
            }
        }
    }

    /// Notes what the watch of the shared folder saw at `now`, and answers whether it may bring another device's bundle, which a merge then reads.
    pub fn noticed(&mut self, seen: Seen<'_>, now: u64) -> bool {
        let counts = folder_rules::counts(seen, self.own.as_deref());
        if counts {
            self.schedule.noticed = Some(now);
        }
        counts
    }

    /// Learns the name of this device's own bundle, whose changes the watch then passes over.
    pub fn learned(&mut self, own: String) {
        self.own = Some(own);
    }

    /// Records whether the window watches the shared folder.
    pub const fn watching(&mut self, watched: bool) {
        self.watched = watched;
    }

    /// Whether a sync failed since the last call, so the window starts its watch again, since Windows drops the watch of a deleted folder without a notice.
    pub const fn rewatch(&mut self) -> bool {
        let failed = self.rewatch;
        self.rewatch = false;
        failed
    }

    /// The syncing as the page draws it.
    #[must_use]
    pub fn view(&self, choice: &SyncChoice, has_account: bool) -> SyncView {
        let state = if !choice.enabled {
            SyncState::Off
        } else if !has_account {
            SyncState::NoAccount
        } else if self.running.is_some() {
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
            watched: self.watched,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SyncState, Syncing};
    use youarel_core::folder_rules::Seen;
    use youarel_core::sync_rules::{BACKOFF, INTERVAL, QUIET, SETTLE, Sync};
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
        assert_eq!(
            syncing.begin(&SyncChoice::default(), true, NOW),
            None,
            "off"
        );
        let unnamed = SyncChoice {
            target: String::new(),
            ..chosen()
        };
        assert_eq!(syncing.begin(&unnamed, true, NOW), None, "no target");
        assert_eq!(syncing.begin(&chosen(), false, NOW), None, "no account");
        assert_eq!(syncing.view(&chosen(), true).state, SyncState::Waiting);
        assert_eq!(
            syncing.begin(&chosen(), true, NOW),
            Some(Sync::Merge),
            "the first sync reads at once"
        );
        assert_eq!(syncing.view(&chosen(), true).state, SyncState::Syncing);
        assert_eq!(
            syncing.begin(&chosen(), true, NOW + INTERVAL),
            None,
            "one sync at a time"
        );
        assert!(syncing.end(Ok(false)));
        assert_eq!(syncing.begin(&chosen(), true, NOW + 1), None);
        syncing.edited(NOW + 1);
        assert_eq!(syncing.begin(&chosen(), true, NOW + SETTLE), None);
        assert_eq!(
            syncing.begin(&chosen(), true, NOW + 1 + SETTLE),
            Some(Sync::Publish),
            "an edit publishes once it settles"
        );
        assert!(!syncing.end(Err("unreachable".to_owned())));
        let failed = syncing.view(&chosen(), true);
        assert_eq!(
            (failed.state, failed.error.as_deref()),
            (SyncState::Failed, Some("unreachable"))
        );
        assert_eq!(syncing.begin(&chosen(), true, NOW + 2 + SETTLE), None);
        assert_eq!(
            syncing.begin(&chosen(), true, NOW + 1 + SETTLE + BACKOFF),
            Some(Sync::Publish),
            "a failed publish tries again once the backoff passes"
        );
        assert!(syncing.end(Ok(false)));
        syncing.now();
        assert_eq!(
            syncing.begin(&chosen(), true, NOW + 2 + SETTLE + BACKOFF),
            Some(Sync::Merge),
            "a request reads at once and leaves the published bundle alone"
        );
        assert!(syncing.end(Ok(false)));
        syncing.edited(NOW + 3 + SETTLE + BACKOFF);
        syncing.now();
        assert_eq!(
            syncing.begin(&chosen(), true, NOW + 3 + SETTLE + BACKOFF),
            Some(Sync::Publish),
            "and publishes at once an edit that waits"
        );
    }

    #[test]
    fn a_merge_that_finds_something_new_brings_one_publish() {
        let mut syncing = Syncing::default();
        assert_eq!(syncing.begin(&chosen(), true, NOW), Some(Sync::Merge));
        assert!(
            syncing.end(Ok(true)),
            "the folder lacks this device's bundle"
        );
        assert_eq!(syncing.begin(&chosen(), true, NOW + 1), Some(Sync::Publish));
        assert!(syncing.end(Ok(false)));
        for tick in 1..=4 {
            assert_eq!(
                syncing.begin(&chosen(), true, NOW + 1 + tick * INTERVAL),
                Some(Sync::Merge),
                "an idle window only reads"
            );
            assert!(syncing.end(Ok(false)));
        }
        let mut restarted = Syncing::default();
        restarted.unpublished();
        assert_eq!(
            restarted.begin(&chosen(), true, NOW),
            Some(Sync::Publish),
            "what an earlier run left unpublished goes at once"
        );
        assert!(restarted.end(Ok(false)));
        assert_eq!(restarted.begin(&chosen(), true, NOW + 1), None);
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
        assert!(!shown.watched);
    }

    #[test]
    fn another_device_s_bundle_brings_a_merge_once_the_folder_rests() {
        let mut syncing = Syncing::default();
        assert_eq!(syncing.begin(&chosen(), true, NOW), Some(Sync::Merge));
        assert!(syncing.end(Ok(false)));
        assert!(syncing.noticed(Seen::Touched("other.youarel"), NOW + 1));
        assert_eq!(
            syncing.begin(&chosen(), true, NOW + QUIET),
            None,
            "the folder rests first"
        );
        assert_eq!(
            syncing.begin(&chosen(), true, NOW + 1 + QUIET),
            Some(Sync::Merge)
        );
        assert!(syncing.end(Ok(false)));
        assert_eq!(
            syncing.begin(&chosen(), true, NOW + 1 + QUIET),
            None,
            "the merge answered the notice and found nothing to publish"
        );
    }

    #[test]
    fn the_device_s_own_bundle_and_what_lies_beside_it_bring_no_merge() {
        let mut syncing = Syncing::default();
        syncing.learned("mine.youarel".to_owned());
        assert!(!syncing.noticed(Seen::Touched("mine.youarel"), NOW));
        assert!(!syncing.noticed(Seen::Touched("._other.youarel"), NOW));
        assert!(!syncing.noticed(Seen::Read, NOW));
        assert_eq!(syncing.begin(&chosen(), true, NOW), Some(Sync::Merge));
        assert!(syncing.end(Ok(false)));
        assert_eq!(syncing.begin(&chosen(), true, NOW + QUIET), None);
        assert!(
            syncing.noticed(Seen::Rescan, NOW + 1),
            "a rescan may hide a bundle"
        );
        assert_eq!(
            syncing.begin(&chosen(), true, NOW + 1 + QUIET),
            Some(Sync::Merge)
        );
    }

    #[test]
    fn a_notice_waits_out_a_backoff_and_a_failure_starts_the_watch_again() {
        let mut syncing = Syncing::default();
        syncing.watching(true);
        assert!(syncing.view(&chosen(), true).watched);
        assert!(!syncing.rewatch());
        assert_eq!(syncing.begin(&chosen(), true, NOW), Some(Sync::Merge));
        assert!(!syncing.end(Err("unreachable".to_owned())));
        assert!(syncing.rewatch(), "a failure starts the watch again");
        assert!(!syncing.rewatch(), "and only once");
        assert!(syncing.noticed(Seen::Touched("other.youarel"), NOW + 1));
        assert_eq!(
            syncing.begin(&chosen(), true, NOW + 1 + QUIET),
            None,
            "no merge during a backoff"
        );
        assert_eq!(
            syncing.begin(&chosen(), true, NOW + BACKOFF),
            Some(Sync::Merge),
            "a failed merge tries again as a merge"
        );
        syncing.watching(false);
        assert!(!syncing.view(&chosen(), true).watched);
    }

    #[test]
    fn a_failure_with_no_sync_begun_counts_as_a_failed_publish() {
        let mut syncing = Syncing::default();
        assert!(!syncing.end(Err("unreachable".to_owned())));
        assert_eq!(
            syncing.begin(&chosen(), true, NOW),
            Some(Sync::Publish),
            "it keeps whatever might wait for a publish"
        );
    }
}
