//! Admission of a device's segment chain, publication watermarks, and whole-record snapshot parts.

mod records;
pub use records::Parts;

use crate::clock_rules::{DeviceId, Stamp};
use alloc::vec::Vec;
use core::num::NonZeroU64;
use core::ops::Range;

pub const DIGEST_BYTES: usize = 32;
pub type Digest = [u8; DIGEST_BYTES];
pub const LONGEST_BODY: usize = 8 * 1024 * 1024;
pub const LONGEST_SEGMENT: usize = 16 * 1024 * 1024;
pub const LONGEST_ENVELOPE: usize = 64 * 1024 * 1024;
pub const MOST_DELTAS: u64 = 256;
pub const COMPACTION_RATIO: usize = 2;

/// A position in a chain, starting at one.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Sequence(NonZeroU64);

impl Sequence {
    pub const FIRST: Self = Self(NonZeroU64::MIN);
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self.get().checked_add(1) {
            Some(value) => Self::new(value),
            None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Delta,
    Snapshot { part: u32, parts: u32 },
}

/// A header whose sequence and snapshot parts fit in a chain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    sequence: Sequence,
    previous: Digest,
    kind: Kind,
}

impl Header {
    #[must_use]
    pub fn new(sequence: Sequence, previous: Digest, kind: Kind) -> Option<Self> {
        if let Kind::Snapshot { part, parts } = kind {
            let following = parts.checked_sub(part)?.checked_sub(1)?;
            if sequence.get() <= u64::from(part)
                || sequence.get().checked_add(u64::from(following)).is_none()
            {
                return None;
            }
        }
        Some(Self {
            sequence,
            previous,
            kind,
        })
    }

    #[must_use]
    pub const fn sequence(self) -> Sequence {
        self.sequence
    }

    #[must_use]
    pub const fn previous(self) -> Digest {
        self.previous
    }

    #[must_use]
    pub const fn kind(self) -> Kind {
        self.kind
    }
}

/// The last plaintext applied from one peer, or no cursor before its first segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
    sequence: Sequence,
    digest: Digest,
}

impl Cursor {
    #[must_use]
    pub const fn new(sequence: Sequence, digest: Digest) -> Self {
        Self { sequence, digest }
    }

    #[must_use]
    pub const fn sequence(self) -> Sequence {
        self.sequence
    }

    #[must_use]
    pub const fn digest(self) -> Digest {
        self.digest
    }
}

