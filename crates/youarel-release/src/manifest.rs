//! The release manifest: the version a release is and, for each target, the asset that holds its program.
//!
//! Decoding accepts only what encoding writes, so a manifest signed once has one meaning.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use youarel_core::update_rules::Version;

/// The format version this module writes and reads.
pub const FORMAT: u32 = 1;
/// The longest manifest a reader accepts.
pub const LONGEST_MANIFEST: usize = 64 * 1024;
/// The largest asset a manifest may name.
pub const LARGEST_ASSET: u64 = 256 * 1024 * 1024;
/// The longest target triple or asset name.
const LONGEST_NAME: usize = 128;
/// The bytes of a SHA-256 digest.
const DIGEST_BYTES: usize = 32;
/// The hexadecimal digits that write one byte.
const DIGITS_PER_BYTE: usize = 2;
/// The lowercase hexadecimal digits, each at the place of its value.
const HEX: &[u8; 16] = b"0123456789abcdef";
/// The bits one hexadecimal digit writes.
const NIBBLE_BITS: u32 = 4;
/// The low half of a byte, which its second digit writes.
const LOW_NIBBLE: u8 = 0x0f;

/// The SHA-256 digest of an asset.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Digest(pub [u8; DIGEST_BYTES]);

impl Digest {
    /// The digest of `bytes`.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// The digest as sixty-four lowercase hexadecimal digits.
    #[must_use]
    pub fn hex(&self) -> String {
        let mut text = String::with_capacity(DIGEST_BYTES.saturating_mul(DIGITS_PER_BYTE));
        for byte in self.0 {
            for nibble in [byte.wrapping_shr(NIBBLE_BITS), byte & LOW_NIBBLE] {
                if let Some(digit) = HEX.get(usize::from(nibble)) {
                    text.push(char::from(*digit));
                }
            }
        }
        text
    }

    /// The digest sixty-four lowercase hexadecimal digits spell, or `None`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (pairs, rest) = text.as_bytes().as_chunks::<DIGITS_PER_BYTE>();
        if !rest.is_empty() || pairs.len() != DIGEST_BYTES {
            return None;
        }
        let mut bytes = [0_u8; DIGEST_BYTES];
        for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
            *byte = nibble(*high)?.wrapping_shl(NIBBLE_BITS) | nibble(*low)?;
        }
        Some(Self(bytes))
    }
}

/// The value of one lowercase hexadecimal digit.
fn nibble(digit: u8) -> Option<u8> {
    HEX.iter()
        .zip(0_u8..)
        .find_map(|(symbol, value)| (*symbol == digit).then_some(value))
}

/// A name that is safe as a file name and a URL path segment: letters, digits, `.`, `_`, and `-`, not starting with `.`.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Name(String);

impl Name {
    /// The text as a name, or `None`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let plain = text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
        (plain && !text.is_empty() && text.len() <= LONGEST_NAME && !text.starts_with('.'))
            .then(|| Self(String::from(text)))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The asset that holds one target's program.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Asset {
    pub name: Name,
    pub size: u64,
    pub sha256: Digest,
}

/// A release: its version and the asset of each target triple it builds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Manifest {
    pub version: Version,
    pub assets: BTreeMap<Name, Asset>,
}

/// Why bytes are not a manifest.
#[derive(Debug)]
pub enum ManifestError {
    TooLong(usize),
    Json(serde_json::Error),
    Format(u32),
    Version(String),
    Name(String),
    Size(u64),
    Digest(String),
}

impl core::fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooLong(length) => write!(
                formatter,
                "a manifest of {length} bytes is longer than {LONGEST_MANIFEST}"
            ),
            Self::Json(error) => write!(formatter, "the manifest is not its JSON shape: {error}"),
            Self::Format(format) => {
                write!(formatter, "the manifest format {format} is not {FORMAT}")
            }
            Self::Version(text) => write!(formatter, "`{text}` is not a version"),
            Self::Name(text) => write!(formatter, "`{text}` is not a plain name"),
            Self::Size(size) => write!(
                formatter,
                "an asset of {size} bytes is larger than {LARGEST_ASSET}"
            ),
            Self::Digest(text) => write!(formatter, "`{text}` is not a SHA-256 digest"),
        }
    }
}

impl std::error::Error for ManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::TooLong(_)
            | Self::Format(_)
            | Self::Version(_)
            | Self::Name(_)
            | Self::Size(_)
            | Self::Digest(_) => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: u32,
    version: String,
    assets: BTreeMap<String, AssetDocument>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetDocument {
    name: String,
    size: u64,
    sha256: String,
}

/// The manifest as the bytes a release signs.
///
/// # Errors
///
/// Returns the serializer's error, which a document of strings and numbers never meets.
pub fn encode(manifest: &Manifest) -> Result<String, serde_json::Error> {
    let assets = manifest
        .assets
        .iter()
        .map(|(target, asset)| {
            let document = AssetDocument {
                name: String::from(asset.name.as_str()),
                size: asset.size,
                sha256: asset.sha256.hex(),
            };
            (String::from(target.as_str()), document)
        })
        .collect();
    let document = Document {
        format: FORMAT,
        version: manifest.version.to_string(),
        assets,
    };
    let mut text = serde_json::to_string_pretty(&document)?;
    text.push('\n');
    Ok(text)
}

