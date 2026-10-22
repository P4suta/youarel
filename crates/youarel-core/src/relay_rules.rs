//! The decisions of the optional relay: which requests it admits, how long it keeps an account, what fits an account, and which pairing nameplate it hands out.
//!
//! The relay stores only sealed segments, so these rules protect its availability and its account's freshness, not the secrecy of the state.

use alloc::vec::Vec;

mod retention;
pub use retention::{RetainError, RetainedSegment, Retention, retain};

#[must_use]
pub const fn incomplete_snapshot(header: crate::segment_rules::Header) -> bool {
    match header.kind() {
        crate::segment_rules::Kind::Delta => false,
        crate::segment_rules::Kind::Snapshot { part, parts } => part.saturating_add(1) < parts,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChainAppend {
    Continue,
    Restart,
    Refuse,
}

/// Admits a successor, a new snapshot, or an explicitly requested delta bridge after another transport caught up.
#[must_use]
pub fn append(
    latest: Option<crate::segment_rules::Header>,
    incoming: crate::segment_rules::Header,
    bridge: bool,
) -> ChainAppend {
    use crate::segment_rules::Kind;
    if latest.is_some_and(|held| held.sequence() >= incoming.sequence()) {
        return ChainAppend::Refuse;
    }
    if matches!(incoming.kind(), Kind::Snapshot { part: 0, .. })
        || (bridge && incoming.kind() == Kind::Delta)
    {
        return ChainAppend::Restart;
    }
    let Some(held) = latest else {
        return ChainAppend::Refuse;
    };
    if held.sequence().next() != Some(incoming.sequence()) {
        return ChainAppend::Refuse;
    }
    let continues = match (held.kind(), incoming.kind()) {
        (Kind::Delta, Kind::Delta) => true,
        (Kind::Snapshot { part, parts }, Kind::Delta) => part.saturating_add(1) == parts,
        (
            Kind::Snapshot { part, parts },
            Kind::Snapshot {
                part: next,
                parts: count,
            },
        ) => part.checked_add(1) == Some(next) && parts == count,
        (Kind::Delta, Kind::Snapshot { .. }) => false,
    };
    if continues {
        ChainAppend::Continue
    } else {
        ChainAppend::Refuse
    }
}

/// The milliseconds a request's stamp may differ from the relay's clock.
pub const WINDOW: u64 = 120_000;

/// The milliseconds in a day.
const DAY: u64 = 86_400_000;

/// The milliseconds the relay keeps an account after its last authenticated request.
pub const RETENTION: u64 = 400 * DAY;

/// The milliseconds after which a device that published nothing to the relay counts as gone, which a member hears of.
pub const INACTIVE: u64 = 180 * DAY;

/// The milliseconds a pairing mailbox stays open.
pub const MAILBOX_LIFETIME: u64 = 5 * 60 * 1000;

/// The most bytes an account's retained segment envelopes may hold together.
pub const MOST_BYTES: usize = 64 * 1024 * 1024;
pub const DELIVERY_FRAME: usize = 64 * 1024;
pub const LONGEST_DELIVERY: usize = MOST_BYTES.saturating_add(DELIVERY_FRAME);
pub const PAGE_SEGMENTS: usize = 64;
const LENGTH_BYTES: usize = size_of::<u32>();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveryPage {
    bytes: usize,
    count: usize,
}

impl DeliveryPage {
    #[must_use]
    pub fn new(metadata_bytes: usize) -> Option<Self> {
        let bytes = metadata_bytes.checked_add(DELIVERY_FRAME)?;
        (bytes <= LONGEST_DELIVERY).then_some(Self { bytes, count: 0 })
    }

    #[must_use]
    pub fn push(self, segment_bytes: usize) -> Option<Self> {
        let bytes = self
            .bytes
            .checked_add(segment_bytes)?
            .checked_add(LENGTH_BYTES)?;
        let count = self.count.checked_add(1)?;
        (bytes <= LONGEST_DELIVERY && count <= PAGE_SEGMENTS).then_some(Self { bytes, count })
    }
}

/// The most devices whose chains an account keeps.
pub const MOST_SLOTS: usize = 64;

/// The most pairing mailboxes the relay keeps open at once.
pub const MOST_MAILBOXES: usize = 4096;

/// The most bytes one mailbox holds, enough for an entry that carries the largest state.
pub const MOST_MAIL_BYTES: usize = 64 * 1024 * 1024;

/// The bytes of a request's nonce.
pub const NONCE: usize = 16;

/// A request's nonce.
pub type Nonce = [u8; NONCE];

/// A nonce the relay accepted, with the stamp of its request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Seen {
    pub nonce: Nonce,
    pub stamped: u64,
}

/// Whether the relay admits a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Admission {
    Fresh,
    /// Its stamp lies outside the window around the relay's clock.
    Stale,
    /// Its nonce came before.
    Replayed,
}

