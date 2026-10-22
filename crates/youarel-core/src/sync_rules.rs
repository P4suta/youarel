//! When the window syncs on its own: every few minutes, soon after an edit, and after a failure only once a backoff has passed.
//!
//! The window reads the clock and the outcome of each sync, and asks here whether the next one is due.

/// One second in milliseconds.
pub const SECOND: u64 = 1_000;
/// How long the window waits between syncs when nothing changed.
pub const INTERVAL: u64 = 5 * 60 * SECOND;
/// How long the window waits after an edit, so a burst of edits syncs once.
pub const SETTLE: u64 = 10 * SECOND;
/// The wait after the first failure, which each further failure doubles.
pub const BACKOFF: u64 = 30 * SECOND;
/// The longest wait after failures.
pub const LONGEST_BACKOFF: u64 = 30 * 60 * SECOND;
/// The doublings after which the backoff stops growing, which keeps the shift in range.
const DOUBLINGS: u32 = 16;

/// What the window knows about its syncs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Schedule {
    /// When the last sync started, or `None` before the first.
    pub last_attempt: Option<u64>,
    /// How many syncs failed in a row since the last that succeeded.
    pub failures: u32,
    /// Whether an edit happened since the last sync started.
    pub edited: bool,
}

/// The wait after `failures` failures in a row: the backoff doubled for each failure after the first, up to the longest.
#[must_use]
pub fn backoff(failures: u32) -> u64 {
    let shift = failures.saturating_sub(1).min(DOUBLINGS);
    BACKOFF.saturating_mul(1_u64 << shift).min(LONGEST_BACKOFF)
}

/// How long after the last attempt the next sync waits.
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

/// Whether a sync is due at `now`; a clock that moved back before the last attempt waits for the full wait again.
#[must_use]
pub fn due(schedule: Schedule, now: u64) -> bool {
    schedule.last_attempt.is_none_or(|last| {
        now.checked_sub(last)
            .is_some_and(|age| age >= wait(schedule))
    })
}

/// The properties Kani proves over every schedule and every clock reading.
#[cfg(any(test, kani))]
mod proofs {
    use super::{BACKOFF, INTERVAL, LONGEST_BACKOFF, SETTLE, Schedule, backoff, due, wait};

    /// What the schedule promises: a backoff between the first and the longest that never shrinks with more failures, a wait no shorter than the settling and no longer than the longest backoff or the interval, the first sync at once, and a due sync that stays due as time passes.
    pub(super) fn spec(schedule: Schedule, now: u64, later: u64) -> bool {
        let failures = schedule.failures;
        let bounded =
            failures == 0 || (BACKOFF <= backoff(failures) && backoff(failures) <= LONGEST_BACKOFF);
        let growing = backoff(failures) <= backoff(failures.saturating_add(1));
        let waits = SETTLE <= wait(schedule) && wait(schedule) <= LONGEST_BACKOFF.max(INTERVAL);
        let first = schedule.last_attempt.is_some() || due(schedule, now);
        let stays = !due(schedule, now) || later < now || due(schedule, later);
        let not_early = !due(schedule, now)
            || schedule.last_attempt.is_none_or(|last| {
                now.checked_sub(last)
                    .is_some_and(|age| age >= wait(schedule))
            });
        bounded && growing && waits && first && stays && not_early
    }

    /// Every schedule and every pair of clock readings.
    #[cfg(kani)]
    #[kani::proof]
    fn a_sync_waits_its_turn_and_stays_due_once_due() {
        let schedule = Schedule {
            last_attempt: kani::any(),
            failures: kani::any(),
            edited: kani::any(),
        };
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
            schedule.last_attempt.is_some() && due(schedule, now),
            "a later sync falls due"
        );
    }

    /// A sync right after another that failed must wait; this probe claims it is due at once and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_retry_without_a_backoff() {
        let schedule = Schedule {
            last_attempt: Some(1_000),
            failures: 1,
            edited: false,
        };
        kani::assert(due(schedule, 1_001), "NEGATIVE_SYNC_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::spec;
    use super::{BACKOFF, INTERVAL, LONGEST_BACKOFF, SECOND, SETTLE, Schedule, backoff, due, wait};

    const NOW: u64 = 1_000_000 * SECOND;

    #[test]
    fn the_spans_are_the_ones_the_window_promises() {
        // The record names five minutes between syncs, ten seconds after an edit, and backoffs from thirty seconds to thirty minutes.
        let minutes = |count: u64| count * 60 * 1_000;
        let seconds = |count: u64| count * 1_000;
        assert_eq!(SECOND, seconds(1));
        assert_eq!(INTERVAL, minutes(5));
        assert_eq!(SETTLE, seconds(10));
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
    fn a_sync_falls_due_after_its_wait() {
        assert!(due(Schedule::default(), 0), "the first sync is due at once");
        let synced = Schedule {
            last_attempt: Some(NOW),
            failures: 0,
            edited: false,
        };
        assert_eq!(wait(synced), INTERVAL);
        assert!(!due(synced, NOW + INTERVAL - 1));
        assert!(due(synced, NOW + INTERVAL));
        let edited = Schedule {
            edited: true,
            ..synced
        };
        assert_eq!(wait(edited), SETTLE);
        assert!(due(edited, NOW + SETTLE));
        let failed = Schedule {
            failures: 2,
            edited: true,
            ..synced
        };
        assert_eq!(wait(failed), 2 * BACKOFF, "a failure outweighs an edit");
        assert!(!due(synced, NOW - 1), "a clock that moved back waits");
    }

    #[test]
    fn the_specification_agrees_over_the_edges() {
        let times = [0, 1, SETTLE, INTERVAL, NOW, NOW + LONGEST_BACKOFF, u64::MAX];
        for last_attempt in times.map(Some).into_iter().chain([None]) {
            for failures in [0, 1, 2, 6, 7, 16, 17, u32::MAX] {
                for edited in [false, true] {
                    let schedule = Schedule {
                        last_attempt,
                        failures,
                        edited,
                    };
                    for now in times {
                        for later in times {
                            assert!(spec(schedule, now, later), "{schedule:?} {now} {later}");
                        }
                    }
                }
            }
        }
    }
}
