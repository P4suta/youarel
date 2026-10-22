//! Dirty record selection and durable publication reservations at real database boundaries.
#![cfg(test)]

use youarel_core::clock_rules::{DeviceId, Stamp};
use youarel_core::segment_rules::{Available, DIGEST_BYTES, Header, Kind, Sequence, Verdict};
use youarel_core::state::{Edit, Entry, ItemId, State};
use youarel_store::{
    PendingPublish, PublishRequest, ReceivedSegment, Record, RecordSet, Store, StoreError,
};

const NOW: u64 = 1_700_000_000_000;
const FIRST: ItemId = ItemId([1; 16]);
const REMOTE: ItemId = ItemId([2; 16]);
const FOLDER: ItemId = ItemId([3; 16]);

fn digest(bytes: &[u8]) -> [u8; DIGEST_BYTES] {
    let mut digest = [0_u8; DIGEST_BYTES];
    for (index, byte) in bytes.iter().copied().enumerate() {
        let place = index % DIGEST_BYTES;
        digest[place] = digest[place].wrapping_add(byte);
    }
    digest
}

fn request(target: &str) -> PublishRequest<'_> {
    let mut request = PublishRequest::new(target, 2);
    request.limit = 200;
    request
}

#[test]
fn sealed_device_names_publish_in_snapshots_and_deltas_without_echoing_a_peer() {
    use youarel_core::device_rules::MemberId;
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    let source = Store::new(left.path().to_owned());
    let peer = Store::new(right.path().to_owned());
    let id = MemberId([9; 32]);
    source
        .session(NOW, |session| {
            session.edit(Edit::NameDevice {
                id,
                name: "Desktop".to_owned(),
            })
        })
        .unwrap();
    let snapshot = prepared(&source, "names");
    transfer_names(&source, &peer, &snapshot);
    assert_eq!(
        peer.session(NOW, |session| session.device_name(id))
            .unwrap()
            .unwrap()
            .value,
        "Desktop"
    );
    assert_eq!(
        records(&peer, RecordSet::Delta { watermark: None }),
        Vec::<Record>::new()
    );
    source
        .session(NOW, |session| {
            session.edit(Edit::NameDevice {
                id,
                name: "仕事の Mac".to_owned(),
            })
        })
        .unwrap();
    let delta = prepared(&source, "names");
    assert!(!delta.is_snapshot());
    assert_eq!(
        records(&source, RecordSet::Delta { watermark: None }).len(),
        1
    );
    transfer_names(&source, &peer, &delta);
    assert_eq!(
        source
            .session(NOW, |session| session.device_names())
            .unwrap(),
        peer.session(NOW, |session| session.device_names()).unwrap()
    );
    assert_eq!(
        peer.session(NOW, |session| session.latest_of(session.device()))
            .unwrap(),
        None
    );
}

fn transfer_names(source: &Store, peer: &Store, pending: &PendingPublish) {
    for sequence in pending.sequences() {
        let part = source
            .session(NOW, |session| session.publication_part(pending, sequence))
            .unwrap();
        let decision = peer
            .session(NOW, |session| {
                session.receive_segment(
                    ReceivedSegment {
                        sender: [9; 32],
                        header: part.header,
                        available: Available {
                            first: Sequence::FIRST,
                            snapshot: Some(Sequence::FIRST),
                        },
                        body: &part.body,
                    },
                    digest,
                )
            })
            .unwrap();
        assert_eq!(decision.verdict, Verdict::Apply);
    }
    seal(source, pending);
    source
        .session(NOW, |session| session.commit_publication(pending))
        .unwrap();
}

fn prepared(store: &Store, target: &str) -> PendingPublish {
    store
        .session(NOW, |session| {
            session.prepare_publication(request(target), digest)
        })
        .unwrap()
        .unwrap()
}