/// Whether a request stamped at `stamped` lies within the window around `now`.
#[must_use]
pub const fn fresh(now: u64, stamped: u64) -> bool {
    now.abs_diff(stamped) <= WINDOW
}

/// Whether the relay still keeps the nonce of a request stamped at `stamped`; once it forgets the nonce, the request is stale anyway.
#[must_use]
pub const fn retained(now: u64, stamped: u64) -> bool {
    now <= stamped.saturating_add(WINDOW)
}

/// Whether the relay admits a request stamped at `stamped` with `nonce`, given the nonces it retains.
#[must_use]
pub fn admit(now: u64, stamped: u64, nonce: &Nonce, seen: &[Seen]) -> Admission {
    if !fresh(now, stamped) {
        Admission::Stale
    } else if seen.iter().any(|entry| entry.nonce == *nonce) {
        Admission::Replayed
    } else {
        Admission::Fresh
    }
}

/// The nonces the relay keeps at `now`.
#[must_use]
pub fn prune(now: u64, seen: &[Seen]) -> Vec<Seen> {
    seen.iter()
        .copied()
        .filter(|entry| retained(now, entry.stamped))
        .collect()
}

/// When the relay deletes an account whose last authenticated request came at `last`.
#[must_use]
pub const fn expires(last: u64) -> u64 {
    last.saturating_add(RETENTION)
}

/// Whether a device whose last bundle reached the relay at `last` counts as gone at `now`.
#[must_use]
pub const fn inactive(now: u64, last: u64) -> bool {
    now.saturating_sub(last) > INACTIVE
}

/// Whether a mailbox holding `total` bytes may take a message of `incoming` bytes.
#[must_use]
pub const fn fits_mail(total: usize, incoming: usize) -> bool {
    match total.checked_add(incoming) {
        Some(after) => after <= MOST_MAIL_BYTES,
        None => false,
    }
}

/// Whether an account with `slots` bundles may take one from a device, which holds a slot already or not.
#[must_use]
pub const fn admits_slot(slots: usize, holds: bool) -> bool {
    holds || slots < MOST_SLOTS
}

/// Whether a mailbox opened at `opened` still takes and hands out messages at `now`.
#[must_use]
pub const fn open(opened: u64, now: u64) -> bool {
    match now.checked_sub(opened) {
        Some(age) => age <= MAILBOX_LIFETIME,
        None => false,
    }
}

/// The smallest positive nameplate that no open mailbox holds, which keeps the codes people type short.
///
/// Among the first `active.len() + 1` positive numbers one is free, so the search marks only those and ends there.
#[must_use]
pub fn nameplate(active: &[u16]) -> Option<u16> {
    let Ok(count) = u16::try_from(active.len()) else {
        return None;
    };
    let last = count.checked_add(1)?;
    let mut taken = alloc::vec![false; usize::from(last).saturating_add(1)];
    for open in active {
        if let Some(mark) = taken.get_mut(usize::from(*open)) {
            *mark = true;
        }
    }
    (1..=last).find(|candidate| !taken.get(usize::from(*candidate)).copied().unwrap_or(true))
}

