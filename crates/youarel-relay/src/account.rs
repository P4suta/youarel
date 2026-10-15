//! One account at the relay: the members' requests, which the relay admits once and only from a member, and the slots that hold each member's newest bundle.

use crate::{Reply, Status, StoreError};
use std::collections::BTreeMap;
use youarel_core::relay_rules::{self, Admission, Seen};
use youarel_crypto::Label;
use youarel_protocol::log::{self, Checked};
use youarel_protocol::message::{Bundle, DevicePublic, Digest, account_of, operation_digest};
use youarel_protocol::relay::{Collected, Request, Slot, Verb};

/// What the relay keeps of one account.
pub trait AccountStore {
    /// The digest of the account's genesis, once the relay holds the account.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn genesis(&self) -> Result<Option<Digest>, StoreError>;

    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn set_genesis(&mut self, genesis: Digest) -> Result<(), StoreError>;

    /// Every operation of the membership log that verified, by digest.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn log(&self) -> Result<BTreeMap<Digest, Vec<u8>>, StoreError>;

    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn keep_operation(&mut self, digest: Digest, bytes: &[u8]) -> Result<(), StoreError>;

    /// Every slot, in any order.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn slots(&self) -> Result<Vec<Slot>, StoreError>;

    /// Keeps a slot in place of the device's earlier one.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn put_slot(&mut self, slot: Slot) -> Result<(), StoreError>;

    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn remove_slot(&mut self, device: &Digest) -> Result<(), StoreError>;

    /// The nonces of the requests the relay admitted.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn seen(&self) -> Result<Vec<Seen>, StoreError>;

    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn set_seen(&mut self, seen: &[Seen]) -> Result<(), StoreError>;

    /// Deletes everything the relay keeps of the account.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn erase(&mut self) -> Result<(), StoreError>;
}

/// What a request did, for the adapter to carry out around the reply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Handled {
    pub reply: Reply,
    /// Whether a member published a bundle, which the adapter announces to the members that listen.
    pub published: bool,
    /// When the relay deletes the account unless another request comes, or `None` once nothing remains or nothing changed.
    pub expires: Option<u64>,
}

impl Handled {
    fn refused(status: Status, reason: &str) -> Self {
        Self {
            reply: Reply::refused(status, reason),
            published: false,
            expires: None,
        }
    }
}

/// The account a request names, which routes it to the account's storage.
#[must_use]
pub fn route(bytes: &[u8]) -> Option<Digest> {
    Request::decode(bytes).map_or(None, |request| Some(request.account))
}

/// A request the relay admitted, from a member whose keys signed it, with the account's log as it stands with the request's operations.
struct Admitted {
    request: Request,
    bundle: Option<Bundle>,
    genesis: Digest,
    stored: bool,
    operations: BTreeMap<Digest, Vec<u8>>,
    checked: Checked,
    keys: DevicePublic,
}

/// The bundle a publication carries, whose operations join the log, or why the payload is none.
fn publication(
    request: &Request,
    operations: &mut BTreeMap<Digest, Vec<u8>>,
) -> Result<Option<Bundle>, Handled> {
    match request.verb {
        Verb::Publish => {
            let Ok(bundle) = Bundle::decode(&request.payload) else {
                return Err(Handled::refused(
                    Status::BadRequest,
                    "the payload is no bundle",
                ));
            };
            if (bundle.account, bundle.sender) != (request.account, request.device) {
                return Err(Handled::refused(
                    Status::BadRequest,
                    "the bundle names another account or sender",
                ));
            }
            for operation in &bundle.operations {
                operations
                    .entry(operation_digest(operation))
                    .or_insert_with(|| operation.clone());
            }
            Ok(Some(bundle))
        }
        Verb::Collect | Verb::Erase => Ok(None),
    }
}

/// Lets a request through when it is fresh and new, and says why not otherwise.
fn freshness(now: u64, request: &Request, seen: &[Seen]) -> Result<(), Handled> {
    match relay_rules::admit(now, request.stamped, &request.nonce, seen) {
        Admission::Fresh => Ok(()),
        Admission::Stale => Err(Handled::refused(
            Status::Unauthorized,
            "the request's time lies outside the relay's window; check this device's clock",
        )),
        Admission::Replayed => Err(Handled::refused(
            Status::Unauthorized,
            "the relay admitted this request before",
        )),
    }
}