fn seal(store: &Store, pending: &PendingPublish) {
    for sequence in pending.sequences() {
        store
            .session(NOW, |session| {
                let part = session.publication_part(pending, sequence)?;
                if let Some(sealed) = part.sealed {
                    assert_eq!(sealed, part.digest);
                } else {
                    session.seal_publication_part(pending, sequence, &part.digest)?;
                }
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn cancelling_snapshot_staging_rolls_back_every_reserved_part_and_keeps_the_records() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    store
        .session(NOW, |session| {
            for index in 1..=8 {
                session.edit(added(ItemId([index; 16]), None))?;
            }
            Ok(())
        })
        .unwrap();
    let before = records(&store, RecordSet::Snapshot);
    let mut checks = 0usize;
    let stopped = store.session(NOW, |session| {
        session.prepare_publication_checked(request("cancelled"), digest, || {
            checks = checks.saturating_add(1);
            if checks == 3 {
                Err(youarel_store::PublicationError::Interrupted)
            } else {
                Ok(())
            }
        })
    });
    assert!(matches!(
        stopped,
        Err(StoreError::Publication(
            youarel_store::PublicationError::Interrupted
        ))
    ));
    assert_eq!(checks, 3);
    assert_eq!(
        store
            .session(NOW, |session| session.pending_publish())
            .unwrap(),
        None
    );
    assert_eq!(records(&store, RecordSet::Snapshot), before);
    let retried = prepared(&store, "cancelled");
    assert_eq!(retried.first(), Sequence::FIRST);
    seal(&store, &retried);
    store
        .session(NOW, |session| session.commit_publication(&retried))
        .unwrap();
}

#[test]
fn a_delta_only_handoff_preserves_data_and_watermarks_and_returns_to_full_folder_snapshots() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    store
        .session(NOW, |session| session.edit(added(FIRST, None)))
        .unwrap();
    let snapshot = prepared(&store, "folder");
    seal(&store, &snapshot);
    store
        .session(NOW, |session| session.commit_publication(&snapshot))
        .unwrap();
    let mut relay = request("relay");
    relay.deltas_only = true;
    let bridge = store
        .session(NOW, |session| session.prepare_publication(relay, digest))
        .unwrap()
        .unwrap();
    assert!(!bridge.is_snapshot());
    assert_eq!(bridge.first(), snapshot.last().next().unwrap());
    let part = store
        .session(NOW, |session| {
            session.publication_part(&bridge, bridge.first())
        })
        .unwrap();
    assert_eq!(
        youarel_core::record_rules::decode_body(&part.body)
            .unwrap()
            .into_state(),
        State::default()
    );
    seal(&store, &bridge);
    store
        .session(NOW, |session| session.commit_publication(&bridge))
        .unwrap();
    let state = store
        .session(NOW, |session| session.publication_state())
        .unwrap();
    assert_eq!(state.first(), Some(bridge.first()));
    assert_eq!(state.snapshot(), None);
    assert_eq!(records(&store, RecordSet::Snapshot).len(), 1);
    assert_eq!(
        store
            .session(NOW, |session| session.prepare_publication(relay, digest))
            .unwrap(),
        None
    );
    store
        .session(NOW + 1, |session| session.edit(added(REMOTE, None)))
        .unwrap();
    let delta = store
        .session(NOW + 1, |session| {
            session.prepare_publication(relay, digest)
        })
        .unwrap()
        .unwrap();
    assert!(!delta.is_snapshot());
    seal(&store, &delta);
    store
        .session(NOW + 1, |session| session.commit_publication(&delta))
        .unwrap();
    let full = prepared(&store, "folder");
    assert!(full.is_snapshot());
    assert_eq!(records(&store, RecordSet::Snapshot).len(), 2);
}

const PEER: [u8; DIGEST_BYTES] = [7; DIGEST_BYTES];

fn remote_state(id: ItemId) -> State {
    let mut state = State::default();
    state
        .apply(
            added(id, None),
            Stamp {
                wall: NOW,
                counter: 0,
                device: DeviceId([99; 16]),
            },
        )
        .unwrap();
    state
}

fn incoming(body: &[u8], sequence: u64, previous: [u8; DIGEST_BYTES]) -> ReceivedSegment<'_> {
    ReceivedSegment {
        sender: PEER,
        header: Header::new(Sequence::new(sequence).unwrap(), previous, Kind::Delta).unwrap(),
        available: Available {
            first: Sequence::FIRST,
            snapshot: None,
        },
        body,
    }
}

#[test]
fn a_received_segment_commits_its_records_and_cursor_together_and_replays_without_a_change() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    let body = youarel_core::record_rules::encode_body(&youarel_core::record_rules::Body::of(
        &remote_state(FIRST),
    ))
    .unwrap();
    let received = incoming(&body, 1, [0; DIGEST_BYTES]);
    let failed: Result<(), StoreError> = store.session(NOW, |session| {
        session.receive_segment(received, digest)?;
        Err(StoreError::Missing("injected receive failure"))
    });
    failed.unwrap_err();
    assert_eq!(
        store
            .session(NOW, |session| session.peer_cursor(&PEER))
            .unwrap(),
        None
    );
    assert_eq!(records(&store, RecordSet::Snapshot), Vec::<Record>::new());
    let applied = store
        .session(NOW, |session| session.receive_segment(received, digest))
        .unwrap();
    assert_eq!(applied.verdict, Verdict::Apply);
    let before = store
        .session(NOW, |session| Ok(session.generation()))
        .unwrap();
    let repeated = store
        .session(NOW, |session| session.receive_segment(received, digest))
        .unwrap();
    assert_eq!(repeated.verdict, Verdict::Reapply);
    assert_eq!(repeated.cursor, applied.cursor);
    assert_eq!(
        store
            .session(NOW, |session| Ok(session.generation()))
            .unwrap(),
        before
    );
    assert_eq!(records(&store, RecordSet::Snapshot).len(), 1);
    assert_eq!(
        records(&store, RecordSet::Delta { watermark: None }),
        Vec::<Record>::new()
    );
}

