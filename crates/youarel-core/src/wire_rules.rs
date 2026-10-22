//! The primitive values of `specs/WIRE.md`: big-endian integers, varints, fixed arrays, and length-prefixed bytes and lists.
//!
//! A reader refuses a length beyond the message, trailing bytes, and a varint longer than its shortest form, so every message has one encoding.
//! A sync message arrives from any carrier, so the reader is the first code an attacker's bytes reach.

use alloc::vec::Vec;

/// The most bytes a varint takes: ten groups of seven bits hold the sixty-four bits of a `u64`.
pub const VARINT_MOST_BYTES: usize = 10;
/// The bits of a varint's byte that carry its value.
const GROUP: u8 = 0x7f;
/// The bit of a varint's byte that another byte follows.
const CONTINUES: u8 = 0x80;
/// The bits each byte of a varint carries.
const GROUP_BITS: u32 = 7;
/// Where each byte's group lands in the value, from the least significant.
const GROUP_SHIFTS: [u32; VARINT_MOST_BYTES] = [0, 7, 14, 21, 28, 35, 42, 49, 56, 63];

/// Why bytes are not a message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireError {
    /// The bytes end inside a value.
    Truncated,
    /// Bytes follow the message.
    Trailing,
    /// The message starts with another magic.
    Magic,
    /// The message names a version this reader does not know.
    Version(u16),
    /// A tag names no known variant.
    Tag(u8),
    /// A list holds its items out of their strictly ascending order.
    Order,
    /// A length or a count exceeds its limit.
    Length,
    /// A value stands in another form than the one its writer gives it, such as a varint longer than its shortest form.
    Canonical,
    /// A value lies outside the values its field admits, such as a varint beyond sixty-four bits.
    Value,
}

impl core::fmt::Display for WireError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("the message ends inside a value"),
            Self::Trailing => formatter.write_str("bytes follow the message"),
            Self::Magic => formatter.write_str("the message is of another kind"),
            Self::Version(version) => write!(
                formatter,
                "the message has version {version}, which this youarel does not read"
            ),
            Self::Tag(tag) => write!(formatter, "the tag {tag} names nothing"),
            Self::Order => formatter.write_str("a list is out of order"),
            Self::Length => formatter.write_str("a length exceeds its limit"),
            Self::Canonical => formatter.write_str("a value is not in the one form youarel writes"),
            Self::Value => formatter.write_str("a value lies outside what its field holds"),
        }
    }
}

impl core::error::Error for WireError {}

/// A cursor over the bytes of one message.
#[derive(Clone, Copy, Debug)]
pub struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { rest: bytes }
    }

    /// The next `length` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] when fewer remain.
    pub fn take(&mut self, length: usize) -> Result<&'a [u8], WireError> {
        let (taken, rest) = self
            .rest
            .split_at_checked(length)
            .ok_or(WireError::Truncated)?;
        self.rest = rest;
        Ok(taken)
    }

    /// The next `N` bytes as an array.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] when fewer remain.
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let mut array = [0_u8; N];
        array.copy_from_slice(self.take(N)?);
        Ok(array)
    }

    /// The next byte.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] at the end.
    pub fn byte(&mut self) -> Result<u8, WireError> {
        Ok(u8::from_be_bytes(self.array()?))
    }

    /// The next big-endian `u16`.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] when fewer bytes remain.
    pub fn short(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    /// The next big-endian `u32`.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] when fewer bytes remain.
    pub fn word(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    /// The next big-endian `u64`.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] when fewer bytes remain.
    pub fn long(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    /// The next varint: a `u64` in groups of seven bits, the least significant first, each byte but the last marked as followed.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Truncated`] when the bytes end before the varint does, [`WireError::Canonical`] for a form longer than the shortest, and [`WireError::Value`] for one beyond sixty-four bits.
    pub fn varint(&mut self) -> Result<u64, WireError> {
        let mut value: u64 = 0;
        for shift in GROUP_SHIFTS {
            let byte = self.byte()?;
            let group = u64::from(byte & GROUP);
            let placed = group.wrapping_shl(shift);
            // The last byte's group holds the one bit left of sixty-four; a greater group passes them.
            if placed.wrapping_shr(shift) != group {
                return Err(WireError::Value);
            }
            value |= placed;
            if byte & CONTINUES == 0 {
                // A last byte of zero after another adds nothing, so the shorter form says the same.
                return if byte == 0 && shift > 0 {
                    Err(WireError::Canonical)
                } else {
                    Ok(value)
                };
            }
        }
        Err(WireError::Value)
    }

    /// A `u32` count, no greater than `limit`.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Length`] for a greater count.
    pub fn count(&mut self, limit: usize) -> Result<usize, WireError> {
        let count = usize::try_from(self.word()?).map_err(|_huge| WireError::Length)?;
        if count > limit {
            return Err(WireError::Length);
        }
        Ok(count)
    }

    /// Length-prefixed bytes, no more than `limit` of them.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Length`] for a longer field and [`WireError::Truncated`] when the message ends first.
    pub fn bytes(&mut self, limit: usize) -> Result<&'a [u8], WireError> {
        let length = self.count(limit)?;
        self.take(length)
    }

    /// How many bytes the message holds after what the reader read.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.rest.len()
    }

    /// Ends the message.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Trailing`] when bytes remain.
    pub const fn finish(self) -> Result<(), WireError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(WireError::Trailing)
        }
    }
}