/// The properties Kani proves over every clock reading, stamp, size, and every short list of nonces and nameplates.
#[cfg(kani)]
mod proofs {
    use super::{
        Admission, INACTIVE, MAILBOX_LIFETIME, MOST_MAIL_BYTES, NONCE, RETENTION, Seen, WINDOW,
        admit, expires, fits_mail, fresh, inactive, nameplate, open, retained,
    };

    #[kani::proof]
    fn only_an_unfinished_snapshot_reports_more_parts() {
        use crate::segment_rules::Kind;
        let Some(segment) = retained_segment(kani::any()) else {
            return;
        };
        let expected = match segment.header.kind() {
            Kind::Delta => false,
            Kind::Snapshot { part, parts } => u64::from(part) + 1 < u64::from(parts),
        };
        let incomplete = super::incomplete_snapshot(segment.header);
        kani::assert(
            incomplete == expected,
            "only a snapshot with a later part is incomplete",
        );
        kani::cover!(incomplete, "a snapshot still needs parts");
        kani::cover!(!incomplete, "a delta or final part is complete");
    }

    #[kani::proof]
    fn delivery_pages_bound_their_metadata_segments_and_framing_without_overflow() {
        let metadata: usize = kani::any();
        let initial = super::DeliveryPage::new(metadata);
        kani::assert(
            initial.is_some()
                == (metadata as u128 + super::DELIVERY_FRAME as u128
                    <= super::LONGEST_DELIVERY as u128),
            "metadata fits exactly within the framed limit",
        );
        let page = super::DeliveryPage {
            bytes: kani::any(),
            count: kani::any(),
        };
        kani::assume(page.bytes <= super::LONGEST_DELIVERY && page.count <= super::PAGE_SEGMENTS);
        let segment: usize = kani::any();
        let next = page.push(segment);
        let bytes = page.bytes as u128 + segment as u128 + super::LENGTH_BYTES as u128;
        let count = page.count as u128 + 1;
        kani::assert(
            next.is_some()
                == (bytes <= super::LONGEST_DELIVERY as u128
                    && count <= super::PAGE_SEGMENTS as u128),
            "a segment fits exactly within both limits",
        );
        if let Some(next) = next {
            kani::assert(
                next.bytes as u128 == bytes && next.count as u128 == count,
                "accepted page accounting is exact",
            );
        }
        kani::cover!(next.is_some(), "a page accepts a segment");
        kani::cover!(next.is_none(), "a full or overflowing page refuses");
    }

    #[kani::proof]
    #[kani::unwind(33)]
    fn appends_require_a_successor_or_an_explicit_restart() {
        use super::ChainAppend;
        use crate::segment_rules::Kind;
        let (Some(before), Some(after)) =
            (retained_segment(kani::any()), retained_segment(kani::any()))
        else {
            return;
        };
        let latest = kani::any::<bool>().then_some(before.header);
        let incoming = after.header;
        let bridge: bool = kani::any();
        let older = latest.is_none_or(|held| held.sequence() < incoming.sequence());
        let snapshot = matches!(incoming.kind(), Kind::Snapshot { part: 0, .. });
        let restart = snapshot || (bridge && incoming.kind() == Kind::Delta);
        let follows = latest.is_some_and(|held| {
            let sequence = incoming.sequence().get() as u128 == held.sequence().get() as u128 + 1;
            let kind = match incoming.kind() {
                Kind::Delta => match held.kind() {
                    Kind::Delta => true,
                    Kind::Snapshot { part, parts } => u64::from(part) + 1 == u64::from(parts),
                },
                Kind::Snapshot { part, parts } => match held.kind() {
                    Kind::Delta => false,
                    Kind::Snapshot {
                        part: prior,
                        parts: count,
                    } => u64::from(prior) + 1 == u64::from(part) && parts == count,
                },
            };
            sequence && kind
        });
        let result = super::append(latest, incoming, bridge);
        kani::assert(
            (result != ChainAppend::Refuse) == (older && (restart || follows)),
            "only an admissible extension or restart passes",
        );
        kani::assert(
            (result == ChainAppend::Restart) == (older && restart),
            "restarts are explicit",
        );
        kani::cover!(result == ChainAppend::Continue, "a successor continues");
        kani::cover!(
            result == ChainAppend::Restart && snapshot,
            "a snapshot restarts"
        );
        kani::cover!(
            result == ChainAppend::Restart && !snapshot,
            "a delta bridge restarts"
        );
        kani::cover!(
            result == ChainAppend::Refuse,
            "an invalid extension refuses"
        );
    }