fn name(text: String) -> Result<Name, ManifestError> {
    Name::parse(&text).ok_or(ManifestError::Name(text))
}

fn asset(document: AssetDocument) -> Result<Asset, ManifestError> {
    if document.size > LARGEST_ASSET {
        return Err(ManifestError::Size(document.size));
    }
    let sha256 = Digest::parse(&document.sha256).ok_or(ManifestError::Digest(document.sha256))?;
    Ok(Asset {
        name: name(document.name)?,
        size: document.size,
        sha256,
    })
}

/// The manifest that `bytes` hold.
///
/// # Errors
///
/// Returns a [`ManifestError`] for bytes beyond the length limit, any other shape, an unknown format, or a field outside its domain.
pub fn decode(bytes: &[u8]) -> Result<Manifest, ManifestError> {
    if bytes.len() > LONGEST_MANIFEST {
        return Err(ManifestError::TooLong(bytes.len()));
    }
    let document: Document = serde_json::from_slice(bytes).map_err(ManifestError::Json)?;
    if document.format != FORMAT {
        return Err(ManifestError::Format(document.format));
    }
    let version =
        Version::parse(&document.version).ok_or(ManifestError::Version(document.version))?;
    let mut assets = BTreeMap::new();
    for (target, entry) in document.assets {
        assets.insert(name(target)?, asset(entry)?);
    }
    Ok(Manifest { version, assets })
}

#[cfg(test)]
mod tests {
    use super::{
        Asset, Digest, LARGEST_ASSET, LONGEST_MANIFEST, Manifest, ManifestError, Name, decode,
        encode,
    };
    use std::collections::BTreeMap;
    use youarel_core::update_rules::Version;

    fn manifest() -> Manifest {
        let mut assets = BTreeMap::new();
        assets.insert(
            Name::parse("x86_64-unknown-linux-gnu").unwrap(),
            Asset {
                name: Name::parse("youarel-x86_64-unknown-linux-gnu").unwrap(),
                size: 3,
                sha256: Digest::of(b"new"),
            },
        );
        Manifest {
            version: Version::parse("1.2.3").unwrap(),
            assets,
        }
    }

    #[test]
    fn a_manifest_reads_back_as_written() {
        let written = encode(&manifest()).unwrap();
        assert_eq!(decode(written.as_bytes()).unwrap(), manifest());
        assert!(written.contains("\"version\": \"1.2.3\""), "{written}");
        assert!(written.ends_with("}\n"));
    }

    #[test]
    fn digests_and_names_parse_strictly() {
        let digest = Digest::of(b"new");
        assert_eq!(Digest::parse(&digest.hex()), Some(digest));
        assert_eq!(digest.hex().len(), 64);
        for text in [
            "",
            "0",
            &digest.hex().to_uppercase(),
            &format!("{}0", digest.hex()),
        ] {
            assert_eq!(Digest::parse(text), None, "{text}");
        }
        assert_eq!(
            Name::parse("youarel-1.0.0.tar.gz").unwrap().as_str(),
            "youarel-1.0.0.tar.gz"
        );
        for text in ["", ".hidden", "a/b", "a\\b", "a b", "ü", &"a".repeat(129)] {
            assert_eq!(Name::parse(text), None, "{text}");
        }
    }

    #[test]
    fn decoding_refuses_anything_encoding_never_writes() {
        let written = encode(&manifest()).unwrap();
        let cases = [
            written.replace("\"format\": 1", "\"format\": 2"),
            written.replace("1.2.3", "1.2"),
            written.replace("\"size\": 3", &format!("\"size\": {}", LARGEST_ASSET + 1)),
            written.replace("youarel-x86_64", "../youarel"),
            written.replace("x86_64-unknown", "x86 64-unknown"),
            written.replace(&Digest::of(b"new").hex(), "00"),
            written.replace("\"version\"", "\"extra\": 1, \"version\""),
            String::from("{}"),
        ];
        for case in &cases {
            assert!(decode(case.as_bytes()).is_err(), "{case}");
        }
        assert!(matches!(
            decode(&vec![b' '; LONGEST_MANIFEST + 1]),
            Err(ManifestError::TooLong(_))
        ));
        let malformed = decode(b"[").unwrap_err();
        assert!(std::error::Error::source(&malformed).is_some());
    }

    #[test]
    fn the_limits_admit_a_manifest_and_an_asset_of_their_full_size() {
        let written = encode(&manifest()).unwrap();
        let mut longest = written.clone().into_bytes();
        longest.resize(LONGEST_MANIFEST, b' ');
        assert_eq!(decode(&longest).unwrap(), manifest());
        let largest = written.replace("\"size\": 3", &format!("\"size\": {LARGEST_ASSET}"));
        let admitted = decode(largest.as_bytes()).unwrap();
        assert!(
            admitted
                .assets
                .values()
                .all(|asset| asset.size == LARGEST_ASSET)
        );
        assert_eq!(LONGEST_MANIFEST, 64 * 1024);
        assert_eq!(LARGEST_ASSET, 256 * 1024 * 1024);
    }
}
