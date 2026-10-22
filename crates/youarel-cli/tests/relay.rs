//! The relay against real accounts in memory: it keeps each member's newest bundle, refuses every request it must, and forgets an account after its retention.
#![cfg(test)]

use youarel_core::relay_rules::{MAILBOX_LIFETIME, RETENTION, WINDOW};
use youarel_core::segment_rules::{Header, Kind, Sequence};
use youarel_crypto::pairing::Code;
use youarel_protocol::message::{Action, Operation};
use youarel_protocol::relay::{Locate, Mail, Registration, Request, Side, Verb, read_nameplate};
use youarel_protocol::segment::{Cursors, Delivered, Segment};
use youarel_relay::Status;
use youarel_relay::memory::MemoryRelay;
use youarel_sync::account::Account;

const NOW: u64 = 1_700_000_000_000;

struct Device {
    directory: tempfile::TempDir,
    sequence: std::cell::Cell<u64>,
}

impl Device {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
            sequence: std::cell::Cell::new(0),
        }
    }

    fn account(&self) -> Account {
        Account::open(&self.directory.path().join("account")).unwrap()
    }

    fn publish(&self, relay: &mut MemoryRelay, now: u64, state: &[u8]) -> (Status, Vec<u8>) {
        let mut account = self.account();
        let number = self.sequence.get().checked_add(1).unwrap();
        self.sequence.set(number);
        let segment = snapshot(&mut account, now, (number, state));
        let request = account.request(now, Verb::Publish, segment).unwrap();
        let reply = relay.request(now, &request).unwrap();
        (reply.status, reply.body)
    }

    fn collect(&self, relay: &mut MemoryRelay, now: u64) -> (Status, Vec<u8>) {
        let request = self
            .account()
            .request(now, Verb::Collect, positions())
            .unwrap();
        let reply = relay.request(now, &request).unwrap();
        (reply.status, reply.body)
    }
}

fn positions() -> Vec<u8> {
    Cursors::default().encode().unwrap()
}

fn snapshot(account: &mut Account, now: u64, (number, state): (u64, &[u8])) -> Vec<u8> {
    let header = Header::new(
        Sequence::new(number).unwrap(),
        [0; 32],
        Kind::Snapshot { part: 0, parts: 1 },
    )
    .unwrap();
    account.publish(now, header, state).unwrap()
}

/// Pairs `joiner` into `inviter`'s account through the four files.
fn pair(inviter: &Device, joiner: &Device) {
    let code = inviter.account().invite().unwrap();
    let join = joiner
        .account()
        .join(Code::parse(&code.text()).unwrap())
        .unwrap();
    let welcome = inviter.account().accept(&join).unwrap();
    let confirmation = joiner.account().confirm(&welcome).unwrap();
    let entry = inviter.account().admit(NOW, &confirmation, None).unwrap();
    joiner.account().finish(&entry).unwrap();
}

#[test]
fn members_publish_and_collect_each_others_bundles() {
    let mut relay = MemoryRelay::default();
    let (first, second) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    assert_eq!(first.collect(&mut relay, NOW).0, Status::NotFound);
    assert_eq!(first.publish(&mut relay, NOW, b"first").0, Status::Ok);
    pair(&first, &second);
    assert_eq!(first.publish(&mut relay, NOW, b"first again").0, Status::Ok);
    assert_eq!(second.publish(&mut relay, NOW, b"second").0, Status::Ok);
    let (status, body) = second.collect(&mut relay, NOW);
    assert_eq!(status, Status::Ok);
    let collected = Delivered::decode(&body).unwrap();
    assert_eq!(collected.chains.len(), 2);
    let from_first = collected
        .chains
        .iter()
        .find(|chain| chain.sender == first.account().device())
        .unwrap();
    let imported = second
        .account()
        .open_segment(from_first.segments.first().unwrap())
        .unwrap();
    assert_eq!(imported.body.as_slice(), b"first again");
}