/// The bytes of one message as it is written.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    #[must_use]
    pub const fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    pub fn byte(&mut self, value: u8) -> &mut Self {
        self.raw(&value.to_be_bytes())
    }

    pub fn short(&mut self, value: u16) -> &mut Self {
        self.raw(&value.to_be_bytes())
    }

    pub fn word(&mut self, value: u32) -> &mut Self {
        self.raw(&value.to_be_bytes())
    }

    pub fn long(&mut self, value: u64) -> &mut Self {
        self.raw(&value.to_be_bytes())
    }

    /// A varint in its shortest form, which [`Reader::varint`] reads back.
    pub fn varint(&mut self, value: u64) -> &mut Self {
        let (form, length) = varint_form(value);
        self.raw(form.get(..length).unwrap_or_default())
    }

    /// A `u32` count.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Length`] for a count beyond 32 bits.
    pub fn count(&mut self, count: usize) -> Result<&mut Self, WireError> {
        let count = u32::try_from(count).map_err(|_huge| WireError::Length)?;
        Ok(self.word(count))
    }

    /// Length-prefixed bytes.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Length`] for a field beyond 32 bits of length.
    pub fn bytes(&mut self, bytes: &[u8]) -> Result<&mut Self, WireError> {
        self.count(bytes.len())?;
        Ok(self.raw(bytes))
    }

    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// The shortest varint of `value` in a buffer, and how many of its bytes it takes.
fn varint_form(value: u64) -> ([u8; VARINT_MOST_BYTES], usize) {
    let mut form = [0_u8; VARINT_MOST_BYTES];
    let mut rest = value;
    let mut length: usize = 0;
    for slot in &mut form {
        *slot = rest.to_le_bytes().first().map_or(0, |low| low & GROUP);
        rest = rest.wrapping_shr(GROUP_BITS);
        length = length.saturating_add(1);
        if rest == 0 {
            break;
        }
        *slot |= CONTINUES;
    }
    (form, length)
}

/// Whether keyed items stand in strictly ascending order of their keys.
pub fn ascending<T: Ord>(items: impl IntoIterator<Item = T>) -> bool {
    let mut previous: Option<T> = None;
    for item in items {
        if previous.as_ref().is_some_and(|before| *before >= item) {
            return false;
        }
        previous = Some(item);
    }
    true
}

/// The properties Kani proves of the reader over every message of a few bytes, and of the order check over every short list.
#[cfg(kani)]
mod proofs {
    use super::{Reader, VARINT_MOST_BYTES, WireError, Writer, ascending, varint_form};

    /// The bytes of the message a harness reads, and of a length prefix.
    const MESSAGE: usize = 8;
    const PREFIX: usize = 4;

