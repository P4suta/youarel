use super::{Account, EpochKeys, GENESIS, Identity};
use crate::SyncError;
use std::collections::BTreeMap;
use youarel_core::segment_rules::Header;
use youarel_crypto::recovery::RecoveryKey;
use youarel_crypto::seal;
use youarel_crypto::{KEY, Label};
use youarel_protocol::log::{self, Checked};
use youarel_protocol::message::{Action, Digest, Operation, account_of, operation_digest};
use youarel_protocol::segment::{self, LONGEST_SEGMENT, Segment, body_digest};
use zeroize::Zeroizing;

struct VerifiedRecovery {
    genesis: Digest,
    author: Identity,
    checked: Checked,
    keys: EpochKeys,
}

fn open_body(segment: &Segment, key: &[u8; KEY]) -> Result<OpenedSegment, SyncError> {
    let body = seal::open(key, &segment.binding(), &segment.sealed)?;
    Ok(OpenedSegment {
        sender: segment.sender,
        header: segment.header,
        digest: body_digest(&body),
        body,
    })
}

pub struct OpenedSegment {
    pub sender: Digest,
    pub header: Header,
    pub digest: Digest,
    pub body: Zeroizing<Vec<u8>>,
}

impl core::fmt::Debug for OpenedSegment {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OpenedSegment")
            .field("sender", &self.sender)
            .field("header", &self.header)
            .field("digest", &self.digest)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

impl Account {
    /// Learns only checked membership operations of this account before reading a delivery.
    ///
    /// # Errors
    ///
    /// Refuses another account or malformed operations, and reports storage or key failures.
    pub fn learn_membership(
        &mut self,
        account: Digest,
        operations: &[Vec<u8>],
    ) -> Result<(), SyncError> {
        if account != self.account_id()? {
            return Err(SyncError::Refused(String::from(
                "the delivery belongs to another account",
            )));
        }
        let mut union = self.log.clone();
        for bytes in operations {
            Operation::decode(bytes)?;
            union
                .entry(operation_digest(bytes))
                .or_insert_with(|| bytes.clone());
        }
        let genesis = self
            .genesis
            .ok_or_else(|| SyncError::Refused(String::from("this device has no account")))?;
        let checked = log::check(genesis, account, &union);
        for digest in checked.operations.keys() {
            if let Some(bytes) = union.get(digest) {
                self.keep_operation(bytes.clone())?;
            }
        }
        self.learn_keys(&checked)
    }

    /// Performs any required rotation before a publication captures its membership count.
    ///
    /// # Errors
    ///
    /// Reports missing membership, epoch keys, signing, or storage failures.
    pub fn prepare_publish(&mut self, now: u64) -> Result<(), SyncError> {
        self.rotate_if_due(now)
    }

    fn segment_key(&self, checked: &Checked) -> Result<(u32, &[u8; KEY]), SyncError> {
        let commitment = checked
            .resolution
            .rotation
            .and_then(|digest| checked.operations.get(&digest))
            .and_then(|operation| match &operation.action {
                Action::Rotate(rotation) => Some(rotation.commitment),
                Action::Genesis(_)
                | Action::Add(_)
                | Action::AddRecovery(_)
                | Action::Remove(_) => None,
            })
            .ok_or_else(|| SyncError::Refused("the account has no epoch key yet".to_owned()))?;
        let (epoch, key) = self.epochs.get(&commitment).ok_or_else(|| {
            SyncError::Refused(
                "this device holds no key of the current epoch; sync the membership log first"
                    .to_owned(),
            )
        })?;
        Ok((*epoch, key))
    }

    /// Seals bounded record bytes and signs their chain header under the current epoch.
    ///
    /// # Errors
    ///
    /// Refuses an oversized body or an account whose current epoch key is unavailable.
    pub fn publish(&mut self, now: u64, header: Header, body: &[u8]) -> Result<Vec<u8>, SyncError> {
        if body.len() > LONGEST_SEGMENT {
            return Err(SyncError::Refused(
                "the segment body exceeds its byte limit".to_owned(),
            ));
        }
        self.prepare_publish(now)?;
        let checked = self.check()?;
        let account = self.account_id()?;
        let sender = self.device();
        let (epoch, key) = self.segment_key(&checked)?;
        let sealed = seal::seal(
            key,
            &segment::binding(&account, &sender, epoch, header),
            body,
        )?;
        let operations: Vec<Vec<u8>> = checked
            .operations
            .keys()
            .filter_map(|digest| self.log.get(digest).cloned())
            .collect();
        let unsigned = segment::Unsigned {
            account: &account,
            operations: &operations,
            sender: &sender,
            epoch,
            header,
            sealed: &sealed,
        };
        let signature = self
            .identity
            .signing
            .sign(Label::Segment, &unsigned.bytes()?)?;
        Ok(Segment {
            account,
            operations,
            sender,
            epoch,
            header,
            sealed,
            signature,
        }
        .encode()?)
    }

