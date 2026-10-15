//! The membership log as every reader checks it: the operations whose signatures verify, the keys they publish, and their resolution.
//!
//! A device and the relay run the same check, so both draw the same members from the same operations.

use crate::message::{Action, DevicePublic, Digest, Operation};
use std::collections::{BTreeMap, BTreeSet};
use youarel_core::membership_rules::{self, Resolution};
use youarel_crypto::Label;

/// The account the genesis operation names, which no account has before it exists.
pub const NO_ACCOUNT: Digest = [0; 32];

/// The log after its checks: every operation that verified, the keys the log publishes, and its resolution.
#[derive(Clone, Debug)]
pub struct Checked {
    pub operations: BTreeMap<Digest, Operation>,
    pub keys: BTreeMap<Digest, DevicePublic>,
    pub resolution: Resolution,
}

impl Checked {
    /// Whether the device is a member of the account.
    #[must_use]
    pub fn member(&self, device: &Digest) -> bool {
        self.resolution.members.contains(device)
    }

    /// The members that a recovery key grows into, each of which consented to this account.
    #[must_use]
    pub fn recoveries(&self) -> Vec<Digest> {
        let added: BTreeSet<Digest> = self
            .operations
            .values()
            .filter_map(|operation| match &operation.action {
                Action::AddRecovery(recovery) => Some(recovery.device.id()),
                Action::Genesis(_) | Action::Add(_) | Action::Remove(_) | Action::Rotate(_) => None,
            })
            .collect();
        self.resolution
            .members
            .iter()
            .filter(|member| added.contains(*member))
            .copied()
            .collect()
    }

    /// The operations no other operation names as a parent, which a new operation names.
    #[must_use]
    pub fn heads(&self) -> Vec<Digest> {
        let named: BTreeSet<Digest> = self
            .operations
            .values()
            .flat_map(|operation| operation.parents.iter().copied())
            .collect();
        self.operations
            .keys()
            .filter(|digest| !named.contains(*digest))
            .copied()
            .collect()
    }
}

/// Checks the log of the account whose genesis is `genesis` and whose identifier is `account`.
///
/// An operation counts only when it decodes, names the account, or no account for the genesis, and carries its author's signature under the keys that the log publishes for that author.
/// A recovery identity counts only with its consent to the account the operation names.
#[must_use]
pub fn check<'a>(
    genesis: Digest,
    account: Digest,
    log: impl IntoIterator<Item = (&'a Digest, &'a Vec<u8>)>,
) -> Checked {
    let mut decoded = BTreeMap::new();
    let mut keys = BTreeMap::new();
    for (digest, bytes) in log {
        let Ok(operation) = Operation::decode(bytes) else {
            continue;
        };
        if let Some(device) = operation.action.published() {
            keys.insert(device.id(), device.clone());
        }
        decoded.insert(*digest, operation);
    }
    let mut operations = BTreeMap::new();
    for (digest, operation) in decoded {
        let expected = if digest == genesis {
            NO_ACCOUNT
        } else {
            account
        };
        let consents = match &operation.action {
            Action::AddRecovery(recovery) => recovery.consents_to(&operation.account),
            Action::Genesis(_) | Action::Add(_) | Action::Remove(_) | Action::Rotate(_) => true,
        };
        let verifies = operation.account == expected
            && consents
            && operation.unsigned().bytes().is_ok_and(|unsigned| {
                keys.get(&operation.author)
                    .is_some_and(|author: &DevicePublic| {
                        author
                            .signing
                            .verify(Label::Operation, &unsigned, &operation.signature)
                    })
            });
        if verifies {
            operations.insert(digest, operation);
        }
    }
    let rules: Vec<membership_rules::Operation> = operations
        .iter()
        .map(|(digest, operation)| membership_rules::Operation {
            digest: *digest,
            parents: operation.parents.clone(),
            author: operation.author,
            action: operation.action.rule(),
        })
        .collect();
    let resolution = membership_rules::resolve(genesis, &rules);
    Checked {
        operations,
        keys,
        resolution,
    }
}

#[cfg(test)]
mod tests {
    use super::{NO_ACCOUNT, check};
    use crate::message::{
        Action, DevicePublic, Digest, Operation, Recovery, account_of, operation_digest,
    };
    use std::collections::BTreeMap;
    use youarel_crypto::Label;
    use youarel_crypto::kem::KeyPair;
    use youarel_crypto::sign::SigningKeys;

