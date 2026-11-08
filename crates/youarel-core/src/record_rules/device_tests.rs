use super::{Body, DeviceTable, decode_body, decode_device, encode_body, encode_device};
use crate::{
    clock_rules::{DeviceId, Stamp},
    device_rules::{LONGEST_NAME, MemberId},
    merge_rules::Register,
    state::{Edit, EditError, State},
    wire_rules::{Reader, WireError, Writer},
};
use alloc::string::String;

const FIRST: MemberId = MemberId([1; 32]);
const SECOND: MemberId = MemberId([2; 32]);
const WRITER: DeviceId = DeviceId([3; 16]);

const fn at(wall: u64) -> Stamp {
    Stamp {
        wall,
        counter: 0,
        device: WRITER,
    }
}

#[test]
fn names_join_as_registers_preserve_full_membership_identifiers_and_observe_the_clock() {
    let mut first = State::default();
    let mut second = State::default();
    first
        .apply(
            Edit::NameDevice {
                id: FIRST,
                name: String::from("Mac 🖥️"),
            },
            at(1),
        )
        .unwrap();
    second
        .apply(
            Edit::NameDevice {
                id: FIRST,
                name: String::from("Windows"),
            },
            at(2),
        )
        .unwrap();
    second
        .apply(
            Edit::NameDevice {
                id: SECOND,
                name: String::new(),
            },
            at(3),
        )
        .unwrap();
    let joined = first.merge(&second);
    assert_eq!(joined, second.merge(&first));
    assert_eq!(joined, joined.merge(&joined));
    assert_eq!(joined.devices[&FIRST].value, "Windows");
    assert_eq!(joined.devices[&SECOND].value, "");
    assert_eq!(joined.latest(), Some(at(3)));
    assert_eq!(joined.latest_of(WRITER), Some(at(3)));
    assert_eq!(
        decode_body(&encode_body(&Body::of(&joined)).unwrap())
            .unwrap()
            .into_state(),
        joined
    );
    let before = first.clone();
    assert_eq!(
        first.apply(
            Edit::NameDevice {
                id: FIRST,
                name: String::from("bad\nname")
            },
            at(4)
        ),
        Err(EditError::DeviceName)
    );
    assert_eq!(first, before);
    assert_eq!(
        Edit::NameDevice {
            id: FIRST,
            name: String::new()
        }
        .entry(),
        None
    );
}

#[test]
fn name_records_refuse_unknown_writers_invalid_names_duplicates_and_old_bodies() {
    let name = Register {
        value: String::from("a"),
        stamp: at(u64::MAX),
    };
    let table = DeviceTable::ascending(alloc::vec![WRITER]).unwrap();
    let mut writer = Writer::new();
    encode_device(&mut writer, &table, (FIRST, &name)).unwrap();
    let mut bytes = writer.finish();
    let mut reader = Reader::new(&bytes);
    assert_eq!(
        decode_device(&mut reader, &table).unwrap(),
        (FIRST, name.clone())
    );
    reader.finish().unwrap();
    *bytes.last_mut().unwrap() = b'\n';
    assert_eq!(
        decode_device(&mut Reader::new(&bytes), &table),
        Err(WireError::Value)
    );
    encode_device(&mut Writer::new(), &DeviceTable::default(), (FIRST, &name)).unwrap_err();
    let invalid = Register {
        value: "x".repeat(LONGEST_NAME + 1),
        ..name.clone()
    };
    assert_eq!(
        encode_device(&mut Writer::new(), &table, (FIRST, &invalid)),
        Err(WireError::Value)
    );
    let mut body = Body {
        devices: alloc::vec![(FIRST, name.clone()), (FIRST, name)],
        ..Body::default()
    };
    assert_eq!(encode_body(&body), Err(WireError::Order));
    body.devices.pop();
    let mut old = encode_body(&body).unwrap();
    old.get_mut(super::MAGIC.len()..super::MAGIC.len() + size_of::<u16>())
        .unwrap()
        .copy_from_slice(&1u16.to_be_bytes());
    assert_eq!(decode_body(&old), Err(WireError::Version(1)));
}
