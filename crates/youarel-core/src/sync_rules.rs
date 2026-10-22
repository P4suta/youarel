//! When the window syncs on its own: every few minutes, soon after an edit, soon after another device's bundle arrives, and after a failure only once a backoff has passed.
//!
//! The window reads the clock, the outcome of each sync, and what its watch of the shared folder notices, and asks here which sync is due.
//! A publish reads the other devices' bundles and writes this device's own, while a merge only reads them.
//! Every publish writes fresh bytes, so a notice brings a merge forward and never a publish, and two devices never answer each other's bundles without end.

/// One second in milliseconds.
pub const SECOND: u64 = 1_000;
/// How long the window waits between publishes when nothing changed.
pub const INTERVAL: u64 = 5 * 60 * SECOND;
/// How long the window waits after an edit, so a burst of edits publishes once.
pub const SETTLE: u64 = 10 * SECOND;
/// How long the shared folder stays quiet after a notice before the merge, so a bundle that another device or its cloud drive still writes settles first.
pub const QUIET: u64 = 2 * SECOND;
/// The wait after the first failure, which each further failure doubles.
pub const BACKOFF: u64 = 30 * SECOND;
/// The longest wait after failures.
pub const LONGEST_BACKOFF: u64 = 30 * 60 * SECOND;
/// The doublings after which the backoff stops growing, which keeps the shift in range.
const DOUBLINGS: u32 = 16;

/// What the window knows about its syncs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Schedule {
    /// When the last sync of either kind started, or `None` before the first; a backoff counts from it.
    pub last_attempt: Option<u64>,
    /// When the last publish started, or `None` before the first; the interval and the settling count from it.
    pub last_publish: Option<u64>,
    /// How many syncs failed in a row since the last that succeeded.
    pub failures: u32,
    /// Whether an edit happened since the last publish started.
    pub edited: bool,
    /// When the watch of the shared folder last noticed another device's bundle since the last sync started, or `None`.
    pub noticed: Option<u64>,
}

/// A sync the window runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Sync {
    /// Reads the other devices' bundles, then writes this device's own.
    Publish,
    /// Reads the other devices' bundles and writes nothing.
    Merge,
}

/// The wait after `failures` failures in a row: the backoff doubled for each failure after the first, up to the longest.
#[must_use]
pub fn backoff(failures: u32) -> u64 {
    let shift = failures.saturating_sub(1).min(DOUBLINGS);
    BACKOFF.saturating_mul(1_u64 << shift).min(LONGEST_BACKOFF)
}

/// How long after its clock the next publish waits: the backoff after failures, the settling after an edit, and the interval otherwise.
#[must_use]
pub fn wait(schedule: Schedule) -> u64 {
    if schedule.failures > 0 {
        backoff(schedule.failures)
    } else if schedule.edited {
        SETTLE
    } else {
        INTERVAL
    }
}

/// Whether `span` has passed since `since` at `now`; a clock that moved back before `since` waits for the whole span again.
const fn passed(since: u64, now: u64, span: u64) -> bool {
    match now.checked_sub(since) {
        Some(age) => age >= span,
        None => false,
    }
}

/// Whether a publish is due at `now`: the first at once, after failures once the backoff has passed since the last attempt, and otherwise once the wait has passed since the last publish.
#[must_use]
pub fn publish_due(schedule: Schedule, now: u64) -> bool {
    let clock = if schedule.failures > 0 {
        schedule.last_attempt
    } else {
        schedule.last_publish
    };
    clock.is_none_or(|since| passed(since, now, wait(schedule)))
}

/// Whether a merge is due at `now`: once the shared folder stayed quiet for [`QUIET`] after a notice, and never after a failure, whose backoff a notice leaves alone.
#[must_use]
pub fn merge_due(schedule: Schedule, now: u64) -> bool {
    schedule.failures == 0
        && schedule
            .noticed
            .is_some_and(|noticed| passed(noticed, now, QUIET))
}

/// The sync due at `now`, a publish before a merge, since a publish reads every bundle a merge would; `None` when neither is due.
#[must_use]
pub fn due(schedule: Schedule, now: u64) -> Option<Sync> {
    if publish_due(schedule, now) {
        Some(Sync::Publish)
    } else if merge_due(schedule, now) {
        Some(Sync::Merge)
    } else {
        None
    }
}

/// The schedule once `sync` begins at `now`: either sync counts as an attempt and answers the notices so far, and a publish also answers the edits and restarts the interval.
#[must_use]
pub const fn begun(schedule: Schedule, sync: Sync, now: u64) -> Schedule {
    match sync {
        Sync::Publish => Schedule {
            last_attempt: Some(now),
            last_publish: Some(now),
            edited: false,
            noticed: None,
            ..schedule
        },
        Sync::Merge => Schedule {
            last_attempt: Some(now),
            noticed: None,
            ..schedule
        },
    }
}

