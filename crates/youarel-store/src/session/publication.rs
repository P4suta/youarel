use super::{Record, Session, decode_shortcut, key_id};
mod receive;
mod stage;
mod state;
use crate::StoreError;
use crate::database::{
    DIRTY, META, PENDING_BODIES, PENDING_DIGESTS, PENDING_SEALED, PUBLISH_WATERMARK_KEY, SHORTCUTS,
    failed, open, read_stamp, stamp_bytes,
};
pub use receive::ReceivedSegment;
use redb::{ReadableTable as _, ReadableTableMetadata as _};
pub use state::{PendingPublish, PublicationError, PublicationState, PublishRequest};
use youarel_core::clock_rules::Stamp;
use youarel_core::segment_rules::{
    self, Cursor, Destination, Digest, Header, Kind, LONGEST_BODY, LONGEST_ENVELOPE, Publication,
    PublishFacts, Sequence, publishes, publishes_record,
};
use youarel_core::state::{Entry, ItemId};
use youarel_core::wire_rules::{Reader, WireError};

const FOLDER: u8 = 0;
const SHORTCUT: u8 = 1;
const DEVICE: u8 = 2;
const PUBLICATION_KEY: &str = "outgoing_segments";
const PENDING_KEY: &str = "pending_segments";

#[derive(Debug)]
pub struct StagedPart {
    pub header: Header,
    pub digest: Digest,
    pub body: Vec<u8>,
    pub sealed: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug)]
pub enum RecordSet {
    Snapshot,
    Delta { watermark: Option<Stamp> },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum DirtyRecord {
    Entry(Entry),
    Device(youarel_core::device_rules::MemberId),
}

pub(super) fn dirty_key(record: DirtyRecord) -> Vec<u8> {
    match record {
        DirtyRecord::Entry(entry) => {
            let (tag, id) = match entry {
                Entry::Folder(id) => (FOLDER, id),
                Entry::Shortcut(id) => (SHORTCUT, id),
            };
            core::iter::once(tag).chain(id.0).collect()
        }
        DirtyRecord::Device(id) => core::iter::once(DEVICE).chain(id.0).collect(),
    }
}

fn dirty_entry(key: &[u8]) -> Result<DirtyRecord, StoreError> {
    let mut reader = Reader::new(key);
    let tag = reader.byte().map_err(StoreError::Damaged)?;
    let record = match tag {
        FOLDER => DirtyRecord::Entry(Entry::Folder(ItemId(
            reader.array().map_err(StoreError::Damaged)?,
        ))),
        SHORTCUT => DirtyRecord::Entry(Entry::Shortcut(ItemId(
            reader.array().map_err(StoreError::Damaged)?,
        ))),
        DEVICE => DirtyRecord::Device(youarel_core::device_rules::MemberId(
            reader.array().map_err(StoreError::Damaged)?,
        )),
        tag => return Err(StoreError::Damaged(WireError::Tag(tag))),
    };
    reader.finish().map_err(StoreError::Damaged)?;
    Ok(record)
}

impl Session {
    fn publication_watermark(&self) -> Result<Option<Stamp>, StoreError> {
        let meta = open(&self.transaction, META)?;
        meta.get(PUBLISH_WATERMARK_KEY)
            .map_err(failed)?
            .map_or_else(|| Ok(None), |held| read_stamp(held.value()).map(Some))
    }

    pub(super) fn record_dirty(
        &self,
        entry: Entry,
        stamps: impl IntoIterator<Item = Stamp>,
    ) -> Result<(), StoreError> {
        self.record_dirty_key(&dirty_key(DirtyRecord::Entry(entry)), stamps)
    }

