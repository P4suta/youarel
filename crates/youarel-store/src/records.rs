//! The state as the binary records of `record_rules`, which a device seals into its bundles and reads from the bundles of the others.
//!
//! Reading admits every link again, so the records of a state hold only links a shortcut may hold.

use crate::codec::DecodeError;
use youarel_core::record_rules::{self, Body};
use youarel_core::state::State;
use youarel_core::wire_rules::WireError;
use youarel_link::Link;

/// The records of a state.
///
/// # Errors
///
/// Returns the writer's error, which the records of a state never meet, since their items ascend and their table holds every device.
pub fn encode(state: &State) -> Result<Vec<u8>, WireError> {
    record_rules::encode_body(&Body::of(state))
}

/// The state that records hold, with every link admitted again.
///
/// # Errors
///
/// Returns [`DecodeError::Records`] for bytes that the records' writer would not have written, and [`DecodeError::Link`] for a link a shortcut may not hold.
pub fn decode(bytes: &[u8]) -> Result<State, DecodeError> {
    Ok(decode_body(bytes)?.into_state())
}

/// The bounded body's records, with every link admitted before a streaming merge.
///
/// # Errors
///
/// Refuses the same malformed or unadmitted records as [`decode`].
pub fn decode_body(bytes: &[u8]) -> Result<Body, DecodeError> {
    let body = record_rules::decode_body(bytes).map_err(DecodeError::Records)?;
    for (_, shortcut) in &body.shortcuts {
        match Link::parse(&shortcut.link.value) {
            Ok(link) if link.as_str() == shortcut.link.value => {}
            Ok(_) | Err(_) => return Err(DecodeError::Link(shortcut.link.value.clone())),
        }
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};
    use crate::codec::DecodeError;
    use youarel_core::clock_rules::{DeviceId, Stamp};
    use youarel_core::merge_rules::Register;
    use youarel_core::position_rules::Key;
    use youarel_core::record_rules::{self, Body};
    use youarel_core::state::{Edit, Entry, ItemId, Presence, Shortcut, State};
    use youarel_core::wire_rules::WireError;

    const fn at(wall: u64) -> Stamp {
        Stamp {
            wall,
            counter: 0,
            device: DeviceId([7; 16]),
        }
    }

    /// A state with a folder and a shortcut inside it, whose link is `link` as the store would have admitted it.
    fn holding(link: &str) -> State {
        let mut state = State::default();
        state
            .apply(
                Edit::AddFolder {
                    id: ItemId([1; 16]),
                    name: "仕事".to_owned(),
                    parent: None,
                },
                at(1),
            )
            .unwrap();
        state
            .apply(
                Edit::AddShortcut {
                    id: ItemId([2; 16]),
                    link: link.to_owned(),
                    title: "Example".to_owned(),
                    folder: Some(ItemId([1; 16])),
                },
                at(2),
            )
            .unwrap();
        state
    }

    #[test]
    fn a_state_reads_back_from_its_records_with_its_links_admitted_again() {
        let state = holding("https://example.com/");
        let bytes = encode(&state).unwrap();
        assert_eq!(bytes, record_rules::encode_body(&Body::of(&state)).unwrap());
        assert_eq!(decode(&bytes).unwrap(), state);
        assert_eq!(
            decode(&encode(&State::default()).unwrap()).unwrap(),
            State::default()
        );
        for refused in ["file:///etc/passwd", "HTTPS://EXAMPLE.COM"] {
            let unadmitted = encode(&holding(refused)).unwrap();
            assert!(
                matches!(decode(&unadmitted), Err(DecodeError::Link(link)) if link == refused),
                "{refused}"
            );
        }
        let error = decode(b"{\"format\": 1}").unwrap_err();
        assert!(matches!(error, DecodeError::Records(WireError::Magic)));
        assert_ne!(error.to_string(), "");
        assert!(std::error::Error::source(&error).is_some());
    }

    /// A state with a shortcut in the trash inside a folder in the trash, and a shortcut gone for good.
    fn trashed() -> State {
        let mut trashed = holding("https://example.com/");
        let gone = ItemId([4; 16]);
        let edits = [
            Edit::Trash {
                entry: Entry::Shortcut(ItemId([2; 16])),
            },
            Edit::Trash {
                entry: Entry::Folder(ItemId([1; 16])),
            },
            Edit::AddShortcut {
                id: gone,
                link: "https://example.net/".to_owned(),
                title: "Gone".to_owned(),
                folder: None,
            },
            Edit::Remove {
                entry: Entry::Shortcut(gone),
            },
        ];
        for (wall, edit) in (5..).zip(edits) {
            trashed.apply(edit, at(wall)).unwrap();
        }
        trashed
    }

    #[test]
    fn a_state_with_a_trash_reads_back_from_its_records() {
        let state = trashed();
        assert_eq!(decode(&encode(&state).unwrap()).unwrap(), state);
    }

    /// Writes records into the seed corpus of their fuzz target, which `cargo nextest run --run-ignored only seeds` regenerates.
    #[test]
    #[ignore = "writes the committed seeds of the fuzz corpus; run it to regenerate them"]
    fn seeds() {
        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/corpus/record_decode");
        crate::fs::create_dir_all(&corpus).unwrap();
        let mut noted = holding("https://example.com/");
        let mut named = State::default();
        named
            .apply(
                Edit::NameDevice {
                    id: youarel_core::device_rules::MemberId([8; 32]),
                    name: "仕事の Mac".to_owned(),
                },
                at(3),
            )
            .unwrap();
        noted
            .apply(
                Edit::SetNote {
                    id: ItemId([2; 16]),
                    note: "read \"this\" first".to_owned(),
                },
                Stamp {
                    wall: 3,
                    counter: 1,
                    device: DeviceId([9; 16]),
                },
            )
            .unwrap();
        let mut stored = holding("https://example.com/");
        // A shortcut at a position that bisection wrote before integers, which every later build reads.
        stored.shortcuts.insert(
            ItemId([3; 16]),
            Shortcut {
                link: Register {
                    value: "https://example.org/".to_owned(),
                    stamp: at(4),
                },
                title: Register {
                    value: "Kept".to_owned(),
                    stamp: at(4),
                },
                alias: Register {
                    value: String::new(),
                    stamp: at(4),
                },
                folder: Register {
                    value: Some(ItemId([1; 16])),
                    stamp: at(4),
                },
                position: Register {
                    value: Key::parse("zzzV").unwrap(),
                    stamp: at(4),
                },
                presence: Register {
                    value: Presence::Live,
                    stamp: at(4),
                },
                note: Register {
                    value: String::new(),
                    stamp: at(4),
                },
            },
        );
        let seeds = [
            ("empty", State::default()),
            ("folder-and-shortcut", holding("https://example.com/")),
            ("two-devices", noted),
            ("stored-positions", stored),
            ("trash", trashed()),
            ("device-names", named),
        ];
        for (name, state) in seeds {
            let bytes = encode(&state).unwrap();
            assert_eq!(decode(&bytes).unwrap(), state, "{name}");
            crate::fs::replace(&corpus.join(name), &bytes).unwrap();
        }
    }
}