/// The properties Kani proves over every schedule and every clock reading.
#[cfg(any(test, kani))]
mod proofs {
    use super::{
        BACKOFF, INTERVAL, LONGEST_BACKOFF, QUIET, SETTLE, Schedule, Sync, backoff, begun, due,
        publish_due, wait,
    };

    /// Whether `span` has passed since `since` at `now`, in arithmetic too wide to wrap.
    fn elapsed(since: u64, now: u64, span: u64) -> bool {
        u128::from(now) >= u128::from(since).saturating_add(u128::from(span))
    }

    /// What the schedule promises, restated.
    ///
    /// The backoff lies between the first wait and the longest and never shrinks with another failure, and a publish waits no less than the settling and no longer than the longest wait.
    /// The first sync publishes at once, a due sync stays due as the clock moves on, and a due publish stays a publish.
    /// A publish falls due only once its wait has passed since its clock, after failures only past the backoff since the last attempt, and a notice never brings one forward.
    /// A merge falls due only after a notice that the folder has rested from, with no failure and no publish due, and such a notice is always answered.
    pub(super) fn spec(schedule: Schedule, now: u64, later: u64) -> bool {
        let failures = schedule.failures;
        let sync = due(schedule, now);
        let bounded =
            failures == 0 || (BACKOFF <= backoff(failures) && backoff(failures) <= LONGEST_BACKOFF);
        let growing = backoff(failures) <= backoff(failures.saturating_add(1));
        let waits = SETTLE <= wait(schedule) && wait(schedule) <= LONGEST_BACKOFF.max(INTERVAL);
        let first = schedule.last_attempt.is_some()
            || schedule.last_publish.is_some()
            || sync == Some(Sync::Publish);
        let stays = sync.is_none() || later < now || due(schedule, later).is_some();
        let publish_stays = sync != Some(Sync::Publish)
            || later < now
            || due(schedule, later) == Some(Sync::Publish);
        let (clock, span) = if failures > 0 {
            (schedule.last_attempt, backoff(failures))
        } else if schedule.edited {
            (schedule.last_publish, SETTLE)
        } else {
            (schedule.last_publish, INTERVAL)
        };
        let not_early =
            sync != Some(Sync::Publish) || clock.is_none_or(|since| elapsed(since, now, span));
        let backoff_kept = failures == 0
            || sync.is_none()
            || (sync == Some(Sync::Publish)
                && schedule
                    .last_attempt
                    .is_none_or(|since| elapsed(since, now, backoff(failures))));
        let unnoticed = Schedule {
            last_attempt: schedule.last_attempt,
            last_publish: schedule.last_publish,
            failures,
            edited: schedule.edited,
            noticed: None,
        };
        let unprompted = publish_due(schedule, now) == publish_due(unnoticed, now);
        let rested = schedule
            .noticed
            .is_some_and(|noticed| elapsed(noticed, now, QUIET));
        let quiet_kept =
            sync != Some(Sync::Merge) || (failures == 0 && rested && !publish_due(schedule, now));
        let answered = failures > 0 || !rested || sync.is_some();
        bounded
            && growing
            && waits
            && first
            && stays
            && publish_stays
            && not_early
            && backoff_kept
            && unprompted
            && quiet_kept
            && answered
    }

    /// What a sync that begins at `now` promises, for a sync due then: nothing stays due at that moment, either sync counts as the last attempt, answers the notices, and keeps the failures for its outcome to settle, a publish answers the edits and restarts the interval, and a merge leaves the edit and the publish's clock as they were.
    pub(super) fn begun_spec(schedule: Schedule, sync: Sync, now: u64) -> bool {
        let after = begun(schedule, sync, now);
        let answered = due(after, now).is_none()
            && after.last_attempt == Some(now)
            && after.noticed.is_none()
            && after.failures == schedule.failures;
        let kept = match sync {
            Sync::Publish => after.last_publish == Some(now) && !after.edited,
            Sync::Merge => {
                after.last_publish == schedule.last_publish && after.edited == schedule.edited
            }
        };
        answered && kept
    }

    /// Any schedule at all, each field drawn on its own.
    #[cfg(kani)]
    fn any_schedule() -> Schedule {
        Schedule {
            last_attempt: kani::any(),
            last_publish: kani::any(),
            failures: kani::any(),
            edited: kani::any(),
            noticed: kani::any(),
        }
    }