    #[kani::proof]
    fn a_forgotten_nonce_belongs_to_a_stale_request() {
        let now: u64 = kani::any();
        let stamped: u64 = kani::any();
        kani::assert(
            fresh(now, stamped) == (now.abs_diff(stamped) <= WINDOW),
            "a request is fresh exactly within the window",
        );
        kani::assert(
            retained(now, stamped) || !fresh(now, stamped),
            "the relay forgets a nonce only once its request is stale",
        );
        kani::cover!(
            fresh(now, stamped) && !retained(now.wrapping_add(WINDOW), stamped),
            "a nonce forgotten later"
        );
    }

    /// Comparing two nonces compares their sixteen bytes, and the list holds three.
    #[kani::proof]
    #[kani::unwind(17)]
    fn a_request_passes_only_fresh_and_new() {
        let now: u64 = kani::any();
        let stamped: u64 = kani::any();
        let nonce: [u8; NONCE] = kani::any();
        let seen: [Seen; 3] = [
            Seen {
                nonce: kani::any(),
                stamped: kani::any(),
            },
            Seen {
                nonce: kani::any(),
                stamped: kani::any(),
            },
            Seen {
                nonce: kani::any(),
                stamped: kani::any(),
            },
        ];
        let count: usize = kani::any();
        kani::assume(count <= seen.len());
        let Some(retained) = seen.get(..count) else {
            return;
        };
        let decision = admit(now, stamped, &nonce, retained);
        let known = retained.iter().any(|entry| entry.nonce == nonce);
        kani::assert(
            (decision == Admission::Fresh) == (fresh(now, stamped) && !known),
            "a request passes exactly when it is fresh and its nonce is new",
        );
        kani::assert(
            decision != Admission::Replayed || known,
            "only a known nonce counts as a replay",
        );
        kani::cover!(decision == Admission::Replayed, "a replay");
    }

    #[kani::proof]
    fn the_quota_and_the_clocks_never_overflow() {
        let (total, incoming): (usize, usize) = (kani::any(), kani::any());
        let (now, last): (u64, u64) = (kani::any(), kani::any());
        kani::assert(
            u128::from(expires(last))
                == (u128::from(last) + u128::from(RETENTION)).min(u128::from(u64::MAX)),
            "an account expires its retention after its last request",
        );
        kani::assert(
            inactive(now, last) == (now > last && now - last > INACTIVE),
            "a device is gone only after its inactivity",
        );
        let opened: u64 = kani::any();
        kani::assert(
            open(opened, now) == (now >= opened && now - opened <= MAILBOX_LIFETIME),
            "a mailbox stays open for its lifetime from its opening",
        );
        kani::assert(
            fits_mail(total, incoming)
                == (incoming <= MOST_MAIL_BYTES && total <= MOST_MAIL_BYTES - incoming),
            "a message fits exactly when the mailbox stays within its limit",
        );
        kani::cover!(fits_mail(total, incoming) && incoming > 0, "a message fits");
    }

