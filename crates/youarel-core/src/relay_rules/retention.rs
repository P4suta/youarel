use crate::segment_rules::{Header, Kind, Sequence};
use core::num::NonZeroUsize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetainedSegment {
    pub header: Header,
    pub bytes: NonZeroUsize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainError {
    Empty,
    Order,
    TooLarge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Retention {
    from: usize,
    first: Sequence,
    snapshot: Option<Sequence>,
    bytes: usize,
}

impl Retention {
    #[must_use]
    pub const fn keep_from(self) -> usize {
        self.from
    }

    #[must_use]
    pub const fn first(self) -> Sequence {
        self.first
    }

    #[must_use]
    pub const fn snapshot(self) -> Option<Sequence> {
        self.snapshot
    }

    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bytes
    }
}

#[derive(Clone, Copy)]
struct Building {
    index: usize,
    first: Sequence,
    next: u32,
    parts: u32,
}

fn newest_snapshot(segments: &[RetainedSegment], from: usize) -> Option<usize> {
    let mut building: Option<Building> = None;
    let mut complete = None;
    for (index, segment) in segments.iter().enumerate().skip(from) {
        building = match segment.header.kind() {
            Kind::Delta => None,
            Kind::Snapshot { part: 0, parts: 1 } => {
                complete = Some(index);
                None
            }
            Kind::Snapshot { part: 0, parts } => Some(Building {
                index,
                first: segment.header.sequence(),
                next: 1,
                parts,
            }),
            Kind::Snapshot { part, parts } => building
                .filter(|held| {
                    held.next == part
                        && held.parts == parts
                        && held.first.get().checked_add(u64::from(part))
                            == Some(segment.header.sequence().get())
                })
                .and_then(|held| {
                    if part.saturating_add(1) == parts {
                        complete = Some(held.index);
                        None
                    } else {
                        Some(Building {
                            next: part.saturating_add(1),
                            ..held
                        })
                    }
                }),
        };
    }
    complete
}

/// Keeps the newest complete snapshot and its suffix when they fit, otherwise a bounded suffix without a false snapshot hint.
///
/// # Errors
///
/// Refuses empty or unordered metadata and an incoming segment that cannot fit by itself.
pub fn retain(segments: &[RetainedSegment], budget: usize) -> Result<Retention, RetainError> {
    if segments.is_empty() {
        return Err(RetainError::Empty);
    }
    if !segments
        .iter()
        .zip(segments.iter().skip(1))
        .all(|(before, after)| before.header.sequence() < after.header.sequence())
    {
        return Err(RetainError::Order);
    }
    let mut from = segments.len();
    let mut bytes: usize = 0;
    for (index, segment) in segments.iter().enumerate().rev() {
        let Some(next) = bytes
            .checked_add(segment.bytes.get())
            .filter(|next| *next <= budget)
        else {
            break;
        };
        from = index;
        bytes = next;
    }
    if from == segments.len() {
        return Err(RetainError::TooLarge);
    }
    let snapshot_index = newest_snapshot(segments, from);
    if let Some(snapshot) = snapshot_index {
        from = snapshot;
        bytes = segments
            .iter()
            .skip(from)
            .try_fold(0usize, |total, segment| {
                total.checked_add(segment.bytes.get())
            })
            .ok_or(RetainError::TooLarge)?;
    }
    let first = segments
        .get(from)
        .ok_or(RetainError::Empty)?
        .header
        .sequence();
    Ok(Retention {
        from,
        first,
        snapshot: snapshot_index.map(|_index| first),
        bytes,
    })
}