#[test]
fn a_replayed_stale_or_forged_request_is_refused() {
    let mut relay = MemoryRelay::default();
    let device = Device::new();
    let mut account = device.account();
    account.found(NOW).unwrap();
    let segment = snapshot(&mut account, NOW, (1, b"s"));
    let request = account.request(NOW, Verb::Publish, segment).unwrap();
    assert_eq!(relay.request(NOW, &request).unwrap().status, Status::Ok);
    let replayed = relay.request(NOW + 1, &request).unwrap();
    assert_eq!(replayed.status, Status::Unauthorized);
    assert!(String::from_utf8_lossy(&replayed.body).contains("before"));
    let late = account.request(NOW, Verb::Collect, positions()).unwrap();
    assert_eq!(
        relay.request(NOW + WINDOW + 1, &late).unwrap().status,
        Status::Unauthorized
    );
    let mut forged =
        Request::decode(&account.request(NOW, Verb::Collect, positions()).unwrap()).unwrap();
    forged.verb = Verb::Erase;
    forged.payload.clear();
    assert_eq!(
        relay
            .request(NOW, &forged.encode().unwrap())
            .unwrap()
            .status,
        Status::Unauthorized
    );
    assert_eq!(
        relay.request(NOW, b"not a request").unwrap().status,
        Status::BadRequest
    );
    assert_eq!(relay.accounts(), 1);
}

