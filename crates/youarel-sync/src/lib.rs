//! The sync of `specs/PROTOCOL.md`: a device's account, its membership log and epoch keys, the pairing of new devices, and the bundles that carry the state.
//!
//! Every bundle travels as a file through any channel: a USB drive, a messenger, or a shared folder.
//! The decisions are the core's; this crate checks signatures, seals and opens, and keeps the account's files.

pub mod account;

use youarel_crypto::CryptoError;
use youarel_protocol::wire::WireError;
use youarel_store::fs::FileError;

/// Why a sync step failed.
#[derive(Debug)]
pub enum SyncError {
    File(FileError),
    Wire(WireError),
    Crypto(CryptoError),
    /// The step does not fit the account's state, such as a bundle of another account.
    Refused(String),
}

impl core::fmt::Display for SyncError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::File(error) => write!(formatter, "{error}"),
            Self::Wire(error) => write!(formatter, "{error}"),
            Self::Crypto(error) => write!(formatter, "{error}"),
            Self::Refused(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for SyncError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::File(error) => std::error::Error::source(error),
            Self::Wire(error) => Some(error),
            Self::Crypto(error) => Some(error),
            Self::Refused(_) => None,
        }
    }
}

/// The lowercase hexadecimal digits, each at the place of its value.
const HEX: &[u8; 16] = b"0123456789abcdef";
/// The bits one hexadecimal digit writes.
const NIBBLE_BITS: u32 = 4;
/// The low half of a byte, which its second digit writes.
const LOW_NIBBLE: u8 = 0x0f;
/// The hexadecimal digits that write one byte.
const DIGITS_PER_BYTE: usize = 2;

/// Bytes as lowercase hexadecimal digits, which name devices and operations in file names and messages.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
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

/// The bytes that lowercase hexadecimal digits spell, or `None`.
#[must_use]
pub fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let (pairs, rest) = text.as_bytes().as_chunks::<DIGITS_PER_BYTE>();
    if !rest.is_empty() || pairs.len() != N {
        return None;
    }
    let value = |digit: u8| {
        HEX.iter()
            .zip(0_u8..)
            .find_map(|(symbol, value)| (*symbol == digit).then_some(value))
    };
    let mut bytes = [0_u8; N];
    for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
        *byte = value(*high)?.wrapping_shl(NIBBLE_BITS) | value(*low)?;
    }
    Some(bytes)
}

impl From<FileError> for SyncError {
    fn from(error: FileError) -> Self {
        Self::File(error)
    }
}

impl From<WireError> for SyncError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

impl From<CryptoError> for SyncError {
    fn from(error: CryptoError) -> Self {
        Self::Crypto(error)
    }
}

#[cfg(test)]
mod tests {
    use super::{SyncError, WireError, hex, unhex};

    #[test]
    fn hexadecimal_reads_back_and_refuses_any_other_length() {
        assert_eq!(hex(&[0xab, 0x01]), "ab01");
        assert_eq!(unhex::<2>("ab01"), Some([0xab, 0x01]));
        for refused in ["abc", "abcd", "a", "", "AB", "zz"] {
            assert_eq!(unhex::<1>(refused), None, "{refused}");
        }
    }

    #[test]
    fn an_error_names_its_cause() {
        let wire = SyncError::Wire(WireError::Truncated);
        assert!(std::error::Error::source(&wire).is_some());
        let refused = SyncError::Refused(String::from("no"));
        assert!(std::error::Error::source(&refused).is_none());
    }
}
