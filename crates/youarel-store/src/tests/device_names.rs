use super::{NOW, Record, Store, StoreError};
use std::collections::BTreeMap;
use youarel_core::{
    clock_rules::{DeviceId, Stamp},
    device_rules::MemberId,
    merge_rules::Register,
    state::State,
    wire_rules::WireError,
};

fn name(value: &str, wall: u64) -> Register<String> {
    Register {
        value: value.to_owned(),
        stamp: Stamp {
            wall,
            counter: 0,
            device: DeviceId([3; 16]),
        },
    }
}

#[test]
fn device_names_merge_once_and_persist_with_their_membership_identifiers() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    let first = MemberId([1; 32]);
    let second = MemberId([2; 32]);
    let desktop = name("Desktop", NOW);
    let laptop = name("Laptop", NOW + 1);
    let state = State {
        devices: BTreeMap::from([(second, laptop.clone())]),
        ..State::default()
    };
    store
        .session(NOW, |session| {
            assert_eq!(session.device_name(first)?, None);
            assert!(session.device_names()?.is_empty());
            assert!(session.merge_record(Record::Device(first, desktop.clone()))?);
            assert!(!session.merge_record(Record::Device(first, desktop.clone()))?);
            assert!(session.merge(&state)?);
            assert!(!session.merge(&state)?);
            Ok(())
        })
        .unwrap();
    let expected = BTreeMap::from([(first, desktop.clone()), (second, laptop)]);
    store
        .session(NOW + 2, |session| {
            assert_eq!(session.device_name(first)?, Some(desktop));
            assert_eq!(session.device_names()?, expected);
            assert!(!session.merge_record(Record::Device(first, name("Old", NOW - 1)))?);
            assert!(session.merge_record(Record::Device(first, name("", NOW + 3)))?);
            assert_eq!(session.device_name(first)?, Some(name("", NOW + 3)));
            Ok(())
        })
        .unwrap();
}

#[test]
fn invalid_merged_names_are_refused_without_a_partial_write() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    let id = MemberId([4; 32]);
    store
        .session(NOW, |session| {
            for invalid in [" padded", "a\nb"] {
                assert!(matches!(
                    session.merge_record(Record::Device(id, name(invalid, NOW))),
                    Err(StoreError::Records(WireError::Value))
                ));
                assert_eq!(session.device_name(id)?, None);
                assert!(session.device_names()?.is_empty());
            }
            Ok(())
        })
        .unwrap();
}
