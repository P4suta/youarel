//! Devices found an account, pair through files alone, and exchange bundles that only members open.
#![cfg(test)]

use std::path::Path;
use youarel_crypto::Label;
use youarel_crypto::kem::KeyPair;
use youarel_crypto::pairing::Code;
use youarel_crypto::sign::SigningKeys;
use youarel_protocol::message::{Action, Bundle, DevicePublic, Operation, operation_digest};
use youarel_sync::SyncError;
use youarel_sync::account::Account;
use youarel_sync::keystore::KeyStore;

const NOW: u64 = 1_700_000_000_000;

struct Device {
    directory: tempfile::TempDir,
}

impl Device {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
        }
    }

    fn account(&self) -> Account {
        let path = self.directory.path().join("account");
        Account::open(
            &path,
            &KeyStore::File(self.directory.path().join("sealing.key")),
        )
        .unwrap()
    }
}

/// Pairs `joiner` into the account `inviter` belongs to, through the four files of the protocol, and returns the inviter's state the joiner received.
fn pair(inviter: &Device, joiner: &Device, state: &[u8]) -> Vec<u8> {
    let code = inviter.account().invite().unwrap();
    let typed = Code::parse(&code.text()).unwrap();
    let join = joiner.account().join(typed).unwrap();
    let welcome = inviter.account().accept(&join).unwrap();
    let confirmation = joiner.account().confirm(&welcome).unwrap();
    let entry = inviter.account().admit(NOW, &confirmation, state).unwrap();
    let received = joiner.account().finish(&entry).unwrap().to_vec();
    assert!(joiner.account().status().unwrap().member);
    let first = joiner.account().export(NOW, b"joiner state").unwrap();
    let imported = inviter.account().import(&first).unwrap();
    assert_eq!(imported.state.as_slice(), b"joiner state");
    let rotated = inviter.account().export(NOW, state).unwrap();
    joiner.account().import(&rotated).unwrap();
    received
}

fn refused(result: Result<impl core::fmt::Debug, SyncError>, reason: &str) {
    let error = result.unwrap_err();
    assert!(error.to_string().contains(reason), "{error}");
}

#[test]
fn a_founded_account_reads_its_own_bundle() {
    let device = Device::new();
    let mut account = device.account();
    assert!(!account.has_account());
    let id = account.found(NOW).unwrap();
    assert!(account.has_account());
    let status = account.status().unwrap();
    assert_eq!(status.account, id);
    assert_eq!(status.members, [account.device()]);
    assert_eq!(status.epoch, 1);
    assert!(status.member);
    let bundle = account.export(NOW, b"state").unwrap();
    let reopened = device.account();
    assert_eq!(reopened.device(), account.device());
    let mut reopened = reopened;
    assert_eq!(reopened.import(&bundle).unwrap().state.as_slice(), b"state");
    refused(reopened.found(NOW), "already");
}

#[test]
fn a_paired_device_becomes_a_member_and_follows_the_rotation() {
    let (first, second) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    let received = pair(&first, &second, b"first state");
    assert_eq!(received, b"first state");
    let status = second.account().status().unwrap();
    assert!(status.member);
    assert_eq!(status.members.len(), 2);
    assert_eq!(status.epoch, 2);
    assert_eq!(first.account().status().unwrap().members, status.members);
    let from_second = second.account().export(NOW, b"second state").unwrap();
    assert_eq!(
        first
            .account()
            .import(&from_second)
            .unwrap()
            .state
            .as_slice(),
        b"second state"
    );
}

