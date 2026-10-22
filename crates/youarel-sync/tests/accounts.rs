//! Devices found an account, pair through files alone, and exchange bundles that only members open.
#![cfg(test)]

use std::path::Path;
use youarel_core::segment_rules::{Header, Kind, Sequence};
use youarel_crypto::Label;
use youarel_crypto::kem::KeyPair;
use youarel_crypto::pairing::Code;
use youarel_crypto::sign::SigningKeys;
use youarel_protocol::message::{Action, DevicePublic, Operation, operation_digest};
use youarel_protocol::segment::Segment;
use youarel_sync::SyncError;
use youarel_sync::account::Account;

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
        Account::open(&self.directory.path().join("account")).unwrap()
    }
}

fn snapshot(account: &mut Account, body: &[u8]) -> Result<Vec<u8>, SyncError> {
    let header = Header::new(
        Sequence::FIRST,
        [0; 32],
        Kind::Snapshot { part: 0, parts: 1 },
    )
    .unwrap();
    account.publish(NOW, header, body)
}

/// Pairs through the four messages, then fetches the state separately.
fn pair(inviter: &Device, joiner: &Device, state: &[u8]) -> Vec<u8> {
    let code = inviter.account().invite().unwrap();
    let typed = Code::parse(&code.text()).unwrap();
    let join = joiner.account().join(typed).unwrap();
    let welcome = inviter.account().accept(&join).unwrap();
    let confirmation = joiner.account().confirm(&welcome).unwrap();
    let entry = inviter.account().admit(NOW, &confirmation, None).unwrap();
    assert_eq!(joiner.account().finish(&entry).unwrap(), None);
    assert!(joiner.account().status().unwrap().member);
    let first = snapshot(&mut joiner.account(), b"joiner state").unwrap();
    let imported = inviter.account().open_segment(&first).unwrap();
    assert_eq!(imported.body.as_slice(), b"joiner state");
    let rotated = snapshot(&mut inviter.account(), state).unwrap();
    joiner
        .account()
        .open_segment(&rotated)
        .unwrap()
        .body
        .to_vec()
}

fn refused(result: Result<impl core::fmt::Debug, SyncError>, reason: &str) {
    let error = result.unwrap_err();
    assert!(error.to_string().contains(reason), "{error}");
}

#[test]
fn a_founded_account_reads_its_own_segment() {
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
    let segment = snapshot(&mut account, b"state").unwrap();
    let reopened = device.account();
    assert_eq!(reopened.device(), account.device());
    let mut reopened = reopened;
    assert_eq!(
        reopened.open_segment(&segment).unwrap().body.as_slice(),
        b"state"
    );
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
    let from_second = snapshot(&mut second.account(), b"second state").unwrap();
    assert_eq!(
        first
            .account()
            .open_segment(&from_second)
            .unwrap()
            .body
            .as_slice(),
        b"second state"
    );
}

#[test]
fn a_removed_device_reads_no_later_segment_and_its_segments_are_refused() {
    let (first, second, third) = (Device::new(), Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    pair(&first, &second, b"a");
    pair(&first, &third, b"b");
    let third_id = third.account().device();
    let mut account = first.account();
    account.remove(NOW, third_id).unwrap();
    let after = snapshot(&mut account, b"secret").unwrap();
    let epoch = account.status().unwrap().epoch;
    snapshot(&mut account, b"again").unwrap();
    assert_eq!(
        account.status().unwrap().epoch,
        epoch,
        "one rotation follows a removal"
    );
    second.account().open_segment(&after).unwrap();
    let from_third = snapshot(&mut third.account(), b"late").unwrap();
    refused(first.account().open_segment(&from_third), "no member");
    refused(second.account().open_segment(&from_third), "no member");
    refused(
        third.account().open_segment(&after),
        "no key of the segment's epoch",
    );
    refused(
        snapshot(&mut third.account(), b"later"),
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
    refused(first.account().admit(NOW, &forged, None), "does not verify");
    assert_eq!(first.account().status().unwrap().members.len(), 1);
    refused(joiner.account().finish(&welcome), "another kind");
}

#[test]
fn a_changed_segment_or_one_of_another_account_is_refused() {
    let (first, other) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    other.account().found(NOW).unwrap();
    let mut segment = snapshot(&mut first.account(), b"state").unwrap();
    refused(other.account().open_segment(&segment), "another account");
    let last = segment.len() - 1;
    segment[last] ^= 1;
    refused(first.account().open_segment(&segment), "signature");
    refused(
        first.account().open_segment(b"not a segment"),
        "another kind",
    );
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
    let segment = Segment::decode(&snapshot(&mut first.account(), b"s").unwrap()).unwrap();
    let forged = Operation {
        account: status.account,
        parents: vec![operation_digest(&segment.operations[0])],
        author: status.this_device,
        wall: NOW,
        action: Action::Add(stranger),
        signature: signing.sign(Label::Operation, b"forged").unwrap(),
    };
    let mut operations = segment.operations.clone();
    operations.push(forged.encode().unwrap());
    operations.sort_by_key(|bytes| operation_digest(bytes));
    let tampered = Segment {
        operations,
        ..segment
    };
    refused(
        second.account().open_segment(&tampered.encode().unwrap()),
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

#[test]
fn a_late_entry_never_replaces_an_association_created_after_confirmation() {
    let (inviter, joiner) = (Device::new(), Device::new());
    inviter.account().found(NOW).unwrap();
    let code = inviter.account().invite().unwrap();
    let join = joiner.account().join(code).unwrap();
    let welcome = inviter.account().accept(&join).unwrap();
    let confirmation = joiner.account().confirm(&welcome).unwrap();
    let entry = inviter.account().admit(NOW, &confirmation, None).unwrap();
    let owned = joiner.account().found(NOW).unwrap();
    refused(joiner.account().finish(&entry), "already");
    assert_eq!(joiner.account().status().unwrap().account, owned);
}

#[test]
fn an_authenticated_entry_must_actually_admit_the_confirming_device() {
    use youarel_crypto::{pairing, seal};
    use youarel_protocol::message::{Admission, Entry, Join, Welcome};
    let (inviter, joiner) = (Device::new(), Device::new());
    inviter.account().found(NOW).unwrap();
    let code = inviter.account().invite().unwrap();
    let join = Join::decode(&joiner.account().join(code).unwrap()).unwrap();
    let accepted = pairing::accept(
        code,
        &join.pake,
        join.joiner.kem.as_slice(),
        &join.joiner.encode(),
    )
    .unwrap();
    let welcome = Welcome {
        invitation: code.number,
        pake: accepted.inviter_message,
        encapsulation: accepted.encapsulation,
        welcomed: accepted.channel.welcomed(),
    };
    joiner
        .account()
        .confirm(&welcome.encode().unwrap())
        .unwrap();
    let segment = Segment::decode(&snapshot(&mut inviter.account(), b"").unwrap()).unwrap();
    let admission = Admission {
        account: segment.account,
        operations: segment.operations,
        epochs: Vec::new(),
        hint: None,
    };
    let entry = Entry {
        invitation: code.number,
        sealed: seal::seal(
            accepted.channel.key(),
            &code.number.to_be_bytes(),
            &admission.encode().unwrap(),
        )
        .unwrap(),
    };
    refused(
        joiner.account().finish(&entry.encode().unwrap()),
        "does not admit this device",
    );
    assert!(!joiner.account().has_account());
    assert_eq!(joiner.account().operations(), 0);
}
