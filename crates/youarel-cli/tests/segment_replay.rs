//! Replays `specs/segments.qnt` traces through `segment_rules::step` and exchanges segments among real stores.
#![cfg(test)]

#[path = "segment_replay/folder.rs"]
mod folder;
#[path = "segment_replay/model.rs"]
mod model;
#[path = "segment_replay/relay.rs"]
mod relay;

use anyhow::{Result, bail, ensure};
use std::collections::BTreeMap;
use youarel_cli::sync::{ACCOUNT_DIRECTORY, segments};
use youarel_core::segment_rules::{Sequence, Verdict};
use youarel_core::state::{Edit, Entry, ItemId, Presence, State};
use youarel_protocol::message::Digest;
use youarel_protocol::segment::{Chain, Head};
use youarel_store::{PublishRequest, Store};
use youarel_sync::account::Account;

const NOW: u64 = 1_700_000_000_000;
const FIRST: ItemId = ItemId([1; 16]);
const TARGET: &str = "segment-replay";
const DEVICES: usize = 3;
const ROUNDS: u64 = 18;
const SETTLE_ROUNDS: usize = 4;
const SEED: u64 = 0x5e6_002a;
const MULTIPLIER: u64 = 6_364_136_223_846_793_005;
const INCREMENT: u64 = 1_442_695_040_888_963_407;

#[test]
fn authenticated_malformed_records_never_advance_the_store_cursor() {
    use youarel_core::segment_rules::{Header, Kind};
    let (first, second) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    pair(&first, &second);
    let mut account = first.account();
    let sender = account.device();
    let head = Head::new(Sequence::FIRST, Sequence::FIRST, Some(Sequence::FIRST)).unwrap();
    let header = Header::new(
        Sequence::FIRST,
        [0; 32],
        Kind::Snapshot { part: 0, parts: 1 },
    )
    .unwrap();
    let json = br#"{"format": 1, "shortcuts": [], "folders": []}"#;
    for body in [b"\xffnot records".as_slice(), json.as_slice()] {
        let bytes = account.publish(NOW, header, body).unwrap();
        segments::apply(
            &second.store,
            NOW,
            &mut second.account(),
            segments::Delivery {
                sender,
                head,
                bytes: &bytes,
            },
        )
        .unwrap_err();
        assert_eq!(
            second
                .store
                .session(NOW, |session| session.peer_cursor(&sender))
                .unwrap(),
            None
        );
        assert_eq!(second.state(), State::default());
    }
    first.edit(NOW, added(FIRST));
    let mut target = Target::default();
    segments::publish(&first.store, NOW, &mut target).unwrap();
    deliver(&second, &target.chain(&first), false);
    assert_eq!(second.state(), first.state());
}

struct Device {
    home: tempfile::TempDir,
    store: Store,
}

impl Device {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_owned());
        Self { home, store }
    }

    fn account(&self) -> Account {
        Account::open(&self.home.path().join(ACCOUNT_DIRECTORY)).unwrap()
    }

    fn state(&self) -> State {
        self.store
            .session(NOW, |session| Ok(session.records()?.into_state()))
            .unwrap()
    }

    fn edit(&self, now: u64, edit: Edit) {
        self.store
            .session(now, |session| session.edit(edit))
            .unwrap();
    }
}

fn pair(first: &Device, second: &Device) {
    let mut inviter = first.account();
    let mut joiner = second.account();
    let code = inviter.invite().unwrap();
    let join = joiner.join(code).unwrap();
    let welcome = inviter.accept(&join).unwrap();
    let confirmed = joiner.confirm(&welcome).unwrap();
    let entry = inviter.admit(NOW, &confirmed, None).unwrap();
    joiner.finish(&entry).unwrap();
}

#[derive(Clone, Debug, Default)]
struct Target {
    head: Option<Head>,
    segments: BTreeMap<Sequence, Vec<u8>>,
    fail_finish: bool,
}

impl segments::SegmentTarget for Target {
    fn identity(&self) -> &str {
        TARGET
    }

    fn head(&mut self, _sender: &Digest) -> Result<Option<Head>> {
        Ok(self.head)
    }

    fn write(&mut self, _sender: &Digest, sequence: Sequence, bytes: &[u8]) -> Result<()> {
        ensure!(
            self.segments
                .get(&sequence)
                .is_none_or(|held| held == bytes),
            "an immutable segment changed"
        );
        self.segments
            .entry(sequence)
            .or_insert_with(|| bytes.to_vec());
        Ok(())
    }

    fn finish(&mut self, _sender: &Digest, head: Head) -> Result<()> {
        self.head = Some(head);
        self.segments
            .retain(|sequence, _| *sequence >= head.first());
        if self.fail_finish {
            self.fail_finish = false;
            bail!("injected lost head acknowledgment");
        }
        Ok(())
    }
}