    /// Every scan and comparison covers at most the message's eight bytes.
    #[kani::proof]
    #[kani::unwind(9)]
    fn a_field_never_passes_its_limit_or_the_message() {
        let message: [u8; MESSAGE] = kani::any();
        let limit: usize = kani::any();
        kani::assume(limit <= MESSAGE);
        let [first, second, third, fourth, ..] = message;
        let declared = usize::try_from(u32::from_be_bytes([first, second, third, fourth]))
            .unwrap_or(usize::MAX);
        let mut reader = Reader::new(&message);
        match reader.bytes(limit) {
            Ok(field) => {
                let length = field.len();
                kani::assert(
                    length == declared && length <= limit && PREFIX + length <= MESSAGE,
                    "a field is as long as its prefix declares, within its limit and the message",
                );
                kani::assert(
                    message.get(PREFIX..PREFIX + length) == Some(field),
                    "a field holds the bytes after its prefix",
                );
                kani::assert(
                    reader.rest.len() == MESSAGE - PREFIX - length,
                    "the reader moves past the field",
                );
                kani::cover!(length > 0, "a nonempty field");
            }
            Err(WireError::Length) => {
                kani::assert(declared > limit, "only a longer declaration is too long");
            }
            Err(WireError::Truncated) => {
                kani::assert(
                    declared <= limit && PREFIX + declared > MESSAGE,
                    "only a field past the end is truncated",
                );
                kani::cover!(true, "a truncated field");
            }
            Err(
                WireError::Trailing
                | WireError::Magic
                | WireError::Version(_)
                | WireError::Tag(_)
                | WireError::Order
                | WireError::Canonical
                | WireError::Value,
            ) => kani::assert(false, "a field fails only as too long or truncated"),
        }
    }

    /// The written message holds twenty-two bytes, which bounds every copy.
    #[kani::proof]
    #[kani::unwind(23)]
    fn every_value_reads_back_as_written() {
        let byte: u8 = kani::any();
        let short: u16 = kani::any();
        let word: u32 = kani::any();
        let long: u64 = kani::any();
        let field: [u8; 3] = kani::any();
        let length: usize = kani::any();
        kani::assume(length <= field.len());
        let Some(bytes) = field.get(..length) else {
            return;
        };
        let mut writer = Writer::new();
        writer.byte(byte).short(short).word(word).long(long);
        kani::assert(writer.bytes(bytes).is_ok(), "a short field writes");
        let written = writer.finish();
        let mut reader = Reader::new(&written);
        kani::assert(
            reader.byte() == Ok(byte)
                && reader.short() == Ok(short)
                && reader.word() == Ok(word)
                && reader.long() == Ok(long),
            "every integer reads back",
        );
        kani::assert(
            reader.bytes(field.len()) == Ok(bytes),
            "the field reads back",
        );
        kani::assert(reader.finish().is_ok(), "nothing follows");
        kani::cover!(length == field.len(), "a full field");
    }

    #[kani::proof]
    #[kani::unwind(4)]
    fn ascending_holds_exactly_for_strictly_increasing_keys() {
        let keys: [u8; 3] = kani::any();
        let [first, second, third] = keys;
        kani::assert(
            ascending(keys) == (first < second && second < third),
            "three keys ascend exactly when each is greater than the one before",
        );
        kani::assert(
            ascending([first, second]) == (first < second),
            "two keys ascend exactly when the second is greater",
        );
        kani::assert(
            ascending([first]) && ascending::<u8>([]),
            "no key or one ascends",
        );
        kani::cover!(ascending(keys), "three ascending keys");
    }