    /// The search visits at most four candidates, and each candidate scans the three nameplates.
    #[kani::proof]
    #[kani::unwind(5)]
    fn the_nameplate_is_the_smallest_free_one() {
        let active: [u16; 3] = kani::any();
        let count: usize = kani::any();
        kani::assume(count <= active.len());
        let Some(open) = active.get(..count) else {
            return;
        };
        let chosen = nameplate(open);
        kani::assert(
            chosen.is_some(),
            "a few open mailboxes leave a nameplate free",
        );
        if let Some(chosen) = chosen {
            kani::assert(
                chosen >= 1 && !open.contains(&chosen),
                "the nameplate is positive and free",
            );
            kani::assert(
                (1..chosen).all(|smaller| open.contains(&smaller)),
                "every smaller nameplate is taken",
            );
        }
        kani::cover!(chosen == Some(2), "the first nameplate is taken");
    }

    /// A request from beyond the window would let a recorded request come back once its nonce is gone; this probe claims one passes and must fail.
    #[kani::proof]
    fn rejects_a_request_from_beyond_the_window() {
        let now: u64 = kani::any();
        kani::assume(now > WINDOW);
        kani::assert(fresh(now, now - WINDOW - 1), "NEGATIVE_RELAY_CONTROL");
    }

    fn retained_segment(number: u64) -> Option<super::RetainedSegment> {
        use crate::segment_rules::{Header, Kind, Sequence};
        let kind = if kani::any() {
            Kind::Delta
        } else {
            Kind::Snapshot {
                part: kani::any(),
                parts: kani::any(),
            }
        };
        Some(super::RetainedSegment {
            header: Header::new(Sequence::new(number)?, [0; 32], kind)?,
            bytes: core::num::NonZeroUsize::new(kani::any())?,
        })
    }

    fn suffix_bytes(segments: &[super::RetainedSegment], from: usize) -> u128 {
        segments
            .iter()
            .skip(from)
            .fold(0u128, |total, segment| total + segment.bytes.get() as u128)
    }

    fn complete_at(segments: &[super::RetainedSegment], at: usize) -> bool {
        use crate::segment_rules::Kind;
        let Some(first) = segments.get(at) else {
            return false;
        };
        let Kind::Snapshot { part: 0, parts } = first.header.kind() else {
            return false;
        };
        if parts as usize > segments.len().saturating_sub(at) {
            return false;
        }
        (0..parts).all(|part| {
            segments.get(at + part as usize).is_some_and(|segment| {
                segment.header.kind() == Kind::Snapshot { part, parts }
                    && segment.header.sequence().get() as u128
                        == first.header.sequence().get() as u128 + u128::from(part)
            })
        })
    }

    #[kani::proof]
    #[kani::unwind(33)]
    fn retention_matches_the_newest_complete_snapshot_or_the_largest_bounded_suffix() {
        let (Some(first), Some(second), Some(third)) = (
            retained_segment(1),
            retained_segment(2),
            retained_segment(3),
        ) else {
            return;
        };
        let held = [first, second, third];
        let count: usize = kani::any();
        kani::assume(count <= held.len());
        let Some(segments) = held.get(..count) else {
            return;
        };
        let budget: usize = kani::any();
        let result = super::retain(segments, budget);
        let complete = (0..segments.len())
            .rev()
            .find(|at| complete_at(segments, *at))
            .filter(|at| suffix_bytes(segments, *at) <= budget as u128);
        let expected = complete.or_else(|| {
            (0..segments.len()).find(|at| suffix_bytes(segments, *at) <= budget as u128)
        });
        match (result, expected) {
            (Ok(retained), Some(from)) => {
                kani::assert(
                    retained.keep_from() == from
                        && retained.first() == segments[from].header.sequence(),
                    "the retained range is the specification's suffix",
                );
                kani::assert(
                    retained.bytes() as u128 == suffix_bytes(segments, from)
                        && retained.bytes() <= budget,
                    "the actual bytes fit without overflow",
                );
                kani::assert(
                    retained.snapshot() == complete.map(|at| segments[at].header.sequence()),
                    "only a complete retained snapshot is advertised",
                );
            }
            (Err(super::RetainError::Empty), None) => {
                kani::assert(segments.is_empty(), "only empty metadata is empty")
            }
            (Err(super::RetainError::TooLarge), None) => {
                kani::assert(!segments.is_empty(), "a nonempty suffix cannot fit")
            }
            _ => kani::assert(false, "retention agrees with every specified outcome"),
        }
        kani::cover!(
            matches!(result, Ok(retained) if retained.snapshot().is_some()),
            "a complete snapshot fits"
        );
        kani::cover!(
            matches!(result, Ok(retained) if retained.snapshot().is_none()),
            "only a delta or snapshot suffix fits"
        );
        kani::cover!(
            result == Err(super::RetainError::TooLarge),
            "even the last segment is too large"
        );
        kani::cover!(
            result == Err(super::RetainError::Empty),
            "there is no metadata"
        );
        kani::cover!(
            suffix_bytes(segments, 0) > usize::MAX as u128 && result.is_ok(),
            "the rejected prefix would overflow a machine word"
        );
    }