    /// Every schedule and every pair of clock readings.
    #[cfg(kani)]
    #[kani::proof]
    fn a_sync_waits_its_turn_and_stays_due_once_due() {
        let schedule = any_schedule();
        let now: u64 = kani::any();
        let later: u64 = kani::any();
        kani::assert(
            spec(schedule, now, later),
            "the schedule keeps its promises",
        );
        kani::cover!(
            schedule.failures > 20 && wait(schedule) == LONGEST_BACKOFF,
            "the longest backoff"
        );
        kani::cover!(
            schedule.last_attempt.is_some() && due(schedule, now).is_some(),
            "a later sync falls due"
        );
        kani::cover!(
            due(schedule, now) == Some(Sync::Merge),
            "a notice brings a merge forward"
        );
        kani::cover!(
            schedule.failures > 0
                && schedule
                    .noticed
                    .is_some_and(|noticed| elapsed(noticed, now, QUIET))
                && due(schedule, now).is_none(),
            "a notice waits out a backoff"
        );
    }

    /// Every schedule and every clock reading at which a sync is due, with the sync due then.
    #[cfg(kani)]
    #[kani::proof]
    fn a_begun_sync_is_due_no_more() {
        let schedule = any_schedule();
        let now: u64 = kani::any();
        let Some(sync) = due(schedule, now) else {
            return;
        };
        kani::assert(
            begun_spec(schedule, sync, now),
            "a sync answers what it begins with",
        );
        kani::cover!(sync == Sync::Publish, "a publish begins");
        kani::cover!(sync == Sync::Merge, "a merge begins");
    }

