//! When the window syncs on its own: a merge every few minutes and after a notice, a publish only for something new, and after a failure only once a backoff has passed.
//!
//! The window reads the clock, the outcome of each sync, what its watch of the shared folder notices, and whether this device holds what no publish carried, and asks here which sync is due.
//! A publish reads the other devices' bundles and writes this device's own, while a merge only reads them.
//! Every publish writes fresh bytes, which every other device's cloud drive then carries, so only something new of this device's brings a publish.
//! A notice brings a merge forward and never a publish, so two devices never answer each other's bundles without end.

use crate::clock_rules::Stamp;

/// One second in milliseconds.
pub const SECOND: u64 = 1_000;
/// One day in milliseconds.
const DAY: u64 = 24 * 60 * 60 * SECOND;
/// How old this device's bundle at a relay grows before the device publishes it again with nothing new, long before the other members hear of the device as gone.
pub const REFRESH: u64 = 30 * DAY;
/// How long the window waits between merges when no notice brings one sooner.
pub const INTERVAL: u64 = 5 * 60 * SECOND;
/// How long the window waits after the last edit before it publishes, so a burst of edits publishes once.
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
    /// When the last sync of either kind started, or `None` before the first; the interval and a backoff count from it.
    pub last_attempt: Option<u64>,
    /// How many syncs failed in a row since the last that succeeded.
    pub failures: u32,
    /// Whether this device holds a write, an operation, or a missing bundle that no publish has carried yet.
    pub pending: bool,
    /// When the last edit happened since the last publish started, or `None`; the settling counts from it.
    pub edited: Option<u64>,
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

/// How a sync ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Outcome {
    /// It ran to its end and found whether this device still has something new to say at the target, which only a merge reads.
    Done { pending: bool },
    /// It failed, so the next sync waits out a backoff.
    Failed,
}

/// What a publish carries, or would carry now: the latest stamp of this device's own writes in the state, and how many operations the account's log holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Carried {
    /// The latest stamp of a write by this device that the state holds, or `None` for a state that holds none.
    pub latest: Option<Stamp>,
    /// How many operations the account's log holds.
    pub operations: usize,
}

/// Whether this device has something new to say at a target: a write of its own or an operation that the last publish there did not carry, or a target that lacks its bundle.
///
/// `held` is what a publish would carry now, `published` what the last publish to the target carried, or `None` when none reached it, and `missing` whether the target lacks this device's bundle.
/// The account's log only grows, and every new write of this device orders after every stamp its state holds, so something new shows as a later stamp or a longer log.
/// A write of this device that another device's later write replaced leaves no stamp behind, and that device's bundle carries what replaced it.
#[must_use]
pub fn pending(held: Carried, published: Option<Carried>, missing: bool) -> bool {
    missing
        || published.is_none_or(|published| {
            held.latest > published.latest || held.operations > published.operations
        })
}

/// Whether this device's bundle at a relay, which the relay stamped at `published`, needs publishing again at `now` with nothing new; a clock behind the relay's waits the whole span again.
#[must_use]
pub const fn stale(published: u64, now: u64) -> bool {
    passed(published, now, REFRESH)
}

/// The wait after `failures` failures in a row: the backoff doubled for each failure after the first, up to the longest.
#[must_use]
pub fn backoff(failures: u32) -> u64 {
    let shift = failures.saturating_sub(1).min(DOUBLINGS);
    BACKOFF.saturating_mul(1_u64 << shift).min(LONGEST_BACKOFF)
}

/// How long after its clock the next publish waits: the backoff since the last attempt after failures, and the settling since the last edit otherwise.
#[must_use]
pub fn wait(schedule: Schedule) -> u64 {
    if schedule.failures > 0 {
        backoff(schedule.failures)
    } else {
        SETTLE
    }
}

/// Whether `span` has passed since `since` at `now`; a clock that moved back before `since` waits for the whole span again.
const fn passed(since: u64, now: u64, span: u64) -> bool {
    match now.checked_sub(since) {
        Some(age) => age >= span,
        None => false,
    }
}

