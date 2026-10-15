//! The JSON form of the state, which a device writes to its own disk.
//!
//! Decoding accepts only what encoding writes: known fields, identifiers of thirty-two hexadecimal digits, position keys, and admitted links.

use serde::{Deserialize, Serialize};
use youarel_core::clock_rules::{DeviceId, Stamp};
use youarel_core::merge_rules::Register;
use youarel_core::position_rules::Key;
use youarel_core::state::{Folder, ItemId, Shortcut, State};
use youarel_link::Link;

/// The format version this codec writes and reads.
pub const FORMAT: u32 = 1;

/// Why a document is not a state.
#[derive(Debug)]
pub enum DecodeError {
    /// The text is not the document's JSON shape.
    Json(serde_json::Error),
    /// The document names another format version.
    Format(u32),
    /// A field holds no identifier of sixteen bytes in hexadecimal.
    Identifier(String),
    /// A position is not a key.
    Position(String),
    /// A link is not one a shortcut may hold.
    Link(String),
    /// Two items share an identifier.
    Duplicate(String),
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "not a youarel state: {error}"),
            Self::Format(format) => write!(
                formatter,
                "the state has format {format}, but this youarel reads format {FORMAT}"
            ),
            Self::Identifier(text) => write!(formatter, "`{text}` is not an identifier"),
            Self::Position(text) => write!(formatter, "`{text}` is not a position"),
            Self::Link(text) => write!(formatter, "`{text}` is not a link a shortcut may hold"),
            Self::Duplicate(text) => write!(formatter, "the identifier {text} appears twice"),
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Format(_)
            | Self::Identifier(_)
            | Self::Position(_)
            | Self::Link(_)
            | Self::Duplicate(_) => None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: u32,
    shortcuts: Vec<ShortcutDocument>,
    folders: Vec<FolderDocument>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StampDocument {
    wall: u64,
    counter: u32,
    device: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RegisterDocument<T> {
    value: T,
    stamp: StampDocument,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ShortcutDocument {
    id: String,
    link: RegisterDocument<String>,
    title: RegisterDocument<String>,
    alias: RegisterDocument<String>,
    folder: RegisterDocument<Option<String>>,
    position: RegisterDocument<String>,
    removed: RegisterDocument<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FolderDocument {
    id: String,
    name: RegisterDocument<String>,
    parent: RegisterDocument<Option<String>>,
    position: RegisterDocument<String>,
    removed: RegisterDocument<bool>,
}

/// The lowercase hexadecimal digits, each at the place of its value.
const HEX: &[u8; 16] = b"0123456789abcdef";
/// The hexadecimal digits that write one byte.
const DIGITS_PER_BYTE: usize = 2;
/// The hexadecimal digits that write an identifier.
const IDENTIFIER_DIGITS: usize = 32;
/// The bits one hexadecimal digit writes.
const NIBBLE_BITS: u32 = 4;
/// The low half of a byte, which its second digit writes.
const LOW_NIBBLE: u8 = 0x0f;

/// Sixteen bytes as thirty-two lowercase hexadecimal digits.
#[must_use]
pub fn hex(bytes: &[u8; 16]) -> String {
    let mut text = String::with_capacity(IDENTIFIER_DIGITS);
    for byte in bytes {
        for nibble in [byte.wrapping_shr(NIBBLE_BITS), byte & LOW_NIBBLE] {
            if let Some(digit) = HEX.get(usize::from(nibble)) {
                text.push(char::from(*digit));
            }
        }
    }
    text
}

/// Thirty-two hexadecimal digits as sixteen bytes.
///
/// # Errors
///
/// Returns [`DecodeError::Identifier`] for any other text.
pub fn unhex(text: &str) -> Result<[u8; 16], DecodeError> {
    let refuse = || DecodeError::Identifier(text.to_owned());
    let digits = text.as_bytes();
    if digits.len() != IDENTIFIER_DIGITS {
        return Err(refuse());
    }
    let mut bytes = [0_u8; 16];
    let (pairs, _) = digits.as_chunks::<DIGITS_PER_BYTE>();
    for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
        let (Some(high), Some(low)) = (nibble(*high), nibble(*low)) else {
            return Err(refuse());
        };
        *byte = high.wrapping_shl(NIBBLE_BITS) | low;
    }
    Ok(bytes)
}

/// The value of one lowercase hexadecimal digit.
fn nibble(digit: u8) -> Option<u8> {
    HEX.iter()
        .zip(0_u8..)
        .find_map(|(symbol, value)| (*symbol == digit).then_some(value))
}

fn stamp_document(stamp: Stamp) -> StampDocument {
    StampDocument {
        wall: stamp.wall,
        counter: stamp.counter,
        device: hex(&stamp.device.0),
    }
}

fn register_document<T: Clone, U>(
    register: &Register<T>,
    value: impl Fn(&T) -> U,
) -> RegisterDocument<U> {
    RegisterDocument {
        value: value(&register.value),
        stamp: stamp_document(register.stamp),
    }
}

fn id_text(id: ItemId) -> String {
    hex(&id.0)
}

/// The state as the document a device writes.
///
/// # Errors
///
/// Returns the serializer's error, which a document of strings, numbers, booleans, and nulls never meets.
pub fn encode(state: &State) -> Result<String, serde_json::Error> {
    let shortcuts = state
        .shortcuts
        .iter()
        .map(|(id, shortcut)| ShortcutDocument {
            id: id_text(*id),
            link: register_document(&shortcut.link, String::clone),
            title: register_document(&shortcut.title, String::clone),
            alias: register_document(&shortcut.alias, String::clone),
            folder: register_document(&shortcut.folder, |folder| folder.map(id_text)),
            position: register_document(&shortcut.position, |key| String::from(key.as_str())),
            removed: register_document(&shortcut.removed, |removed| *removed),
        })
        .collect();
    let folders = state
        .folders
        .iter()
        .map(|(id, folder)| FolderDocument {
            id: id_text(*id),
            name: register_document(&folder.name, String::clone),
            parent: register_document(&folder.parent, |parent| parent.map(id_text)),
            position: register_document(&folder.position, |key| String::from(key.as_str())),
            removed: register_document(&folder.removed, |removed| *removed),
        })
        .collect();
    let document = Document {
        format: FORMAT,
        shortcuts,
        folders,
    };
    let mut text = serde_json::to_string_pretty(&document)?;
    text.push('\n');
    Ok(text)
}

fn stamp(document: &StampDocument) -> Result<Stamp, DecodeError> {
    Ok(Stamp {
        wall: document.wall,
        counter: document.counter,
        device: DeviceId(unhex(&document.device)?),
    })
}

fn register<T>(document: RegisterDocument<T>) -> Result<Register<T>, DecodeError> {
    Ok(Register {
        stamp: stamp(&document.stamp)?,
        value: document.value,
    })
}

fn item(text: &str) -> Result<ItemId, DecodeError> {
    unhex(text).map(ItemId)
}

fn optional_item(
    document: &RegisterDocument<Option<String>>,
) -> Result<Register<Option<ItemId>>, DecodeError> {
    let value = document.value.as_deref().map(item).transpose()?;
    Ok(Register {
        value,
        stamp: stamp(&document.stamp)?,
    })
}

fn position(document: RegisterDocument<String>) -> Result<Register<Key>, DecodeError> {
    let Some(value) = Key::parse(&document.value) else {
        return Err(DecodeError::Position(document.value));
    };
    Ok(Register {
        value,
        stamp: stamp(&document.stamp)?,
    })
}

fn link(document: RegisterDocument<String>) -> Result<Register<String>, DecodeError> {
    match Link::parse(&document.value) {
        Ok(link) if link.as_str() == document.value => register(document),
        Ok(_) | Err(_) => Err(DecodeError::Link(document.value)),
    }
}

/// The state a document holds.
///
/// # Errors
///
/// Returns a [`DecodeError`] naming the first field that the codec would not have written.
pub fn decode(text: &str) -> Result<State, DecodeError> {
    let document: Document = serde_json::from_str(text).map_err(DecodeError::Json)?;
    if document.format != FORMAT {
        return Err(DecodeError::Format(document.format));
    }
    let mut state = State::default();
    for shortcut in document.shortcuts {
        let id = item(&shortcut.id)?;
        let decoded = Shortcut {
            link: link(shortcut.link)?,
            title: register(shortcut.title)?,
            alias: register(shortcut.alias)?,
            folder: optional_item(&shortcut.folder)?,
            position: position(shortcut.position)?,
            removed: register(shortcut.removed)?,
        };
        if state.shortcuts.insert(id, decoded).is_some() {
            return Err(DecodeError::Duplicate(shortcut.id));
        }
    }
    for folder in document.folders {
        let id = item(&folder.id)?;
        let decoded = Folder {
            name: register(folder.name)?,
            parent: optional_item(&folder.parent)?,
            position: position(folder.position)?,
            removed: register(folder.removed)?,
        };
        if state.folders.insert(id, decoded).is_some() {
            return Err(DecodeError::Duplicate(folder.id));
        }
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::{DecodeError, FORMAT, decode, encode, hex, unhex};
    use youarel_core::clock_rules::{DeviceId, Stamp};
    use youarel_core::state::{Edit, ItemId, State};

    fn sample() -> State {
        let mut state = State::default();
        let stamp = |wall| Stamp {
            wall,
            counter: 0,
            device: DeviceId([7; 16]),
        };
        state
            .apply(
                Edit::AddFolder {
                    id: ItemId([1; 16]),
                    name: "仕事".to_owned(),
                    parent: None,
                },
                stamp(1),
            )
            .unwrap();
        state
            .apply(
                Edit::AddShortcut {
                    id: ItemId([2; 16]),
                    link: "https://example.com/".to_owned(),
                    title: "Example \"quoted\"".to_owned(),
                    folder: Some(ItemId([1; 16])),
                },
                stamp(2),
            )
            .unwrap();
        state
    }

    #[test]
    fn a_state_reads_back_from_its_document() {
        let state = sample();
        let text = encode(&state).unwrap();
        assert_eq!(decode(&text).unwrap(), state);
        assert_eq!(encode(&decode(&text).unwrap()).unwrap(), text);
        assert_eq!(
            decode(&encode(&State::default()).unwrap()).unwrap(),
            State::default()
        );
    }

    #[test]
    fn identifiers_are_sixteen_bytes_of_hexadecimal() {
        let bytes = [0xab; 16];
        assert_eq!(hex(&bytes), "ab".repeat(16));
        assert_eq!(unhex(&hex(&bytes)).unwrap(), bytes);
        for text in [
            "",
            "ab",
            &"g".repeat(32),
            &"AB".repeat(16),
            &"ab".repeat(17),
        ] {
            assert!(
                matches!(unhex(text), Err(DecodeError::Identifier(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn a_document_the_codec_would_not_write_is_refused() {
        let text = encode(&sample()).unwrap();
        let refusals = [
            text.replace(&format!("\"format\": {FORMAT}"), "\"format\": 9"),
            text.replace("https://example.com/", "file:///etc/passwd"),
            text.replace("https://example.com/", "HTTPS://EXAMPLE.COM"),
            text.replacen(&"01".repeat(16), "nothex", 1),
            text.replace("\"V\"", "\"V0\""),
            text.replace("\"format\"", "\"extra\": 1, \"format\""),
            "not json".to_owned(),
        ];
        for refused in refusals {
            let error = decode(&refused).unwrap_err();
            assert_ne!(error.to_string(), "");
        }
        assert!(matches!(
            decode(&text.replace(&format!("\"format\": {FORMAT}"), "\"format\": 9")),
            Err(DecodeError::Format(9))
        ));
        assert!(std::error::Error::source(&decode("{").unwrap_err()).is_some());
    }

    #[test]
    fn a_duplicate_identifier_is_refused() {
        let mut state = sample();
        let shortcut = state.shortcuts.values().next().unwrap().clone();
        state.shortcuts.insert(ItemId([3; 16]), shortcut);
        let text = encode(&state)
            .unwrap()
            .replace(&"03".repeat(16), &"02".repeat(16));
        assert!(matches!(decode(&text), Err(DecodeError::Duplicate(_))));
    }
}