#[test]
fn a_stranger_or_a_removed_member_reaches_nothing() {
    let mut relay = MemoryRelay::default();
    let (first, second, stranger) = (Device::new(), Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    stranger.account().found(NOW).unwrap();
    pair(&first, &second);
    assert_eq!(first.publish(&mut relay, NOW, b"a").0, Status::Ok);
    assert_eq!(second.publish(&mut relay, NOW, b"b").0, Status::Ok);
    let account = stranger.account();
    let mut request =
        Request::decode(&account.request(NOW, Verb::Collect, positions()).unwrap()).unwrap();
    request.account = Request::decode(
        &first
            .account()
            .request(NOW, Verb::Collect, positions())
            .unwrap(),
    )
    .unwrap()
    .account;
    assert_eq!(
        relay
            .request(NOW, &request.encode().unwrap())
            .unwrap()
            .status,
        Status::Forbidden
    );
    let second_id = second.account().device();
    first.account().remove(NOW, second_id).unwrap();
    assert_eq!(first.publish(&mut relay, NOW + 1, b"c").0, Status::Ok);
    assert_eq!(second.collect(&mut relay, NOW + 2).0, Status::Forbidden);
    let collected = Delivered::decode(&first.collect(&mut relay, NOW + 3).1).unwrap();
    assert!(
        collected
            .chains
            .iter()
            .all(|chain| chain.sender != second_id)
    );
}

#[test]
fn an_account_ends_on_erasure_or_after_its_retention_and_a_member_seeds_it_again() {
    let mut relay = MemoryRelay::default();
    let device = Device::new();
    device.account().found(NOW).unwrap();
    assert_eq!(device.publish(&mut relay, NOW, b"s").0, Status::Ok);
    assert_eq!(relay.accounts(), 1);
    let later = NOW + RETENTION;
    assert_eq!(device.collect(&mut relay, later).0, Status::NotFound);
    assert_eq!(relay.accounts(), 0);
    assert_eq!(device.publish(&mut relay, later, b"s").0, Status::Ok);
    let erase = device
        .account()
        .request(later, Verb::Erase, Vec::new())
        .unwrap();
    assert_eq!(relay.request(later, &erase).unwrap().status, Status::Ok);
    assert_eq!(relay.accounts(), 0);
}

/// Writes real relay messages into the seed corpus of the wire fuzz target, which `cargo nextest run --run-ignored only seeds` regenerates.
#[test]
#[ignore = "writes the committed seeds of the fuzz corpus; run it to regenerate them"]
fn seeds() {
    let mut relay = MemoryRelay::default();
    let (first, second) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    pair(&first, &second);
    let mut account = first.account();
    let key = account.enroll_recovery(NOW).unwrap();
    let member = Account::recovery_member(&key).unwrap();
    let segment = snapshot(&mut account, NOW, (1, b"{}"));
    let decoded = Segment::decode(&segment).unwrap();
    let recovery = decoded
        .operations
        .iter()
        .find(|bytes| {
            matches!(
                Operation::decode(bytes).map(|operation| operation.action),
                Ok(Action::AddRecovery(_))
            )
        })
        .unwrap()
        .clone();
    let publish = account.request(NOW, Verb::Publish, segment).unwrap();
    let collect = account.request(NOW, Verb::Collect, positions()).unwrap();
    assert_eq!(relay.request(NOW, &publish).unwrap().status, Status::Ok);
    assert_eq!(second.publish(&mut relay, NOW, b"{}").0, Status::Ok);
    let collected = relay.request(NOW, &collect).unwrap().body;
    let opened = relay.mail(NOW, &Mail::Open.encode().unwrap()).unwrap().body;
    let locate = Locate { member }.encode();
    let located = relay.locate(&locate).unwrap();
    assert_eq!(located.status, Status::Ok);
    let registration = Registration {
        account: decoded.account,
        members: vec![member],
    }
    .encode()
    .unwrap();
    let seeds: [(&str, Vec<u8>); 11] = [
        ("operation-recovery", recovery),
        ("relay-locate", locate),
        ("relay-located", located.body),
        ("relay-registration", registration),
        ("relay-publish", publish),
        ("relay-collect", collect),
        ("relay-collected", collected),
        ("mail-open", Mail::Open.encode().unwrap()),
        (
            "mail-drop",
            Mail::Drop {
                nameplate: 1,
                to: Side::Inviter,
                message: b"join".to_vec(),
            }
            .encode()
            .unwrap(),
        ),
        (
            "mail-pick",
            Mail::Pick {
                nameplate: 1,
                to: Side::Joiner,
            }
            .encode()
            .unwrap(),
        ),
        ("nameplate", opened),
    ];
    let corpus =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/wire_decode");
    for (name, bytes) in seeds {
        youarel_store::fs::replace(&corpus.join(name), &bytes).unwrap();
    }
}

#[test]
fn a_mailbox_carries_each_side_s_messages_in_order_until_it_closes() {
    let mut relay = MemoryRelay::default();
    let opened = relay.mail(NOW, &Mail::Open.encode().unwrap()).unwrap();
    assert_eq!(opened.status, Status::Ok);
    let nameplate = read_nameplate(&opened.body).unwrap();
    assert_eq!(nameplate, 1);
    let second =
        read_nameplate(&relay.mail(NOW, &Mail::Open.encode().unwrap()).unwrap().body).unwrap();
    assert_eq!(second, 2);
    let drop = |message: &[u8]| {
        Mail::Drop {
            nameplate,
            to: Side::Inviter,
            message: message.to_vec(),
        }
        .encode()
        .unwrap()
    };
    let pick = |to: Side| Mail::Pick { nameplate, to }.encode().unwrap();
    assert_eq!(relay.mail(NOW, &drop(b"one")).unwrap().status, Status::Ok);
    assert_eq!(relay.mail(NOW, &drop(b"two")).unwrap().status, Status::Ok);
    assert_eq!(
        relay.mail(NOW, &pick(Side::Joiner)).unwrap().status,
        Status::NoContent
    );
    assert_eq!(relay.mail(NOW, &pick(Side::Inviter)).unwrap().body, b"one");
    assert_eq!(relay.mail(NOW, &pick(Side::Inviter)).unwrap().body, b"two");
    let closed = NOW + MAILBOX_LIFETIME + 1;
    assert_eq!(
        relay.mail(closed, &pick(Side::Inviter)).unwrap().status,
        Status::NotFound
    );
    assert_eq!(
        read_nameplate(
            &relay
                .mail(closed, &Mail::Open.encode().unwrap())
                .unwrap()
                .body
        )
        .unwrap(),
        1
    );
}
