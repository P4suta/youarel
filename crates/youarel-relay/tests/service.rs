//! The relay against accounts built from signed operations: what it admits, keeps, hands out, and refuses.
#![cfg(test)]

use youarel_core::relay_rules::{
    MAILBOX_LIFETIME, MOST_BYTES, MOST_MAILBOXES, MOST_SLOTS, RETENTION, WINDOW,
};
use youarel_crypto::Label;
use youarel_crypto::kem::KeyPair;
use youarel_crypto::seal::Sealed;
use youarel_crypto::sign::SigningKeys;
use youarel_protocol::message::{
    Action, Bundle, DevicePublic, Digest, Operation, Recovery, account_of, operation_digest,
};
use youarel_protocol::relay::{
    Collected, Locate, Mail, Registration, Request, Side, Verb, read_located, read_nameplate,
};
use youarel_relay::Status;
use youarel_relay::directory::{self, DirectoryStore as _};
use youarel_relay::memory::{MemoryDirectory, MemoryRelay};

const NOW: u64 = 1_700_000_000_000;

/// A device: its keys and public keys.
struct Device {
    keys: SigningKeys,
    public: DevicePublic,
}

impl Device {
    fn new() -> Self {
        let keys = SigningKeys::generate().unwrap();
        let kem = KeyPair::generate().unwrap();
        let public = DevicePublic {
            signing: keys.public().clone(),
            kem: Box::new(*kem.public()),
        };
        Self { keys, public }
    }

    fn id(&self) -> Digest {
        self.public.id()
    }

    /// An operation of this device in `account` after `parents`.
    fn operation(&self, account: Digest, parents: Vec<Digest>, action: Action) -> Vec<u8> {
        let mut operation = Operation {
            account,
            parents,
            author: self.id(),
            wall: NOW,
            action,
            signature: self.keys.sign(Label::Operation, b"").unwrap(),
        };
        operation.signature = self
            .keys
            .sign(Label::Operation, &operation.unsigned().bytes().unwrap())
            .unwrap();
        operation.encode().unwrap()
    }

    /// This device's bundle of `log`, with a state of `size` bytes.
    fn bundle(&self, account: Digest, log: &[Vec<u8>], size: usize) -> Vec<u8> {
        let mut operations = log.to_vec();
        operations.sort_by_key(|bytes| operation_digest(bytes));
        let mut bundle = Bundle {
            account,
            operations,
            sender: self.id(),
            epoch: 0,
            sealed: Sealed {
                commitment: [1; 32],
                nonce: [2; 24],
                ciphertext: vec![3; size],
            },
            signature: self.keys.sign(Label::Bundle, b"").unwrap(),
        };
        bundle.signature = self
            .keys
            .sign(Label::Bundle, &bundle.unsigned().bytes().unwrap())
            .unwrap();
        bundle.encode().unwrap()
    }

    /// This device's request, stamped `stamped`, with a nonce of `nonce`.
    fn request(
        &self,
        account: Digest,
        (verb, payload): (Verb, Vec<u8>),
        (stamped, nonce): (u64, u8),
    ) -> Vec<u8> {
        let mut request = Request {
            account,
            device: self.id(),
            verb,
            stamped,
            nonce: [nonce; 16],
            payload,
            signature: self.keys.sign(Label::Request, b"").unwrap(),
        };
        request.signature = self
            .keys
            .sign(Label::Request, &request.unsigned().bytes().unwrap())
            .unwrap();
        request.encode().unwrap()
    }
}

/// An account that `founder` founded, with its log.
struct Account {
    id: Digest,
    genesis: Digest,
    log: Vec<Vec<u8>>,
}

impl Account {
    fn found(founder: &Device) -> Self {
        let genesis =
            founder.operation([0; 32], Vec::new(), Action::Genesis(founder.public.clone()));
        Self {
            id: account_of(&genesis),
            genesis: operation_digest(&genesis),
            log: vec![genesis],
        }
    }

    fn add(&mut self, author: &Device, device: &Device) {
        let operation = author.operation(
            self.id,
            vec![self.genesis],
            Action::Add(device.public.clone()),
        );
        self.log.push(operation);
    }