/// The first retained segment and the start of the newest complete snapshot, when one fits the transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Available {
    pub first: Sequence,
    pub snapshot: Option<Sequence>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    Apply,
    Reapply,
    Old,
    Gap,
    Jump(Sequence),
    Lost,
    Forked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Decision {
    pub cursor: Option<Cursor>,
    pub verdict: Verdict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fetch {
    Segment(Sequence),
    Complete,
    Lost,
}

#[must_use]
pub fn fetch(cursor: Option<Cursor>, available: Available, latest: Sequence) -> Fetch {
    if cursor.is_some_and(|cursor| cursor.sequence() >= latest) {
        return Fetch::Complete;
    }
    let Some(expected) = cursor.map_or(Some(Sequence::FIRST), |cursor| cursor.sequence().next())
    else {
        return Fetch::Complete;
    };
    if expected >= available.first {
        Fetch::Segment(expected)
    } else if let Some(snapshot) = available
        .snapshot
        .filter(|snapshot| available.first <= *snapshot && *snapshot <= latest)
    {
        Fetch::Segment(snapshot)
    } else {
        Fetch::Lost
    }
}

/// Admits authenticated plaintext in sequence, or the first part of an advertised complete snapshot after a gap.
/// Only Apply advances the cursor; Jump asks the adapter to fetch its named snapshot first.
#[must_use]
pub fn step(
    cursor: Option<Cursor>,
    header: Header,
    body_digest: Digest,
    available: Available,
) -> Decision {
    let previous = cursor.map_or([0; DIGEST_BYTES], Cursor::digest);
    let last = cursor.map_or(0, |held| held.sequence.get());
    let expected = last.saturating_add(1);
    let sequence = header.sequence.get();
    let verdict = if sequence < last {
        Verdict::Old
    } else if sequence == last {
        if body_digest == previous {
            Verdict::Reapply
        } else {
            Verdict::Forked
        }
    } else if sequence == expected {
        if header.previous == previous {
            Verdict::Apply
        } else {
            Verdict::Forked
        }
    } else if available.snapshot == Some(header.sequence)
        && available.first <= header.sequence
        && matches!(header.kind, Kind::Snapshot { part: 0, .. })
    {
        Verdict::Apply
    } else if let Some(snapshot) = available.snapshot.filter(|snapshot| {
        snapshot.get() >= expected && *snapshot >= available.first && snapshot.get() <= sequence
    }) {
        Verdict::Jump(snapshot)
    } else if available.first.get() > expected {
        Verdict::Lost
    } else {
        Verdict::Gap
    };
    Decision {
        cursor: if verdict == Verdict::Apply {
            Some(Cursor::new(header.sequence, body_digest))
        } else {
            cursor
        },
        verdict,
    }
}

#[must_use]
pub fn publishes(stamp: Stamp, me: DeviceId, watermark: Option<Stamp>) -> bool {
    stamp != crate::record_rules::NEVER
        && stamp.device == me
        && watermark.is_none_or(|watermark| stamp > watermark)
}

#[must_use]
pub fn publishes_record(
    record: &crate::record_rules::Record,
    me: DeviceId,
    watermark: Option<Stamp>,
) -> bool {
    record
        .latest_of(me)
        .is_some_and(|stamp| publishes(stamp, me, watermark))
}

#[must_use]
pub const fn compaction_due(snapshot_bytes: usize, delta_bytes: usize, deltas: u64) -> bool {
    deltas >= MOST_DELTAS || (delta_bytes > 0 && delta_bytes / COMPACTION_RATIO >= snapshot_bytes)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Destination {
    Present,
    Missing,
    Changed,
    DeltasOnly,
    Bridge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublishFacts {
    pub cursor: Option<Cursor>,
    pub has_changes: bool,
    pub destination: Destination,
    pub force_snapshot: bool,
    pub snapshot_bytes: usize,
    pub delta_bytes: usize,
    pub deltas: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Publication {
    Quiet,
    Delta,
    Snapshot,
}

#[must_use]
pub const fn publication(facts: PublishFacts) -> Publication {
    if facts.cursor.is_none() || facts.force_snapshot {
        Publication::Snapshot
    } else if matches!(facts.destination, Destination::Bridge) {
        Publication::Delta
    } else if matches!(facts.destination, Destination::DeltasOnly) {
        if facts.has_changes {
            Publication::Delta
        } else {
            Publication::Quiet
        }
    } else if !matches!(facts.destination, Destination::Present) {
        Publication::Snapshot
    } else if !facts.has_changes {
        Publication::Quiet
    } else if compaction_due(facts.snapshot_bytes, facts.delta_bytes, facts.deltas) {
        Publication::Snapshot
    } else {
        Publication::Delta
    }
}

#[must_use]
pub fn remote_ahead(remote: Option<Sequence>, owned: Option<Sequence>) -> bool {
    remote.is_some_and(|remote| owned.is_none_or(|owned| remote > owned))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SplitError {
    ZeroLimit,
    Record {
        index: usize,
        bytes: usize,
        limit: usize,
    },
}

/// Packs whole records in input order, with one empty part for an empty snapshot.
///
/// # Errors
///
/// Refuses a zero limit or the first record that cannot fit by itself.
pub fn split(sizes: &[usize], limit: usize) -> Result<Vec<Range<usize>>, SplitError> {
    if limit == 0 {
        return Err(SplitError::ZeroLimit);
    }
    let mut parts = Vec::new();
    let mut start = 0;
    let mut used: usize = 0;
    for (index, bytes) in sizes.iter().copied().enumerate() {
        if bytes > limit {
            return Err(SplitError::Record {
                index,
                bytes,
                limit,
            });
        }
        match used.checked_add(bytes) {
            Some(next) if next <= limit => used = next,
            Some(_) | None => {
                parts.push(start..index);
                start = index;
                used = bytes;
            }
        }
    }
    parts.push(start..sizes.len());
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::{Available, Cursor, Header, Kind, Sequence, SplitError, Verdict, step};
    use crate::clock_rules::{DeviceId, Stamp};
    use alloc::vec;

    #[test]
    fn publication_keeps_only_new_own_writes_and_compaction_observes_both_bounds() {
        let me = DeviceId([1; 16]);
        let watermark = Stamp {
            wall: 10,
            counter: 1,
            device: me,
        };
        assert!(!super::publishes(watermark, me, Some(watermark)));
        assert!(super::publishes(
            Stamp {
                counter: 2,
                ..watermark
            },
            me,
            Some(watermark)
        ));
        assert!(!super::publishes(
            Stamp {
                device: DeviceId([2; 16]),
                ..watermark
            },
            me,
            None
        ));
        assert!(super::publishes(watermark, me, None));
        assert!(!super::publishes(
            crate::record_rules::NEVER,
            DeviceId([0; 16]),
            None
        ));
        assert!(!super::compaction_due(0, 0, 0));
        assert!(super::compaction_due(0, 1, 0));
        assert!(!super::compaction_due(100, 199, 255));
        assert!(super::compaction_due(100, 200, 255));
        assert!(super::compaction_due(100, 0, 256));
        assert!(!super::compaction_due(usize::MAX, usize::MAX, 0));
        assert_eq!(super::MOST_DELTAS, 256);
        assert_eq!(super::LONGEST_BODY, 8 * 1024 * 1024);
    }

    #[test]
    fn a_publish_is_quiet_only_while_its_chain_is_present_and_unchanged() {
        let facts = super::PublishFacts {
            cursor: Some(Cursor::new(Sequence::FIRST, [0; 32])),
            has_changes: false,
            destination: super::Destination::Present,
            force_snapshot: false,
            snapshot_bytes: 100,
            delta_bytes: 0,
            deltas: 0,
        };
        assert_eq!(super::publication(facts), super::Publication::Quiet);
        assert_eq!(
            super::publication(super::PublishFacts {
                has_changes: true,
                ..facts
            }),
            super::Publication::Delta
        );
        for destination in [super::Destination::Missing, super::Destination::Changed] {
            assert_eq!(
                super::publication(super::PublishFacts {
                    destination,
                    ..facts
                }),
                super::Publication::Snapshot
            );
        }
        assert_eq!(
            super::publication(super::PublishFacts {
                cursor: None,
                ..facts
            }),
            super::Publication::Snapshot
        );
        assert_eq!(
            super::publication(super::PublishFacts {
                force_snapshot: true,
                ..facts
            }),
            super::Publication::Snapshot
        );
        assert!(!super::remote_ahead(None, None));
        assert!(super::remote_ahead(Some(Sequence::FIRST), None));
        assert!(!super::remote_ahead(
            Some(Sequence::FIRST),
            Some(Sequence::FIRST)
        ));
    }

    #[test]
    fn parts_keep_whole_records_and_refuse_a_record_that_cannot_fit() {
        assert_eq!(super::split(&[2, 3, 1, 4], 5), Ok(vec![0..2, 2..4]));
        assert_eq!(super::split(&[], 5), Ok(core::iter::once(0..0).collect()));
        assert_eq!(
            super::split(&[0, 0], 1),
            Ok(core::iter::once(0..2).collect())
        );
        assert_eq!(super::split(&[], 0), Err(SplitError::ZeroLimit));
        assert_eq!(
            super::split(&[1, 6], 5),
            Err(SplitError::Record {
                index: 1,
                bytes: 6,
                limit: 5
            })
        );
        assert_eq!(
            super::split(&[usize::MAX, 1], usize::MAX),
            Ok(vec![0..1, 1..2])
        );
    }

    #[test]
    fn automatic_compaction_waits_for_the_next_change_and_never_wakes_an_idle_chain() {
        for snapshot_bytes in [0, 9, usize::MAX] {
            let quiet = super::PublishFacts {
                cursor: Some(Cursor::new(Sequence::FIRST, [0; 32])),
                has_changes: false,
                destination: super::Destination::Present,
                force_snapshot: false,
                snapshot_bytes,
                delta_bytes: usize::MAX,
                deltas: super::MOST_DELTAS,
            };
            assert_eq!(super::publication(quiet), super::Publication::Quiet);
            assert_eq!(
                super::publication(super::PublishFacts {
                    has_changes: true,
                    ..quiet
                }),
                super::Publication::Snapshot
            );
        }
    }

    #[test]
    fn a_bounded_relay_bridges_changed_transports_without_rebuilding_an_oversized_snapshot() {
        let facts = super::PublishFacts {
            cursor: Some(Cursor::new(Sequence::FIRST, [0; 32])),
            has_changes: true,
            destination: super::Destination::DeltasOnly,
            force_snapshot: false,
            snapshot_bytes: usize::MAX,
            delta_bytes: usize::MAX,
            deltas: u64::MAX,
        };
        assert_eq!(super::publication(facts), super::Publication::Delta);
        assert_eq!(
            super::publication(super::PublishFacts {
                has_changes: false,
                ..facts
            }),
            super::Publication::Quiet
        );
        assert_eq!(
            super::publication(super::PublishFacts {
                has_changes: false,
                destination: super::Destination::Bridge,
                ..facts
            }),
            super::Publication::Delta
        );
        assert_eq!(
            super::publication(super::PublishFacts {
                cursor: None,
                ..facts
            }),
            super::Publication::Snapshot
        );
        assert_eq!(
            super::publication(super::PublishFacts {
                force_snapshot: true,
                ..facts
            }),
            super::Publication::Snapshot
        );
    }

    fn sequence(value: u64) -> Sequence {
        Sequence::new(value).unwrap()
    }

    fn header(value: u64, previous: u8, kind: Kind) -> Header {
        Header::new(sequence(value), [previous; 32], kind).unwrap()
    }

    fn available(first: u64, snapshot: Option<u64>) -> Available {
        Available {
            first: sequence(first),
            snapshot: snapshot.map(sequence),
        }
    }

    #[test]
    fn a_reader_accepts_the_next_segment_and_replays_only_the_same_plaintext() {
        let first = header(1, 0, Kind::Delta);
        let applied = step(None, first, [1; 32], available(1, None));
        assert_eq!(applied.verdict, Verdict::Apply);
        let cursor = applied.cursor.unwrap();
        assert_eq!(cursor.sequence(), sequence(1));
        assert_eq!(cursor.digest(), [1; 32]);
        for (digest, verdict) in [(1, Verdict::Reapply), (2, Verdict::Forked)] {
            let repeated = step(Some(cursor), first, [digest; 32], available(1, None));
            assert_eq!(repeated.verdict, verdict);
            assert_eq!(repeated.cursor, Some(cursor));
        }
        let next = header(2, 1, Kind::Delta);
        assert_eq!(
            step(Some(cursor), next, [2; 32], available(1, None)).verdict,
            Verdict::Apply
        );
        assert_eq!(
            step(
                Some(cursor),
                header(2, 3, Kind::Delta),
                [2; 32],
                available(1, None)
            )
            .verdict,
            Verdict::Forked
        );
    }

    #[test]
    fn a_gap_waits_jumps_to_a_snapshot_or_reports_that_bootstrap_is_needed() {
        let cursor = Some(Cursor::new(sequence(1), [1; 32]));
        let future = header(4, 3, Kind::Delta);
        for (held, verdict) in [
            (available(1, None), Verdict::Gap),
            (available(3, Some(3)), Verdict::Jump(sequence(3))),
            (available(3, None), Verdict::Lost),
        ] {
            let decision = step(cursor, future, [4; 32], held);
            assert_eq!(decision.verdict, verdict);
            assert_eq!(decision.cursor, cursor);
        }
        let snapshot = header(3, 2, Kind::Snapshot { part: 0, parts: 2 });
        let jumped = step(cursor, snapshot, [3; 32], available(3, Some(3)));
        assert_eq!(jumped.verdict, Verdict::Apply);
        let continued = step(
            jumped.cursor,
            header(4, 3, Kind::Snapshot { part: 1, parts: 2 }),
            [4; 32],
            available(3, Some(3)),
        );
        assert_eq!(continued.verdict, Verdict::Apply);
        assert_eq!(
            step(continued.cursor, snapshot, [3; 32], available(3, Some(3))).verdict,
            Verdict::Old
        );
    }

    #[test]
    fn sequence_and_snapshot_parts_refuse_zero_overflow_and_incomplete_shapes() {
        assert_eq!(Sequence::new(0), None);
        for (value, part, parts) in [(1, 0, 0), (1, 1, 1), (1, 1, 2), (u64::MAX, 0, 2)] {
            assert_eq!(
                Header::new(sequence(value), [0; 32], Kind::Snapshot { part, parts }),
                None
            );
        }
        assert!(Header::new(sequence(u64::MAX), [0; 32], Kind::Delta).is_some());
        assert_eq!(sequence(u64::MAX).next(), None);
        assert_eq!(sequence(1).next(), Some(sequence(2)));
    }
}

#[cfg(kani)]
mod proofs {
    use super::{
        Available, Cursor, Header, Kind, Sequence, Verdict, compaction_due, publishes, split, step,
    };
    use crate::clock_rules::{DeviceId, Stamp};

    #[kani::proof]
    #[kani::unwind(33)]
    fn a_snapshot_header_has_positive_parts_and_fits_the_chain() {
        let Some(sequence) = Sequence::new(kani::any()) else {
            return;
        };
        let part: u32 = kani::any();
        let parts: u32 = kani::any();
        let delta: bool = kani::any();
        let kind = if delta {
            Kind::Delta
        } else {
            Kind::Snapshot { part, parts }
        };
        let end = u128::from(sequence.get())
            + u128::from(parts)
                .saturating_sub(u128::from(part))
                .saturating_sub(1);
        let valid_snapshot =
            part < parts && sequence.get() > u64::from(part) && end <= u128::from(u64::MAX);
        let made = Header::new(sequence, [0; 32], kind);
        kani::assert(
            made.is_some() == (delta || valid_snapshot),
            "only snapshots with a positive start and representable final part are admitted",
        );
        kani::cover!(delta && made.is_some(), "deltas need only a sequence");
        kani::cover!(!delta && valid_snapshot, "the snapshot fits");
        kani::cover!(!delta && part >= parts, "the part is outside its snapshot");
        kani::cover!(
            !delta && sequence.get() <= u64::from(part),
            "the snapshot would start before one"
        );
        kani::cover!(
            !delta && part < parts && end > u128::from(u64::MAX),
            "the final part would overflow"
        );
    }

    fn stamp() -> Stamp {
        Stamp {
            wall: kani::any(),
            counter: kani::any(),
            device: DeviceId(kani::any()),
        }
    }

    #[kani::proof]
    #[kani::unwind(33)]
    fn fetching_chooses_the_next_segment_or_a_retained_snapshot_without_overflow() {
        let last: u64 = kani::any();
        let Some(first) = Sequence::new(kani::any()) else {
            return;
        };
        let Some(latest) = Sequence::new(kani::any()) else {
            return;
        };
        let snapshot = Sequence::new(kani::any());
        kani::assume(first <= latest);
        kani::assume(snapshot.is_none_or(|snapshot| first <= snapshot && snapshot <= latest));
        let cursor = Sequence::new(last).map(|sequence| Cursor::new(sequence, [0; 32]));
        let result = super::fetch(cursor, Available { first, snapshot }, latest);
        let expected = u128::from(last) + 1;
        match result {
            super::Fetch::Complete => {
                kani::assert(last >= latest.get(), "only a caught-up cursor is complete")
            }
            super::Fetch::Lost => kani::assert(
                last < latest.get() && u128::from(first.get()) > expected && snapshot.is_none(),
                "only a missing prefix with no snapshot is lost",
            ),
            super::Fetch::Segment(sequence) => {
                kani::assert(
                    last < sequence.get() && sequence <= latest,
                    "fetch stays after the cursor and within the head",
                );
                kani::assert(
                    (u128::from(sequence.get()) == expected && sequence >= first)
                        || (u128::from(first.get()) > expected && snapshot == Some(sequence)),
                    "fetch takes the next sequence or the snapshot beyond a missing prefix",
                );
            }
        }
        kani::cover!(result == super::Fetch::Complete, "the reader caught up");
        kani::cover!(
            result == super::Fetch::Lost,
            "the reader needs another bootstrap source"
        );
        kani::cover!(
            matches!(result, super::Fetch::Segment(sequence) if u128::from(sequence.get()) == expected),
            "the next sequence is retained"
        );
        kani::cover!(
            matches!(result, super::Fetch::Segment(sequence) if u128::from(sequence.get()) > expected),
            "a snapshot restarts after the gap"
        );
    }

    fn small_sequence() -> Option<Sequence> {
        let value: u8 = kani::any();
        kani::assume(value <= 4);
        Sequence::new(u64::from(value))
    }

    fn small_digest() -> super::Digest {
        let byte: u8 = kani::any();
        kani::assume(byte <= 3);
        [byte; super::DIGEST_BYTES]
    }

    fn header() -> Option<Header> {
        let sequence = small_sequence()?;
        let kind = if kani::any() {
            Kind::Delta
        } else {
            let part: u32 = kani::any();
            let parts: u32 = kani::any();
            kani::assume(part <= 2 && parts <= 2);
            Kind::Snapshot { part, parts }
        };
        Header::new(sequence, small_digest(), kind)
    }

    #[kani::proof]
    fn a_publish_plan_preserves_quiet_and_repairs_missing_or_compacted_chains() {
        let cursor = Sequence::new(kani::any())
            .map(|sequence| Cursor::new(sequence, [0; super::DIGEST_BYTES]));
        let destination = match kani::any::<u8>() {
            0 => super::Destination::Present,
            1 => super::Destination::Missing,
            2 => super::Destination::Changed,
            3 => super::Destination::DeltasOnly,
            _ => super::Destination::Bridge,
        };
        let facts = super::PublishFacts {
            cursor,
            destination,
            has_changes: kani::any(),
            force_snapshot: kani::any(),
            snapshot_bytes: kani::any(),
            delta_bytes: kani::any(),
            deltas: kani::any(),
        };
        let result = super::publication(facts);
        let bounded = matches!(
            destination,
            super::Destination::DeltasOnly | super::Destination::Bridge
        );
        let bridge = destination == super::Destination::Bridge;
        let repair = cursor.is_none()
            || facts.force_snapshot
            || (!bounded && destination != super::Destination::Present);
        let compact = !bounded
            && facts.has_changes
            && compaction_due(facts.snapshot_bytes, facts.delta_bytes, facts.deltas);
        let correct = match result {
            super::Publication::Quiet => !repair && !bridge && !facts.has_changes,
            super::Publication::Delta => !repair && (facts.has_changes || bridge) && !compact,
            super::Publication::Snapshot => repair || compact,
        };
        kani::assert(
            correct,
            "ordinary repair takes a snapshot and bounded relay transport explicitly bridges deltas",
        );
        kani::cover!(
            result == super::Publication::Quiet,
            "an unchanged target stays quiet"
        );
        kani::cover!(
            result == super::Publication::Delta,
            "own changes produce a delta"
        );
        kani::cover!(
            result == super::Publication::Snapshot,
            "a missing or compacted chain needs a snapshot"
        );
    }

    #[kani::proof]
    fn a_target_past_every_owned_sequence_is_never_overwritten() {
        let remote = Sequence::new(kani::any());
        let owned = Sequence::new(kani::any());
        let ahead = super::remote_ahead(remote, owned);
        let safe = match (remote, owned) {
            (None, _) => !ahead,
            (Some(_), None) => ahead,
            (Some(remote), Some(owned)) => ahead == (remote.get() > owned.get()),
        };
        kani::assert(
            safe,
            "only a remote sequence beyond this device's ledger blocks publishing",
        );
        kani::cover!(remote.is_none(), "a new target is writable");
        kani::cover!(ahead, "a newer remote head refuses");
        kani::cover!(
            remote.is_some() && !ahead,
            "an owned or earlier head can resume"
        );
    }

    fn collect_part(bytes: &[u8], limit: usize, collected: &mut crate::record_rules::Body) {
        kani::assert(
            bytes.len() <= limit,
            "each emitted part fits its actual encoded byte limit",
        );
        match crate::record_rules::decode_body(bytes) {
            Ok(mut body) => {
                collected.folders.append(&mut body.folders);
                collected.shortcuts.append(&mut body.shortcuts);
                collected.devices.append(&mut body.devices);
            }
            Err(_) => kani::assert(false, "every emitted part is canonical records"),
        }
    }

    #[kani::proof]
    #[kani::unwind(65)]
    fn canonical_parts_preserve_two_whole_records_as_the_dictionary_changes() {
        let limit: u16 = kani::any();
        kani::assume((10..=512).contains(&limit));
        let limit = usize::from(limit);
        let choices: [u8; 2] = kani::any();
        kani::assume(choices.iter().all(|kind| *kind <= 2));
        let writers: [u8; 2] = kani::any();
        kani::assume(writers.iter().all(|writer| *writer <= 2));
        let Some(first) = super::records::proof_record(1, writers[0], choices[0]) else {
            return;
        };
        let Some(second) = super::records::proof_record(2, writers[1], choices[1]) else {
            return;
        };
        let Ok(mut parts) = super::Parts::new(limit) else {
            kani::assert(false, "the admitted limit starts a partition");
            return;
        };
        let mut expected = crate::record_rules::Body::default();
        let mut collected = crate::record_rules::Body::default();
        let mut emitted: usize = 0;
        for record in [first, second] {
            let mut one = super::records::proof_body(record.clone());
            let Ok(bytes) = crate::record_rules::encode_body(&one) else {
                kani::assert(false, "the fixture is a canonical record");
                return;
            };
            kani::assume(bytes.len() <= limit);
            expected.folders.append(&mut one.folders);
            expected.shortcuts.append(&mut one.shortcuts);
            expected.devices.append(&mut one.devices);
            match parts.push(record) {
                Ok(Some(bytes)) => {
                    collect_part(&bytes, limit, &mut collected);
                    emitted += 1;
                }
                Ok(None) => {}
                Err(_) => {
                    kani::assert(false, "each individually fitting record has a whole part");
                    return;
                }
            }
        }
        let Ok(last) = parts.finish() else {
            kani::assert(false, "the last part finishes");
            return;
        };
        collect_part(&last, limit, &mut collected);
        emitted += 1;
        kani::assert(
            collected == expected,
            "parts keep both records and every field unchanged",
        );
        kani::cover!(emitted == 1, "both records fit one canonical part");
        kani::cover!(
            emitted == 2,
            "the second record starts another canonical part"
        );
    }

    #[kani::proof]
    #[kani::unwind(33)]
    fn a_cursor_advances_only_to_the_next_segment_or_a_snapshot_restart() {
        let Some(header) = header() else { return };
        let cursor = small_sequence().map(|sequence| Cursor::new(sequence, small_digest()));
        let Some(first) = small_sequence() else {
            return;
        };
        let available = Available {
            first,
            snapshot: small_sequence(),
        };
        let digest = small_digest();
        let decision = step(cursor, header, digest, available);
        let last = cursor.map_or(0, |cursor| cursor.sequence().get());
        let next = last + 1;
        let follows = header.sequence().get() == next
            && header.previous() == cursor.map_or([0; super::DIGEST_BYTES], Cursor::digest);
        let restarts = header.sequence().get() > next
            && available.snapshot == Some(header.sequence())
            && available.first <= header.sequence()
            && matches!(header.kind(), Kind::Snapshot { part: 0, .. });
        kani::assert(
            (decision.verdict == Verdict::Apply) == (follows || restarts),
            "only a successor or an advertised snapshot applies",
        );
        if decision.verdict == Verdict::Apply {
            kani::assert(
                decision.cursor == Some(Cursor::new(header.sequence(), digest)),
                "application records this plaintext",
            );
            kani::assert(header.sequence().get() > last, "application advances");
        } else {
            kani::assert(
                decision.cursor == cursor,
                "refusal and replay leave the cursor",
            );
        }
        kani::cover!(
            decision.verdict == Verdict::Apply && follows,
            "the next segment applies"
        );
        kani::cover!(
            decision.verdict == Verdict::Apply && restarts,
            "a snapshot restarts after a gap"
        );
        kani::cover!(
            decision.verdict == Verdict::Reapply,
            "the same plaintext repeats"
        );
        kani::cover!(
            decision.verdict == Verdict::Old,
            "an earlier segment stays old"
        );
        kani::cover!(
            decision.verdict == Verdict::Gap,
            "a missing successor may still arrive"
        );
        kani::cover!(
            matches!(decision.verdict, Verdict::Jump(_)),
            "a snapshot can fill the gap"
        );
        kani::cover!(
            decision.verdict == Verdict::Lost,
            "the retained chain needs bootstrap"
        );
        kani::cover!(
            decision.verdict == Verdict::Forked,
            "another plaintext or predecessor conflicts"
        );
    }

    #[kani::proof]
    #[kani::unwind(33)]
    fn the_same_plaintext_replays_without_advancing() {
        let Some(sequence) = Sequence::new(kani::any()) else {
            return;
        };
        let digest = kani::any();
        let cursor = Cursor::new(sequence, digest);
        let Some(header) = Header::new(sequence, kani::any(), Kind::Delta) else {
            return;
        };
        let decision = step(
            Some(cursor),
            header,
            digest,
            Available {
                first: sequence,
                snapshot: None,
            },
        );
        kani::assert(
            decision.verdict == Verdict::Reapply && decision.cursor == Some(cursor),
            "replay is idempotent at every sequence",
        );
        kani::cover!(sequence.get() == u64::MAX, "the last sequence replays");
    }

    #[kani::proof]
    #[kani::unwind(17)]
    fn a_delta_admits_exactly_own_writes_after_its_watermark() {
        let write = stamp();
        let me = DeviceId(kani::any());
        let watermark = if kani::any() { Some(stamp()) } else { None };
        let expected = write != crate::record_rules::NEVER
            && write.device == me
            && match watermark {
                None => true,
                Some(mark) => {
                    (write.wall, write.counter, write.device)
                        > (mark.wall, mark.counter, mark.device)
                }
            };
        kani::assert(
            publishes(write, me, watermark) == expected,
            "only new own writes publish",
        );
        kani::cover!(expected && watermark.is_none(), "a first own write");
        kani::cover!(expected && watermark.is_some(), "a later own write");
    }

    #[kani::proof]
    fn compaction_follows_the_ratio_or_the_delta_count_without_overflow() {
        let snapshot: usize = kani::any();
        let delta: usize = kani::any();
        let count: u64 = kani::any();
        let ratio = (delta as u128) >= (snapshot as u128) * (super::COMPACTION_RATIO as u128);
        kani::assert(
            compaction_due(snapshot, delta, count)
                == (count >= super::MOST_DELTAS || (delta > 0 && ratio)),
            "the compaction threshold has exact wide arithmetic",
        );
        kani::cover!(
            count >= super::MOST_DELTAS && delta == 0,
            "the count alone compacts"
        );
        kani::cover!(count == 0 && delta > 0 && ratio, "the ratio alone compacts");
    }

    #[kani::proof]
    #[kani::unwind(17)]
    fn a_split_never_cuts_a_record_or_passes_its_limit() {
        let sizes: [usize; 3] = kani::any();
        let length: usize = kani::any();
        let limit: usize = kani::any();
        kani::assume(length <= sizes.len() && limit <= 4);
        kani::assume(sizes.iter().all(|size| *size <= 4));
        let Some(sizes) = sizes.get(..length) else {
            return;
        };
        match split(sizes, limit) {
            Ok(parts) => {
                let mut end = 0;
                for part in parts {
                    kani::assert(
                        part.start == end && part.end <= sizes.len(),
                        "parts are contiguous record ranges",
                    );
                    let held = sizes.get(part.clone()).unwrap_or_default();
                    let bytes: usize = held.iter().sum();
                    kani::assert(bytes <= limit, "a part fits");
                    end = part.end;
                }
                kani::assert(end == sizes.len(), "every record has a part");
                kani::assert(
                    limit > 0 && sizes.iter().all(|size| *size <= limit),
                    "all admitted records fit",
                );
            }
            Err(_) => kani::assert(
                limit == 0 || sizes.iter().any(|size| *size > limit),
                "only an invalid limit or an oversized record refuses",
            ),
        }
        kani::cover!(
            limit > 0 && sizes.is_empty(),
            "an empty snapshot has one part"
        );
        kani::cover!(
            split(sizes, limit).is_ok_and(|parts| parts.len() > 1),
            "whole records span parts"
        );
    }

    #[kani::proof]
    #[kani::unwind(33)]
    fn rejects_a_segment_from_the_past() {
        let Some(first) = Sequence::new(1) else {
            return;
        };
        let Some(second) = Sequence::new(2) else {
            return;
        };
        let Some(header) = Header::new(first, [0; super::DIGEST_BYTES], Kind::Delta) else {
            return;
        };
        kani::assert(
            step(
                Some(Cursor::new(second, [2; super::DIGEST_BYTES])),
                header,
                [1; super::DIGEST_BYTES],
                Available {
                    first,
                    snapshot: None,
                },
            )
            .verdict
                == Verdict::Apply,
            "NEGATIVE_SEGMENT_CONTROL",
        );
    }
}
