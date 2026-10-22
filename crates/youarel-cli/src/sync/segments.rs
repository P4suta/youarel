//! The reservation, sealing, delivery, and commit shared by segment transports.

use anyhow::{Context, Result, bail, ensure};
use youarel_core::segment_rules::{Cursor, Decision, Destination, Sequence, remote_ahead};
use youarel_protocol::message::Digest;
use youarel_protocol::segment::{Head, body_digest};
use youarel_store::{PendingPublish, PublicationState, PublishRequest, ReceivedSegment, Store};
use youarel_sync::account::Account;

pub mod folder;
pub mod relay;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Report {
    pub lines: Vec<String>,
    pub problems: Vec<String>,
    pub more: bool,
}

impl Report {
    #[must_use]
    pub fn into_lines(mut self) -> Vec<String> {
        self.lines.extend(self.problems);
        if self.more {
            self.lines
                .push(String::from("more segments remain; sync again to continue"));
        }
        self.lines
    }

    /// # Errors
    ///
    /// Reports incomplete or refused peer chains instead of claiming a complete sync.
    pub fn complete(&self) -> Result<()> {
        ensure!(self.problems.is_empty(), "{}", self.problems.join("\n"));
        ensure!(!self.more, "Still receiving changes from other devices.");
        Ok(())
    }
}

pub trait SegmentTarget {
    fn identity(&self) -> &str;

    fn deltas_only(&self, _state: &PublicationState) -> bool {
        false
    }

    /// # Errors
    ///
    /// Reports a failed read or an invalid committed head.
    fn head(&mut self, sender: &Digest) -> Result<Option<Head>>;

    /// # Errors
    ///
    /// Refuses different bytes at an existing sequence or a failed write.
    fn write(&mut self, sender: &Digest, sequence: Sequence, bytes: &[u8]) -> Result<()>;

    /// # Errors
    ///
    /// Reports a failure to commit the complete transport head.
    fn finish(&mut self, sender: &Digest, head: Head) -> Result<()>;
}

fn head(pending: &PendingPublish, previous: &PublicationState) -> Result<Head> {
    let restart =
        previous.target() != Some(pending.target()) || previous.chain_cursor() != previous.cursor();
    let first = if pending.is_snapshot() || restart {
        pending.first()
    } else {
        previous
            .first()
            .context("the outgoing chain has no first segment")?
    };
    let snapshot = if pending.is_snapshot() {
        Some(pending.first())
    } else if restart {
        None
    } else {
        previous.snapshot()
    };
    Head::new(first, pending.last(), snapshot).context("the outgoing chain has an invalid head")
}

fn remote_is_ahead(
    remote: Option<Head>,
    state: &PublicationState,
    pending: Option<&PendingPublish>,
) -> bool {
    let last = pending
        .map(PendingPublish::last)
        .or_else(|| state.chain_cursor().map(Cursor::sequence));
    remote_ahead(remote.map(Head::latest), last)
}

/// Publishes immutable envelopes, then commits their watermark after the transport confirms its head.
///
/// # Errors
///
/// Leaves the reservation for retry when sealing or delivery fails, and refuses a target whose own chain is newer than this store.
pub fn publish(store: &Store, now: u64, target: &mut impl SegmentTarget) -> Result<Option<Head>> {
    publish_with(store, now, target, false)
}

/// Publishes a complete snapshot directory, finishing an earlier delta reservation first when necessary.
///
/// # Errors
///
/// Leaves a failed reservation for retry and refuses a conflicting target chain.
pub fn snapshot(store: &Store, now: u64, target: &mut impl SegmentTarget) -> Result<Option<Head>> {
    let pending = store.session(now, |session| session.pending_publish())?;
    if pending
        .is_some_and(|pending| pending.target() == target.identity() && !pending.is_snapshot())
    {
        publish(store, now, target)?;
    }
    publish_with(store, now, target, true)
}

fn publish_with(
    store: &Store,
    now: u64,
    target: &mut impl SegmentTarget,
    force_snapshot: bool,
) -> Result<Option<Head>> {
    let _publishing = store.publishing()?;
    let mut account = super::account(store)?;
    ensure!(
        account.status().map_err(super::sync_error)?.member,
        "This device is no longer a member and cannot sync."
    );
    account.prepare_publish(now).map_err(super::sync_error)?;
    let sender = account.device();
    let (state, existing_pending) = store.session(now, |session| {
        Ok((session.publication_state()?, session.pending_publish()?))
    })?;
    let remote = target.head(&sender)?;
    if remote_is_ahead(remote, &state, existing_pending.as_ref()) {
        bail!(
            "The sync target has a newer chain for this device. Open the latest local library or join from another device."
        );
    }
    let identity = target.identity().to_owned();
    let mut request = PublishRequest::new(&identity, account.operations());
    request.deltas_only = target.deltas_only(&state);
    request.force_snapshot = force_snapshot;
    if remote.is_none_or(|remote| {
        state
            .cursor()
            .is_none_or(|cursor| cursor.sequence() != remote.latest())
    }) {
        request.destination = Destination::Missing;
    }
    let Some(pending) = store.session(now, |session| {
        session.prepare_publication(request, body_digest)
    })?
    else {
        return Ok(None);
    };
    let head = head(&pending, &state)?;
    for sequence in pending.sequences() {
        let part = store.session(now, |session| session.publication_part(&pending, sequence))?;
        ensure!(
            body_digest(&part.body) == part.digest,
            "the reserved segment's plaintext changed"
        );
        let bytes = if let Some(bytes) = part.sealed {
            bytes
        } else {
            let bytes = account
                .publish(now, part.header, &part.body)
                .map_err(super::sync_error)?;
            store.session(now, |session| {
                session.seal_publication_part(&pending, sequence, &bytes)
            })?;
            bytes
        };
        target.write(&sender, sequence, &bytes)?;
    }
    target.finish(&sender, head)?;
    store.session(now, |session| session.commit_publication(&pending))?;
    Ok(Some(head))
}

#[derive(Clone, Copy, Debug)]
pub struct Delivery<'a> {
    pub sender: Digest,
    pub head: Head,
    pub bytes: &'a [u8],
}

/// Authenticates one transport segment before admitting its record merge and cursor.
///
/// # Errors
///
/// Refuses a forged or misplaced segment, unadmitted records, or a storage error.
pub fn apply(
    store: &Store,
    now: u64,
    account: &mut Account,
    delivery: Delivery<'_>,
) -> Result<Decision> {
    let opened = account
        .open_segment(delivery.bytes)
        .map_err(super::sync_error)?;
    ensure!(
        opened.sender == delivery.sender,
        "the segment belongs to another device's chain"
    );
    ensure!(
        delivery.head.first() <= opened.header.sequence()
            && opened.header.sequence() <= delivery.head.latest(),
        "the segment lies outside its advertised chain"
    );
    store
        .session(now, |session| {
            session.receive_segment(
                ReceivedSegment {
                    sender: opened.sender,
                    header: opened.header,
                    available: delivery.head.available(),
                    body: &opened.body,
                },
                body_digest,
            )
        })
        .map_err(Into::into)
}
