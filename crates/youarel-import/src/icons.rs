//! Bounded reads of the browser's local favicon databases.

mod sqlite;
#[cfg(test)]
mod tests;

pub use sqlite::Snapshot;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    Chromium,
    Firefox,
    Safari,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Image {
    Bytes(Vec<u8>),
    SafariFile(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub quality: youarel_core::icon_rules::Quality,
    pub image: Image,
}

#[derive(Debug)]
pub enum CacheError {
    Sqlite(rusqlite::Error),
    Temporary(std::io::Error),
    Stopped,
    TooLarge,
    Busy,
    Unsupported,
}

impl core::fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "the icon cache cannot be read: {error}"),
            Self::Temporary(error) => {
                write!(formatter, "the icon snapshot cannot be made: {error}")
            }
            Self::Stopped => formatter.write_str("the icon cache read was stopped"),
            Self::TooLarge => formatter.write_str("the icon cache passes the snapshot limit"),
            Self::Busy => formatter.write_str("the browser's icon cache is busy; try again later"),
            Self::Unsupported => {
                formatter.write_str("the browser's icon cache has an unsupported schema")
            }
        }
    }
}

impl std::error::Error for CacheError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Temporary(error) => Some(error),
            Self::Stopped | Self::TooLarge | Self::Busy | Self::Unsupported => None,
        }
    }
}

impl From<rusqlite::Error> for CacheError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

#[must_use]
pub fn safari_file(uuid: &str) -> String {
    use md5::{Digest as _, Md5};
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    const NIBBLE_BITS: u8 = 4;
    const NIBBLE_MASK: u8 = 0x0f;
    let mut name = String::new();
    for byte in Md5::digest(uuid.as_bytes()) {
        for nibble in [byte >> NIBBLE_BITS, byte & NIBBLE_MASK] {
            if let Some(digit) = HEX.get(usize::from(nibble)) {
                name.push(char::from(*digit));
            }
        }
    }
    name
}