    /// Every `u64` writes one form of one to ten bytes that reads back whole, every shorter part of the form is truncated, and any ten bytes that read as a value begin with that value's own form.
    ///
    /// A reader never looks past a varint's tenth byte, so ten bytes stand for every longer input, and each loop runs at most ten times.
    #[kani::proof]
    #[kani::unwind(11)]
    fn every_varint_reads_back_canonically() {
        let value: u64 = kani::any();
        let mut writer = Writer::new();
        writer.varint(value);
        let written = writer.finish();
        let length = written.len();
        let mut reader = Reader::new(&written);
        kani::assert(
            reader.varint() == Ok(value) && reader.finish().is_ok(),
            "a varint reads back as written, and nothing follows it",
        );
        kani::assert(
            (1..=VARINT_MOST_BYTES).contains(&length),
            "a varint takes one to ten bytes",
        );
        let cut: usize = kani::any();
        kani::assume(cut < length);
        kani::assert(
            written
                .get(..cut)
                .is_some_and(|part| Reader::new(part).varint() == Err(WireError::Truncated)),
            "a varint cut short is truncated",
        );
        let bytes: [u8; VARINT_MOST_BYTES] = kani::any();
        let mut any = Reader::new(&bytes);
        let read = any.varint();
        let used = VARINT_MOST_BYTES - any.rest.len();
        match read {
            Ok(found) => {
                let (form, form_length) = varint_form(found);
                kani::assert(
                    used == form_length && bytes.get(..used) == form.get(..used),
                    "ten bytes read as a value only from that value's own form",
                );
            }
            Err(error) => kani::assert(
                matches!(error, WireError::Canonical | WireError::Value),
                "ten bytes fail only as a longer form than the shortest or beyond sixty-four bits",
            ),
        }
        kani::cover!(length == VARINT_MOST_BYTES, "the longest form");
        kani::cover!(
            read == Err(WireError::Canonical),
            "a longer form than the shortest"
        );
        kani::cover!(
            read == Err(WireError::Value),
            "a varint beyond sixty-four bits"
        );
    }

    /// A field longer than its limit would make a reader hold more than it allows; this probe claims one reads and must fail.
    #[kani::proof]
    #[kani::unwind(10)]
    fn rejects_a_field_longer_than_its_limit() {
        let message = [0, 0, 0, 5, 1, 2, 3, 4, 5];
        kani::assert(
            Reader::new(&message).bytes(PREFIX).is_ok(),
            "NEGATIVE_WIRE_CONTROL",
        );
    }