#[test]
fn conflicting_or_unadmitted_records_do_not_advance_and_a_complete_snapshot_can_fill_a_gap() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    let first = remote_state(FIRST);
    let first_body = youarel_store::records::encode(&first).unwrap();
    let applied = store
        .session(NOW, |session| {
            session.receive_segment(incoming(&first_body, 1, [0; DIGEST_BYTES]), digest)
        })
        .unwrap();
    let mut changed = remote_state(REMOTE);
    changed.shortcuts.get_mut(&REMOTE).unwrap().title.value = "Changed".to_owned();
    let changed_body = youarel_store::records::encode(&changed).unwrap();
    assert_ne!(digest(&changed_body), digest(&first_body));
    let conflict = store
        .session(NOW, |session| {
            session.receive_segment(incoming(&changed_body, 1, [0; DIGEST_BYTES]), digest)
        })
        .unwrap();
    assert_eq!(conflict.verdict, Verdict::Forked);
    let mut invalid = changed.clone();
    invalid.shortcuts.get_mut(&REMOTE).unwrap().link.value = "javascript:alert(1)".to_owned();
    let invalid_body = youarel_store::records::encode(&invalid).unwrap();
    let refused = store
        .session(NOW, |session| {
            session.receive_segment(incoming(&invalid_body, 2, digest(&first_body)), digest)
        })
        .unwrap_err();
    assert!(matches!(refused, StoreError::Received(_)));
    assert_eq!(
        store
            .session(NOW, |session| session.peer_cursor(&PEER))
            .unwrap(),
        applied.cursor
    );
    assert_eq!(records(&store, RecordSet::Snapshot).len(), 1);
    let snapshot = youarel_store::records::encode(&first.merge(&changed)).unwrap();
    let sequence = Sequence::new(3).unwrap();
    let received = ReceivedSegment {
        sender: PEER,
        header: Header::new(
            sequence,
            [9; DIGEST_BYTES],
            Kind::Snapshot { part: 0, parts: 1 },
        )
        .unwrap(),
        available: Available {
            first: sequence,
            snapshot: Some(sequence),
        },
        body: &snapshot,
    };
    assert_eq!(
        store
            .session(NOW, |session| session.receive_segment(received, digest))
            .unwrap()
            .verdict,
        Verdict::Apply
    );
    assert_eq!(records(&store, RecordSet::Snapshot).len(), 2);
}

#[test]
fn a_crash_reuses_reserved_plaintext_and_a_commit_keeps_writes_made_after_reservation() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    store
        .session(NOW, |session| {
            for number in 1..=8 {
                session.edit(added(ItemId([number; 16]), None))?;
            }
            Ok(())
        })
        .unwrap();
    let pending = prepared(&store, "folder-a");
    assert!(pending.is_snapshot());
    assert!(pending.last() > pending.first());
    let first = store
        .session(NOW, |session| {
            session.publication_part(&pending, pending.first())
        })
        .unwrap();
    store
        .session(NOW, |session| {
            session.seal_publication_part(&pending, pending.first(), &first.digest)
        })
        .unwrap();
    store
        .session(NOW + 1, |session| {
            session.edit(Edit::Retitle {
                id: FIRST,
                title: "After reservation".to_owned(),
            })
        })
        .unwrap();
    let reopened = Store::new(directory.path().to_owned());
    assert_eq!(prepared(&reopened, "folder-a"), pending);
    let repeated = reopened
        .session(NOW, |session| {
            session.publication_part(&pending, pending.first())
        })
        .unwrap();
    assert_eq!(repeated.body, first.body);
    assert_eq!(repeated.sealed, Some(first.digest.to_vec()));
    reopened
        .session(NOW, |session| session.commit_publication(&pending))
        .unwrap_err();
    seal(&reopened, &pending);
    reopened
        .session(NOW, |session| session.commit_publication(&pending))
        .unwrap();
    let delta = prepared(&reopened, "folder-a");
    assert!(!delta.is_snapshot());
    let changed = reopened
        .session(NOW, |session| {
            session.publication_part(&delta, delta.first())
        })
        .unwrap();
    let body = youarel_core::record_rules::decode_body(&changed.body).unwrap();
    assert_eq!(body.shortcuts.len(), 1);
    assert_eq!(body.shortcuts[0].1.title.value, "After reservation");
    seal(&reopened, &delta);
    reopened
        .session(NOW, |session| session.commit_publication(&delta))
        .unwrap();
    assert_eq!(
        reopened
            .session(NOW, |session| session
                .prepare_publication(request("folder-a"), digest))
            .unwrap(),
        None
    );
}

