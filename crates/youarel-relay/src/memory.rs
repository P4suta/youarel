//! A relay that keeps everything in memory, for the tests and a local relay.

use crate::account::{self, AccountStore, Handled};
use crate::mailbox::{self, MailStore};
use crate::{Reply, Status, StoreError};
use std::collections::{BTreeMap, VecDeque};
use youarel_core::relay_rules::Seen;
use youarel_protocol::message::Digest;
use youarel_protocol::relay::{Side, Slot};

/// One account's storage.
#[derive(Clone, Debug, Default)]
pub struct MemoryAccount {
    genesis: Option<Digest>,
    log: BTreeMap<Digest, Vec<u8>>,
    slots: BTreeMap<Digest, Slot>,
    seen: Vec<Seen>,
}

impl AccountStore for MemoryAccount {
    fn genesis(&self) -> Result<Option<Digest>, StoreError> {
        Ok(self.genesis)
    }

    fn set_genesis(&mut self, genesis: Digest) -> Result<(), StoreError> {
        self.genesis = Some(genesis);
        Ok(())
    }

    fn log(&self) -> Result<BTreeMap<Digest, Vec<u8>>, StoreError> {
        Ok(self.log.clone())
    }

    fn keep_operation(&mut self, digest: Digest, bytes: &[u8]) -> Result<(), StoreError> {
        self.log.entry(digest).or_insert_with(|| bytes.to_vec());
        Ok(())
    }

    fn slots(&self) -> Result<Vec<Slot>, StoreError> {
        Ok(self.slots.values().cloned().collect())
    }

    fn put_slot(&mut self, slot: Slot) -> Result<(), StoreError> {
        self.slots.insert(slot.device, slot);
        Ok(())
    }

    fn remove_slot(&mut self, device: &Digest) -> Result<(), StoreError> {
        self.slots.remove(device);
        Ok(())
    }

    fn seen(&self) -> Result<Vec<Seen>, StoreError> {
        Ok(self.seen.clone())
    }

    fn set_seen(&mut self, seen: &[Seen]) -> Result<(), StoreError> {
        self.seen = seen.to_vec();
        Ok(())
    }

    fn erase(&mut self) -> Result<(), StoreError> {
        *self = Self::default();
        Ok(())
    }
}

/// One mailbox: when it opened and the messages for each side, oldest first.
#[derive(Clone, Debug, Default)]
struct Mailbox {
    opened: u64,
    to_inviter: VecDeque<Vec<u8>>,
    to_joiner: VecDeque<Vec<u8>>,
}

impl Mailbox {
    const fn queue(&mut self, to: Side) -> &mut VecDeque<Vec<u8>> {
        match to {
            Side::Inviter => &mut self.to_inviter,
            Side::Joiner => &mut self.to_joiner,
        }
    }
}

/// The mailboxes' storage.
#[derive(Clone, Debug, Default)]
pub struct MemoryMail {
    boxes: BTreeMap<u16, Mailbox>,
}

impl MemoryMail {
    fn mailbox(&mut self, nameplate: u16) -> Result<&mut Mailbox, StoreError> {
        self.boxes
            .get_mut(&nameplate)
            .ok_or_else(|| StoreError(format!("no mailbox {nameplate}")))
    }
}

impl MailStore for MemoryMail {
    fn mailboxes(&self) -> Result<Vec<(u16, u64)>, StoreError> {
        Ok(self
            .boxes
            .iter()
            .map(|(nameplate, mailbox)| (*nameplate, mailbox.opened))
            .collect())
    }

    fn open_mailbox(&mut self, nameplate: u16, now: u64) -> Result<(), StoreError> {
        self.boxes.insert(
            nameplate,
            Mailbox {
                opened: now,
                ..Mailbox::default()
            },
        );
        Ok(())
    }

    fn close_mailbox(&mut self, nameplate: u16) -> Result<(), StoreError> {
        self.boxes.remove(&nameplate);
        Ok(())
    }

    fn held(&self, nameplate: u16) -> Result<usize, StoreError> {
        Ok(self.boxes.get(&nameplate).map_or(0, |mailbox| {
            mailbox
                .to_inviter
                .iter()
                .chain(&mailbox.to_joiner)
                .fold(0, |total: usize, message| {
                    total.saturating_add(message.len())
                })
        }))
    }

    fn push(&mut self, nameplate: u16, to: Side, message: &[u8]) -> Result<(), StoreError> {
        self.mailbox(nameplate)?
            .queue(to)
            .push_back(message.to_vec());
        Ok(())
    }

    fn pop(&mut self, nameplate: u16, to: Side) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.mailbox(nameplate)?.queue(to).pop_front())
    }
}

/// A whole relay in memory: one storage per account, the mailboxes, and each account's expiry.
#[derive(Clone, Debug, Default)]
pub struct MemoryRelay {
    accounts: BTreeMap<Digest, (MemoryAccount, u64)>,
    mail: MemoryMail,
}

impl MemoryRelay {
    /// Handles a member's request, after deleting every account past its retention.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    pub fn request(&mut self, now: u64, bytes: &[u8]) -> Result<Reply, StoreError> {
        self.accounts.retain(|_, (_, expires)| now < *expires);
        let Some(id) = account::route(bytes) else {
            return Ok(Reply::refused(
                Status::BadRequest,
                "the request is malformed",
            ));
        };
        let (store, expires) = self.accounts.entry(id).or_default();
        let Handled {
            reply,
            expires: after,
            ..
        } = account::handle(store, now, bytes)?;
        match after {
            Some(after) => *expires = after,
            None if store.genesis()?.is_none() => {
                self.accounts.remove(&id);
            }
            None => {}
        }
        Ok(reply)
    }

    /// Handles a piece of a pairing's mail.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    pub fn mail(&mut self, now: u64, bytes: &[u8]) -> Result<Reply, StoreError> {
        mailbox::handle(&mut self.mail, now, bytes)
    }

    /// How many accounts the relay holds.
    #[must_use]
    pub fn accounts(&self) -> usize {
        self.accounts.len()
    }
}
