//! Positions among siblings as fractional-index keys, so an item moves by one write and never renumbers the others.
//!
//! A key is a nonempty string over [`ALPHABET`] that never ends with its first symbol, and keys order as strings.
//! Between any two keys, and before or after any key, there is always another key.
//! Two devices that place an item at the same spot may write the same key, which the item identifier then orders.

use alloc::string::String;
use alloc::vec::Vec;

/// The symbols of a key, in ascending order of both their digit value and their byte value.
pub const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// The digit one past the last symbol, which stands for the open upper end.
const BASE: u8 = 62;
/// The digit value of `A`, which follows the ten decimal digits.
const FIRST_CAPITAL: u8 = 10;
/// The digit value of `a`, which follows the decimal digits and the twenty-six capitals.
const FIRST_SMALL: u8 = 36;

/// A place among siblings, which orders as its text.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Key(String);

/// The lower bound of a new key is not below its upper bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Unordered;

impl Key {
    /// The text as a key, or `None` when it is empty, leaves the alphabet, or ends with the first symbol.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        digits(text).map(|_| Self(String::from(text)))
    }

    /// The key as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The digits of the key, one for each of its symbols.
    fn digits(&self) -> Vec<u8> {
        self.0.bytes().filter_map(digit).collect()
    }
}

/// The digit value of one symbol.
const fn digit(symbol: u8) -> Option<u8> {
    match symbol {
        b'0'..=b'9' => Some(symbol.wrapping_sub(b'0')),
        b'A'..=b'Z' => Some(symbol.wrapping_sub(b'A').wrapping_add(FIRST_CAPITAL)),
        b'a'..=b'z' => Some(symbol.wrapping_sub(b'a').wrapping_add(FIRST_SMALL)),
        _ => None,
    }
}

/// The digits of a key, or `None` when the text is not one.
fn digits(key: &str) -> Option<Vec<u8>> {
    let mut found = Vec::with_capacity(key.len());
    for symbol in key.bytes() {
        found.push(digit(symbol)?);
    }
    match found.last() {
        Some(last) if *last != 0 => Some(found),
        Some(_) | None => None,
    }
}

/// The digits strictly between `low` and `high`, where an exhausted `low` reads as zeros and `None` is the open end.
fn midpoint(mut low: &[u8], mut high: Option<&[u8]>) -> Result<Vec<u8>, Unordered> {
    let mut found = Vec::new();
    loop {
        if let Some(upper) = high {
            let shared = upper
                .iter()
                .enumerate()
                .take_while(|(index, upper_digit)| {
                    low.get(*index).copied().unwrap_or(0) == **upper_digit
                })
                .count();
            let (prefix, rest) = upper.split_at_checked(shared).ok_or(Unordered)?;
            found.extend_from_slice(prefix);
            low = low.get(shared..).unwrap_or_default();
            high = Some(rest);
        }
        let low_digit = low.first().copied().unwrap_or(0);
        let high_digit = match high {
            Some(upper) => *upper.first().ok_or(Unordered)?,
            None => BASE,
        };
        let gap = high_digit.checked_sub(low_digit).ok_or(Unordered)?;
        if gap > 1 {
            found.push(low_digit.midpoint(high_digit));
            return Ok(found);
        }
        if gap == 0 {
            return Err(Unordered);
        }
        if let Some(upper) = high
            && upper.len() > 1
        {
            found.push(high_digit);
            return Ok(found);
        }
        found.push(low_digit);
        low = low.get(1..).unwrap_or_default();
        high = None;
    }
}

/// A key strictly between `low` and `high`, where `None` is the open end on that side.
///
/// # Errors
///
/// Returns [`Unordered`] when `low` is not below `high`.
pub fn between(low: Option<&Key>, high: Option<&Key>) -> Result<Key, Unordered> {
    if let (Some(low), Some(high)) = (low, high)
        && low >= high
    {
        return Err(Unordered);
    }
    let low_digits = low.map_or_else(Vec::new, Key::digits);
    let high_digits = high.map(Key::digits);
    let found = midpoint(&low_digits, high_digits.as_deref())?;
    let text = found
        .iter()
        .filter_map(|value| ALPHABET.get(usize::from(*value)))
        .map(|symbol| char::from(*symbol))
        .collect();
    Ok(Key(text))
}

/// The properties Kani proves: the midpoint over every pair of digit keys of up to [`proofs::DIGIT_BOUND`] digits, and the symbol encoding over every byte.
///
/// `between` decodes its bounds, compares them, takes the midpoint, and encodes it, so the two proofs together cover it for keys of that length.
#[cfg(any(test, kani))]
mod proofs {
    use super::{BASE, midpoint};
    #[cfg(test)]
    use super::{Key, Unordered, between};

