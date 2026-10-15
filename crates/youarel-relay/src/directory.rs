//! The relay's directory, which tells a device without an account which account lists a member.
//!
//! A device that recovers with a recovery key knows the member the key grows into but not its account, and the account routes every request, so the relay notes each account's recovery members whenever a member publishes.
//! The directory answers without a signature: it names an account only to a device that already knows a recovery member's identifier, which only the recovery key or the account's log reveals.
//! It notes only a member whose consent to the account the log holds, so a member that learned the identifier cannot point it at an account of its own.

use crate::{Reply, Status, StoreError};
use youarel_protocol::message::Digest;
use youarel_protocol::relay::{Locate, Registration, located_bytes};

/// What the relay keeps of the members of every account.
pub trait DirectoryStore {
    /// The account that lists `member`, if the directory knows one.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn account_of(&self, member: &Digest) -> Result<Option<Digest>, StoreError>;

    /// Notes that `account` lists `members`.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the storage fails.
    fn register(&mut self, account: &Digest, members: &[Digest]) -> Result<(), StoreError>;
}

/// Answers a question of location.
///
/// # Errors
///
/// Returns a [`StoreError`] when the storage fails; every refusal is a reply instead.
pub fn locate(store: &impl DirectoryStore, bytes: &[u8]) -> Result<Reply, StoreError> {
    let Ok(question) = Locate::decode(bytes) else {
        return Ok(Reply::refused(
            Status::BadRequest,
            "the question is malformed",
        ));
    };
    Ok(store.account_of(&question.member)?.map_or_else(
        || {
            Reply::refused(
                Status::NotFound,
                "the relay knows no account that lists this member",
            )
        },
        |account| Reply::ok(located_bytes(&account)),
    ))
}

/// Notes a registration, which only the relay's own account storage sends.
///
/// # Errors
///
/// Returns a [`StoreError`] when the storage fails; a malformed registration is a refusal instead.
pub fn register(store: &mut impl DirectoryStore, bytes: &[u8]) -> Result<Reply, StoreError> {
    let Ok(registration) = Registration::decode(bytes) else {
        return Ok(Reply::refused(
            Status::BadRequest,
            "the registration is malformed",
        ));
    };
    store.register(&registration.account, &registration.members)?;
    Ok(Reply::ok(Vec::new()))
}