    fn remove(&mut self, author: &Device, device: &Device) {
        let heads: Vec<Digest> = {
            let mut digests: Vec<Digest> = self
                .log
                .iter()
                .map(|bytes| operation_digest(bytes))
                .collect();
            digests.sort_unstable();
            digests
        };
        let operation = author.operation(self.id, heads, Action::Remove(device.id()));
        self.log.push(operation);
    }

    /// Publishes `device`'s bundle with a state of `size` bytes.
    fn publish(
        &self,
        relay: &mut MemoryRelay,
        (device, size): (&Device, usize),
        (now, nonce): (u64, u8),
    ) -> Status {
        let bundle = device.bundle(self.id, &self.log, size);
        let request = device.request(self.id, (Verb::Publish, bundle), (now, nonce));
        relay.request(now, &request).unwrap().status
    }

    fn collect(
        &self,
        relay: &mut MemoryRelay,
        device: &Device,
        (now, nonce): (u64, u8),
    ) -> (Status, Vec<u8>) {
        let request = device.request(self.id, (Verb::Collect, Vec::new()), (now, nonce));
        let reply = relay.request(now, &request).unwrap();
        (reply.status, reply.body)
    }
}

#[test]
fn members_publish_and_collect_each_others_newest_bundle() {
    let mut relay = MemoryRelay::default();
    let (first, second) = (Device::new(), Device::new());
    let mut account = Account::found(&first);
    assert_eq!(
        account.collect(&mut relay, &first, (NOW, 1)).0,
        Status::NotFound
    );
    assert_eq!(
        account.publish(&mut relay, (&first, 4), (NOW, 2)),
        Status::Ok
    );
    account.add(&first, &second);
    assert_eq!(
        account.publish(&mut relay, (&second, 5), (NOW, 3)),
        Status::Ok
    );
    assert_eq!(
        account.publish(&mut relay, (&first, 6), (NOW, 4)),
        Status::Ok
    );
    let (status, body) = account.collect(&mut relay, &second, (NOW, 5));
    assert_eq!(status, Status::Ok);
    let collected = Collected::decode(&body).unwrap();
    let mut devices: Vec<Digest> = vec![first.id(), second.id()];
    devices.sort_unstable();
    assert_eq!(
        collected
            .slots
            .iter()
            .map(|slot| slot.device)
            .collect::<Vec<_>>(),
        devices
    );
    let newest = collected
        .slots
        .iter()
        .find(|slot| slot.device == first.id())
        .unwrap();
    assert_eq!(
        Bundle::decode(&newest.bundle)
            .unwrap()
            .sealed
            .ciphertext
            .len(),
        6
    );
    assert_eq!(newest.published, NOW);
}

#[test]
fn the_directory_locates_a_recovery_member_once_a_member_publishes() {
    let mut relay = MemoryRelay::default();
    let (first, recovery, plain) = (Device::new(), Device::new(), Device::new());
    let mut account = Account::found(&first);
    let consent = recovery
        .keys
        .sign(Label::RecoveryConsent, &account.id)
        .unwrap();
    account.log.push(first.operation(
        account.id,
        vec![account.genesis],
        Action::AddRecovery(Recovery {
            device: recovery.public.clone(),
            consent,
        }),
    ));
    account.add(&first, &plain);
    let locate =
        |served: &MemoryRelay, member: Digest| served.locate(&Locate { member }.encode()).unwrap();
    assert_eq!(locate(&relay, recovery.id()).status, Status::NotFound);
    assert_eq!(
        account.publish(&mut relay, (&first, 4), (NOW, 1)),
        Status::Ok
    );
    let found = locate(&relay, recovery.id());
    assert_eq!(found.status, Status::Ok);
    assert_eq!(read_located(&found.body), Ok(account.id));
    assert_eq!(
        locate(&relay, plain.id()).status,
        Status::NotFound,
        "the directory lists only the members that consented as recovery members"
    );
    assert_eq!(relay.locate(b"YRF1").unwrap().status, Status::BadRequest);
    let (status, _) = account.collect(&mut relay, &recovery, (NOW, 2));
    assert_eq!(
        status,
        Status::Ok,
        "a recovery member collects as any member"
    );
}