    /// The longest key the string-level tests draw.
    #[cfg(test)]
    pub(super) const KEY_BOUND: usize = 2;
    /// The longest digit key the harness draws.
    pub(super) const DIGIT_BOUND: usize = 2;

    /// Whether `digits` form a key: nonempty, each below the base, and the last one nonzero.
    pub(super) fn is_digit_key(digits: &[u8]) -> bool {
        !digits.is_empty() && digits.iter().all(|value| *value < BASE) && digits.last() != Some(&0)
    }

    /// What `midpoint` promises for ordered bounds: a key strictly between them, as digits compare in order.
    pub(super) fn midpoint_spec(low: &[u8], high: Option<&[u8]>) -> bool {
        midpoint(low, high).is_ok_and(|middle| {
            is_digit_key(&middle)
                && low < middle.as_slice()
                && high.is_none_or(|high| middle.as_slice() < high)
        })
    }

    /// What the symbol encoding promises: a symbol decodes exactly when it is in the alphabet, back to its place there, and in the same order as its byte.
    pub(super) fn encoding_spec(first: u8, second: u8) -> bool {
        let decodes = super::digit(first).map_or_else(
            || !super::ALPHABET.contains(&first),
            |value| super::ALPHABET.get(usize::from(value)) == Some(&first),
        );
        let ordered = match (super::digit(first), super::digit(second)) {
            (Some(low), Some(high)) => (first < second) == (low < high),
            _ => true,
        };
        decodes && ordered
    }

    /// What `between` promises: a key strictly inside the bounds whenever they are ordered, and the refusal otherwise.
    #[cfg(test)]
    pub(super) fn spec(low: Option<&Key>, high: Option<&Key>) -> bool {
        let ordered = match (low, high) {
            (Some(low), Some(high)) => low < high,
            _ => true,
        };
        match between(low, high) {
            Ok(key) => {
                ordered
                    && Key::parse(key.as_str()).as_ref() == Some(&key)
                    && low.is_none_or(|low| *low < key)
                    && high.is_none_or(|high| key < *high)
            }
            Err(Unordered) => !ordered,
        }
    }

    /// Digits of up to [`DIGIT_BOUND`] places, as the harness draws them.
    #[cfg(kani)]
    fn any_digits(values: &[u8; DIGIT_BOUND]) -> &[u8] {
        let length: usize = kani::any();
        kani::assume(length <= DIGIT_BOUND);
        values.get(..length).unwrap_or_default()
    }