    #[kani::proof]
    #[kani::unwind(33)]
    fn retention_refuses_duplicate_or_reversed_sequences() {
        use crate::segment_rules::{Header, Kind, Sequence};
        let (Some(first), Some(second)) = (Sequence::new(kani::any()), Sequence::new(kani::any()))
        else {
            return;
        };
        let Some(header) = Header::new(first, [0; 32], Kind::Delta) else {
            return;
        };
        let Some(other) = Header::new(second, [0; 32], Kind::Delta) else {
            return;
        };
        let held = [
            super::RetainedSegment {
                header,
                bytes: core::num::NonZeroUsize::MIN,
            },
            super::RetainedSegment {
                header: other,
                bytes: core::num::NonZeroUsize::MIN,
            },
        ];
        let result = super::retain(&held, 2);
        kani::assert(
            (result == Err(super::RetainError::Order)) == (first >= second),
            "duplicates and reversed sequences refuse before retention",
        );
        kani::cover!(first >= second, "unordered metadata refuses");
        kani::cover!(result.is_ok(), "ordered metadata fits");
    }

    #[kani::proof]
    #[kani::unwind(33)]
    fn rejects_a_partial_snapshot_as_a_bootstrap() {
        use crate::segment_rules::{Header, Kind, Sequence};
        let Some(header) = Header::new(
            Sequence::FIRST,
            [0; 32],
            Kind::Snapshot { part: 0, parts: 2 },
        ) else {
            return;
        };
        let held = [super::RetainedSegment {
            header,
            bytes: core::num::NonZeroUsize::MIN,
        }];
        if let Ok(retained) = super::retain(&held, 1) {
            kani::assert(
                retained.snapshot().is_some(),
                "NEGATIVE_RELAY_RETENTION_CONTROL",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Admission, INACTIVE, MAILBOX_LIFETIME, MOST_BYTES, MOST_MAIL_BYTES, MOST_SLOTS, RETENTION,
        Seen, WINDOW, admit, admits_slot, expires, fits_mail, fresh, inactive, nameplate, open,
        prune,
    };
    use alloc::vec::Vec;

    #[test]
    fn delivery_pages_stop_at_the_byte_or_count_boundary() {
        assert_eq!(super::DeliveryPage::new(usize::MAX), None);
        let full = super::DeliveryPage::new(MOST_BYTES).unwrap();
        assert_eq!(full.push(0), None);
        let mut page = super::DeliveryPage::new(0).unwrap();
        for _ in 0..super::PAGE_SEGMENTS {
            page = page.push(1).unwrap();
        }
        assert_eq!(page.push(0), None);
        assert_eq!(super::DeliveryPage::new(0).unwrap().push(usize::MAX), None);
        let exact = super::DeliveryPage::new(0)
            .unwrap()
            .push(MOST_BYTES - size_of::<u32>())
            .unwrap();
        assert_eq!(exact.bytes, super::LONGEST_DELIVERY);
        assert_eq!(exact.push(0), None);
    }

    #[test]
    fn chains_accept_only_successors_complete_parts_or_explicit_restarts() {
        use super::{ChainAppend, append};
        use crate::segment_rules::{Header, Kind, Sequence};
        let header =
            |number, kind| Header::new(Sequence::new(number).unwrap(), [0; 32], kind).unwrap();
        let delta = header(1, Kind::Delta);
        let next = header(2, Kind::Delta);
        let gap = header(3, Kind::Delta);
        let snapshot = header(4, Kind::Snapshot { part: 0, parts: 2 });
        let last = header(5, Kind::Snapshot { part: 1, parts: 2 });
        assert_eq!(append(None, delta, false), ChainAppend::Refuse);
        assert_eq!(append(Some(delta), next, false), ChainAppend::Continue);
        assert_eq!(append(Some(delta), gap, false), ChainAppend::Refuse);
        assert_eq!(append(Some(delta), gap, true), ChainAppend::Restart);
        assert_eq!(append(None, gap, true), ChainAppend::Restart);
        assert_eq!(append(Some(delta), snapshot, false), ChainAppend::Restart);
        assert_eq!(append(Some(snapshot), last, false), ChainAppend::Continue);
        assert_eq!(append(None, last, false), ChainAppend::Refuse);
        assert_eq!(append(Some(last), snapshot, true), ChainAppend::Refuse);
        assert_eq!(
            append(Some(snapshot), header(5, Kind::Delta), false),
            ChainAppend::Refuse
        );
        assert_eq!(
            append(Some(last), header(6, Kind::Delta), false),
            ChainAppend::Continue
        );
        assert_eq!(
            append(
                Some(snapshot),
                header(5, Kind::Snapshot { part: 1, parts: 3 }),
                false
            ),
            ChainAppend::Refuse
        );
    }

    #[test]
    fn retention_keeps_a_complete_snapshot_or_reports_its_missing_prefix() {
        use crate::segment_rules::{Available, Fetch, Header, Kind, Sequence, fetch};
        use core::num::NonZeroUsize;
        let entry = |number, kind, bytes| super::RetainedSegment {
            header: Header::new(Sequence::new(number).unwrap(), [0; 32], kind).unwrap(),
            bytes: NonZeroUsize::new(bytes).unwrap(),
        };
        let old = entry(1, Kind::Snapshot { part: 0, parts: 1 }, 2);
        let delta = entry(2, Kind::Delta, 2);
        let snapshot = entry(3, Kind::Snapshot { part: 0, parts: 2 }, 3);
        let last = entry(4, Kind::Snapshot { part: 1, parts: 2 }, 3);
        let kept = super::retain(&[old, delta, snapshot, last], 6).unwrap();
        assert_eq!(kept.keep_from(), 2);
        assert_eq!(kept.snapshot(), Some(snapshot.header.sequence()));
        assert_eq!(kept.bytes(), 6);
        let partial = super::retain(&[old, delta, snapshot, last], 5).unwrap();
        assert_eq!(partial.keep_from(), 3);
        assert_eq!(partial.snapshot(), None);
        assert_eq!(
            fetch(
                None,
                Available {
                    first: partial.first(),
                    snapshot: partial.snapshot()
                },
                last.header.sequence()
            ),
            Fetch::Lost
        );
        assert_eq!(super::retain(&[last], 2), Err(super::RetainError::TooLarge));
        assert_eq!(
            super::retain(&[last, snapshot], 9),
            Err(super::RetainError::Order)
        );
    }

    #[test]
    fn retention_refuses_empty_input_and_handles_a_prefix_wider_than_a_machine_word() {
        use crate::segment_rules::{Header, Kind, Sequence};
        use core::num::NonZeroUsize;
        assert_eq!(
            super::retain(&[], usize::MAX),
            Err(super::RetainError::Empty)
        );
        let entry = |number, bytes| super::RetainedSegment {
            header: Header::new(Sequence::new(number).unwrap(), [0; 32], Kind::Delta).unwrap(),
            bytes,
        };
        let held = [entry(1, NonZeroUsize::MAX), entry(2, NonZeroUsize::MIN)];
        let kept = super::retain(&held, usize::MAX).unwrap();
        assert_eq!(kept.keep_from(), 1);
        assert_eq!(kept.bytes(), 1);
        assert_eq!(kept.snapshot(), None);
    }

    #[test]
    fn a_request_passes_within_the_window_once() {
        let now = 1_000_000;
        let seen = [Seen {
            nonce: [1; 16],
            stamped: now,
        }];
        assert_eq!(admit(now, now + WINDOW, &[2; 16], &seen), Admission::Fresh);
        assert_eq!(admit(now, now - WINDOW, &[2; 16], &seen), Admission::Fresh);
        assert_eq!(
            admit(now, now + WINDOW + 1, &[2; 16], &seen),
            Admission::Stale
        );
        assert_eq!(admit(now, now, &[1; 16], &seen), Admission::Replayed);
        assert!(fresh(u64::MAX, u64::MAX - WINDOW));
        assert_eq!(prune(now + WINDOW, &seen), seen);
        assert_eq!(prune(now + WINDOW + 1, &seen), []);
    }

    #[test]
    fn accounts_devices_and_mailboxes_age_by_their_limits() {
        assert_eq!(expires(5), 5 + RETENTION);
        assert_eq!(expires(u64::MAX), u64::MAX);
        assert!(!inactive(INACTIVE, 0));
        assert!(inactive(INACTIVE + 1, 0));
        assert!(!inactive(0, 5));
        assert!(open(10, 10 + MAILBOX_LIFETIME));
        assert!(!open(10, 11 + MAILBOX_LIFETIME));
        assert!(!open(10, 9));
    }

    #[test]
    fn an_account_holds_its_quota_and_its_slots() {
        assert!(fits_mail(MOST_MAIL_BYTES - 1, 1));
        assert!(!fits_mail(MOST_MAIL_BYTES, 1));
        assert!(!fits_mail(usize::MAX, 1));
        assert!(admits_slot(MOST_SLOTS - 1, false));
        assert!(!admits_slot(MOST_SLOTS, false));
        assert!(admits_slot(MOST_SLOTS, true));
    }

    /// The limits that `specs/PROTOCOL.md` states, in the units it states them in.
    #[test]
    fn the_limits_are_those_of_the_protocol() {
        const MINUTE: u64 = 60_000;
        const MEBIBYTE: usize = 1 << 20;
        assert!(expires(0) > 399 * super::DAY && expires(0) <= 400 * super::DAY);
        assert!(!inactive(180 * super::DAY, 0) && inactive(181 * super::DAY, 0));
        assert!(open(0, 4 * MINUTE) && !open(0, 6 * MINUTE));
        assert_eq!(MOST_BYTES, 64 * MEBIBYTE);
        assert!(fits_mail(0, 64 * MEBIBYTE) && !fits_mail(0, 64 * MEBIBYTE + 1));
        assert!(fresh(2 * MINUTE, 0) && !fresh(3 * MINUTE, 0));
    }

    #[test]
    fn the_nameplate_is_the_smallest_free_one() {
        assert_eq!(nameplate(&[]), Some(1));
        assert_eq!(nameplate(&[1, 3]), Some(2));
        assert_eq!(nameplate(&[2, 1]), Some(3));
        let full: Vec<u16> = (1..=u16::MAX).collect();
        assert_eq!(nameplate(&full), None);
    }
}
