//! The mailboxes of pairings at the relay: short-lived, unauthenticated, and named by a small nameplate that the code begins with.
//!
//! The pairing messages authenticate themselves through the code, so the relay only carries them, keeps them for a few minutes, and bounds what a mailbox holds.

use crate::{Reply, Status, StoreError};
use youarel_core::relay_rules::{self, MOST_MAILBOXES};
use youarel_protocol::relay::{Mail, Side, nameplate_bytes};

/// What the relay keeps of the open mailboxes.
pub trait MailStore {
    /// Every mailbox, with when it opened.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn mailboxes(&self) -> Result<Vec<(u16, u64)>, StoreError>;

    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn open_mailbox(&mut self, nameplate: u16, now: u64) -> Result<(), StoreError>;

    /// Deletes a mailbox and its messages.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn close_mailbox(&mut self, nameplate: u16) -> Result<(), StoreError>;

    /// The bytes of every message a mailbox holds.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn held(&self, nameplate: u16) -> Result<usize, StoreError>;

    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn push(&mut self, nameplate: u16, to: Side, message: &[u8]) -> Result<(), StoreError>;

    /// Takes the oldest message for one side, if any.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn pop(&mut self, nameplate: u16, to: Side) -> Result<Option<Vec<u8>>, StoreError>;
}

/// Handles a piece of a pairing's mail at the relay's clock `now`, after closing every mailbox past its lifetime.
///
/// # Errors
///
/// Returns a [`StoreError`] when the storage fails; every refusal is a reply instead.
pub fn handle(store: &mut impl MailStore, now: u64, bytes: &[u8]) -> Result<Reply, StoreError> {
    let Ok(mail) = Mail::decode(bytes) else {
        return Ok(Reply::refused(Status::BadRequest, "the mail is malformed"));
    };
    let mut live = Vec::new();
    for (nameplate, opened) in store.mailboxes()? {
        if relay_rules::open(opened, now) {
            live.push(nameplate);
        } else {
            store.close_mailbox(nameplate)?;
        }
    }
    match mail {
        Mail::Open => {
            let free = if live.len() < MOST_MAILBOXES {
                relay_rules::nameplate(&live)
            } else {
                None
            };
            let Some(nameplate) = free else {
                return Ok(Reply::refused(
                    Status::Busy,
                    "the relay keeps as many pairings as it allows; try again in a few minutes",
                ));
            };
            store.open_mailbox(nameplate, now)?;
            Ok(Reply::ok(nameplate_bytes(nameplate)))
        }
        Mail::Drop {
            nameplate,
            to,
            message,
        } => {
            if !live.contains(&nameplate) {
                return Ok(Reply::refused(
                    Status::NotFound,
                    "no pairing waits under this number",
                ));
            }
            if !relay_rules::fits_mail(store.held(nameplate)?, message.len()) {
                return Ok(Reply::refused(
                    Status::TooLarge,
                    "the pairing's messages would pass what the relay keeps",
                ));
            }
            store.push(nameplate, to, &message)?;
            Ok(Reply::ok(Vec::new()))
        }
        Mail::Pick { nameplate, to } | Mail::Poll { nameplate, to } => {
            if !live.contains(&nameplate) {
                return Ok(Reply::refused(
                    Status::NotFound,
                    "no pairing waits under this number",
                ));
            }
            Ok(store.pop(nameplate, to)?.map_or(
                Reply {
                    status: Status::NoContent,
                    body: Vec::new(),
                },
                Reply::ok,
            ))
        }
    }
}