impl Target {
    fn chain(&self, device: &Device) -> Chain {
        Chain {
            incomplete: false,
            published: 0,
            sender: device.account().device(),
            head: self.head.unwrap(),
            segments: self.segments.values().cloned().collect(),
        }
    }
}

fn deliver(device: &Device, chain: &Chain, lose: bool) {
    let mut account = device.account();
    if account.device() == chain.sender {
        return;
    }
    for (index, bytes) in chain.segments.iter().enumerate() {
        if lose && index.is_multiple_of(2) {
            continue;
        }
        let decision = segments::apply(
            &device.store,
            NOW,
            &mut account,
            segments::Delivery {
                sender: chain.sender,
                head: chain.head,
                bytes,
            },
        )
        .unwrap();
        assert_ne!(decision.verdict, Verdict::Forked);
    }
}

fn added(id: ItemId) -> Edit {
    Edit::AddShortcut {
        id,
        link: format!("https://example.com/{}", id.0[0]),
        title: "First".to_owned(),
        folder: None,
    }
}

#[test]
fn a_lost_acknowledgment_retries_the_exact_envelope_and_keeps_a_later_edit() {
    let (first, second) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    pair(&first, &second);
    first.edit(NOW, added(FIRST));
    let mut target = Target {
        fail_finish: true,
        ..Target::default()
    };
    segments::publish(&first.store, NOW, &mut target).unwrap_err();
    let original = target.segments.clone();
    deliver(&second, &target.chain(&first), false);
    first.edit(
        NOW + 1,
        Edit::Retitle {
            id: FIRST,
            title: "After crash".to_owned(),
        },
    );
    segments::publish(&first.store, NOW + 2, &mut target).unwrap();
    assert_eq!(target.segments, original);
    deliver(&second, &target.chain(&first), false);
    segments::publish(&first.store, NOW + 3, &mut target).unwrap();
    deliver(&second, &target.chain(&first), false);
    assert_eq!(second.state(), first.state());
    assert_eq!(
        segments::publish(&first.store, NOW + 4, &mut target).unwrap(),
        None
    );
}

struct Schedule(u64);

impl Schedule {
    fn pick(&mut self, choices: usize) -> usize {
        self.0 = self.0.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT);
        usize::try_from(self.0.checked_rem(u64::try_from(choices).unwrap()).unwrap()).unwrap()
    }
}

fn change(device: &Device, round: u64, choice: usize) -> Edit {
    if choice == 0 {
        return added(ItemId(
            [u8::try_from(round).unwrap().checked_add(2).unwrap(); 16],
        ));
    }
    let state = device.state();
    let presence = state.shortcuts.get(&FIRST).unwrap().presence.value;
    if presence == Presence::Trashed {
        return Edit::PutBack {
            entry: Entry::Shortcut(FIRST),
        };
    }
    match choice {
        1 => Edit::Retitle {
            id: FIRST,
            title: format!("Title {round}"),
        },
        2 => Edit::SetNote {
            id: FIRST,
            note: format!("Note {round}"),
        },
        3 => Edit::Trash {
            entry: Entry::Shortcut(FIRST),
        },
        _ => Edit::Realias {
            id: FIRST,
            alias: format!("Alias {round}"),
        },
    }
}

#[test]
fn a_seeded_exchange_among_three_stores_converges_to_the_reference_join() {
    let devices = [Device::new(), Device::new(), Device::new()];
    devices[0].account().found(NOW).unwrap();
    pair(&devices[0], &devices[1]);
    pair(&devices[0], &devices[2]);
    devices[0].edit(NOW, added(FIRST));
    let mut targets: [Target; DEVICES] = core::array::from_fn(|_| Target::default());
    segments::publish(&devices[0].store, NOW, &mut targets[0]).unwrap();
    let initial = targets[0].chain(&devices[0]);
    for device in &devices {
        deliver(device, &initial, false);
    }
    let mut history = vec![initial];
    let mut reference = devices[0].state();
    let mut schedule = Schedule(SEED);
    for round in 1..=ROUNDS {
        let actor = schedule.pick(DEVICES);
        let edit = change(&devices[actor], round, schedule.pick(5));
        devices[actor].edit(NOW + round, edit);
        reference = reference.merge(&devices[actor].state());
        segments::publish(&devices[actor].store, NOW + round, &mut targets[actor]).unwrap();
        history.push(targets[actor].chain(&devices[actor]));
        let delayed = schedule.pick(history.len());
        let reader = schedule.pick(DEVICES);
        deliver(&devices[reader], &history[delayed], round.is_multiple_of(2));
    }
    for _ in 0..SETTLE_ROUNDS {
        for (device, target) in devices.iter().zip(&mut targets) {
            if segments::publish(&device.store, NOW + ROUNDS, target)
                .unwrap()
                .is_some()
            {
                history.push(target.chain(device));
            }
        }
        for chain in &history {
            for device in &devices {
                deliver(device, chain, false);
            }
        }
    }
    for device in &devices {
        assert_eq!(device.state(), reference);
        assert!(
            !device
                .store
                .session(NOW, |session| session.publication_due(PublishRequest::new(
                    TARGET,
                    device.account().operations()
                )))
                .unwrap()
        );
    }
}

