//! Segment authentication, historical epoch keys, and revocation at the account boundary.
#![cfg(test)]

use youarel_core::segment_rules::{Header, Kind, Sequence};
use youarel_protocol::segment::{Segment, body_digest};
use youarel_sync::account::Account;

const NOW: u64 = 1_700_000_000_000;

fn header(sequence: u64, previous: &[u8], kind: Kind) -> Header {
    let previous = if sequence == 1 {
        [0; 32]
    } else {
        body_digest(previous)
    };
    Header::new(Sequence::new(sequence).unwrap(), previous, kind).unwrap()
}

fn paired(first: &mut Account, second: &mut Account) {
    let code = first.invite().unwrap();
    let join = second.join(code).unwrap();
    let welcome = first.accept(&join).unwrap();
    let confirmed = second.confirm(&welcome).unwrap();
    let entry = first.admit(NOW, &confirmed, None).unwrap();
    second.finish(&entry).unwrap();
}

#[test]
fn a_segment_opens_only_for_its_account_with_its_complete_header_and_signature() {
    let directory = tempfile::tempdir().unwrap();
    let mut account = Account::open(directory.path()).unwrap();
    account.found(NOW).unwrap();
    let header = header(1, b"", Kind::Snapshot { part: 0, parts: 1 });
    let bytes = account.publish(NOW, header, b"first").unwrap();
    let opened = account.open_segment(&bytes).unwrap();
    assert_eq!(opened.sender, account.device());
    assert_eq!(opened.header, header);
    assert_eq!(opened.body.as_slice(), b"first");
    assert_eq!(opened.digest, body_digest(b"first"));
    let other = tempfile::tempdir().unwrap();
    let mut stranger = Account::open(other.path()).unwrap();
    stranger.found(NOW).unwrap();
    assert!(
        stranger
            .open_segment(&bytes)
            .unwrap_err()
            .to_string()
            .contains("another account")
    );
    let mut forged = Segment::decode(&bytes).unwrap();
    forged.header = Header::new(Sequence::new(2).unwrap(), [0; 32], Kind::Delta).unwrap();
    assert!(
        account
            .open_segment(&forged.encode().unwrap())
            .unwrap_err()
            .to_string()
            .contains("signature")
    );
}

#[test]
fn a_later_member_opens_earlier_epochs_and_a_removed_member_opens_no_new_epoch() {
    let homes = [
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    ];
    let mut first = Account::open(homes[0].path()).unwrap();
    let mut second = Account::open(homes[1].path()).unwrap();
    let mut removed = Account::open(homes[2].path()).unwrap();
    first.found(NOW).unwrap();
    let old = first
        .publish(NOW, header(1, b"", Kind::Delta), b"old")
        .unwrap();
    paired(&mut first, &mut second);
    paired(&mut first, &mut removed);
    assert_eq!(second.open_segment(&old).unwrap().body.as_slice(), b"old");
    first.remove(NOW, removed.device()).unwrap();
    let next = first
        .publish(NOW, header(2, b"old", Kind::Delta), b"new secret")
        .unwrap();
    assert_eq!(
        second.open_segment(&next).unwrap().body.as_slice(),
        b"new secret"
    );
    removed.open_segment(&next).unwrap_err();
    let earlier = removed.publish(NOW, header(1, b"", Kind::Delta), b"late");
    earlier.unwrap_err();
}

#[test]
fn recovery_authenticates_a_segment_before_associating_the_new_device() {
    let home = tempfile::tempdir().unwrap();
    let mut first = Account::open(home.path()).unwrap();
    first.found(NOW).unwrap();
    let key = first.enroll_recovery(NOW).unwrap();
    let bytes = first
        .publish(
            NOW,
            header(1, b"", Kind::Snapshot { part: 0, parts: 1 }),
            b"snapshot",
        )
        .unwrap();
    let fresh = tempfile::tempdir().unwrap();
    let mut recovered = Account::open(fresh.path()).unwrap();
    let wrong = youarel_crypto::recovery::RecoveryKey::generate().unwrap();
    recovered.recover_segment(NOW, &wrong, &bytes).unwrap_err();
    assert!(!youarel_sync::account::founded(fresh.path()).unwrap());
    let mut forged = Segment::decode(&bytes).unwrap();
    forged.sealed.ciphertext[0] ^= 1;
    recovered
        .recover_segment(NOW, &key, &forged.encode().unwrap())
        .unwrap_err();
    assert!(!youarel_sync::account::founded(fresh.path()).unwrap());
    assert_eq!(recovered.operations(), 0);
    let opened = recovered.recover_segment(NOW, &key, &bytes).unwrap();
    assert_eq!(opened.body.as_slice(), b"snapshot");
    assert!(recovered.status().unwrap().member);
    assert_eq!(recovered.open_segment(&bytes).unwrap().body, opened.body);
}

#[test]
fn a_confirmed_device_receives_a_hint_and_fetches_the_plaintext_separately() {
    use youarel_protocol::message::SnapshotHint;
    let home = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut inviter = Account::open(home.path()).unwrap();
    let mut joiner = Account::open(other.path()).unwrap();
    inviter.found(NOW).unwrap();
    let bytes = inviter
        .publish(
            NOW,
            header(1, b"", Kind::Snapshot { part: 0, parts: 1 }),
            b"separate snapshot",
        )
        .unwrap();
    let code = inviter.invite().unwrap();
    let join = joiner.join(code).unwrap();
    let welcome = inviter.accept(&join).unwrap();
    let confirmation = joiner.confirm(&welcome).unwrap();
    let hint = Some(SnapshotHint {
        sender: inviter.device(),
        sequence: Sequence::FIRST,
    });
    let entry = inviter.admit(NOW, &confirmation, hint).unwrap();
    assert_eq!(joiner.finish(&entry).unwrap(), hint);
    assert_eq!(
        joiner.open_segment(&bytes).unwrap().body.as_slice(),
        b"separate snapshot"
    );
}