    /// The midpoint walks at most both keys and one more step, and each comparison at most one key.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn a_key_lands_strictly_between_its_bounds() {
        let low_values: [u8; DIGIT_BOUND] = kani::any();
        let high_values: [u8; DIGIT_BOUND] = kani::any();
        let low = any_digits(&low_values);
        let high = if kani::any() {
            Some(any_digits(&high_values))
        } else {
            None
        };
        kani::assume(low.is_empty() || is_digit_key(low));
        kani::assume(high.is_none_or(is_digit_key));
        kani::assume(high.is_none_or(|high| low < high));
        kani::assert(midpoint_spec(low, high), "the key lands between its bounds");
        kani::cover!(!low.is_empty() && high.is_some(), "a key between two keys");
        kani::cover!(high.is_none(), "a key after the last");
    }

    /// The alphabet holds sixty-two symbols, which the membership check visits once each.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(63)]
    fn every_symbol_decodes_to_its_place_in_byte_order() {
        let first: u8 = kani::any();
        let second: u8 = kani::any();
        kani::assert(encoding_spec(first, second), "the encoding keeps the order");
        kani::cover!(super::digit(first).is_none(), "a byte outside the alphabet");
    }

    /// A key equal to its lower bound would leave two siblings unordered; this probe claims one and must fail.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn rejects_a_key_at_its_lower_bound() {
        let made = midpoint(&[31], None);
        kani::assert(
            made.as_deref() == Ok(&[31][..]),
            "NEGATIVE_POSITION_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{DIGIT_BOUND, KEY_BOUND, encoding_spec, is_digit_key, midpoint_spec, spec};
    use super::{ALPHABET, Key, Unordered, between};
    use alloc::string::String;
    use alloc::vec::Vec;

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    fn made(low: Option<&str>, high: Option<&str>) -> Result<String, Unordered> {
        between(low.map(key).as_ref(), high.map(key).as_ref())
            .map(|made| String::from(made.as_str()))
    }

    #[test]
    fn the_first_key_sits_in_the_middle_and_others_fit_around_it() {
        assert_eq!(made(None, None).as_deref(), Ok("V"));
        assert_eq!(made(Some("V"), None).as_deref(), Ok("k"));
        assert_eq!(made(None, Some("V")).as_deref(), Ok("F"));
        assert_eq!(made(Some("V"), Some("W")).as_deref(), Ok("VV"));
        assert_eq!(made(Some("z"), None).as_deref(), Ok("zV"));
        assert_eq!(made(None, Some("1")).as_deref(), Ok("0V"));
        assert_eq!(made(Some("1"), Some("1V")).as_deref(), Ok("1F"));
        assert_eq!(made(Some("V"), Some("W1")).as_deref(), Ok("W"));
    }

    #[test]
    fn bad_bounds_and_bad_text_are_refused() {
        assert_eq!(made(Some("W"), Some("V")), Err(Unordered));
        assert_eq!(made(Some("V"), Some("V")), Err(Unordered));
        for text in ["V0", "", "V-", "0", "é"] {
            assert_eq!(Key::parse(text), None, "{text:?}");
        }
        assert_eq!(key("0V").as_str(), "0V");
    }

    #[test]
    fn repeated_insertion_keeps_every_key_between_its_neighbors() {
        let mut keys: Vec<Key> = Vec::new();
        let mut low: Option<Key> = None;
        for _ in 0..200 {
            let made = between(low.as_ref(), None).unwrap();
            keys.push(made.clone());
            low = Some(made);
        }
        let mut high: Option<Key> = None;
        for _ in 0..200 {
            let made = between(None, high.as_ref()).unwrap();
            keys.push(made.clone());
            high = Some(made);
        }
        let mut bisect = (key("V"), key("W"));
        for _ in 0..200 {
            let made = between(Some(&bisect.0), Some(&bisect.1)).unwrap();
            assert!(bisect.0 < made && made < bisect.1);
            bisect.1 = made;
        }
        assert!(
            keys.iter()
                .all(|made| Key::parse(made.as_str()).as_ref() == Some(made))
        );
    }

    #[test]
    fn the_specification_agrees_over_every_short_pair() {
        let mut texts: Vec<String> = alloc::vec![String::new(), String::from("-")];
        for first in ALPHABET {
            texts.push(String::from(char::from(*first)));
            for second in *b"01Vyz" {
                let mut text = String::from(char::from(*first));
                text.push(char::from(second));
                texts.push(text);
            }
        }
        assert!(texts.iter().all(|text| text.len() <= KEY_BOUND));
        let mut keys: Vec<Option<Key>> = alloc::vec![None];
        for text in &texts {
            let parsed = Key::parse(text);
            let expected = !text.is_empty()
                && text.bytes().all(|symbol| ALPHABET.contains(&symbol))
                && !text.ends_with('0');
            assert_eq!(parsed.is_some(), expected, "{text:?}");
            keys.extend(parsed.map(Some));
        }
        for low in &keys {
            for high in &keys {
                assert!(spec(low.as_ref(), high.as_ref()), "{low:?} {high:?}");
            }
        }
    }

    #[test]
    fn the_digit_specifications_agree_over_a_drawn_domain() {
        let values = [0, 1, 30, 31, 32, 60, 61];
        let mut drawn: Vec<Vec<u8>> = alloc::vec![Vec::new()];
        // One digit longer than the harness draws, over a sample of the digits.
        for _ in 0..=DIGIT_BOUND {
            let longer: Vec<Vec<u8>> = drawn
                .iter()
                .flat_map(|digits| {
                    values.iter().map(move |value| {
                        let mut next = digits.clone();
                        next.push(*value);
                        next
                    })
                })
                .collect();
            drawn.extend(longer);
            drawn.sort();
            drawn.dedup();
        }
        let lows: Vec<&Vec<u8>> = drawn
            .iter()
            .filter(|digits| digits.is_empty() || is_digit_key(digits))
            .collect();
        let highs: Vec<&Vec<u8>> = drawn.iter().filter(|digits| is_digit_key(digits)).collect();
        for low in &lows {
            assert!(midpoint_spec(low, None), "{low:?}");
            for high in highs.iter().filter(|high| low.as_slice() < high.as_slice()) {
                assert!(midpoint_spec(low, Some(high)), "{low:?} {high:?}");
            }
        }
        for first in 0..=u8::MAX {
            for second in 0..=u8::MAX {
                assert!(encoding_spec(first, second), "{first} {second}");
            }
        }
    }
}
