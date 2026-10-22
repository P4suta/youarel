//! The hash that the store's index of links files each shortcut under, so the shortcut that holds a link turns up without a read of every shortcut.
//!
//! The hash is 64-bit FNV-1a: each byte joins the state by exclusive or, and the state then takes a multiplication by an odd prime.
//! Both operations undo, so texts of one length that differ in a single byte never share a hash; texts that differ more may, so a reader compares the link itself.

/// The state before any byte.
pub const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// The odd prime that absorbing each byte multiplies by.
pub const PRIME: u64 = 0x0000_0100_0000_01b3;

/// The state after it absorbs one more byte.
#[must_use]
pub fn absorb(state: u64, byte: u8) -> u64 {
    (state ^ u64::from(byte)).wrapping_mul(PRIME)
}

/// The state after more bytes.
#[must_use]
pub fn extend(state: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(state, |state, byte| absorb(state, *byte))
}

/// The hash of a text.
#[must_use]
pub fn hash(bytes: &[u8]) -> u64 {
    extend(OFFSET, bytes)
}

/// The properties Kani proves: absorbing tells every two bytes apart from every state, and a hash continues over a joined text.
///
/// Absorbing also tells two states apart, since a multiplication modulo two to the sixty-fourth undoes by the inverse of [`PRIME`], whose product with the prime the tests check is one.
#[cfg(any(test, kani))]
mod proofs {
    #[cfg(test)]
    use super::PRIME;
    #[cfg(kani)]
    use super::{OFFSET, absorb, extend, hash};

    /// The inverse of [`PRIME`] modulo two to the sixty-fourth.
    #[cfg(test)]
    pub(super) const INVERSE: u64 = inverse(PRIME);

    /// The inverse of an odd number modulo two to the sixty-fourth, by Newton's iteration, which doubles the correct low bits each round from the three that the number itself gets right.
    #[cfg(test)]
    const fn inverse(odd: u64) -> u64 {
        let mut inverse = odd;
        let mut round = 0_u32;
        while round < 6 {
            inverse = inverse.wrapping_mul(2_u64.wrapping_sub(odd.wrapping_mul(inverse)));
            round = round.wrapping_add(1);
        }
        inverse
    }

    /// The most bytes of each part of a joined text the harness draws.
    #[cfg(kani)]
    const PART: usize = 2;

    /// Every state and every pair of bytes: absorbing tells two bytes apart, so texts of one length that differ in one byte part at that byte.
    #[cfg(kani)]
    #[kani::proof]
    fn absorbing_tells_two_bytes_apart() {
        let state: u64 = kani::any();
        let byte: u8 = kani::any();
        let another: u8 = kani::any();
        kani::assert(
            (absorb(state, byte) == absorb(state, another)) == (byte == another),
            "absorbing tells two bytes apart",
        );
        kani::cover!(byte != another, "two bytes");
    }

    /// Every text of every length up to two joined to another of every length up to two, of every byte: the hash of the joined text continues the hash of its start.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn a_hash_continues_over_a_joined_text() {
        let start: [u8; PART] = kani::any();
        let rest: [u8; PART] = kani::any();
        for front in 0..=PART {
            for back in 0..=PART {
                let mut joined = [0_u8; 2 * PART];
                joined[..front].copy_from_slice(&start[..front]);
                joined[front..front + back].copy_from_slice(&rest[..back]);
                kani::assert(
                    hash(&joined[..front + back]) == extend(hash(&start[..front]), &rest[..back]),
                    "a hash continues over a joined text",
                );
            }
        }
        kani::assert(hash(&[]) == OFFSET, "the empty text keeps the offset");
        kani::cover!(start != rest, "two different parts");
    }

    /// A hash that passed over a byte would file two links of one host as one; this probe claims the hash of two bytes is the hash of the second alone and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_hash_that_passes_over_a_byte() {
        let first: u8 = kani::any();
        let second: u8 = kani::any();
        kani::assert(
            hash(&[first, second]) == hash(&[second]),
            "NEGATIVE_HASH_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::INVERSE;
    use super::{OFFSET, PRIME, absorb, extend, hash};

    #[test]
    fn the_hash_is_fnv_1a_of_the_text() {
        assert_eq!(hash(b""), OFFSET);
        assert_eq!(hash(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(hash(b"foobar"), 0x8594_4171_f739_67e8);
        assert_eq!(absorb(OFFSET, b'a'), hash(b"a"));
        assert_eq!(extend(hash(b"foo"), b"bar"), hash(b"foobar"));
        assert_eq!(
            PRIME.wrapping_mul(INVERSE),
            1,
            "the prime has an inverse, so a multiplication undoes"
        );
        for state in [0, 1, OFFSET, u64::MAX, 0x8000_0000_0000_0000] {
            for byte in [0, 1, b'a', u8::MAX] {
                assert_eq!(
                    absorb(state, byte).wrapping_mul(INVERSE) ^ u64::from(byte),
                    state,
                    "absorbing a byte undoes"
                );
            }
        }
    }

    #[test]
    fn texts_of_one_length_that_differ_in_one_byte_hash_apart() {
        let base = *b"https://example.com/";
        for place in 0..base.len() {
            for byte in 0..=u8::MAX {
                let mut changed = base;
                changed[place] = byte;
                assert_eq!(
                    hash(&changed) == hash(&base),
                    changed == base,
                    "{place} {byte}"
                );
            }
        }
    }
}