    /// A varint whose last group is zero would give its number a second form; this probe claims the longer form of one reads and must fail.
    #[kani::proof]
    #[kani::unwind(11)]
    fn rejects_an_overlong_varint() {
        let overlong = [0x81, 0x00];
        kani::assert(
            Reader::new(&overlong).varint() == Ok(1),
            "NEGATIVE_WIRE_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Reader, VARINT_MOST_BYTES, WireError, Writer, ascending, varint_form};
    use alloc::string::ToString as _;
    use alloc::vec::Vec;

    fn written(value: u64) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.varint(value);
        writer.finish()
    }

    #[test]
    fn a_varint_takes_seven_bits_a_byte_the_least_significant_first() {
        let known: [(u64, &[u8]); 9] = [
            (0, &[0x00]),
            (1, &[0x01]),
            (0x7f, &[0x7f]),
            (0x80, &[0x80, 0x01]),
            (300, &[0xac, 0x02]),
            (0x3fff, &[0xff, 0x7f]),
            (0x4000, &[0x80, 0x80, 0x01]),
            (
                1 << 63,
                &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01],
            ),
            (
                u64::MAX,
                &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01],
            ),
        ];
        for (value, form) in known {
            assert_eq!(written(value), form, "{value}");
            let mut reader = Reader::new(form);
            assert_eq!(reader.varint(), Ok(value), "{value}");
            assert_eq!(reader.finish(), Ok(()), "{value}");
        }
        assert_eq!(written(u64::MAX).len(), VARINT_MOST_BYTES);
        let mut followed = Reader::new(&[0x05, 0x07]);
        assert_eq!(followed.varint(), Ok(5));
        assert_eq!(followed.byte(), Ok(7));
    }

    #[test]
    fn a_varint_longer_than_its_shortest_form_or_beyond_sixty_four_bits_is_refused() {
        let mut nine_and_zero = [0x80; VARINT_MOST_BYTES];
        nine_and_zero[VARINT_MOST_BYTES - 1] = 0x00;
        for longer in [&[0x80, 0x00][..], &[0x81, 0x80, 0x00], &nine_and_zero] {
            assert_eq!(
                Reader::new(longer).varint(),
                Err(WireError::Canonical),
                "{longer:?}"
            );
        }
        let mut sixty_five_bits = [0xff; VARINT_MOST_BYTES];
        sixty_five_bits[VARINT_MOST_BYTES - 1] = 0x02;
        let mut eleven_bytes = [0x80; VARINT_MOST_BYTES];
        eleven_bytes[VARINT_MOST_BYTES - 1] = 0x81;
        for beyond in [
            sixty_five_bits,
            eleven_bytes,
            [0x80; VARINT_MOST_BYTES],
            [0xff; VARINT_MOST_BYTES],
        ] {
            assert_eq!(
                Reader::new(&beyond).varint(),
                Err(WireError::Value),
                "{beyond:?}"
            );
        }
        for cut in [&[][..], &[0x80], &[0xff; VARINT_MOST_BYTES - 1]] {
            assert_eq!(
                Reader::new(cut).varint(),
                Err(WireError::Truncated),
                "{cut:?}"
            );
        }
        assert_ne!(
            WireError::Canonical.to_string(),
            WireError::Value.to_string()
        );
    }

    #[test]
    fn every_two_bytes_read_as_a_value_only_from_its_own_form() {
        for first in 0..=u8::MAX {
            for second in 0..=u8::MAX {
                let bytes = [first, second];
                let expected = if first & 0x80 == 0 {
                    Ok((u64::from(first), 1))
                } else if second & 0x80 != 0 {
                    Err(WireError::Truncated)
                } else if second == 0 {
                    Err(WireError::Canonical)
                } else {
                    Ok((u64::from(first & 0x7f) | (u64::from(second) << 7), 2))
                };
                let mut reader = Reader::new(&bytes);
                let read = reader
                    .varint()
                    .map(|value| (value, bytes.len() - reader.rest.len()));
                assert_eq!(read, expected, "{bytes:?}");
                if let Ok((value, used)) = read {
                    let (form, length) = varint_form(value);
                    assert_eq!(form.get(..length), bytes.get(..used), "{bytes:?}");
                }
            }
        }
    }

    #[test]
    fn values_read_back_as_written_and_nothing_more() {
        let mut writer = Writer::new();
        writer.byte(1).short(2).word(3).long(4);
        writer.bytes(b"abc").unwrap();
        writer.raw(&[9; 2]);
        let bytes = writer.finish();
        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.byte(), Ok(1));
        assert_eq!(reader.short(), Ok(2));
        assert_eq!(reader.word(), Ok(3));
        assert_eq!(reader.long(), Ok(4));
        assert_eq!(reader.bytes(3), Ok(&b"abc"[..]));
        assert_eq!(reader.array::<2>(), Ok([9, 9]));
        assert_eq!(reader.finish(), Ok(()));
        let mut short = Reader::new(&bytes);
        assert_eq!(short.take(bytes.len() + 1), Err(WireError::Truncated));
        let mut trailing = Reader::new(&bytes);
        assert_eq!(trailing.remaining(), bytes.len());
        trailing.byte().unwrap();
        assert_eq!(trailing.remaining(), bytes.len() - 1);
        assert_eq!(trailing.finish(), Err(WireError::Trailing));
    }

    #[test]
    fn limits_and_order_are_enforced() {
        let mut writer = Writer::new();
        writer.bytes(b"abcd").unwrap();
        let bytes = writer.finish();
        assert_eq!(Reader::new(&bytes).bytes(3), Err(WireError::Length));
        assert_eq!(Reader::new(&bytes).bytes(4), Ok(&b"abcd"[..]));
        assert_eq!(
            Reader::new(&[0, 0, 0, 9, 1]).bytes(9),
            Err(WireError::Truncated)
        );
        assert!(ascending([1, 2, 3]));
        assert!(!ascending([1, 1]));
        assert!(!ascending([2, 1]));
        assert!(ascending(Vec::<u8>::new()));
    }
}
