use super::{AccountStore, Admitted, Handled};
use crate::{Status, StoreError};
use std::collections::BTreeMap;
use youarel_core::relay_rules::{self, Admission, Seen};
use youarel_crypto::Label;
use youarel_protocol::log;
use youarel_protocol::message::{Digest, account_of, operation_digest};
use youarel_protocol::relay::{Request, Verb};
use youarel_protocol::segment::{Cursors, Segment};

fn publication(
    request: &Request,
    operations: &mut BTreeMap<Digest, Vec<u8>>,
) -> Result<Option<Segment>, Handled> {
    match request.verb {
        Verb::Publish | Verb::Bridge => {
            let Ok(segment) = Segment::decode(&request.payload) else {
                return Err(Handled::refused(
                    Status::BadRequest,
                    "the payload is no segment",
                ));
            };
            if request.verb == Verb::Bridge
                && segment.header.kind() != youarel_core::segment_rules::Kind::Delta
            {
                return Err(Handled::refused(
                    Status::BadRequest,
                    "a delta bridge carries only a delta",
                ));
            }
            if (segment.account, segment.sender) != (request.account, request.device) {
                return Err(Handled::refused(
                    Status::BadRequest,
                    "the segment names another account or sender",
                ));
            }
            for operation in &segment.operations {
                operations
                    .entry(operation_digest(operation))
                    .or_insert_with(|| operation.clone());
            }
            Ok(Some(segment))
        }
        Verb::Collect => match Cursors::decode(&request.payload) {
            Ok(_cursors) => Ok(None),
            Err(_invalid) => Err(Handled::refused(
                Status::BadRequest,
                "the collection needs valid cursors",
            )),
        },
        Verb::Erase if request.payload.is_empty() => Ok(None),
        Verb::Erase => Err(Handled::refused(
            Status::BadRequest,
            "erasure takes no payload",
        )),
    }
}

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

pub(super) fn admit(
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
    let segment = match publication(&request, &mut operations) {
        Ok(segment) => segment,
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
    if !request.unsigned().bytes().is_ok_and(|unsigned| {
        keys.signing
            .verify(Label::Request, &unsigned, &request.signature)
    }) {
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
        segment,
        genesis,
        stored: stored.is_some(),
        operations,
        checked,
        keys,
    }))
}