    fn verify_segment(checked: &Checked, segment: &Segment) -> Result<(), SyncError> {
        let sender = checked.keys.get(&segment.sender).ok_or_else(|| {
            SyncError::Refused(
                "the segment comes from a device this account never listed".to_owned(),
            )
        })?;
        if !sender.signing.verify(
            Label::Segment,
            &segment.unsigned().bytes()?,
            &segment.signature,
        ) {
            return Err(SyncError::Refused(
                "the segment's signature does not verify".to_owned(),
            ));
        }
        if !checked.resolution.members.contains(&segment.sender) {
            return Err(SyncError::Refused(
                "the segment's sender is no member of the account".to_owned(),
            ));
        }
        Ok(())
    }

    /// Authenticates the membership, sender, complete header, and ciphertext before returning plaintext.
    ///
    /// # Errors
    ///
    /// Refuses another account, a removed sender, a forged signature or seal, or an unavailable epoch key.
    pub fn open_segment(&mut self, bytes: &[u8]) -> Result<OpenedSegment, SyncError> {
        let segment = Segment::decode(bytes)?;
        if segment.account != self.account_id()? {
            return Err(SyncError::Refused(
                "the segment belongs to another account".to_owned(),
            ));
        }
        for operation in &segment.operations {
            Operation::decode(operation)?;
            self.keep_operation(operation.clone())?;
        }
        let checked = self.check()?;
        Self::verify_segment(&checked, &segment)?;
        self.learn_keys(&checked)?;
        let (epoch, key) = self.epochs.get(&segment.sealed.commitment).ok_or_else(|| {
            SyncError::Refused("this device holds no key of the segment's epoch".to_owned())
        })?;
        if *epoch != segment.epoch {
            return Err(SyncError::Refused(
                "the segment names another epoch for its key".to_owned(),
            ));
        }
        open_body(&segment, key)
    }

    fn segment_recovery(
        &self,
        key: &RecoveryKey,
        segment: &Segment,
    ) -> Result<VerifiedRecovery, SyncError> {
        let genesis = segment
            .operations
            .iter()
            .find(|operation| account_of(operation) == segment.account)
            .map(|operation| operation_digest(operation))
            .ok_or_else(|| {
                SyncError::Refused(String::from("the segment lacks its account's genesis"))
            })?;
        let author = Identity::from_recovery(key)?;
        let operations: BTreeMap<Digest, Vec<u8>> = segment
            .operations
            .iter()
            .map(|operation| (operation_digest(operation), operation.clone()))
            .collect();
        let checked = log::check(genesis, segment.account, &operations);
        if !checked.recoveries().contains(&author.public.id()) {
            return Err(SyncError::Refused(String::from(
                "the recovery key belongs to no member of this account",
            )));
        }
        Self::verify_segment(&checked, segment)?;
        let keys = self.unwrapped(&segment.account, &checked, &author)?;
        Ok(VerifiedRecovery {
            genesis,
            author,
            checked,
            keys,
        })
    }

    /// Authenticates one segment with a recovery key and enrolls this device without loading a whole library.
    ///
    /// # Errors
    ///
    /// Refuses an associated device, a key outside the checked membership, or a forged or unreadable segment before saving the account association.
    pub fn recover_segment(
        &mut self,
        now: u64,
        key: &RecoveryKey,
        bytes: &[u8],
    ) -> Result<OpenedSegment, SyncError> {
        if self.genesis.is_some() {
            return Err(SyncError::Refused(String::from(
                "this device belongs to an account already",
            )));
        }
        let segment = Segment::decode(bytes)?;
        let recovery = self.segment_recovery(key, &segment)?;
        let epoch_key = recovery
            .keys
            .iter()
            .find(|(epoch, candidate)| {
                *epoch == segment.epoch
                    && seal::commitment(candidate)
                        .is_ok_and(|made| made == segment.sealed.commitment)
            })
            .map(|(_, candidate)| candidate)
            .ok_or_else(|| {
                SyncError::Refused(String::from(
                    "the recovery key holds no key of the segment's epoch",
                ))
            })?;
        let opened = open_body(&segment, epoch_key)?;
        let added = Self::operation_by(
            &recovery.author,
            segment.account,
            &recovery.checked,
            (now, Action::Add(self.identity.public.clone())),
        )?;
        for operation in segment.operations {
            self.keep_operation(operation)?;
        }
        youarel_store::fs::replace(
            &self.directory.join(GENESIS),
            format!("{}\n", crate::hex(&recovery.genesis)).as_bytes(),
        )?;
        self.genesis = Some(recovery.genesis);
        self.keep_keys(recovery.keys)?;
        self.keep_operation(added)?;
        Ok(opened)
    }
}
