//! How search results rank: by match score, then by how often and how recently the device opened them.
//!
//! Visits stay on the device that made them, so each device ranks by its own habits.

use core::cmp::Reverse;

/// One day in milliseconds.
pub const DAY: u64 = 86_400_000;
/// The visits beyond which more visits add nothing, which bounds every score.
pub const VISIT_CAP: u32 = 1_000;
/// How a visit's age weighs it: the age limit in days of each bucket and its weight, newest first.
pub const BUCKETS: [(u64, u32); 4] = [(4, 100), (14, 70), (31, 50), (90, 30)];
/// The weight of a visit older than every bucket.
pub const OLDEST_WEIGHT: u32 = 10;
/// The highest score `frecency` returns: the capped count at the newest bucket's weight.
pub const MOST: u32 = 100_000;

/// How often and how recently the device opened a shortcut.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Visits {
    pub count: u32,
    /// The time of the latest visit in milliseconds since the Unix epoch.
    pub last: u64,
}

/// The weight of a visit `age` milliseconds old.
fn weight(age: u64) -> u32 {
    for (days, weight) in BUCKETS {
        if age <= days.saturating_mul(DAY) {
            return weight;
        }
    }
    OLDEST_WEIGHT
}

/// The frecency of `visits` at time `now`: the capped visit count, weighted by the age of the latest visit.
///
/// A visit stamped after `now`, as a clock that moved back leaves it, counts as just made.
#[must_use]
pub fn frecency(visits: Visits, now: u64) -> u32 {
    if visits.count == 0 {
        return 0;
    }
    let age = now.saturating_sub(visits.last);
    visits.count.min(VISIT_CAP).saturating_mul(weight(age))
}

/// The place of a result, which sorts better results first and breaks every tie by the identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Rank<Id> {
    score: Reverse<u32>,
    frecency: Reverse<u32>,
    id: Id,
}

impl<Id> Rank<Id> {
    /// The rank of a result with a match score and a frecency.
    pub const fn new(score: u32, frecency: u32, id: Id) -> Self {
        Self {
            score: Reverse(score),
            frecency: Reverse(frecency),
            id,
        }
    }

    /// The result the rank places.
    pub fn into_id(self) -> Id {
        self.id
    }
}

/// The properties Kani proves over every visit count, every visit time, and every clock reading.
#[cfg(any(test, kani))]
mod proofs {
    use super::{MOST, VISIT_CAP, Visits, frecency};

    /// What `frecency` promises: nothing for no visits, a bounded score, and no loss from one more visit or a more recent one.
    pub(super) fn spec(visits: Visits, later: u64, now: u64) -> bool {
        let found = frecency(visits, now);
        let more = Visits {
            count: visits.count.saturating_add(1),
            ..visits
        };
        let recent = Visits {
            last: visits.last.max(later),
            ..visits
        };
        let none = visits.count != 0 || found == 0;
        let bounded = found <= MOST;
        let counts = frecency(more, now) >= found
            && (visits.count == 0 || visits.count >= VISIT_CAP || frecency(more, now) > found);
        let recency = frecency(recent, now) >= found;
        none && bounded && counts && recency
    }

    /// The buckets are four, so the weight takes at most five steps.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(6)]
    fn frecency_never_falls_with_more_or_more_recent_visits() {
        let visits = Visits {
            count: kani::any(),
            last: kani::any(),
        };
        let later: u64 = kani::any();
        let now: u64 = kani::any();
        kani::assert(spec(visits, later, now), "more visits never rank lower");
        kani::cover!(frecency(visits, now) == MOST, "the capped score");
        kani::cover!(
            visits.count > 0
                && frecency(visits, now) == visits.count.min(VISIT_CAP) * super::OLDEST_WEIGHT,
            "an old visit"
        );
    }

    /// A shortcut never opened must rank last; this probe claims it scores and must fail.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(6)]
    fn rejects_a_score_without_visits() {
        let visits = Visits {
            count: 0,
            last: kani::any(),
        };
        kani::assert(frecency(visits, kani::any()) > 0, "NEGATIVE_RANK_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::spec;
    use super::{DAY, MOST, Rank, Visits, frecency};
    use alloc::vec::Vec;

    const NOW: u64 = 1_000 * DAY;

    fn visits(count: u32, days_ago: u64) -> Visits {
        let age = days_ago.checked_mul(DAY).unwrap();
        Visits {
            count,
            last: NOW.checked_sub(age).unwrap(),
        }
    }

    #[test]
    fn recent_and_frequent_visits_score_higher() {
        assert_eq!(frecency(Visits::default(), NOW), 0);
        assert_eq!(frecency(visits(3, 0), NOW), 300);
        assert_eq!(frecency(visits(3, 4), NOW), 300);
        assert_eq!(frecency(visits(3, 5), NOW), 210);
        assert_eq!(frecency(visits(3, 31), NOW), 150);
        assert_eq!(frecency(visits(3, 90), NOW), 90);
        assert_eq!(frecency(visits(3, 91), NOW), 30);
        assert_eq!(frecency(visits(u32::MAX, 0), NOW), MOST);
        assert_eq!(
            frecency(
                Visits {
                    count: 2,
                    last: NOW + DAY
                },
                NOW
            ),
            200
        );
    }

    #[test]
    fn results_rank_by_score_then_frecency_then_identifier() {
        let mut ranks = alloc::vec![
            Rank::new(10, 0, 3),
            Rank::new(20, 0, 2),
            Rank::new(10, 5, 4),
            Rank::new(10, 5, 1),
        ];
        ranks.sort();
        let order: Vec<i32> = ranks.iter().map(|rank| rank.id).collect();
        assert_eq!(order, [2, 1, 4, 3]);
    }

    #[test]
    fn the_specification_agrees_over_the_edges() {
        let times = [0, DAY, 4 * DAY, 4 * DAY + 1, 90 * DAY + 1, NOW, u64::MAX];
        for count in [0, 1, 2, 999, 1_000, 1_001, u32::MAX] {
            for last in times {
                for later in times {
                    for now in times {
                        assert!(
                            spec(Visits { count, last }, later, now),
                            "{count} {last} {later} {now}"
                        );
                    }
                }
            }
        }
    }
}
