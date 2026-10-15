//! Crockford's base32, which a person reads and types: five bits a symbol, without I, L, O, and U, and forgiving of case and of the letters people confuse with digits.

/// The symbols, in the order of their values.
pub const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// The bits one symbol spells, and the mask of one symbol's value.
const BITS: u32 = 5;
const MASK: u32 = 0b1_1111;
/// The bits of a byte, and the mask of one byte.
const BYTE: u32 = 8;
const BYTE_MASK: u32 = 0xff;

/// The value of one symbol, reading `I` and `L` as `1`, `O` as `0`, and either case.
#[must_use]
pub fn value(symbol: u8) -> Option<u8> {
    let canonical = match symbol.to_ascii_uppercase() {
        b'I' | b'L' => b'1',
        b'O' => b'0',
        other => other,
    };
    ALPHABET
        .iter()
        .zip(0_u8..)
        .find_map(|(candidate, value)| (*candidate == canonical).then_some(value))
}

/// The symbols that spell `bytes`, most significant bit first, with the last symbol padded by zero bits.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    let mut text = String::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    let mut push = |index: u32| {
        if let Some(symbol) =
            usize::try_from(index & MASK).map_or(None, |index| ALPHABET.get(index))
        {
            text.push(char::from(*symbol));
        }
    };
    for byte in bytes {
        buffer = buffer.wrapping_shl(BYTE) | u32::from(*byte);
        bits = bits.saturating_add(BYTE);
        while bits >= BITS {
            bits = bits.saturating_sub(BITS);
            push(buffer.wrapping_shr(bits));
        }
        buffer &= (1_u32.wrapping_shl(bits)).wrapping_sub(1);
    }
    if bits > 0 {
        push(buffer.wrapping_shl(BITS.saturating_sub(bits)));
    }
    text
}

/// The `N` bytes that `symbols` spell, when they spell exactly that many and pad with zero bits alone.
#[must_use]
pub fn decode<const N: usize>(symbols: &[u8]) -> Option<[u8; N]> {
    let mut bytes = [0_u8; N];
    let mut filled = 0_usize;
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for symbol in symbols {
        buffer = buffer.wrapping_shl(BITS) | u32::from(value(*symbol)?);
        bits = bits.saturating_add(BITS);
        if bits >= BYTE {
            bits = bits.saturating_sub(BYTE);
            let Ok(byte) = u8::try_from(buffer.wrapping_shr(bits) & BYTE_MASK) else {
                return None;
            };
            *bytes.get_mut(filled)? = byte;
            filled = filled.saturating_add(1);
            buffer &= (1_u32.wrapping_shl(bits)).wrapping_sub(1);
        }
    }
    (filled == N && buffer == 0 && bits < BITS).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::{decode, encode, value};

    #[test]
    fn bytes_read_back_from_their_symbols_and_nothing_else_does() {
        let bytes: [u8; 7] = [0, 1, 0x7f, 0x80, 0xfe, 0xff, 0x42];
        let text = encode(&bytes);
        assert_eq!(text.len(), 12);
        assert_eq!(decode::<7>(text.as_bytes()), Some(bytes));
        assert_eq!(decode::<7>(text.to_lowercase().as_bytes()), Some(bytes));
        assert_eq!(encode(&[0xff]), "ZW");
        assert_eq!(decode::<1>(b"ZW"), Some([0xff]));
        assert_eq!(decode::<1>(b"ZZ"), None, "padding bits must be zero");
        assert_eq!(decode::<1>(b"Z"), None);
        assert_eq!(decode::<1>(b"ZW0"), None);
        assert_eq!(decode::<1>(b"U0"), None);
        assert_eq!(encode(&[]), "");
        assert_eq!(
            encode(&[0; 5]),
            "00000000",
            "forty bits need no padding symbol"
        );
        assert_eq!(decode::<5>(b"00000000"), Some([0; 5]));
        assert_eq!(
            decode::<5>(b"000000000"),
            None,
            "a symbol past the last byte spells nothing"
        );
        assert_eq!(value(b'l'), Some(1));
        assert_eq!(value(b'o'), Some(0));
        assert_eq!(value(b'u'), None);
    }
}