#[test]
fn the_directory_notes_only_a_well_formed_registration() {
    let mut store = MemoryDirectory::default();
    let registration = Registration {
        account: [1; 32],
        members: vec![[2; 32]],
    };
    let noted = directory::register(&mut store, &registration.encode().unwrap()).unwrap();
    assert_eq!(noted.status, Status::Ok);
    assert_eq!(store.account_of(&[2; 32]).unwrap(), Some([1; 32]));
    let refused = directory::register(&mut store, b"YRG1").unwrap();
    assert_eq!(refused.status, Status::BadRequest);
}

#[test]
fn the_relay_refuses_what_it_must() {
    let mut relay = MemoryRelay::default();
    let (first, stranger) = (Device::new(), Device::new());
    let account = Account::found(&first);
    let other = Account::found(&stranger);
    assert_eq!(
        account.publish(&mut relay, (&first, 4), (NOW, 1)),
        Status::Ok
    );
    let repeated = first.request(account.id, (Verb::Collect, Vec::new()), (NOW, 9));
    assert_eq!(relay.request(NOW, &repeated).unwrap().status, Status::Ok);
    assert_eq!(
        relay.request(NOW, &repeated).unwrap().status,
        Status::Unauthorized
    );
    let stale = first.request(
        account.id,
        (Verb::Collect, Vec::new()),
        (NOW - WINDOW - 1, 10),
    );
    assert_eq!(
        relay.request(NOW, &stale).unwrap().status,
        Status::Unauthorized
    );
    let ahead = first.request(account.id, (Verb::Collect, Vec::new()), (NOW + WINDOW, 11));
    assert_eq!(relay.request(NOW, &ahead).unwrap().status, Status::Ok);
    let strange = stranger.request(account.id, (Verb::Collect, Vec::new()), (NOW, 12));
    assert_eq!(
        relay.request(NOW, &strange).unwrap().status,
        Status::Forbidden
    );
    let mut forged =
        Request::decode(&first.request(account.id, (Verb::Collect, Vec::new()), (NOW, 13)))
            .unwrap();
    forged.verb = Verb::Erase;
    assert_eq!(
        relay
            .request(NOW, &forged.encode().unwrap())
            .unwrap()
            .status,
        Status::Unauthorized
    );
    let not_a_bundle = first.request(account.id, (Verb::Publish, b"x".to_vec()), (NOW, 14));
    assert_eq!(
        relay.request(NOW, &not_a_bundle).unwrap().status,
        Status::BadRequest
    );
    let elsewhere = first.request(
        account.id,
        (Verb::Publish, stranger.bundle(other.id, &other.log, 1)),
        (NOW, 15),
    );
    assert_eq!(
        relay.request(NOW, &elsewhere).unwrap().status,
        Status::BadRequest
    );
    let mut tampered = Bundle::decode(&first.bundle(account.id, &account.log, 4)).unwrap();
    tampered.epoch = 7;
    let unsigned = first.request(
        account.id,
        (Verb::Publish, tampered.encode().unwrap()),
        (NOW, 16),
    );
    assert_eq!(
        relay.request(NOW, &unsigned).unwrap().status,
        Status::Forbidden
    );
    assert_eq!(
        relay.request(NOW, b"no request").unwrap().status,
        Status::BadRequest
    );
    let unknown = stranger.request(other.id, (Verb::Collect, Vec::new()), (NOW, 17));
    assert_eq!(
        relay.request(NOW, &unknown).unwrap().status,
        Status::NotFound
    );
    assert_eq!(relay.accounts(), 1);
}

