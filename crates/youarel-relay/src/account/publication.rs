use super::{AccountStore, Admitted, Append, Handled, StoredChain};
use crate::{Status, StoreError};
use std::num::NonZeroUsize;
use youarel_core::relay_rules::{self, ChainAppend, RetainedSegment};
use youarel_crypto::Label;
use youarel_protocol::relay::{Registration, Verb};
use youarel_protocol::segment::Head;

fn remember(store: &mut impl AccountStore, admitted: &Admitted) -> Result<(), StoreError> {
    for digest in admitted.checked.operations.keys() {
        if let Some(operation) = admitted.operations.get(digest) {
            store.keep_operation(*digest, operation)?;
        }
    }
    if !admitted.stored {
        store.set_genesis(admitted.genesis)?;
    }
    Ok(())
}

fn saved(now: u64, admitted: &Admitted) -> Handled {
    Handled {
        published: Some(Registration {
            account: admitted.request.account,
            members: admitted.checked.recoveries(),
        }),
        ..Handled::accepted(now, Vec::new())
    }
}

fn replay(
    store: &mut impl AccountStore,
    now: u64,
    admitted: &Admitted,
    earlier: Option<&StoredChain>,
) -> Result<Option<Handled>, StoreError> {
    let Some(held) = earlier else { return Ok(None) };
    let segment = admitted
        .segment
        .as_ref()
        .ok_or_else(|| StoreError(String::from("a publication lost its segment")))?;
    let sequence = segment.header.sequence();
    if sequence > held.head.latest() {
        return Ok(None);
    }
    if sequence < held.head.first() {
        return Ok(Some(Handled::accepted(now, Vec::new())));
    }
    if store.segment(&held.sender, sequence)?.as_deref()
        != Some(admitted.request.payload.as_slice())
    {
        return Ok(Some(Handled::refused(
            Status::Conflict,
            "different bytes already occupy this sequence",
        )));
    }
    if sequence == held.head.latest() {
        store.renew(&held.sender, now)?;
    }
    Ok(Some(saved(now, admitted)))
}

pub(super) fn publish(
    store: &mut impl AccountStore,
    now: u64,
    admitted: &Admitted,
    members: &[StoredChain],
) -> Result<Handled, StoreError> {
    let segment = admitted
        .segment
        .as_ref()
        .ok_or_else(|| StoreError(String::from("a publication lost its segment")))?;
    if !segment.unsigned().bytes().is_ok_and(|unsigned| {
        admitted
            .keys
            .signing
            .verify(Label::Segment, &unsigned, &segment.signature)
    }) {
        return Ok(Handled::refused(
            Status::Forbidden,
            "the segment's signature does not verify",
        ));
    }
    let earlier = members
        .iter()
        .find(|chain| chain.sender == admitted.request.device);
    if let Some(reply) = replay(store, now, admitted, earlier)? {
        return Ok(reply);
    }
    let mut chain = store.chain(&admitted.request.device)?;
    let bridge = admitted.request.verb == Verb::Bridge;
    if relay_rules::append(
        chain.last().map(|entry| entry.header),
        segment.header,
        bridge,
    ) == ChainAppend::Refuse
    {
        return Ok(Handled::refused(
            Status::Conflict,
            "the segment is no successor or valid restart",
        ));
    }
    if bridge {
        chain.clear();
    }
    let other_bytes = members
        .iter()
        .filter(|held| held.sender != admitted.request.device)
        .fold(0usize, |total, held| total.saturating_add(held.bytes));
    if !relay_rules::admits_slot(members.len(), earlier.is_some()) {
        return Ok(Handled::refused(
            Status::TooLarge,
            "the account holds its maximum number of chains",
        ));
    }
    let bytes = NonZeroUsize::new(admitted.request.payload.len())
        .ok_or_else(|| StoreError(String::from("a segment is empty")))?;
    chain.push(RetainedSegment {
        header: segment.header,
        bytes,
    });
    let Ok(retained) =
        relay_rules::retain(&chain, relay_rules::MOST_BYTES.saturating_sub(other_bytes))
    else {
        return Ok(Handled::refused(
            Status::TooLarge,
            "the segment exceeds the account's remaining relay quota",
        ));
    };
    let head = Head::new(
        retained.first(),
        segment.header.sequence(),
        retained.snapshot(),
    )
    .ok_or_else(|| StoreError(String::from("retention returned no valid head")))?;
    remember(store, admitted)?;
    store.append(&Append {
        sender: admitted.request.device,
        published: now,
        head,
        header: segment.header,
        bytes: &admitted.request.payload,
    })?;
    Ok(saved(now, admitted))
}