#[test]
fn a_target_with_a_newer_own_chain_refuses_before_writing() {
    let device = Device::new();
    device.account().found(NOW).unwrap();
    let latest = Sequence::new(2).unwrap();
    let mut target = Target {
        head: Head::new(Sequence::FIRST, latest, Some(Sequence::FIRST)),
        ..Target::default()
    };
    let error = segments::publish(&device.store, NOW, &mut target).unwrap_err();
    assert!(error.to_string().contains("newer chain"));
    assert_eq!(target.segments.len(), 0);
    assert_eq!(
        device
            .store
            .session(NOW, |session| session.pending_publish())
            .unwrap(),
        None
    );
}

#[test]
fn a_rotation_for_a_new_member_is_acknowledged_by_the_same_publication() {
    let (first, second) = (Device::new(), Device::new());
    first.account().found(NOW).unwrap();
    pair(&first, &second);
    let mut target = Target::default();
    segments::publish(&first.store, NOW, &mut target)
        .unwrap()
        .unwrap();
    assert_eq!(
        segments::publish(&first.store, NOW, &mut target).unwrap(),
        None
    );
}

fn parser_segment(
    sequence: Sequence,
    kind: youarel_core::segment_rules::Kind,
) -> youarel_protocol::segment::Segment {
    use youarel_crypto::seal::{self, Sealed};
    use youarel_crypto::sign::{ED25519_SIGNATURE, ML_DSA_SIGNATURE, Signature};
    youarel_protocol::segment::Segment {
        account: [1; 32],
        operations: Vec::new(),
        sender: [2; 32],
        epoch: 1,
        header: youarel_core::segment_rules::Header::new(sequence, [0; 32], kind).unwrap(),
        sealed: Sealed {
            commitment: [0; 32],
            nonce: [0; seal::NONCE],
            ciphertext: vec![0; seal::TAG],
        },
        signature: Signature {
            ed25519: [0; ED25519_SIGNATURE],
            ml_dsa: Box::new([0; ML_DSA_SIGNATURE]),
        },
    }
}

#[test]
#[ignore = "writes the committed seeds of the wire fuzz corpus"]
fn seeds() {
    use youarel_core::segment_rules::{Cursor, Kind};
    use youarel_protocol::segment::{Cursors, Delivered};
    let delta = parser_segment(Sequence::FIRST, Kind::Delta);
    let first = parser_segment(Sequence::FIRST, Kind::Snapshot { part: 0, parts: 2 });
    let last = parser_segment(
        Sequence::new(2).unwrap(),
        Kind::Snapshot { part: 1, parts: 2 },
    );
    let head = Head::new(
        Sequence::FIRST,
        last.header.sequence(),
        Some(Sequence::FIRST),
    )
    .unwrap();
    let cursors = Cursors {
        peers: BTreeMap::from([(first.sender, Cursor::new(last.header.sequence(), [3; 32]))]),
    };
    let delivered = Delivered {
        account: first.account,
        operations: Vec::new(),
        chains: vec![Chain {
            incomplete: false,
            published: 0,
            sender: first.sender,
            head,
            segments: vec![first.encode().unwrap(), last.encode().unwrap()],
        }],
        more: false,
    };
    let seeds = [
        (
            "mail-poll",
            youarel_protocol::relay::Mail::Poll {
                nameplate: 7,
                to: youarel_protocol::relay::Side::Joiner,
            }
            .encode()
            .unwrap(),
        ),
        ("segment-delta", delta.encode().unwrap()),
        ("segment-snapshot-first", first.encode().unwrap()),
        ("segment-snapshot-last", last.encode().unwrap()),
        ("segment-cursors", cursors.encode().unwrap()),
        ("segment-head", head.encode()),
        ("segment-delivered", delivered.encode().unwrap()),
    ];
    let corpus =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/wire_decode");
    for (name, bytes) in seeds {
        youarel_store::fs::replace(&corpus.join(name), &bytes).unwrap();
    }
}

#[test]
#[ignore = "writes the committed admission parser seeds"]
fn admission_seeds() {
    use youarel_protocol::message::{Admission, SnapshotHint};
    let corpus =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/wire_decode");
    for (name, hint) in [
        ("admission-without-state", None),
        (
            "admission-with-hint",
            Some(SnapshotHint {
                sender: [3; 32],
                sequence: Sequence::FIRST,
            }),
        ),
    ] {
        let bytes = Admission {
            account: [1; 32],
            operations: Vec::new(),
            epochs: vec![(1, [2; 32])],
            hint,
        }
        .encode()
        .unwrap();
        youarel_store::fs::replace(&corpus.join(name), &bytes).unwrap();
    }
}
