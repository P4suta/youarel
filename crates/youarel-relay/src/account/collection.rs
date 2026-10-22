use super::{AccountStore, Admitted, Handled, StoredChain};
use crate::StoreError;
use youarel_core::relay_rules::DeliveryPage;
use youarel_core::segment_rules::{Fetch, fetch};
use youarel_protocol::segment::{Chain, Cursors, Delivered};

pub(super) fn collect(
    store: &impl AccountStore,
    now: u64,
    admitted: &Admitted,
    members: &[StoredChain],
) -> Result<Handled, StoreError> {
    let cursors = Cursors::decode(&admitted.request.payload)
        .map_err(|error| StoreError(error.to_string()))?;
    let operations: Vec<Vec<u8>> = admitted
        .checked
        .operations
        .keys()
        .filter_map(|digest| admitted.operations.get(digest).cloned())
        .collect();
    let metadata_bytes = operations.iter().fold(0usize, |size, operation| {
        size.saturating_add(operation.len())
            .saturating_add(size_of::<u32>())
    });
    let mut page = DeliveryPage::new(metadata_bytes)
        .ok_or_else(|| StoreError(String::from("the public log exceeds a delivery page")))?;
    let mut delivered = Delivered {
        account: admitted.request.account,
        operations,
        chains: Vec::new(),
        more: false,
    };
    let mut ordered = members.to_vec();
    ordered.sort_by_key(|chain| chain.sender);
    for held in ordered {
        let mut chain = Chain {
            sender: held.sender,
            incomplete: held.incomplete,
            published: held.published,
            head: held.head,
            segments: Vec::new(),
        };
        if let Fetch::Segment(first) = fetch(
            cursors.peers.get(&held.sender).copied(),
            held.head.available(),
            held.head.latest(),
        ) {
            for entry in store
                .chain(&held.sender)?
                .into_iter()
                .filter(|entry| entry.header.sequence() >= first)
            {
                let Some(next) = page.push(entry.bytes.get()) else {
                    delivered.more = true;
                    break;
                };
                let segment = store
                    .segment(&held.sender, entry.header.sequence())?
                    .ok_or_else(|| StoreError(String::from("the relay lost a retained segment")))?;
                chain.segments.push(segment);
                page = next;
            }
        }
        delivered.chains.push(chain);
    }
    let body = delivered
        .encode()
        .map_err(|error| StoreError(error.to_string()))?;
    Ok(Handled::accepted(now, body))
}
