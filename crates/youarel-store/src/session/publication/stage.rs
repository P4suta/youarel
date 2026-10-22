use super::state::{PendingPublish, PublicationError, PublishRequest};
use super::{Record, RecordSet, Session, StoreError};
use crate::database::{PENDING_BODIES, PENDING_DIGESTS, failed, open};
use youarel_core::clock_rules::Stamp;
use youarel_core::segment_rules::{Cursor, Digest, Parts, Sequence};

const CHECK_RECORDS: usize = 256;

#[derive(Debug)]
pub(super) struct Hooks<D, C> {
    pub digest: D,
    pub check: C,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Plan {
    pub previous: Option<Cursor>,
    pub watermark: Option<Stamp>,
    pub snapshot: bool,
}

struct Sink<'a, F, C> {
    session: &'a Session,
    digest: F,
    check: C,
    records: usize,
    first: Sequence,
    last: Option<Sequence>,
    next: Option<Sequence>,
    bytes: usize,
    watermark: Option<Stamp>,
}

impl<F: Fn(&[u8]) -> Digest, C: FnMut() -> Result<(), PublicationError>> Sink<'_, F, C> {
    fn save(&mut self, body: &[u8]) -> Result<(), StoreError> {
        (self.check)()?;
        let sequence = self.next.ok_or(PublicationError::Exhausted)?;
        open(&self.session.transaction, PENDING_BODIES)?
            .insert(sequence.get(), body)
            .map_err(failed)?;
        open(&self.session.transaction, PENDING_DIGESTS)?
            .insert(sequence.get(), (self.digest)(body).as_slice())
            .map_err(failed)?;
        self.bytes = self
            .bytes
            .checked_add(body.len())
            .ok_or(StoreError::Records(
                youarel_core::wire_rules::WireError::Length,
            ))?;
        self.last = Some(sequence);
        self.next = sequence.next();
        Ok(())
    }

    fn observe(&mut self, record: &Record) {
        self.watermark = self
            .watermark
            .into_iter()
            .chain(record.latest_of(self.session.device()))
            .max();
    }

    fn checked_record(&mut self) -> Result<(), StoreError> {
        self.records = self.records.saturating_add(1);
        if self.records.is_multiple_of(CHECK_RECORDS) {
            (self.check)()?;
        }
        Ok(())
    }
}

pub(super) fn prepare(
    session: &Session,
    request: PublishRequest<'_>,
    plan: Plan,
    mut hooks: Hooks<impl Fn(&[u8]) -> Digest, impl FnMut() -> Result<(), PublicationError>>,
) -> Result<PendingPublish, StoreError> {
    (hooks.check)()?;
    let first = match plan.previous {
        Some(cursor) => cursor
            .sequence()
            .next()
            .ok_or(PublicationError::Exhausted)?,
        None => Sequence::FIRST,
    };
    let mut sink = Sink {
        session,
        digest: hooks.digest,
        check: hooks.check,
        records: 0,
        first,
        last: None,
        next: Some(first),
        bytes: 0,
        watermark: plan.watermark,
    };
    let mut parts = Parts::new(request.limit).map_err(StoreError::Records)?;
    let set = if plan.snapshot {
        RecordSet::Snapshot
    } else {
        RecordSet::Delta {
            watermark: session.publication_watermark()?,
        }
    };
    session.stream_records(set, |record| {
        sink.checked_record()?;
        sink.observe(&record);
        if let Some(completed) = parts.push(record).map_err(StoreError::Records)? {
            sink.save(&completed)?;
        }
        Ok(())
    })?;
    sink.save(&parts.finish().map_err(StoreError::Records)?)?;
    let last = sink.last.ok_or(PublicationError::Exhausted)?;
    if last
        .get()
        .checked_sub(first.get())
        .is_none_or(|count| count >= u64::from(u32::MAX))
    {
        return Err(PublicationError::Exhausted.into());
    }
    Ok(PendingPublish {
        target: request.target.to_owned(),
        first: sink.first,
        last,
        previous: plan.previous,
        watermark: sink.watermark,
        operations: request.operations,
        snapshot: plan.snapshot,
        body_bytes: sink.bytes,
    })
}