#[test]
fn a_removed_device_reads_no_later_bundle_and_its_bundles_are_refused() {
    let (first, second, third) = (Device::new(), Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    pair(&first, &second, b"a");
    pair(&first, &third, b"b");
    let third_id = third.account().device();
    let mut account = first.account();
    account.remove(NOW, third_id).unwrap();
    let after = account.export(NOW, b"secret").unwrap();
    let epoch = account.status().unwrap().epoch;
    account.export(NOW, b"again").unwrap();
    assert_eq!(
        account.status().unwrap().epoch,
        epoch,
        "one rotation follows a removal"
    );
    second.account().import(&after).unwrap();
    let from_third = third.account().export(NOW, b"late").unwrap();
    refused(first.account().import(&from_third), "no member");
    refused(second.account().import(&from_third), "no member");
    refused(
        third.account().import(&after),
        "no key of the bundle's epoch",
    );
    refused(
        third.account().export(NOW, b"later"),
        "no key of the current epoch",
    );
    assert!(!third.account().status().unwrap().member);
    assert_eq!(second.account().status().unwrap().members.len(), 2);
}

#[test]
fn a_wrong_code_opens_no_channel_and_adds_nobody() {
    let (first, stranger) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    let code = first.account().invite().unwrap();
    let wrong = Code::parse(&format!("{}-0000-0000", code.number)).unwrap();
    let join = stranger.account().join(wrong).unwrap();
    let welcome = first.account().accept(&join).unwrap();
    refused(stranger.account().confirm(&welcome), "confirmation");
    assert_eq!(first.account().status().unwrap().members.len(), 1);
    refused(first.account().accept(&join), "no invitation");
}

/// A carrier that answers the welcome itself, without the code, gets no entry, so it holds nothing to open later.
#[test]
fn the_inviter_seals_nothing_for_a_joiner_that_never_confirmed() {
    let (first, joiner) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    let code = first.account().invite().unwrap();
    let join = joiner.account().join(code).unwrap();
    let welcome = first.account().accept(&join).unwrap();
    let mut forged = joiner.account().confirm(&welcome).unwrap();
    let last = forged.len() - 1;
    forged[last] ^= 1;
    refused(
        first.account().admit(NOW, &forged, b"secret"),
        "does not verify",
    );
    assert_eq!(first.account().status().unwrap().members.len(), 1);
    refused(joiner.account().finish(&welcome), "another kind");
}

#[test]
fn a_changed_bundle_or_one_of_another_account_is_refused() {
    let (first, other) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    other.account().found(NOW).unwrap();
    let mut bundle = first.account().export(NOW, b"state").unwrap();
    refused(other.account().import(&bundle), "another account");
    let last = bundle.len() - 1;
    bundle[last] ^= 1;
    refused(first.account().import(&bundle), "signature");
    refused(first.account().import(b"not a bundle"), "another kind");
}

/// A carrier slips an addition of its own device into a bundle, naming a member as its author; the signature fails, so the log ignores it.
#[test]
fn an_operation_whose_signature_fails_takes_no_part() {
    let (first, second) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    pair(&first, &second, b"a");
    let status = first.account().status().unwrap();
    let signing = SigningKeys::generate().unwrap();
    let kem = KeyPair::generate().unwrap();
    let stranger = DevicePublic {
        signing: signing.public().clone(),
        kem: Box::new(*kem.public()),
    };
    let bundle = Bundle::decode(&first.account().export(NOW, b"s").unwrap()).unwrap();
    let forged = Operation {
        account: status.account,
        parents: vec![operation_digest(&bundle.operations[0])],
        author: status.this_device,
        wall: NOW,
        action: Action::Add(stranger),
        signature: signing.sign(Label::Operation, b"forged").unwrap(),
    };
    let mut operations = bundle.operations.clone();
    operations.push(forged.encode().unwrap());
    operations.sort_by_key(|bytes| operation_digest(bytes));
    let tampered = Bundle {
        operations,
        ..bundle
    };
    refused(
        second.account().import(&tampered.encode().unwrap()),
        "signature",
    );
    assert_eq!(second.account().status().unwrap().members.len(), 2);
}

#[test]
fn only_a_member_invites_and_only_a_device_without_an_account_joins() {
    let (first, second) = (Device::new(), Device::new());
    refused(second.account().invite(), "no account");
    first.account().found(NOW).unwrap();
    let code = first.account().invite().unwrap();
    refused(first.account().join(code), "already");
    let path: &Path = first.directory.path();
    let log = youarel_store::fs::list(&path.join("account").join("log")).unwrap();
    assert_eq!(log.len(), 2, "the genesis and the first rotation");
}
