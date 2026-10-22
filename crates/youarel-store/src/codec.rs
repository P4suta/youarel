//! The text form of the store's identifiers, and the refusals of what this youarel would not have written.
//!
//! An identifier reads back only from the form it writes: two lowercase hexadecimal digits a byte.

use youarel_core::wire_rules::WireError;

/// Why a text or the bytes of a state hold what this youarel would not have written.
#[derive(Debug)]
pub enum DecodeError {
    /// A text holds no identifier of its length in lowercase hexadecimal.
    Identifier(String),
    /// A link is not one a shortcut may hold.
    Link(String),
    /// The bytes are not the binary records of a state.
    Records(WireError),
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Identifier(text) => write!(formatter, "`{text}` is not an identifier"),
            Self::Link(text) => write!(formatter, "`{text}` is not a link a shortcut may hold"),
            Self::Records(_) => formatter.write_str("not a youarel state"),
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Records(error) => Some(error),
            Self::Identifier(_) | Self::Link(_) => None,
        }
    }
}

/// The lowercase hexadecimal digits, each at the place of its value.
const HEX: &[u8; 16] = b"0123456789abcdef";
/// The hexadecimal digits that write one byte.
const DIGITS_PER_BYTE: usize = 2;
/// The bits one hexadecimal digit writes.
const NIBBLE_BITS: u32 = 4;
/// The low half of a byte, which its second digit writes.
const LOW_NIBBLE: u8 = 0x0f;

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

#[cfg(test)]
mod tests {
    use super::{DecodeError, hex, unhex};
    use youarel_core::wire_rules::WireError;

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
    fn each_refusal_names_what_it_refused() {
        let identifier = unhex("zz").unwrap_err();
        assert_eq!(identifier.to_string(), "`zz` is not an identifier");
        assert!(std::error::Error::source(&identifier).is_none());
        let link = DecodeError::Link("file:///etc/passwd".to_owned());
        assert_eq!(
            link.to_string(),
            "`file:///etc/passwd` is not a link a shortcut may hold"
        );
        assert!(std::error::Error::source(&link).is_none());
        let records = DecodeError::Records(WireError::Magic);
        assert_eq!(records.to_string(), "not a youarel state");
        assert!(std::error::Error::source(&records).is_some());
    }
}