#[test]
fn changing_targets_restarts_with_a_snapshot_after_every_previously_reserved_sequence() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    store
        .session(NOW, |session| session.edit(added(FIRST, None)))
        .unwrap();
    let abandoned = prepared(&store, "folder-a");
    let next = prepared(&store, "folder-b");
    assert!(next.is_snapshot());
    assert!(next.first() > abandoned.last());
    store
        .session(NOW, |session| {
            session.publication_part(&abandoned, abandoned.first())
        })
        .unwrap_err();
    seal(&store, &next);
    store
        .session(NOW, |session| {
            session.seal_publication_part(&next, next.first(), b"conflict")
        })
        .unwrap_err();
    store
        .session(NOW, |session| session.commit_publication(&next))
        .unwrap();
    let back = prepared(&store, "folder-a");
    assert!(back.is_snapshot());
    assert!(back.first() > next.last());
}

fn added(id: ItemId, folder: Option<ItemId>) -> Edit {
    Edit::AddShortcut {
        id,
        link: format!("https://example.com/{}", id.0[0]),
        title: "Kept".to_owned(),
        folder,
    }
}

fn records(store: &Store, set: RecordSet) -> Vec<Record> {
    store
        .session(NOW, |session| {
            let mut records = Vec::new();
            session.stream_records(set, |record| {
                records.push(record);
                Ok(())
            })?;
            Ok(records)
        })
        .unwrap()
}

#[test]
fn a_delta_contains_changed_own_records_and_never_an_unrelated_peer_record() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    let watermark = store
        .session(NOW, |session| {
            session.edit(added(FIRST, None))?;
            session.edit(Edit::AddFolder {
                id: FOLDER,
                name: "Folder".to_owned(),
                parent: None,
            })?;
            session.latest_of(session.device())
        })
        .unwrap();
    assert_eq!(
        records(&store, RecordSet::Delta { watermark: None }).len(),
        2
    );
    assert_eq!(
        records(&store, RecordSet::Delta { watermark }),
        Vec::<Record>::new()
    );
    store
        .session(NOW + 1, |session| {
            session.edit(Edit::Retitle {
                id: FIRST,
                title: "Edited".to_owned(),
            })
        })
        .unwrap();
    let mut remote = State::default();
    remote
        .apply(
            added(REMOTE, None),
            Stamp {
                wall: NOW + 2,
                counter: 0,
                device: DeviceId([99; 16]),
            },
        )
        .unwrap();
    store
        .session(NOW + 2, |session| session.merge(&remote))
        .unwrap();
    let delta = records(&store, RecordSet::Delta { watermark });
    assert_eq!(delta.len(), 1);
    assert!(
        matches!(delta.first(), Some(Record::Shortcut(id, shortcut)) if *id == FIRST && shortcut.title.value == "Edited")
    );
    assert_eq!(records(&store, RecordSet::Snapshot).len(), 3);
}

#[test]
fn a_folder_purge_marks_its_own_descendants_and_a_failed_session_marks_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    let refused: Result<(), StoreError> = store.session(NOW, |session| {
        session.edit(added(FIRST, None))?;
        Err(StoreError::Missing("injected failure"))
    });
    refused.unwrap_err();
    assert_eq!(
        records(&store, RecordSet::Delta { watermark: None }),
        Vec::<Record>::new()
    );

    let watermark = store
        .session(NOW, |session| {
            session.edit(Edit::AddFolder {
                id: FOLDER,
                name: "Folder".to_owned(),
                parent: None,
            })?;
            session.edit(added(FIRST, Some(FOLDER)))?;
            session.latest_of(session.device())
        })
        .unwrap();
    store
        .session(NOW + 1, |session| {
            session.edit(Edit::Remove {
                entry: Entry::Folder(FOLDER),
            })
        })
        .unwrap();
    assert_eq!(records(&store, RecordSet::Delta { watermark }).len(), 2);
}
