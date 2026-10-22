//! The JSON form of the state, which a device writes to its own disk.
//!
//! Decoding accepts only what encoding writes: known fields, identifiers of thirty-two hexadecimal digits, position keys, and admitted links.
//! A document from before icons left the state still names an icon for its shortcuts and holds their images, which decoding hands to the store rather than to the state.

use crate::icons::{IconId, LONGEST_IMAGE};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use youarel_core::clock_rules::{DeviceId, Stamp};
use youarel_core::merge_rules::Register;
use youarel_core::position_rules::Key;
use youarel_core::state::{Folder, ItemId, Shortcut, State};
use youarel_core::wire_rules::WireError;
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
    /// An icon has no identifier of thirty-two bytes, or no image that fits.
    Icon(String),
    /// The bytes are not the binary records of a state.
    Records(WireError),
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Json(_) | Self::Records(_) => formatter.write_str("not a youarel state"),
            Self::Format(format) => write!(
                formatter,
                "the state has format {format}, but this youarel reads format {FORMAT}"
            ),
            Self::Identifier(text) => write!(formatter, "`{text}` is not an identifier"),
            Self::Position(text) => write!(formatter, "`{text}` is not a position"),
            Self::Link(text) => write!(formatter, "`{text}` is not a link a shortcut may hold"),
            Self::Duplicate(text) => write!(formatter, "the identifier {text} appears twice"),
            Self::Icon(text) => write!(formatter, "the icon {text} is malformed or too large"),
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Records(error) => Some(error),
            Self::Format(_)
            | Self::Identifier(_)
            | Self::Position(_)
            | Self::Link(_)
            | Self::Duplicate(_)
            | Self::Icon(_) => None,
        }
    }
}

/// What a document holds: the state, and the icon that each live shortcut of a document from before icons left the state showed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Decoded {
    pub state: State,
    pub icons: Vec<Shown>,
}

/// The icon that a live shortcut of a document from before icons left the state showed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Shown {
    /// The shortcut's link, whose host the icon belongs to.
    pub link: String,
    /// The stamp of the write that chose the icon.
    pub chosen: Stamp,
    pub image: IconId,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: u32,
    shortcuts: Vec<ShortcutDocument>,
    folders: Vec<FolderDocument>,
    /// The images of the icons, which only a document from before icons left the state holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    icons: Vec<IconDocument>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IconDocument {
    id: String,
    /// The image in base64.
    image: String,
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
    /// The shortcut's icon, which only a document from before icons left the state names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<RegisterDocument<Option<String>>>,
    /// The shortcut's note, which a document from before notes lacks, and which then reads as never written.
    #[serde(default)]
    note: Option<RegisterDocument<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FolderDocument {
    id: String,
    name: RegisterDocument<String>,
    /// The folder's mark, which a document from before marks lacks, and which then reads as never written.
    #[serde(default)]
    mark: Option<RegisterDocument<String>>,
    parent: RegisterDocument<Option<String>>,
    position: RegisterDocument<String>,
    removed: RegisterDocument<bool>,
}

/// The lowercase hexadecimal digits, each at the place of its value.
const HEX: &[u8; 16] = b"0123456789abcdef";
/// The hexadecimal digits that write one byte.
const DIGITS_PER_BYTE: usize = 2;
/// The bits one hexadecimal digit writes.
const NIBBLE_BITS: u32 = 4;
/// The low half of a byte, which its second digit writes.
const LOW_NIBBLE: u8 = 0x0f;
/// The stamp of a register that no device wrote, which every write follows.
const NEVER: Stamp = Stamp {
    wall: 0,
    counter: 0,
    device: DeviceId([0; 16]),
};

/// Bytes as lowercase hexadecimal digits, two to a byte.
pub(crate) fn hex_digits(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().saturating_mul(DIGITS_PER_BYTE));
    for byte in bytes {
        for nibble in [byte.wrapping_shr(NIBBLE_BITS), byte & LOW_NIBBLE] {
            if let Some(digit) = HEX.get(usize::from(nibble)) {
                text.push(char::from(*digit));
            }
        }
    }
    text
}

/// Sixteen bytes as thirty-two lowercase hexadecimal digits.
#[must_use]
pub fn hex(bytes: &[u8; 16]) -> String {
    hex_digits(bytes)
}