    fn device() -> (SigningKeys, DevicePublic) {
        let keys = SigningKeys::generate().unwrap();
        let kem = KeyPair::generate().unwrap();
        let public = DevicePublic {
            signing: keys.public().clone(),
            kem: Box::new(*kem.public()),
        };
        (keys, public)
    }

    /// An operation of `author` in `account`, signed by `signer`.
    fn signed(
        (signer, author): (&SigningKeys, Digest),
        account: Digest,
        parents: Vec<Digest>,
        action: Action,
    ) -> Vec<u8> {
        let mut operation = Operation {
            account,
            parents,
            author,
            wall: 1,
            action,
            signature: signer.sign(Label::Operation, b"").unwrap(),
        };
        operation.signature = signer
            .sign(Label::Operation, &operation.unsigned().bytes().unwrap())
            .unwrap();
        operation.encode().unwrap()
    }

    #[test]
    fn only_the_accounts_operations_under_their_authors_signatures_count() {
        let (first_keys, first) = device();
        let (second_keys, second) = device();
        let (_, third) = device();
        let founder = (&first_keys, first.id());
        let genesis = signed(
            founder,
            NO_ACCOUNT,
            Vec::new(),
            Action::Genesis(first.clone()),
        );
        let account = account_of(&genesis);
        let root = operation_digest(&genesis);
        let added = signed(founder, account, vec![root], Action::Add(second.clone()));
        let elsewhere = signed(founder, [9; 32], vec![root], Action::Add(third.clone()));
        let forged = signed(
            (&second_keys, first.id()),
            account,
            vec![root],
            Action::Add(third.clone()),
        );
        let log: BTreeMap<Digest, Vec<u8>> = [genesis, added.clone(), elsewhere, forged]
            .into_iter()
            .map(|bytes| (operation_digest(&bytes), bytes))
            .collect();
        let checked = check(root, account, &log);
        assert_eq!(checked.operations.len(), 2);
        assert!(checked.member(&first.id()) && checked.member(&second.id()));
        assert!(!checked.member(&third.id()));
        assert_eq!(checked.heads(), [operation_digest(&added)]);
    }

    #[test]
    fn a_recovery_identity_counts_only_in_the_account_it_consented_to() {
        let (first_keys, first) = device();
        let (recovery_keys, recovery) = device();
        let founder = (&first_keys, first.id());
        let genesis = signed(founder, NO_ACCOUNT, Vec::new(), Action::Genesis(first));
        let account = account_of(&genesis);
        let root = operation_digest(&genesis);
        let after_genesis = |action: Action| {
            let added = signed(founder, account, vec![root], action);
            let log: BTreeMap<Digest, Vec<u8>> = [genesis.clone(), added]
                .into_iter()
                .map(|bytes| (operation_digest(&bytes), bytes))
                .collect();
            check(root, account, &log)
        };
        let consenting = |to: Digest| {
            Action::AddRecovery(Recovery {
                device: recovery.clone(),
                consent: recovery_keys.sign(Label::RecoveryConsent, &to).unwrap(),
            })
        };
        let misbound = after_genesis(consenting([9; 32]));
        assert!(!misbound.member(&recovery.id()));
        assert_eq!(misbound.recoveries(), Vec::<Digest>::new());
        let plain = after_genesis(Action::Add(recovery.clone()));
        assert!(plain.member(&recovery.id()));
        assert_eq!(
            plain.recoveries(),
            Vec::<Digest>::new(),
            "a plain addition carries no consent"
        );
        let consented = after_genesis(consenting(account));
        assert_eq!(consented.recoveries(), [recovery.id()]);
        assert_eq!(consented.keys.get(&recovery.id()), Some(&recovery));
    }

    #[test]
    fn a_genesis_that_names_an_account_founds_nothing() {
        let (keys, public) = device();
        let genesis = signed(
            (&keys, public.id()),
            [9; 32],
            Vec::new(),
            Action::Genesis(public.clone()),
        );
        let root = operation_digest(&genesis);
        let log = BTreeMap::from([(root, genesis.clone())]);
        let checked = check(root, account_of(&genesis), &log);
        assert!(checked.operations.is_empty());
        assert!(!checked.member(&public.id()));
    }
}