    pub(super) fn record_dirty_key(
        &self,
        key: &[u8],
        stamps: impl IntoIterator<Item = Stamp>,
    ) -> Result<(), StoreError> {
        let watermark = self.publication_watermark()?;
        let Some(stamp) = stamps
            .into_iter()
            .filter(|stamp| publishes(*stamp, self.device, watermark))
            .max()
        else {
            return Ok(());
        };
        let mut dirty = open(&self.transaction, DIRTY)?;
        let before = match dirty.get(key).map_err(failed)? {
            Some(held) => Some(read_stamp(held.value())?),
            None => None,
        };
        let latest = before.map_or(stamp, |before| before.max(stamp));
        dirty
            .insert(key, stamp_bytes(latest).as_slice())
            .map_err(failed)?;
        Ok(())
    }

    fn dirty_record(&self, key: &[u8]) -> Result<Record, StoreError> {
        match dirty_entry(key)? {
            DirtyRecord::Device(id) => self
                .device_name(id)?
                .map(|name| Record::Device(id, name))
                .ok_or(StoreError::Missing("a dirty device name")),
            DirtyRecord::Entry(Entry::Folder(id)) => self
                .folders
                .records()
                .get(&id)
                .cloned()
                .map(|folder| Record::Folder(id, folder))
                .ok_or(StoreError::Missing("a dirty folder")),
            DirtyRecord::Entry(Entry::Shortcut(id)) => self
                .record(id)?
                .map(|shortcut| Record::Shortcut(id, shortcut))
                .ok_or(StoreError::Missing("a dirty shortcut")),
        }
    }