/// Whether a publish is due at `now`: never while nothing is pending, after failures once the backoff has passed since the last attempt, and otherwise once the settling has passed since the last edit, or at once when no edit waits.
#[must_use]
pub fn publish_due(schedule: Schedule, now: u64) -> bool {
    let clock = if schedule.failures > 0 {
        schedule.last_attempt
    } else {
        schedule.edited
    };
    schedule.pending && clock.is_none_or(|since| passed(since, now, wait(schedule)))
}

/// Whether a merge is due at `now`: after failures once the backoff has passed since the last attempt, and otherwise once the interval has passed since it.
///
/// With no failure, a notice brings a merge forward once the shared folder stayed quiet for [`QUIET`].
/// A notice leaves a backoff alone, so a failing folder sees no sync at every change.
#[must_use]
pub fn merge_due(schedule: Schedule, now: u64) -> bool {
    let attempted = |span| {
        schedule
            .last_attempt
            .is_none_or(|since| passed(since, now, span))
    };
    if schedule.failures > 0 {
        return attempted(backoff(schedule.failures));
    }
    attempted(INTERVAL)
        || schedule
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

/// The schedule once `sync` begins at `now`: either sync counts as an attempt and answers the notices so far, and a publish also carries what was pending, edits included.
#[must_use]
pub const fn begun(schedule: Schedule, sync: Sync, now: u64) -> Schedule {
    match sync {
        Sync::Publish => Schedule {
            last_attempt: Some(now),
            pending: false,
            edited: None,
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

/// The schedule once `sync` ended with `outcome`: a success clears the failures and keeps pending what the sync found, and a failure counts one more and keeps a failed publish's writes pending.
#[must_use]
pub const fn ended(schedule: Schedule, sync: Sync, outcome: Outcome) -> Schedule {
    match outcome {
        Outcome::Done { pending } => Schedule {
            failures: 0,
            pending: schedule.pending || pending,
            ..schedule
        },
        Outcome::Failed => Schedule {
            failures: schedule.failures.saturating_add(1),
            pending: schedule.pending || matches!(sync, Sync::Publish),
            ..schedule
        },
    }
}

/// The properties Kani proves over every schedule and every clock reading.
#[cfg(any(test, kani))]
mod proofs {
    use super::{
        BACKOFF, Carried, INTERVAL, LONGEST_BACKOFF, Outcome, QUIET, REFRESH, SETTLE, Schedule,
        Sync, backoff, begun, due, ended, pending, publish_due, stale, wait,
    };
    use crate::relay_rules;

    /// Whether `span` has passed since `since` at `now`, in arithmetic too wide to wrap.
    fn elapsed(since: u64, now: u64, span: u64) -> bool {
        u128::from(now) >= u128::from(since).saturating_add(u128::from(span))
    }

    /// What the schedule promises, restated.
    ///
    /// The backoff lies between the first wait and the longest and never shrinks with another failure, and a publish waits no less than the settling and no longer than the longest backoff.
    /// The first sync after a start begins at once, a due sync stays due as the clock moves on, and a due publish stays a publish.
    /// No publish falls due while nothing is pending, and a pending device publishes once the settling since its last edit has passed, or after failures the backoff since the last attempt, and never sooner; a notice never brings a publish forward.
    /// After failures no sync falls due before the backoff has passed since the last attempt, and once it has, a sync tries again.
    /// With no failure a merge falls due once the interval has passed since the last attempt or the folder rested after a notice, which is always answered, and a merge never stands in for a publish that is due.
    pub(super) fn spec(schedule: Schedule, now: u64, later: u64) -> bool {
        let failures = schedule.failures;
        let sync = due(schedule, now);
        let attempted = |span: u64| {
            schedule
                .last_attempt
                .is_none_or(|since| elapsed(since, now, span))
        };
        let bounded =
            failures == 0 || (BACKOFF <= backoff(failures) && backoff(failures) <= LONGEST_BACKOFF);
        let growing = backoff(failures) <= backoff(failures.saturating_add(1));
        let waits = SETTLE <= wait(schedule) && wait(schedule) <= LONGEST_BACKOFF;
        let first = schedule.last_attempt.is_some() || sync.is_some();
        let stays = sync.is_none() || later < now || due(schedule, later).is_some();
        let publish_stays = sync != Some(Sync::Publish)
            || later < now
            || due(schedule, later) == Some(Sync::Publish);
        let backed_off = attempted(backoff(failures));
        let settled = schedule
            .edited
            .is_none_or(|edit| elapsed(edit, now, SETTLE));
        let ready = if failures > 0 { backed_off } else { settled };
        let nothing_without_pending = schedule.pending || sync != Some(Sync::Publish);
        let pending_answered = !schedule.pending || !ready || sync == Some(Sync::Publish);
        let not_early = sync != Some(Sync::Publish) || ready;
        let backoff_kept = failures == 0 || sync.is_none() || backed_off;
        let retried = failures == 0 || !backed_off || sync.is_some();
        let merge_paced = failures > 0 || !attempted(INTERVAL) || sync.is_some();
        let unnoticed = Schedule {
            last_attempt: schedule.last_attempt,
            failures,
            pending: schedule.pending,
            edited: schedule.edited,
            noticed: None,
        };
        let unprompted = publish_due(schedule, now) == publish_due(unnoticed, now);
        let rested = schedule
            .noticed
            .is_some_and(|noticed| elapsed(noticed, now, QUIET));
        let merge_kept = sync != Some(Sync::Merge)
            || (!publish_due(schedule, now) && (failures > 0 || rested || attempted(INTERVAL)));
        let answered = failures > 0 || !rested || sync.is_some();
        bounded
            && growing
            && waits
            && first
            && stays
            && publish_stays
            && nothing_without_pending
            && pending_answered
            && not_early
            && backoff_kept
            && retried
            && merge_paced
            && unprompted
            && merge_kept
            && answered
    }

    /// What a sync that begins at `now` promises, for a sync due then: nothing stays due at that moment, either sync counts as the last attempt, answers the notices, and keeps the failures for its outcome to settle, a publish carries what was pending, edits included, and a merge leaves both as they were.
    pub(super) fn begun_spec(schedule: Schedule, sync: Sync, now: u64) -> bool {
        let after = begun(schedule, sync, now);
        let answered = due(after, now).is_none()
            && after.last_attempt == Some(now)
            && after.noticed.is_none()
            && after.failures == schedule.failures;
        let kept = match sync {
            Sync::Publish => !after.pending && after.edited.is_none(),
            Sync::Merge => after.pending == schedule.pending && after.edited == schedule.edited,
        };
        answered && kept
    }

    /// What a sync's end promises: a success clears the failures and leaves pending what was pending or what the sync found, a failure counts one more and leaves a failed publish's writes pending, and neither touches the clocks.
    /// A sync due at `now` that begins and then fails leaves the device as pending as it found it, so a failed publish loses no write.
    pub(super) fn ended_spec(schedule: Schedule, sync: Sync, outcome: Outcome, now: u64) -> bool {
        let after = ended(schedule, sync, outcome);
        let clocks = after.last_attempt == schedule.last_attempt
            && after.edited == schedule.edited
            && after.noticed == schedule.noticed;
        let counted = match outcome {
            Outcome::Done { pending } => {
                after.failures == 0 && after.pending == (schedule.pending || pending)
            }
            Outcome::Failed => {
                after.failures == schedule.failures.saturating_add(1)
                    && after.pending == (schedule.pending || sync == Sync::Publish)
            }
        };
        let kept = due(schedule, now) != Some(sync)
            || ended(begun(schedule, sync, now), sync, Outcome::Failed).pending == schedule.pending;
        clocks && counted && kept
    }

    /// What the pending decision promises, restated.
    ///
    /// A device that holds what its last publish to a target carried, while the target holds its bundle, has nothing new to say there.
    /// A target without the bundle, a target no publish reached, a later write of the device's own, and a longer log each make it pending, and nothing else does.
    pub(super) fn pending_spec(held: Carried, published: Option<Carried>, missing: bool) -> bool {
        let found = pending(held, published, missing);
        let quiet = published != Some(held) || missing || !found;
        let unheld = !missing || found;
        let unreached = published.is_some() || found;
        let written = published.is_none_or(|published| held.latest <= published.latest || found);
        let logged =
            published.is_none_or(|published| held.operations <= published.operations || found);
        let only_new = !found
            || missing
            || published.is_none_or(|published| {
                held.latest > published.latest || held.operations > published.operations
            });
        quiet && unheld && unreached && written && logged && only_new
    }

    /// What the renewal at a relay promises: a bundle needs it exactly once the span has passed since the relay stamped it, and a bundle renewed within the span never counts as gone.
    pub(super) fn stale_spec(published: u64, now: u64) -> bool {
        let renewed = stale(published, now) == elapsed(published, now, REFRESH);
        let present = stale(published, now) || !relay_rules::inactive(now, published);
        renewed && present
    }

    /// Any stamp or none, each field drawn on its own.
    #[cfg(kani)]
    fn any_stamp() -> Option<crate::clock_rules::Stamp> {
        kani::any::<bool>().then(|| crate::clock_rules::Stamp {
            wall: kani::any(),
            counter: kani::any(),
            device: crate::clock_rules::DeviceId(kani::any()),
        })
    }

    /// Anything a publish may carry.
    #[cfg(kani)]
    fn any_carried() -> Carried {
        Carried {
            latest: any_stamp(),
            operations: kani::any(),
        }
    }

    /// Every pair of what a device holds and what its last publish carried, or no publish, and either answer about its bundle at the target.
    #[cfg(kani)]
    #[kani::proof]
    fn only_something_new_makes_a_device_pending() {
        let held = any_carried();
        let published = kani::any::<bool>().then(any_carried);
        let missing: bool = kani::any();
        kani::assert(
            pending_spec(held, published, missing),
            "only something new makes a device pending",
        );
        let found = pending(held, published, missing);
        kani::cover!(
            !found && published == Some(held),
            "a device with nothing new"
        );
        kani::cover!(
            found
                && !missing
                && published.is_some_and(|published| {
                    held.latest > published.latest && held.operations == published.operations
                }),
            "a write of its own"
        );
        kani::cover!(
            found
                && !missing
                && published.is_some_and(|published| {
                    held.latest == published.latest && held.operations > published.operations
                }),
            "an operation of the log"
        );
        kani::cover!(
            found && missing && published == Some(held),
            "a target without its bundle"
        );
        kani::cover!(
            !found && published.is_some_and(|published| held.latest < published.latest),
            "a write that another device's replaced"
        );
    }

    /// Every stamp a relay gave a bundle and every clock reading.
    #[cfg(kani)]
    #[kani::proof]
    fn a_renewed_bundle_never_counts_as_gone() {
        let published: u64 = kani::any();
        let now: u64 = kani::any();
        kani::assert(
            stale_spec(published, now),
            "a bundle renewed in time never counts as gone",
        );
        kani::cover!(stale(published, now), "a bundle due for renewal");
        kani::cover!(
            !stale(published, now) && now > published,
            "a bundle renewed within the span"
        );
    }

    /// Any schedule at all, each field drawn on its own.
    #[cfg(kani)]
    fn any_schedule() -> Schedule {
        Schedule {
            last_attempt: kani::any(),
            failures: kani::any(),
            pending: kani::any(),
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
        let sync = due(schedule, now);
        let rested = schedule
            .noticed
            .is_some_and(|noticed| elapsed(noticed, now, QUIET));
        kani::cover!(
            schedule.failures > 20 && wait(schedule) == LONGEST_BACKOFF,
            "the longest backoff"
        );
        kani::cover!(
            schedule.failures == 0 && schedule.edited.is_some() && sync == Some(Sync::Publish),
            "an edit publishes once it settles"
        );
        kani::cover!(
            !schedule.pending
                && schedule.failures == 0
                && schedule.noticed.is_none()
                && schedule.last_attempt.is_some()
                && sync == Some(Sync::Merge),
            "an idle device merges on the interval"
        );
        kani::cover!(
            schedule.failures == 0
                && schedule
                    .last_attempt
                    .is_some_and(|since| !elapsed(since, now, INTERVAL))
                && sync == Some(Sync::Merge),
            "a notice brings a merge forward"
        );
        kani::cover!(
            schedule.failures > 0 && rested && sync.is_none(),
            "a notice waits out a backoff"
        );
        kani::cover!(
            schedule.failures > 0 && !schedule.pending && sync == Some(Sync::Merge),
            "a failed sync tries again as a merge"
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

    /// Every schedule, either sync, every way it may end, and every clock reading.
    #[cfg(kani)]
    #[kani::proof]
    fn an_ended_sync_keeps_what_waits_for_a_publish() {
        let schedule = any_schedule();
        let now: u64 = kani::any();
        let sync = if kani::any() {
            Sync::Publish
        } else {
            Sync::Merge
        };
        let outcome = if kani::any() {
            Outcome::Done {
                pending: kani::any(),
            }
        } else {
            Outcome::Failed
        };
        kani::assert(
            ended_spec(schedule, sync, outcome, now),
            "a sync's end keeps what waits for a publish",
        );
        let after = ended(schedule, sync, outcome);
        kani::cover!(
            sync == Sync::Publish
                && outcome == Outcome::Failed
                && !schedule.pending
                && after.pending,
            "a failed publish keeps its writes pending"
        );
        kani::cover!(
            sync == Sync::Merge && !schedule.pending && after.pending,
            "a merge finds something new"
        );
        kani::cover!(
            schedule.failures > 0 && after.failures == 0,
            "a success clears the failures"
        );
    }

    /// A sync right after another that failed must wait; this probe claims one is due at once and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_retry_without_a_backoff() {
        let schedule = Schedule {
            last_attempt: Some(1_000),
            failures: 1,
            pending: true,
            edited: None,
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
            failures: 1,
            pending: false,
            edited: None,
            noticed: Some(1_000),
        };
        kani::assert(
            due(schedule, 1_000 + QUIET) == Some(Sync::Merge),
            "NEGATIVE_SYNC_CONTROL",
        );
    }

    /// A device that holds just what its last publish carried, while the target holds its bundle, has nothing new to say; this probe claims a publish falls due for it anyway and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_publish_with_nothing_pending() {
        let carried = Carried {
            latest: Some(crate::clock_rules::Stamp {
                wall: 1_000,
                counter: 0,
                device: crate::clock_rules::DeviceId([1; 16]),
            }),
            operations: 3,
        };
        let schedule = Schedule {
            last_attempt: Some(1_000),
            failures: 0,
            pending: pending(carried, Some(carried), false),
            edited: None,
            noticed: None,
        };
        kani::assert(
            due(schedule, u64::MAX) == Some(Sync::Publish),
            "NEGATIVE_SYNC_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{begun_spec, ended_spec, pending_spec, spec, stale_spec};
    use super::{
        BACKOFF, Carried, INTERVAL, LONGEST_BACKOFF, Outcome, QUIET, REFRESH, SECOND, SETTLE,
        Schedule, Sync, backoff, begun, due, ended, merge_due, pending, publish_due, stale, wait,
    };
    use crate::clock_rules::{DeviceId, Stamp};
    use crate::relay_rules::INACTIVE;
    use alloc::vec::Vec;

    const NOW: u64 = 1_000_000 * SECOND;

    /// A schedule whose last sync began at `NOW` and succeeded, with nothing pending.
    const SYNCED: Schedule = Schedule {
        last_attempt: Some(NOW),
        failures: 0,
        pending: false,
        edited: None,
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
        let days = |count: u64| count * 24 * minutes(60);
        assert_eq!(REFRESH, days(30));
        assert_eq!(
            INACTIVE,
            days(180),
            "a bundle renewed in time never counts as gone"
        );
    }

    /// What a publish carries: this device's latest write at the wall time `latest`, if any, and `operations` operations of the log.
    fn carried(latest: Option<u64>, operations: usize) -> Carried {
        Carried {
            latest: latest.map(|wall| Stamp {
                wall,
                counter: 0,
                device: DeviceId([1; 16]),
            }),
            operations,
        }
    }

    #[test]
    fn only_something_new_makes_a_device_pending() {
        let published = carried(Some(5), 3);
        assert!(!pending(published, Some(published), false), "nothing new");
        assert!(
            pending(published, Some(published), true),
            "a target without the bundle"
        );
        assert!(
            pending(published, None, false),
            "a target no publish reached"
        );
        assert!(
            pending(carried(Some(6), 3), Some(published), false),
            "a later write"
        );
        assert!(
            pending(carried(Some(5), 4), Some(published), false),
            "a longer log"
        );
        assert!(
            !pending(carried(Some(4), 3), Some(published), false),
            "a write that another device's replaced"
        );
        assert!(
            !pending(carried(Some(5), 2), Some(published), false),
            "a log that holds less"
        );
        assert!(
            pending(carried(Some(1), 3), Some(carried(None, 3)), false),
            "a first write"
        );
        assert!(!pending(carried(None, 0), Some(carried(None, 0)), false));
    }

    #[test]
    fn a_bundle_at_a_relay_needs_renewal_once_the_span_passed() {
        assert!(!stale(NOW, NOW + REFRESH - 1));
        assert!(stale(NOW, NOW + REFRESH));
        assert!(!stale(NOW, NOW - 1), "a clock behind the relay's waits");
        assert!(stale(0, u64::MAX));
    }

    #[test]
    fn the_pending_specifications_agree_over_the_edges() {
        let stamps = [None, Some(0), Some(5), Some(u64::MAX)];
        let counts = [0, 3, usize::MAX];
        let mut held = Vec::new();
        for latest in stamps {
            for operations in counts {
                held.push(carried(latest, operations));
            }
        }
        for now in &held {
            for published in &held {
                for missing in [false, true] {
                    assert!(pending_spec(*now, Some(*published), missing));
                    assert!(pending_spec(*now, None, missing));
                }
            }
        }
        let times = [0, 1, NOW, NOW + REFRESH, NOW + INACTIVE, u64::MAX];
        for published in times {
            for now in times {
                assert!(stale_spec(published, now), "{published} {now}");
            }
        }
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
    fn a_publish_falls_due_only_while_something_is_pending() {
        assert_eq!(
            due(Schedule::default(), 0),
            Some(Sync::Merge),
            "the first sync after a start reads at once"
        );
        let unpublished = Schedule {
            pending: true,
            ..Schedule::default()
        };
        assert_eq!(
            due(unpublished, 0),
            Some(Sync::Publish),
            "and publishes at once what waits"
        );
        assert!(!publish_due(SYNCED, u64::MAX), "nothing new, no publish");
        let edited = Schedule {
            pending: true,
            edited: Some(NOW + SECOND),
            ..SYNCED
        };
        assert_eq!(wait(edited), SETTLE);
        assert_eq!(due(edited, NOW + SECOND + SETTLE - 1), None);
        assert_eq!(due(edited, NOW + SECOND + SETTLE), Some(Sync::Publish));
        assert_eq!(
            due(edited, NOW + SECOND - 1),
            None,
            "a clock that moved back waits"
        );
        let failed = Schedule {
            failures: 2,
            ..edited
        };
        assert_eq!(wait(failed), 2 * BACKOFF, "a failure outweighs an edit");
        assert_eq!(due(failed, NOW + 2 * BACKOFF - 1), None);
        assert_eq!(
            due(failed, NOW + 2 * BACKOFF),
            Some(Sync::Publish),
            "the retry counts from the last attempt"
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
    fn a_merge_falls_due_on_the_interval_and_a_failed_one_tries_again_after_the_backoff() {
        assert_eq!(due(SYNCED, NOW + INTERVAL - 1), None);
        assert_eq!(due(SYNCED, NOW + INTERVAL), Some(Sync::Merge));
        assert_eq!(due(SYNCED, NOW - 1), None, "a clock that moved back waits");
        let failed = Schedule {
            failures: 1,
            ..SYNCED
        };
        assert_eq!(due(failed, NOW + BACKOFF - 1), None);
        assert_eq!(due(failed, NOW + BACKOFF), Some(Sync::Merge));
        let failing = Schedule {
            failures: 7,
            ..SYNCED
        };
        assert_eq!(
            due(failing, NOW + INTERVAL),
            None,
            "a backoff outlasts the interval"
        );
        assert_eq!(due(failing, NOW + LONGEST_BACKOFF), Some(Sync::Merge));
        assert!(
            merge_due(
                Schedule {
                    last_attempt: None,
                    ..failing
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
                    pending: true,
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
            Some(Sync::Merge),
            "and the retry after it reads"
        );
    }

    #[test]
    fn a_begun_sync_answers_what_it_covers() {
        let noticed = Schedule {
            pending: true,
            edited: Some(NOW + SECOND),
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
            "a merge keeps the edit and what is pending"
        );
        assert_eq!(
            due(merged, NOW + SECOND + SETTLE),
            Some(Sync::Publish),
            "the edit still publishes once it settles"
        );
        assert_eq!(
            begun(noticed, Sync::Publish, NOW + SETTLE),
            Schedule {
                last_attempt: Some(NOW + SETTLE),
                failures: 0,
                pending: false,
                edited: None,
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

    #[test]
    fn an_ended_sync_keeps_what_waits_for_a_publish() {
        let published = begun(
            Schedule {
                pending: true,
                failures: 2,
                ..SYNCED
            },
            Sync::Publish,
            NOW,
        );
        let failed = ended(published, Sync::Publish, Outcome::Failed);
        assert_eq!(
            (failed.failures, failed.pending),
            (3, true),
            "a failed publish keeps its writes pending"
        );
        let carried = ended(published, Sync::Publish, Outcome::Done { pending: false });
        assert_eq!((carried.failures, carried.pending), (0, false));
        let unread = ended(SYNCED, Sync::Merge, Outcome::Failed);
        assert_eq!(
            (unread.failures, unread.pending),
            (1, false),
            "a failed merge leaves nothing to publish"
        );
        let found = ended(SYNCED, Sync::Merge, Outcome::Done { pending: true });
        assert!(found.pending, "a merge that finds something new");
        let edited = Schedule {
            pending: true,
            ..SYNCED
        };
        assert!(
            ended(edited, Sync::Merge, Outcome::Done { pending: false }).pending,
            "a merge never drops an edit that waits"
        );
        assert_eq!(
            ended(
                Schedule {
                    failures: u32::MAX,
                    ..SYNCED
                },
                Sync::Merge,
                Outcome::Failed
            )
            .failures,
            u32::MAX
        );
    }

    /// Every schedule whose clocks and failures sit at the edges.
    fn edge_schedules() -> Vec<Schedule> {
        let clocks = [None, Some(NOW), Some(u64::MAX)];
        let mut schedules = Vec::new();
        for last_attempt in clocks {
            for edited in clocks {
                for noticed in clocks {
                    for failures in [0, 1, 7, 17, u32::MAX] {
                        for pending in [false, true] {
                            schedules.push(Schedule {
                                last_attempt,
                                failures,
                                pending,
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
            NOW + BACKOFF,
            NOW + INTERVAL,
            u64::MAX,
        ];
        let outcomes = [
            Outcome::Done { pending: false },
            Outcome::Done { pending: true },
            Outcome::Failed,
        ];
        for schedule in edge_schedules() {
            for now in times {
                if let Some(sync) = due(schedule, now) {
                    assert!(begun_spec(schedule, sync, now), "{schedule:?} {now}");
                }
                for sync in [Sync::Publish, Sync::Merge] {
                    for outcome in outcomes {
                        assert!(
                            ended_spec(schedule, sync, outcome, now),
                            "{schedule:?} {sync:?} {outcome:?} {now}"
                        );
                    }
                }
                for later in times {
                    assert!(spec(schedule, now, later), "{schedule:?} {now} {later}");
                }
            }
        }
    }
}
