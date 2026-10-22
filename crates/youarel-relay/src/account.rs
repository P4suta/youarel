//! Authenticated account requests and bounded immutable segment chains.

use crate::{Reply, Status, StoreError};
use std::collections::BTreeMap;
use youarel_core::relay_rules::{self, RetainedSegment, Seen};
use youarel_core::segment_rules::{Header, Sequence};
use youarel_protocol::log::Checked;
use youarel_protocol::message::{DevicePublic, Digest};
use youarel_protocol::relay::{Registration, Request, Verb};
use youarel_protocol::segment::{Head, Segment};

mod admission;
mod collection;
mod publication;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredChain {
    pub sender: Digest,
    pub incomplete: bool,
    pub published: u64,
    pub head: Head,
    pub bytes: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Append<'a> {
    pub sender: Digest,
    pub published: u64,
    pub head: Head,
    pub header: Header,
    pub bytes: &'a [u8],
}

/// An adapter commits one request atomically, including its log, nonces, chain, and pruning.
pub trait AccountStore {
    /// # Errors
    ///
    /// Reports a storage failure.
    fn genesis(&self) -> Result<Option<Digest>, StoreError>;
    /// # Errors
    ///
    /// Reports a storage failure.
    fn set_genesis(&mut self, genesis: Digest) -> Result<(), StoreError>;
    /// # Errors
    ///
    /// Reports a storage failure.
    fn log(&self) -> Result<BTreeMap<Digest, Vec<u8>>, StoreError>;
    /// # Errors
    ///
    /// Reports a storage failure.
    fn keep_operation(&mut self, digest: Digest, bytes: &[u8]) -> Result<(), StoreError>;
    /// Returns chain metadata without loading ciphertext.
    ///
    /// # Errors
    ///
    /// Reports a storage failure.
    fn chains(&self) -> Result<Vec<StoredChain>, StoreError>;
    /// Returns retained headers and lengths in sequence order without loading ciphertext.
    ///
    /// # Errors
    ///
    /// Reports a storage failure.
    fn chain(&self, sender: &Digest) -> Result<Vec<RetainedSegment>, StoreError>;
    /// # Errors
    ///
    /// Reports a storage failure.
    fn segment(&self, sender: &Digest, sequence: Sequence) -> Result<Option<Vec<u8>>, StoreError>;
    /// Saves an immutable segment, removes sequences before `head.first()`, and commits the head.
    ///
    /// # Errors
    ///
    /// Reports a storage failure.
    fn append(&mut self, append: &Append<'_>) -> Result<(), StoreError>;
    /// # Errors
    ///
    /// Reports a storage failure.
    fn renew(&mut self, sender: &Digest, now: u64) -> Result<(), StoreError>;
    /// # Errors
    ///
    /// Reports a storage failure.
    fn remove_chain(&mut self, sender: &Digest) -> Result<(), StoreError>;
    /// # Errors
    ///
    /// Reports a storage failure.
    fn seen(&self) -> Result<Vec<Seen>, StoreError>;
    /// # Errors
    ///
    /// Reports a storage failure.
    fn set_seen(&mut self, seen: &[Seen]) -> Result<(), StoreError>;
    /// # Errors
    ///
    /// Reports a storage failure.
    fn erase(&mut self) -> Result<(), StoreError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Handled {
    pub reply: Reply,
    pub published: Option<Registration>,
    pub expires: Option<u64>,
}

impl Handled {
    fn refused(status: Status, reason: &str) -> Self {
        Self {
            reply: Reply::refused(status, reason),
            published: None,
            expires: None,
        }
    }

    const fn accepted(now: u64, body: Vec<u8>) -> Self {
        Self {
            reply: Reply::ok(body),
            published: None,
            expires: Some(relay_rules::expires(now)),
        }
    }
}

#[must_use]
pub fn route(bytes: &[u8]) -> Option<Digest> {
    Request::decode(bytes).map_or(None, |request| Some(request.account))
}

struct Admitted {
    request: Request,
    segment: Option<Segment>,
    genesis: Digest,
    stored: bool,
    operations: BTreeMap<Digest, Vec<u8>>,
    checked: Checked,
    keys: DevicePublic,
}

/// Handles a signed request, pruning removed members before publication or collection.
///
/// # Errors
///
/// Reports storage failures; protocol refusals are replies.
pub fn handle(
    store: &mut impl AccountStore,
    now: u64,
    bytes: &[u8],
) -> Result<Handled, StoreError> {
    let admitted = match admission::admit(store, now, bytes)? {
        Ok(admitted) => admitted,
        Err(refused) => return Ok(refused),
    };
    let mut members = Vec::new();
    for chain in store.chains()? {
        if admitted.checked.member(&chain.sender) {
            members.push(chain);
        } else {
            store.remove_chain(&chain.sender)?;
        }
    }
    match admitted.request.verb {
        Verb::Publish | Verb::Bridge => publication::publish(store, now, &admitted, &members),
        Verb::Collect => collection::collect(store, now, &admitted, &members),
        Verb::Erase => {
            store.erase()?;
            Ok(Handled {
                reply: Reply::ok(Vec::new()),
                published: None,
                expires: None,
            })
        }
    }
}
