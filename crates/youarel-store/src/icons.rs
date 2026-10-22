//! The icons a device gathered, one image for each host, which never leave the device.
//!
//! The images sit under [`IMAGES_DIRECTORY`] beside the state, each named by its digest, and [`INDEX_FILE`] names the image of each host and when the device fetched it.
//! Decoding accepts only what encoding writes: known fields, each host once, and identifiers of sixty-four lowercase hexadecimal digits.

use crate::codec::{from_hex, hex_digits};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The index of the icons, beside the state.
pub const INDEX_FILE: &str = "icons.json";
/// The directory of the icons' images, beside the state.
pub const IMAGES_DIRECTORY: &str = "icons";
/// The extension of an image's file.
pub const IMAGE_EXTENSION: &str = "png";
/// The most bytes of one image the device keeps, which a small square icon fits many times over.
pub const LONGEST_IMAGE: u64 = 32 * 1024;

/// The identifier of an icon's image: the digest of its bytes, which the shell computes, so one image keeps one name.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct IconId(pub [u8; 32]);

impl IconId {
    /// The identifier that sixty-four lowercase hexadecimal digits name.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Icon`] for any other text.
    pub fn parse(text: &str) -> Result<Self, DecodeError> {
        from_hex(text)
            .map(Self)
            .ok_or_else(|| DecodeError::Icon(text.to_owned()))
    }

    /// The identifier as sixty-four lowercase hexadecimal digits, which [`IconId::parse`] reads back.
    #[must_use]
    pub fn name(self) -> String {
        hex_digits(&self.0)
    }
}

/// The icon a device holds for a host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Icon {
    /// The image the host shows.
    pub image: IconId,
    /// When the device fetched the image, in milliseconds since the Unix epoch.
    pub fetched: u64,
}

/// Why an index is not one this youarel writes.
#[derive(Debug)]
pub enum DecodeError {
    /// The text is not the index's JSON shape.
    Json(serde_json::Error),
    /// An image has no identifier of thirty-two bytes in hexadecimal.
    Icon(String),
    /// A host is empty or appears twice.
    Host(String),
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Json(_) => formatter.write_str("not an index of icons"),
            Self::Icon(text) => write!(formatter, "`{text}` names no image"),
            Self::Host(host) => write!(formatter, "the host `{host}` is empty or appears twice"),
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Icon(_) | Self::Host(_) => None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IndexDocument {
    icons: Vec<IconDocument>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IconDocument {
    host: String,
    image: String,
    fetched: u64,
}

/// The index as the document a device writes, its hosts in ascending order.
///
/// # Errors
///
/// Returns the serializer's error, which a document of strings and numbers never meets.
pub fn encode(index: &BTreeMap<String, Icon>) -> Result<String, serde_json::Error> {
    let document = IndexDocument {
        icons: index
            .iter()
            .map(|(host, icon)| IconDocument {
                host: host.clone(),
                image: icon.image.name(),
                fetched: icon.fetched,
            })
            .collect(),
    };
    let mut text = serde_json::to_string_pretty(&document)?;
    text.push('\n');
    Ok(text)
}

/// The index a document holds.
///
/// # Errors
///
/// Returns a [`DecodeError`] naming the first field that encoding would not have written.
pub fn decode(text: &str) -> Result<BTreeMap<String, Icon>, DecodeError> {
    let document: IndexDocument = serde_json::from_str(text).map_err(DecodeError::Json)?;
    let mut index = BTreeMap::new();
    for held in document.icons {
        let icon = Icon {
            image: IconId::parse(&held.image)?,
            fetched: held.fetched,
        };
        if held.host.is_empty() || index.contains_key(&held.host) {
            return Err(DecodeError::Host(held.host));
        }
        index.insert(held.host, icon);
    }
    Ok(index)
}

/// The file name of an image.
#[must_use]
pub fn file_name(image: IconId) -> String {
    format!("{}.{IMAGE_EXTENSION}", image.name())
}

#[cfg(test)]
mod tests {
    use super::{DecodeError, Icon, IconId, decode, encode, file_name};
    use std::collections::BTreeMap;

    fn index() -> BTreeMap<String, Icon> {
        BTreeMap::from([
            (
                "example.com".to_owned(),
                Icon {
                    image: IconId([0xab; 32]),
                    fetched: 1_700_000_000_000,
                },
            ),
            (
                "url.spec.whatwg.org".to_owned(),
                Icon {
                    image: IconId([0x01; 32]),
                    fetched: 0,
                },
            ),
        ])
    }

    #[test]
    fn an_image_holds_at_most_thirty_two_kibibytes() {
        assert_eq!(super::LONGEST_IMAGE, 32 * 1024);
    }

    #[test]
    fn an_index_reads_back_from_its_document_with_its_hosts_in_order() {
        let text = encode(&index()).unwrap();
        assert_eq!(decode(&text).unwrap(), index());
        assert_eq!(encode(&decode(&text).unwrap()).unwrap(), text);
        assert!(
            text.find("example.com").unwrap() < text.find("url.spec.whatwg.org").unwrap(),
            "{text}"
        );
        assert_eq!(
            decode(&encode(&BTreeMap::new()).unwrap()).unwrap(),
            BTreeMap::new()
        );
    }

    #[test]
    fn an_identifier_is_thirty_two_bytes_of_hexadecimal_and_names_a_file() {
        let id = IconId([0xcd; 32]);
        assert_eq!(id.name(), "cd".repeat(32));
        assert_eq!(IconId::parse(&id.name()).unwrap(), id);
        assert_eq!(file_name(id), format!("{}.png", "cd".repeat(32)));
        for text in [
            "",
            "cd",
            &"CD".repeat(32),
            &"cd".repeat(33),
            &"zz".repeat(32),
        ] {
            assert!(
                matches!(IconId::parse(text), Err(DecodeError::Icon(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn an_index_the_codec_would_not_write_is_refused() {
        let text = encode(&index()).unwrap();
        let twice = text.replace("url.spec.whatwg.org", "example.com");
        assert!(matches!(decode(&twice), Err(DecodeError::Host(_))));
        let empty = text.replace("url.spec.whatwg.org", "");
        assert!(matches!(decode(&empty), Err(DecodeError::Host(_))));
        let unnamed = text.replace(&"ab".repeat(32), "ab");
        assert!(matches!(decode(&unnamed), Err(DecodeError::Icon(_))));
        let unknown = text.replace("\"fetched\"", "\"seen\": 1, \"fetched\"");
        assert!(matches!(decode(&unknown), Err(DecodeError::Json(_))));
        for refused in [twice, empty, unnamed, unknown] {
            let error = decode(&refused).unwrap_err();
            assert_ne!(error.to_string(), "");
        }
        assert!(std::error::Error::source(&decode("{").unwrap_err()).is_some());
        assert!(std::error::Error::source(&DecodeError::Host(String::new())).is_none());
    }
}