#[test]
fn a_removed_member_loses_its_slot_and_its_access() {
    let mut relay = MemoryRelay::default();
    let (first, second) = (Device::new(), Device::new());
    let mut account = Account::found(&first);
    account.add(&first, &second);
    assert_eq!(
        account.publish(&mut relay, (&first, 4), (NOW, 1)),
        Status::Ok
    );
    assert_eq!(
        account.publish(&mut relay, (&second, 4), (NOW, 2)),
        Status::Ok
    );
    account.remove(&first, &second);
    assert_eq!(
        account.publish(&mut relay, (&first, 4), (NOW, 3)),
        Status::Ok
    );
    assert_eq!(
        account.collect(&mut relay, &second, (NOW, 4)).0,
        Status::Forbidden
    );
    let collected = Collected::decode(&account.collect(&mut relay, &first, (NOW, 5)).1).unwrap();
    assert_eq!(
        collected
            .slots
            .iter()
            .map(|slot| slot.device)
            .collect::<Vec<_>>(),
        [first.id()]
    );
}

#[test]
fn an_account_ends_on_erasure_or_after_its_retention_and_returns_with_a_bundle() {
    let mut relay = MemoryRelay::default();
    let device = Device::new();
    let account = Account::found(&device);
    assert_eq!(
        account.publish(&mut relay, (&device, 4), (NOW, 1)),
        Status::Ok
    );
    let kept = NOW + RETENTION - 1;
    assert_eq!(
        account.collect(&mut relay, &device, (kept, 2)).0,
        Status::Ok
    );
    let gone = kept + RETENTION;
    assert_eq!(
        account.collect(&mut relay, &device, (gone, 3)).0,
        Status::NotFound
    );
    assert_eq!(relay.accounts(), 0);
    assert_eq!(
        account.publish(&mut relay, (&device, 4), (gone, 4)),
        Status::Ok
    );
    let erase = device.request(account.id, (Verb::Erase, Vec::new()), (gone, 5));
    assert_eq!(relay.request(gone, &erase).unwrap().status, Status::Ok);
    assert_eq!(relay.accounts(), 0);
    assert_eq!(
        account.collect(&mut relay, &device, (gone, 6)).0,
        Status::NotFound
    );
}

#[test]
fn an_account_holds_at_most_its_slots() {
    let mut relay = MemoryRelay::default();
    let founder = Device::new();
    let mut account = Account::found(&founder);
    let members: Vec<Device> = (0..MOST_SLOTS).map(|_| Device::new()).collect();
    for member in &members {
        account.add(&founder, member);
    }
    for (index, member) in members.iter().enumerate().take(MOST_SLOTS - 1) {
        let nonce = u8::try_from(index).unwrap();
        assert_eq!(
            account.publish(&mut relay, (member, 4), (NOW, nonce)),
            Status::Ok
        );
    }
    assert_eq!(
        account.publish(&mut relay, (&founder, 4), (NOW, 200)),
        Status::Ok
    );
    let last = members.last().unwrap();
    assert_eq!(
        account.publish(&mut relay, (last, 4), (NOW, 201)),
        Status::TooLarge
    );
    assert_eq!(
        account.publish(&mut relay, (&founder, 5), (NOW, 202)),
        Status::Ok
    );
}

#[test]
fn an_account_holds_at_most_its_bytes() {
    let mut relay = MemoryRelay::default();
    let founder = Device::new();
    let mut account = Account::found(&founder);
    let members: Vec<Device> = (0..4).map(|_| Device::new()).collect();
    for member in &members {
        account.add(&founder, member);
    }
    // Five such bundles pass the quota, and each fits within a bundle's longest state.
    let large = MOST_BYTES / 5 + 1;
    assert_eq!(
        account.publish(&mut relay, (&founder, large), (NOW, 1)),
        Status::Ok
    );
    for (member, nonce) in members.iter().take(3).zip(2..) {
        assert_eq!(
            account.publish(&mut relay, (member, large), (NOW, nonce)),
            Status::Ok
        );
    }
    let fifth = members.get(3).unwrap();
    assert_eq!(
        account.publish(&mut relay, (fifth, large), (NOW, 10)),
        Status::TooLarge
    );
    assert_eq!(
        account.publish(&mut relay, (fifth, 4), (NOW, 11)),
        Status::Ok
    );
    assert_eq!(
        account.publish(&mut relay, (&founder, large), (NOW, 12)),
        Status::Ok
    );
}