    /// Streams complete records from a snapshot or the items with new writes of this device.
    ///
    /// # Errors
    ///
    /// Returns a database, record, or callback error without reading the whole library into memory.
    pub fn stream_records(
        &self,
        set: RecordSet,
        mut each: impl FnMut(Record) -> Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        match set {
            RecordSet::Snapshot => {
                for (id, folder) in self.folders.records() {
                    each(Record::Folder(*id, folder.clone()))?;
                }
                for entry in open(&self.transaction, SHORTCUTS)?.iter().map_err(failed)? {
                    let (key, value) = entry.map_err(failed)?;
                    each(Record::Shortcut(
                        ItemId(key_id(key.value())?),
                        decode_shortcut(value.value(), &self.table)?,
                    ))?;
                }
                for (id, name) in self.device_names()? {
                    each(Record::Device(id, name))?;
                }
            }
            RecordSet::Delta { watermark } => {
                for entry in open(&self.transaction, DIRTY)?.iter().map_err(failed)? {
                    let (key, value) = entry.map_err(failed)?;
                    let stamp = read_stamp(value.value())?;
                    if stamp.device != self.device {
                        return Err(StoreError::Damaged(WireError::Value));
                    }
                    if !publishes(stamp, self.device, watermark) {
                        continue;
                    }
                    let record = self.dirty_record(key.value())?;
                    if publishes_record(&record, self.device, watermark) {
                        each(record)?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl Session {
    /// The committed outgoing cursor and its transport, with any retired reservation kept separately.
    ///
    /// # Errors
    ///
    /// Returns a database or metadata error.
    pub fn publication_state(&self) -> Result<PublicationState, StoreError> {
        let meta = open(&self.transaction, META)?;
        meta.get(PUBLICATION_KEY).map_err(failed)?.map_or_else(
            || Ok(PublicationState::default()),
            |held| PublicationState::decode(held.value()).map_err(StoreError::Damaged),
        )
    }

    /// The durable reservation, including one whose delivery was interrupted.
    ///
    /// # Errors
    ///
    /// Returns a database or metadata error.
    pub fn pending_publish(&self) -> Result<Option<PendingPublish>, StoreError> {
        let meta = open(&self.transaction, META)?;
        meta.get(PENDING_KEY).map_err(failed)?.map_or_else(
            || Ok(None),
            |held| {
                PendingPublish::decode(held.value())
                    .map(Some)
                    .map_err(StoreError::Damaged)
            },
        )
    }

    fn store_publication(&self, state: &PublicationState) -> Result<(), StoreError> {
        let bytes = state.encode().map_err(StoreError::Records)?;
        open(&self.transaction, META)?
            .insert(PUBLICATION_KEY, bytes.as_slice())
            .map_err(failed)?;
        Ok(())
    }

    fn dirty_latest(&self) -> Result<Option<Stamp>, StoreError> {
        let mut latest = self.publication_watermark()?;
        for entry in open(&self.transaction, DIRTY)?.iter().map_err(failed)? {
            let stamp = read_stamp(entry.map_err(failed)?.1.value())?;
            if stamp.device != self.device {
                return Err(StoreError::Damaged(WireError::Value));
            }
            latest = latest.into_iter().chain(Some(stamp)).max();
        }
        Ok(latest)
    }

    fn publish_facts(
        &self,
        state: &PublicationState,
        request: PublishRequest<'_>,
    ) -> Result<PublishFacts, StoreError> {
        let destination =
            if state.cursor.is_some() && state.target.as_deref() != Some(request.target) {
                Destination::Changed
            } else {
                request.destination
            };
        let destination = if request.deltas_only {
            if destination == Destination::Present {
                Destination::DeltasOnly
            } else {
                Destination::Bridge
            }
        } else {
            destination
        };
        Ok(PublishFacts {
            cursor: state.cursor,
            has_changes: open(&self.transaction, DIRTY)?.len().map_err(failed)? > 0
                || request.operations > state.operations,
            destination,
            force_snapshot: request.force_snapshot
                || (state.retired.is_some() && !request.deltas_only),
            snapshot_bytes: state.snapshot_bytes,
            delta_bytes: state.delta_bytes,
            deltas: state.deltas,
        })
    }

    /// Whether this target needs a reservation or an interrupted delivery still waits.
    ///
    /// # Errors
    ///
    /// Returns a database or metadata error.
    pub fn publication_due(&self, request: PublishRequest<'_>) -> Result<bool, StoreError> {
        if self.pending_publish()?.is_some() {
            return Ok(true);
        }
        Ok(
            segment_rules::publication(self.publish_facts(&self.publication_state()?, request)?)
                != Publication::Quiet,
        )
    }

    fn clear_pending(&self) -> Result<(), StoreError> {
        for definition in [PENDING_BODIES, PENDING_DIGESTS, PENDING_SEALED] {
            open(&self.transaction, definition)?
                .retain(|_, _| false)
                .map_err(failed)?;
        }
        open(&self.transaction, META)?
            .remove(PENDING_KEY)
            .map_err(failed)?;
        Ok(())
    }

    fn pending_digest(&self, sequence: Sequence) -> Result<Digest, StoreError> {
        let table = open(&self.transaction, PENDING_DIGESTS)?;
        let held = table
            .get(sequence.get())
            .map_err(failed)?
            .ok_or(PublicationError::MissingPart(sequence))?;
        held.value()
            .try_into()
            .map_err(|_length| StoreError::Damaged(WireError::Value))
    }

    fn retire_pending(
        &self,
        state: &mut PublicationState,
        pending: &PendingPublish,
    ) -> Result<(), StoreError> {
        state.retired = Some(Cursor::new(
            pending.last,
            self.pending_digest(pending.last)?,
        ));
        self.clear_pending()?;
        self.store_publication(state)
    }

    /// Reserves immutable record bodies and their chain positions in the same transaction.
    ///
    /// # Errors
    ///
    /// Refuses an invalid target, exhausted sequence space, oversized record, or storage error.
    pub fn prepare_publication(
        &mut self,
        request: PublishRequest<'_>,
        digest: impl Fn(&[u8]) -> Digest,
    ) -> Result<Option<PendingPublish>, StoreError> {
        self.prepare_publication_checked(request, digest, || Ok(()))
    }

    /// Reserves publication with cooperative cancellation between bounded record batches and parts.
    ///
    /// # Errors
    ///
    /// Refuses the same inputs as ordinary publication or an interrupted reservation, which its session rolls back.
    pub fn prepare_publication_checked(
        &mut self,
        request: PublishRequest<'_>,
        digest: impl Fn(&[u8]) -> Digest,
        check: impl FnMut() -> Result<(), PublicationError>,
    ) -> Result<Option<PendingPublish>, StoreError> {
        if request.target.is_empty() || request.target.len() > state::LONGEST_TARGET {
            return Err(PublicationError::Target.into());
        }
        let mut state = self.publication_state()?;
        if let Some(pending) = self.pending_publish()? {
            if pending.target == request.target {
                return Ok(Some(pending));
            }
            self.retire_pending(&mut state, &pending)?;
        }
        if request.operations < state.operations {
            return Err(PublicationError::Stale.into());
        }
        let mode = segment_rules::publication(self.publish_facts(&state, request)?);
        if mode == Publication::Quiet {
            return Ok(None);
        }
        self.clear_pending()?;
        let plan = stage::Plan {
            previous: state.retired.or(state.cursor),
            watermark: self.dirty_latest()?,
            snapshot: mode == Publication::Snapshot,
        };
        let pending = stage::prepare(self, request, plan, stage::Hooks { digest, check })?;
        let bytes = pending.encode().map_err(StoreError::Records)?;
        open(&self.transaction, META)?
            .insert(PENDING_KEY, bytes.as_slice())
            .map_err(failed)?;
        self.changed = true;
        Ok(Some(pending))
    }

    fn check_pending(
        &self,
        pending: &PendingPublish,
        sequence: Sequence,
    ) -> Result<(), StoreError> {
        if self.pending_publish()?.as_ref() != Some(pending) {
            return Err(PublicationError::Stale.into());
        }
        if sequence < pending.first || sequence > pending.last {
            return Err(PublicationError::MissingPart(sequence).into());
        }
        Ok(())
    }

    fn part_header(
        &self,
        pending: &PendingPublish,
        sequence: Sequence,
    ) -> Result<Header, StoreError> {
        let previous = if sequence == pending.first {
            pending
                .previous
                .map_or([0; segment_rules::DIGEST_BYTES], Cursor::digest)
        } else {
            let previous = sequence
                .get()
                .checked_sub(1)
                .and_then(Sequence::new)
                .ok_or(PublicationError::Exhausted)?;
            self.pending_digest(previous)?
        };
        let kind = if pending.snapshot {
            let offset = sequence
                .get()
                .checked_sub(pending.first.get())
                .ok_or(PublicationError::Exhausted)?;
            let part = u32::try_from(offset).map_err(|_length| PublicationError::Exhausted)?;
            let count = pending
                .last
                .get()
                .checked_sub(pending.first.get())
                .and_then(|parts| parts.checked_add(1))
                .ok_or(PublicationError::Exhausted)?;
            let parts = u32::try_from(count).map_err(|_length| PublicationError::Exhausted)?;
            Kind::Snapshot { part, parts }
        } else {
            Kind::Delta
        };
        Header::new(sequence, previous, kind).ok_or(StoreError::Damaged(WireError::Value))
    }

    /// Reads one immutable part and its saved envelope, when sealing already completed.
    ///
    /// # Errors
    ///
    /// Refuses a stale reservation, missing or oversized part, or storage error.
    pub fn publication_part(
        &self,
        pending: &PendingPublish,
        sequence: Sequence,
    ) -> Result<StagedPart, StoreError> {
        self.check_pending(pending, sequence)?;
        let bodies = open(&self.transaction, PENDING_BODIES)?;
        let body = bodies
            .get(sequence.get())
            .map_err(failed)?
            .ok_or(PublicationError::MissingPart(sequence))?;
        if body.value().len() > LONGEST_BODY {
            return Err(StoreError::Damaged(WireError::Length));
        }
        let sealed = open(&self.transaction, PENDING_SEALED)?
            .get(sequence.get())
            .map_err(failed)?
            .map(|held| held.value().to_vec());
        if sealed
            .as_ref()
            .is_some_and(|sealed| sealed.len() > LONGEST_ENVELOPE)
        {
            return Err(StoreError::Damaged(WireError::Length));
        }
        Ok(StagedPart {
            header: self.part_header(pending, sequence)?,
            digest: self.pending_digest(sequence)?,
            body: body.value().to_vec(),
            sealed,
        })
    }

    /// Saves the exact envelope before any transport can see it, so a retry reuses its bytes.
    ///
    /// # Errors
    ///
    /// Refuses a stale reservation, changed envelope, invalid size, or storage error.
    pub fn seal_publication_part(
        &mut self,
        pending: &PendingPublish,
        sequence: Sequence,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        self.check_pending(pending, sequence)?;
        self.pending_digest(sequence)?;
        if bytes.is_empty() || bytes.len() > LONGEST_ENVELOPE {
            return Err(StoreError::Records(WireError::Length));
        }
        let mut table = open(&self.transaction, PENDING_SEALED)?;
        let same = table
            .get(sequence.get())
            .map_err(failed)?
            .map(|held| held.value() == bytes);
        match same {
            Some(true) => return Ok(()),
            Some(false) => return Err(PublicationError::SealedConflict(sequence).into()),
            None => {}
        }
        table.insert(sequence.get(), bytes).map_err(failed)?;
        self.changed = true;
        Ok(())
    }

    fn clear_covered(&self, watermark: Option<Stamp>) -> Result<(), StoreError> {
        let mut invalid = None;
        open(&self.transaction, DIRTY)?
            .retain(|_, value| match read_stamp(value) {
                Ok(stamp) if stamp.device == self.device => {
                    publishes(stamp, self.device, watermark)
                }
                Ok(_) => {
                    invalid = Some(StoreError::Damaged(WireError::Value));
                    true
                }
                Err(error) => {
                    invalid = Some(error);
                    true
                }
            })
            .map_err(failed)?;
        if let Some(error) = invalid {
            return Err(error);
        }
        Ok(())
    }

    /// Commits a reservation after its envelopes and transport head were delivered successfully.
    ///
    /// # Errors
    ///
    /// Refuses a stale or incompletely sealed reservation, overflow, or storage error.
    pub fn commit_publication(&mut self, pending: &PendingPublish) -> Result<(), StoreError> {
        self.check_pending(pending, pending.last)?;
        for sequence in pending.sequences() {
            if open(&self.transaction, PENDING_SEALED)?
                .get(sequence.get())
                .map_err(failed)?
                .is_none()
            {
                return Err(PublicationError::UnsealedPart(sequence).into());
            }
        }
        let mut state = self.publication_state()?;
        let restart = state.target.as_deref() != Some(pending.target()) || state.retired.is_some();
        state.cursor = Some(Cursor::new(
            pending.last,
            self.pending_digest(pending.last)?,
        ));
        state.retired = None;
        state.target = Some(pending.target.clone());
        state.operations = pending.operations;
        if pending.snapshot {
            state.first = Some(pending.first);
            state.snapshot = Some(pending.first);
            state.snapshot_bytes = pending.body_bytes;
            state.delta_bytes = 0;
            state.deltas = 0;
        } else {
            if restart {
                state.first = Some(pending.first);
                state.snapshot = None;
                state.delta_bytes = 0;
                state.deltas = 0;
            }
            state.delta_bytes = state
                .delta_bytes
                .checked_add(pending.body_bytes)
                .ok_or(PublicationError::Exhausted)?;
            let parts = pending
                .last
                .get()
                .checked_sub(pending.first.get())
                .and_then(|parts| parts.checked_add(1))
                .ok_or(PublicationError::Exhausted)?;
            state.deltas = state
                .deltas
                .checked_add(parts)
                .ok_or(PublicationError::Exhausted)?;
        }
        self.store_publication(&state)?;
        if let Some(watermark) = pending.watermark {
            if watermark.device != self.device {
                return Err(StoreError::Damaged(WireError::Value));
            }
            open(&self.transaction, META)?
                .insert(PUBLISH_WATERMARK_KEY, stamp_bytes(watermark).as_slice())
                .map_err(failed)?;
        }
        self.clear_covered(pending.watermark)?;
        self.clear_pending()?;
        self.changed = true;
        Ok(())
    }
}