/// Admits a request once, from a member, under its signature, or says why not.
fn admit(
    store: &mut impl AccountStore,
    now: u64,
    bytes: &[u8],
) -> Result<Result<Admitted, Handled>, StoreError> {
    let Ok(request) = Request::decode(bytes) else {
        return Ok(Err(Handled::refused(
            Status::BadRequest,
            "the request is malformed",
        )));
    };
    let mut seen = relay_rules::prune(now, &store.seen()?);
    if let Err(refused) = freshness(now, &request, &seen) {
        return Ok(Err(refused));
    }
    let mut operations = store.log()?;
    let bundle = match publication(&request, &mut operations) {
        Ok(bundle) => bundle,
        Err(refused) => return Ok(Err(refused)),
    };
    let stored = store.genesis()?;
    let Some(genesis) = stored.or_else(|| {
        operations
            .iter()
            .find(|(_, operation)| account_of(operation) == request.account)
            .map(|(digest, _)| *digest)
    }) else {
        return Ok(Err(Handled::refused(
            Status::NotFound,
            "the relay holds no such account",
        )));
    };
    let checked = log::check(genesis, request.account, &operations);
    let Some(keys) = checked
        .keys
        .get(&request.device)
        .filter(|_| checked.member(&request.device))
        .cloned()
    else {
        return Ok(Err(Handled::refused(
            Status::Forbidden,
            "the device is no member of the account",
        )));
    };
    let signed = request.unsigned().bytes().is_ok_and(|unsigned| {
        keys.signing
            .verify(Label::Request, &unsigned, &request.signature)
    });
    if !signed {
        return Ok(Err(Handled::refused(
            Status::Unauthorized,
            "the request's signature does not verify",
        )));
    }
    seen.push(Seen {
        nonce: request.nonce,
        stamped: request.stamped,
    });
    store.set_seen(&seen)?;
    Ok(Ok(Admitted {
        request,
        bundle,
        genesis,
        stored: stored.is_some(),
        operations,
        checked,
        keys,
    }))
}

/// Keeps a member's bundle in its slot, with the operations of the log that verified.
fn publish(
    store: &mut impl AccountStore,
    now: u64,
    admitted: Admitted,
    members: &[Slot],
) -> Result<Handled, StoreError> {
    let Some(bundle) = &admitted.bundle else {
        return Err(StoreError(String::from("a publication lost its bundle")));
    };
    let sealed = bundle.unsigned().bytes().is_ok_and(|unsigned| {
        admitted
            .keys
            .signing
            .verify(Label::Bundle, &unsigned, &bundle.signature)
    });
    if !sealed {
        return Ok(Handled::refused(
            Status::Forbidden,
            "the bundle's signature does not verify",
        ));
    }
    let request = admitted.request;
    let total = members.iter().fold(0, |total: usize, slot| {
        total.saturating_add(slot.bundle.len())
    });
    let earlier = members.iter().find(|slot| slot.device == request.device);
    let replaced = earlier.map_or(0, |slot| slot.bundle.len());
    if !relay_rules::fits(total, replaced, request.payload.len())
        || !relay_rules::admits_slot(members.len(), earlier.is_some())
    {
        return Ok(Handled::refused(
            Status::TooLarge,
            "the account's bundles would pass what the relay keeps",
        ));
    }
    for digest in admitted.checked.operations.keys() {
        if let Some(operation) = admitted.operations.get(digest) {
            store.keep_operation(*digest, operation)?;
        }
    }
    if !admitted.stored {
        store.set_genesis(admitted.genesis)?;
    }
    store.put_slot(Slot {
        device: request.device,
        published: now,
        bundle: request.payload,
    })?;
    Ok(Handled {
        reply: Reply::ok(Vec::new()),
        published: true,
        expires: Some(relay_rules::expires(now)),
    })
}

/// Handles a member's request at the relay's clock `now`.
///
/// The relay drops the slot of every device the log no longer lists before it answers.
///
/// # Errors
///
/// Returns a [`StoreError`] when the storage fails; every refusal of the request is a [`Handled`] reply instead.
pub fn handle(
    store: &mut impl AccountStore,
    now: u64,
    bytes: &[u8],
) -> Result<Handled, StoreError> {
    let admitted = match admit(store, now, bytes)? {
        Ok(admitted) => admitted,
        Err(refused) => return Ok(refused),
    };
    let mut members = Vec::new();
    for slot in store.slots()? {
        if admitted.checked.member(&slot.device) {
            members.push(slot);
        } else {
            store.remove_slot(&slot.device)?;
        }
    }
    match admitted.request.verb {
        Verb::Publish => publish(store, now, admitted, &members),
        Verb::Collect => {
            members.sort_by_key(|slot| slot.device);
            let Ok(body) = (Collected { slots: members }).encode() else {
                return Err(StoreError(String::from(
                    "the account holds slots that no collection carries",
                )));
            };
            Ok(Handled {
                reply: Reply::ok(body),
                published: false,
                expires: Some(relay_rules::expires(now)),
            })
        }
        Verb::Erase => {
            store.erase()?;
            Ok(Handled {
                reply: Reply::ok(Vec::new()),
                published: false,
                expires: None,
            })
        }
    }
}
