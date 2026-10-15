//! The primitive values of `specs/WIRE.md`: big-endian integers, fixed arrays, and length-prefixed bytes and lists.
//!
//! A reader refuses a length beyond the message and trailing bytes, so every message has one encoding.
//! A sync message arrives from any carrier, so the reader is the first code an attacker's bytes reach.

use alloc::vec::Vec;

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
    use super::{Reader, WireError, Writer, ascending};

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
                | WireError::Order,
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
}

#[cfg(test)]
mod tests {
    use super::{Reader, WireError, Writer, ascending};
    use alloc::vec::Vec;

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
        trailing.byte().unwrap();
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
