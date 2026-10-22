//! The state as the binary records of `record_rules`, which a device keeps on its disk and seals into its bundles.
//!
//! Reading admits every link again, as the JSON reader does, so the records of a state hold only links a shortcut may hold.

use crate::codec::{self, DecodeError};
use youarel_core::record_rules::{self, Body, MAGIC};
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
    let body = record_rules::decode_body(bytes).map_err(DecodeError::Records)?;
    for (_, shortcut) in &body.shortcuts {
        match Link::parse(&shortcut.link.value) {
            Ok(link) if link.as_str() == shortcut.link.value => {}
            Ok(_) | Err(_) => return Err(DecodeError::Link(shortcut.link.value.clone())),
        }
    }
    Ok(body.into_state())
}

/// The state that a bundle carries: its records, or the JSON form that a build from before the records sends, whose icons the state leaves behind.
///
/// # Errors
///
/// Returns the error of [`decode`] for bytes that begin as records, the JSON reader's for other text, and [`DecodeError::Records`] for other bytes.
pub fn decode_received(bytes: &[u8]) -> Result<State, DecodeError> {
    if bytes.starts_with(&MAGIC) {
        return decode(bytes);
    }
    core::str::from_utf8(bytes).map_or(Err(DecodeError::Records(WireError::Magic)), |text| {
        codec::decode(text).map(|decoded| decoded.state)
    })
}

#[cfg(test)]
mod tests {
    use super::{decode, decode_received, encode};
    use crate::codec::{self, DecodeError};
    use youarel_core::clock_rules::{DeviceId, Stamp};
    use youarel_core::record_rules::{self, Body};
    use youarel_core::state::{Edit, ItemId, State};
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

    #[test]
    fn a_received_state_reads_as_its_records_or_as_the_json_form_of_an_older_build() {
        let state = holding("https://example.com/");
        assert_eq!(decode_received(&encode(&state).unwrap()).unwrap(), state);
        let text = codec::encode(&state).unwrap();
        assert_eq!(decode_received(text.as_bytes()).unwrap(), state);
        let mut unadmitted = encode(&holding("file:///etc/passwd")).unwrap();
        assert!(matches!(
            decode_received(&unadmitted),
            Err(DecodeError::Link(_))
        ));
        unadmitted.truncate(record_rules::MAGIC.len());
        assert!(
            matches!(
                decode_received(&unadmitted),
                Err(DecodeError::Records(WireError::Truncated))
            ),
            "bytes that begin as records read only as records"
        );
        assert!(matches!(
            decode_received(b"not a state"),
            Err(DecodeError::Json(_))
        ));
        assert!(matches!(
            decode_received(&[0xff, b'{']),
            Err(DecodeError::Records(WireError::Magic))
        ));
    }

    /// Writes records into the seed corpus of their fuzz target, which `cargo nextest run --run-ignored only seeds` regenerates.
    #[test]
    #[ignore = "writes the committed seeds of the fuzz corpus; run it to regenerate them"]
    fn seeds() {
        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/corpus/record_decode");
        crate::fs::create_dir_all(&corpus).unwrap();
        let mut noted = holding("https://example.com/");
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
        let seeds = [
            ("empty", State::default()),
            ("folder-and-shortcut", holding("https://example.com/")),
            ("two-devices", noted),
        ];
        for (name, state) in seeds {
            let bytes = encode(&state).unwrap();
            assert_eq!(decode(&bytes).unwrap(), state, "{name}");
            crate::fs::replace(&corpus.join(name), &bytes).unwrap();
        }
        let older = codec::encode(&holding("https://example.com/")).unwrap();
        assert_eq!(
            decode_received(older.as_bytes()).unwrap(),
            holding("https://example.com/")
        );
        crate::fs::replace(&corpus.join("json-form"), older.as_bytes()).unwrap();
    }
}