/// The `N` bytes that exactly two lowercase hexadecimal digits a byte spell.
pub(crate) fn from_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let digits = text.as_bytes();
    if digits.len() != N.saturating_mul(DIGITS_PER_BYTE) {
        return None;
    }
    let mut bytes = [0_u8; N];
    let (pairs, _) = digits.as_chunks::<DIGITS_PER_BYTE>();
    for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
        let (Some(high), Some(low)) = (nibble(*high), nibble(*low)) else {
            return None;
        };
        *byte = high.wrapping_shl(NIBBLE_BITS) | low;
    }
    Some(bytes)
}

/// Thirty-two hexadecimal digits as sixteen bytes.
///
/// # Errors
///
/// Returns [`DecodeError::Identifier`] for any other text.
pub fn unhex(text: &str) -> Result<[u8; 16], DecodeError> {
    from_hex(text).ok_or_else(|| DecodeError::Identifier(text.to_owned()))
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
            icon: None,
            note: Some(register_document(&shortcut.note, String::clone)),
        })
        .collect();
    let folders = state
        .folders
        .iter()
        .map(|(id, folder)| FolderDocument {
            id: id_text(*id),
            name: register_document(&folder.name, String::clone),
            mark: Some(register_document(&folder.mark, String::clone)),
            parent: register_document(&folder.parent, |parent| parent.map(id_text)),
            position: register_document(&folder.position, |key| String::from(key.as_str())),
            removed: register_document(&folder.removed, |removed| *removed),
        })
        .collect();
    let document = Document {
        format: FORMAT,
        shortcuts,
        folders,
        icons: Vec::new(),
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

/// The icon that sixty-four hexadecimal digits name.
fn icon_id(text: &str) -> Result<IconId, DecodeError> {
    from_hex(text)
        .map(IconId)
        .ok_or_else(|| DecodeError::Icon(text.to_owned()))
}

/// The icon a shortcut of a document from before icons left the state chose, and the stamp of its choice, or none.
fn icon(
    document: Option<RegisterDocument<Option<String>>>,
) -> Result<Option<(IconId, Stamp)>, DecodeError> {
    let Some(document) = document else {
        return Ok(None);
    };
    let chosen = stamp(&document.stamp)?;
    Ok(document
        .value
        .as_deref()
        .map(icon_id)
        .transpose()?
        .map(|icon| (icon, chosen)))
}

/// A shortcut's note or a folder's mark, or an empty one that no device wrote when the document predates it.
fn text_or_unwritten(
    document: Option<RegisterDocument<String>>,
) -> Result<Register<String>, DecodeError> {
    document.map_or_else(
        || {
            Ok(Register {
                value: String::new(),
                stamp: NEVER,
            })
        },
        register,
    )
}

/// An icon's image, which holds no more than [`LONGEST_IMAGE`] bytes.
fn image(document: &IconDocument) -> Result<Vec<u8>, DecodeError> {
    match BASE64.decode(&document.image) {
        Ok(image) if u64::try_from(image.len()).is_ok_and(|length| length <= LONGEST_IMAGE) => {
            Ok(image)
        }
        Ok(_) | Err(_) => Err(DecodeError::Icon(document.id.clone())),
    }
}

/// The images of a document from before icons left the state, by their identifiers.
fn images(documents: &[IconDocument]) -> Result<BTreeMap<IconId, Vec<u8>>, DecodeError> {
    let mut images = BTreeMap::new();
    for held in documents {
        if images.insert(icon_id(&held.id)?, image(held)?).is_some() {
            return Err(DecodeError::Duplicate(held.id.clone()));
        }
    }
    Ok(images)
}

fn link(document: RegisterDocument<String>) -> Result<Register<String>, DecodeError> {
    match Link::parse(&document.value) {
        Ok(link) if link.as_str() == document.value => register(document),
        Ok(_) | Err(_) => Err(DecodeError::Link(document.value)),
    }
}

/// The state a document holds, and the icon each of its live shortcuts showed when the document is from before icons left the state.
///
/// # Errors
///
/// Returns a [`DecodeError`] naming the first field that the codec would not have written.
pub fn decode(text: &str) -> Result<Decoded, DecodeError> {
    let document: Document = serde_json::from_str(text).map_err(DecodeError::Json)?;
    if document.format != FORMAT {
        return Err(DecodeError::Format(document.format));
    }
    let images = images(&document.icons)?;
    let mut state = State::default();
    let mut icons = Vec::new();
    for shortcut in document.shortcuts {
        let id = item(&shortcut.id)?;
        let decoded = Shortcut {
            link: link(shortcut.link)?,
            title: register(shortcut.title)?,
            alias: register(shortcut.alias)?,
            folder: optional_item(&shortcut.folder)?,
            position: position(shortcut.position)?,
            removed: register(shortcut.removed)?,
            note: text_or_unwritten(shortcut.note)?,
        };
        let chosen = icon(shortcut.icon)?;
        if let Some((image, chosen)) = chosen
            && !decoded.removed.value
            && let Some(bytes) = images.get(&image)
        {
            icons.push(Shown {
                link: decoded.link.value.clone(),
                chosen,
                image,
                bytes: bytes.clone(),
            });
        }
        if state.shortcuts.insert(id, decoded).is_some() {
            return Err(DecodeError::Duplicate(shortcut.id));
        }
    }
    for folder in document.folders {
        let id = item(&folder.id)?;
        let decoded = Folder {
            name: register(folder.name)?,
            mark: text_or_unwritten(folder.mark)?,
            parent: optional_item(&folder.parent)?,
            position: position(folder.position)?,
            removed: register(folder.removed)?,
        };
        if state.folders.insert(id, decoded).is_some() {
            return Err(DecodeError::Duplicate(folder.id));
        }
    }
    Ok(Decoded { state, icons })
}

#[cfg(test)]
mod tests {
    use super::{BASE64, DecodeError, Decoded, FORMAT, Shown, decode, encode, hex, unhex};
    use crate::icons::{IconId, LONGEST_IMAGE};
    use base64::Engine as _;
    use youarel_core::clock_rules::{DeviceId, Stamp};
    use youarel_core::state::{Edit, Entry, ItemId, State};

    const fn at(wall: u64) -> Stamp {
        Stamp {
            wall,
            counter: 0,
            device: DeviceId([7; 16]),
        }
    }

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

    /// The document an earlier build wrote for `state`: each shortcut chose the icon `chosen` at wall time three, and the document holds the images of `held`.
    fn legacy(state: &State, chosen: Option<IconId>, held: &[(IconId, &[u8])]) -> String {
        let mut document: serde_json::Value =
            serde_json::from_str(&encode(state).unwrap()).unwrap();
        let register = serde_json::json!({
            "value": chosen.map(IconId::name),
            "stamp": { "wall": 3, "counter": 0, "device": "07".repeat(16) },
        });
        for shortcut in document["shortcuts"].as_array_mut().unwrap() {
            let fields = shortcut.as_object_mut().unwrap();
            fields.insert("icon".to_owned(), register.clone());
        }
        let images: Vec<serde_json::Value> = held
            .iter()
            .map(
                |(id, image)| serde_json::json!({ "id": id.name(), "image": BASE64.encode(image) }),
            )
            .collect();
        let fields = document.as_object_mut().unwrap();
        fields.insert("icons".to_owned(), serde_json::Value::from(images));
        serde_json::to_string_pretty(&document).unwrap()
    }

    #[test]
    fn a_state_reads_back_from_its_document() {
        let state = sample();
        let text = encode(&state).unwrap();
        let decoded = decode(&text).unwrap();
        assert_eq!((decoded.state, decoded.icons), (state, Vec::new()));
        assert_eq!(encode(&decode(&text).unwrap().state).unwrap(), text);
        assert_eq!(
            decode(&encode(&State::default()).unwrap()).unwrap(),
            Decoded::default()
        );
    }

    #[test]
    fn a_document_from_before_icons_left_the_state_hands_over_its_live_shortcuts_icons() {
        let image: &[u8] = b"\x89PNG image";
        let nine = IconId([9; 32]);
        let decoded = decode(&legacy(&sample(), Some(nine), &[(nine, image)])).unwrap();
        assert_eq!(decoded.state, sample(), "the state holds no icon");
        assert_eq!(
            decoded.icons,
            [Shown {
                link: "https://example.com/".to_owned(),
                chosen: Stamp {
                    wall: 3,
                    counter: 0,
                    device: DeviceId([7; 16]),
                },
                image: nine,
                bytes: image.to_vec(),
            }]
        );
        let written = encode(&decoded.state).unwrap();
        assert!(!written.contains("icon"), "{written}");
        let mut removed = sample();
        let gone = Edit::Remove {
            entry: Entry::Shortcut(ItemId([2; 16])),
        };
        removed.apply(gone, at(4)).unwrap();
        let unshown = [
            legacy(&removed, Some(nine), &[(nine, image)]),
            legacy(&sample(), None, &[(nine, image)]),
            legacy(&sample(), Some(nine), &[(IconId([8; 32]), image)]),
        ];
        for text in unshown {
            assert_eq!(decode(&text).unwrap().icons, [], "{text}");
        }
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

    fn annotated() -> State {
        let mut state = sample();
        let stamp = |wall| Stamp {
            wall,
            counter: 0,
            device: DeviceId([7; 16]),
        };
        state
            .apply(
                Edit::SetNote {
                    id: ItemId([2; 16]),
                    note: "read \"this\" first\n".to_owned(),
                },
                stamp(3),
            )
            .unwrap();
        state
    }

    #[test]
    fn a_note_reads_back_and_a_document_from_before_notes_reads_without_one() {
        let state = annotated();
        let text = encode(&state).unwrap();
        assert_eq!(decode(&text).unwrap().state, state);
        assert_eq!(encode(&decode(&text).unwrap().state).unwrap(), text);
        let mut older: serde_json::Value =
            serde_json::from_str(&encode(&sample()).unwrap()).unwrap();
        for shortcut in older["shortcuts"].as_array_mut().unwrap() {
            shortcut.as_object_mut().unwrap().remove("note");
        }
        let without = older.to_string();
        assert!(!without.contains("note"), "{without}");
        let read = decode(&without).unwrap().state;
        let shortcut = read.shortcuts.values().next().unwrap();
        assert_eq!(shortcut.note.value, "");
        assert_eq!(shortcut.note.stamp, super::NEVER);
    }

    #[test]
    fn a_mark_reads_back_and_a_document_from_before_marks_reads_without_one() {
        let mut state = sample();
        let stamp = Stamp {
            wall: 3,
            counter: 0,
            device: DeviceId([7; 16]),
        };
        state
            .apply(
                Edit::MarkFolder {
                    id: ItemId([1; 16]),
                    mark: "📚".to_owned(),
                },
                stamp,
            )
            .unwrap();
        let text = encode(&state).unwrap();
        assert_eq!(decode(&text).unwrap().state, state);
        let mut older: serde_json::Value =
            serde_json::from_str(&encode(&sample()).unwrap()).unwrap();
        for folder in older["folders"].as_array_mut().unwrap() {
            folder.as_object_mut().unwrap().remove("mark");
        }
        let without = older.to_string();
        assert!(!without.contains("mark"), "{without}");
        let read = decode(&without).unwrap().state;
        let folder = read.folders.values().next().unwrap();
        assert_eq!(
            (folder.mark.value.as_str(), folder.mark.stamp),
            ("", super::NEVER)
        );
    }

    #[test]
    fn a_malformed_or_oversized_icon_is_refused() {
        let image: &[u8] = b"\x89PNG image";
        let nine = IconId([9; 32]);
        let text = legacy(&sample(), Some(nine), &[(nine, image)]);
        let encoded = BASE64.encode(image);
        let longest = usize::try_from(LONGEST_IMAGE).unwrap();
        let refusals = [
            text.replace(&format!("\"id\": \"{}\"", nine.name()), "\"id\": \"0909\""),
            text.replace(
                &format!("\"value\": \"{}\"", nine.name()),
                "\"value\": \"zz\"",
            ),
            text.replace(&encoded, "not base64!"),
            text.replace(&encoded, &BASE64.encode(vec![0; longest + 1])),
        ];
        for refused in refusals {
            assert!(
                matches!(decode(&refused), Err(DecodeError::Icon(_))),
                "{refused}"
            );
        }
        let largest = decode(&text.replace(&encoded, &BASE64.encode(vec![0; longest]))).unwrap();
        assert_eq!(
            largest.icons.first().map(|shown| shown.bytes.len()),
            Some(longest)
        );
        let twice = legacy(&sample(), Some(nine), &[(nine, image), (nine, image)]);
        assert!(matches!(decode(&twice), Err(DecodeError::Duplicate(_))));
    }

    /// Writes a document from before icons left the state into the seed corpus of the state's fuzz target, which `cargo nextest run --run-ignored only seeds` regenerates.
    #[test]
    #[ignore = "writes the committed seeds of the fuzz corpus; run it to regenerate them"]
    fn seeds() {
        let corpus =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/state_decode");
        let nine = IconId([9; 32]);
        let text = legacy(&sample(), Some(nine), &[(nine, b"\x89PNG image")]);
        assert_eq!(decode(&text).unwrap().icons.len(), 1);
        crate::fs::replace(&corpus.join("iconed"), text.as_bytes()).unwrap();
    }
}