    /// A sync right after another that failed must wait; this probe claims one is due at once and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_retry_without_a_backoff() {
        let schedule = Schedule {
            last_attempt: Some(1_000),
            last_publish: Some(1_000),
            failures: 1,
            edited: false,
            noticed: None,
        };
        kani::assert(due(schedule, 1_001).is_some(), "NEGATIVE_SYNC_CONTROL");
    }

    /// A notice during a backoff waits it out; this probe claims it brings a merge once the folder rests and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_merge_during_a_backoff() {
        let schedule = Schedule {
            last_attempt: Some(1_000),
            last_publish: Some(1_000),
            failures: 1,
            edited: false,
            noticed: Some(1_000),
        };
        kani::assert(
            due(schedule, 1_000 + QUIET) == Some(Sync::Merge),
            "NEGATIVE_SYNC_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{begun_spec, spec};
    use super::{
        BACKOFF, INTERVAL, LONGEST_BACKOFF, QUIET, SECOND, SETTLE, Schedule, Sync, backoff, begun,
        due, merge_due, publish_due, wait,
    };
    use alloc::vec::Vec;

    const NOW: u64 = 1_000_000 * SECOND;

    /// A schedule whose last sync, a publish, began at `NOW` and succeeded.
    const SYNCED: Schedule = Schedule {
        last_attempt: Some(NOW),
        last_publish: Some(NOW),
        failures: 0,
        edited: false,
        noticed: None,
    };

    #[test]
    fn the_spans_are_the_ones_the_window_promises() {
        // The records name five minutes between syncs, ten seconds after an edit, two seconds of quiet after a notice, and backoffs from thirty seconds to thirty minutes.
        let minutes = |count: u64| count * 60 * 1_000;
        let seconds = |count: u64| count * 1_000;
        assert_eq!(SECOND, seconds(1));
        assert_eq!(INTERVAL, minutes(5));
        assert_eq!(SETTLE, seconds(10));
        assert_eq!(QUIET, seconds(2));
        assert_eq!(BACKOFF, seconds(30));
        assert_eq!(LONGEST_BACKOFF, minutes(30));
    }

    #[test]
    fn the_backoff_doubles_up_to_the_longest() {
        assert_eq!(backoff(1), BACKOFF);
        assert_eq!(backoff(2), 2 * BACKOFF);
        assert_eq!(backoff(3), 4 * BACKOFF);
        assert_eq!(backoff(7), LONGEST_BACKOFF);
        assert_eq!(backoff(u32::MAX), LONGEST_BACKOFF);
    }

    #[test]
    fn a_publish_falls_due_after_its_wait() {
        assert_eq!(
            due(Schedule::default(), 0),
            Some(Sync::Publish),
            "the first sync publishes at once"
        );
        assert_eq!(wait(SYNCED), INTERVAL);
        assert_eq!(due(SYNCED, NOW + INTERVAL - 1), None);
        assert_eq!(due(SYNCED, NOW + INTERVAL), Some(Sync::Publish));
        let edited = Schedule {
            edited: true,
            ..SYNCED
        };
        assert_eq!(wait(edited), SETTLE);
        assert_eq!(due(edited, NOW + SETTLE - 1), None);
        assert_eq!(due(edited, NOW + SETTLE), Some(Sync::Publish));
        let failed = Schedule {
            failures: 2,
            edited: true,
            ..SYNCED
        };
        assert_eq!(wait(failed), 2 * BACKOFF, "a failure outweighs an edit");
        assert_eq!(due(SYNCED, NOW - 1), None, "a clock that moved back waits");
    }

    #[test]
    fn the_interval_counts_from_the_last_publish_and_a_backoff_from_the_last_attempt() {
        let merged = Schedule {
            last_attempt: Some(NOW + SETTLE),
            ..SYNCED
        };
        assert_eq!(
            due(merged, NOW + INTERVAL),
            Some(Sync::Publish),
            "a merge leaves the interval alone"
        );
        let failed = Schedule {
            failures: 1,
            ..merged
        };
        assert_eq!(due(failed, NOW + SETTLE + BACKOFF - 1), None);
        assert_eq!(
            due(failed, NOW + SETTLE + BACKOFF),
            Some(Sync::Publish),
            "the backoff counts from the failed merge"
        );
        assert!(
            publish_due(
                Schedule {
                    last_attempt: None,
                    ..failed
                },
                NOW
            ),
            "a failure with no attempt behind it retries at once"
        );
    }

    #[test]
    fn a_notice_merges_after_the_quiet_span_and_never_during_a_backoff() {
        let noticed = Schedule {
            noticed: Some(NOW + SECOND),
            ..SYNCED
        };
        assert_eq!(
            due(noticed, NOW + SECOND + QUIET - 1),
            None,
            "the folder rests first"
        );
        assert_eq!(due(noticed, NOW + SECOND + QUIET), Some(Sync::Merge));
        assert_eq!(
            due(noticed, NOW + SECOND - 1),
            None,
            "a clock that moved back waits"
        );
        assert_eq!(
            due(
                Schedule {
                    last_publish: None,
                    ..noticed
                },
                NOW + SECOND + QUIET
            ),
            Some(Sync::Publish),
            "a publish due reads the bundle itself"
        );
        let failed = Schedule {
            failures: 1,
            ..noticed
        };
        assert!(!merge_due(failed, NOW + SECOND + QUIET));
        assert_eq!(
            due(failed, NOW + SECOND + QUIET),
            None,
            "a notice waits out the backoff"
        );
        assert_eq!(
            due(failed, NOW + BACKOFF),
            Some(Sync::Publish),
            "and the retry after it publishes"
        );
        assert!(!merge_due(SYNCED, u64::MAX), "no notice, no merge");
    }

    #[test]
    fn a_begun_sync_answers_what_it_covers() {
        let noticed = Schedule {
            edited: true,
            noticed: Some(NOW + SECOND),
            ..SYNCED
        };
        let merged = begun(noticed, Sync::Merge, NOW + SECOND + QUIET);
        assert_eq!(
            merged,
            Schedule {
                last_attempt: Some(NOW + SECOND + QUIET),
                noticed: None,
                ..noticed
            },
            "a merge keeps the edit and the publish's clock"
        );
        assert_eq!(
            due(merged, NOW + SETTLE),
            Some(Sync::Publish),
            "the edit still publishes once it settles"
        );
        assert_eq!(
            begun(noticed, Sync::Publish, NOW + SETTLE),
            Schedule {
                last_attempt: Some(NOW + SETTLE),
                last_publish: Some(NOW + SETTLE),
                failures: 0,
                edited: false,
                noticed: None,
            }
        );
        let failed = Schedule {
            failures: 3,
            ..noticed
        };
        assert_eq!(
            begun(failed, Sync::Publish, NOW).failures,
            3,
            "only the outcome settles the failures"
        );
    }

    /// Every schedule whose clocks and failures sit at the edges.
    fn edge_schedules() -> Vec<Schedule> {
        let clocks = [None, Some(NOW), Some(u64::MAX)];
        let mut schedules = Vec::new();
        for last_attempt in clocks {
            for last_publish in clocks {
                for noticed in clocks {
                    for failures in [0, 1, 7, 17, u32::MAX] {
                        for edited in [false, true] {
                            schedules.push(Schedule {
                                last_attempt,
                                last_publish,
                                failures,
                                edited,
                                noticed,
                            });
                        }
                    }
                }
            }
        }
        schedules
    }

    #[test]
    fn the_specification_agrees_over_the_edges() {
        let times = [
            0,
            1,
            NOW,
            NOW + QUIET,
            NOW + SETTLE,
            NOW + INTERVAL,
            u64::MAX,
        ];
        for schedule in edge_schedules() {
            for now in times {
                if let Some(sync) = due(schedule, now) {
                    assert!(begun_spec(schedule, sync, now), "{schedule:?} {now}");
                }
                for later in times {
                    assert!(spec(schedule, now, later), "{schedule:?} {now} {later}");
                }
            }
        }
    }
}