#[test]
fn a_mailbox_carries_each_side_s_messages_in_order_until_it_closes() {
    let mut relay = MemoryRelay::default();
    let open = || Mail::Open.encode().unwrap();
    let opened = relay.mail(NOW, &open()).unwrap();
    assert_eq!(opened.status, Status::Ok);
    let nameplate = read_nameplate(&opened.body).unwrap();
    assert_eq!(nameplate, 1);
    assert_eq!(
        read_nameplate(&relay.mail(NOW, &open()).unwrap().body).unwrap(),
        2
    );
    let drop = |to: Side, message: &[u8]| {
        Mail::Drop {
            nameplate,
            to,
            message: message.to_vec(),
        }
        .encode()
        .unwrap()
    };
    let pick = |to: Side| Mail::Pick { nameplate, to }.encode().unwrap();
    assert_eq!(
        relay
            .mail(NOW, &drop(Side::Inviter, b"one"))
            .unwrap()
            .status,
        Status::Ok
    );
    assert_eq!(
        relay
            .mail(NOW, &drop(Side::Inviter, b"two"))
            .unwrap()
            .status,
        Status::Ok
    );
    assert_eq!(
        relay
            .mail(NOW, &drop(Side::Joiner, b"three"))
            .unwrap()
            .status,
        Status::Ok
    );
    assert_eq!(relay.mail(NOW, &pick(Side::Joiner)).unwrap().body, b"three");
    assert_eq!(
        relay.mail(NOW, &pick(Side::Joiner)).unwrap().status,
        Status::NoContent
    );
    assert_eq!(relay.mail(NOW, &pick(Side::Inviter)).unwrap().body, b"one");
    assert_eq!(relay.mail(NOW, &pick(Side::Inviter)).unwrap().body, b"two");
    let at_close = NOW + MAILBOX_LIFETIME;
    assert_eq!(
        relay.mail(at_close, &pick(Side::Inviter)).unwrap().status,
        Status::NoContent
    );
    let closed = at_close + 1;
    assert_eq!(
        relay.mail(closed, &pick(Side::Inviter)).unwrap().status,
        Status::NotFound
    );
    assert_eq!(
        read_nameplate(&relay.mail(closed, &open()).unwrap().body).unwrap(),
        1
    );
}

#[test]
fn mail_for_no_mailbox_or_no_mail_at_all_is_refused() {
    let mut relay = MemoryRelay::default();
    let missing = Mail::Pick {
        nameplate: 9,
        to: Side::Inviter,
    }
    .encode()
    .unwrap();
    assert_eq!(relay.mail(NOW, &missing).unwrap().status, Status::NotFound);
    let lost = Mail::Drop {
        nameplate: 9,
        to: Side::Inviter,
        message: Vec::new(),
    }
    .encode()
    .unwrap();
    assert_eq!(relay.mail(NOW, &lost).unwrap().status, Status::NotFound);
    assert_eq!(
        relay.mail(NOW, b"no mail").unwrap().status,
        Status::BadRequest
    );
}

#[test]
fn the_relay_keeps_as_many_mailboxes_and_bytes_as_it_allows() {
    let mut relay = MemoryRelay::default();
    let open = Mail::Open.encode().unwrap();
    for _ in 0..MOST_MAILBOXES {
        assert_eq!(relay.mail(NOW, &open).unwrap().status, Status::Ok);
    }
    assert_eq!(relay.mail(NOW, &open).unwrap().status, Status::Busy);
    let half = youarel_core::relay_rules::MOST_MAIL_BYTES / 2;
    let drop = |size: usize| {
        Mail::Drop {
            nameplate: 1,
            to: Side::Joiner,
            message: vec![0; size],
        }
        .encode()
        .unwrap()
    };
    assert_eq!(relay.mail(NOW, &drop(half)).unwrap().status, Status::Ok);
    assert_eq!(relay.mail(NOW, &drop(half)).unwrap().status, Status::Ok);
    assert_eq!(relay.mail(NOW, &drop(1)).unwrap().status, Status::TooLarge);
}
