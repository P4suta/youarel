//! Positions among siblings as keys that order as their text, so an item moves by one write and never renumbers the others.
//!
//! A key is a nonempty string over [`ALPHABET`] that never ends with its first symbol, and keys order as strings.
//! Between any two keys, and before or after any key, there is always another key.
//! A key that starts with a letter reads as an integer, the letter and as many symbols as its class holds, with a fraction after it.
//! The small letters `a` to `z` hold one to twenty-six symbols, and the capitals `Z` down to `A` hold one to twenty-six, so integers order as their keys.
//! A key after the last one is the next integer and a key before the first one the integer before it, so a long run at either end keeps its keys short.
//! A key between two others is their midpoint, symbol by symbol.
//! No integer ends with the first symbol either, so every key keeps the form that builds from before the integers read.
//! Two devices that place an item at the same spot may write the same key, which the item identifier then orders.

use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// The symbols of a key, in ascending order of both their digit value and their byte value.
pub const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
/// The symbols of the longest integer, a letter and twenty-six more, so a key past one bound is at most one symbol longer than the longer of that bound and this.
pub const LONGEST_INTEGER: usize = 27;

/// The digit one past the last symbol, which stands for the open upper end.
const BASE: u8 = 62;
/// The digit of the last symbol, `z`.
const LAST_DIGIT: u8 = BASE - 1;
/// The digit value of `A`, which follows the ten decimal digits.
const FIRST_CAPITAL: u8 = 10;
/// The digit value of `a`, which follows the decimal digits and the twenty-six capitals.
const FIRST_SMALL: u8 = 36;
/// The steps of a midpoint beyond one per digit of the lower bound: the one past the shared prefix and the one past the last digit.
const STEPS_PAST_LOW: usize = 2;
/// The first key of a folder, `a1`: the least integer after a small letter.
const FIRST: [u8; 2] = [FIRST_SMALL, 1];
/// The integer just before [`FIRST`], `Zz`: the greatest integer after a capital.
const BEFORE_FIRST: [u8; 2] = [FIRST_SMALL - 1, LAST_DIGIT];

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

/// The text of digits, each written as its symbol.
fn text(digits: &[u8]) -> String {
    digits
        .iter()
        .filter_map(|value| ALPHABET.get(usize::from(*value)))
        .map(|symbol| char::from(*symbol))
        .collect()
}

/// How many digits of an integer follow the first digit of a key, or `None` for a decimal digit, which starts no integer.
fn class(head: u8) -> Option<usize> {
    let digits = match head {
        FIRST_SMALL..BASE => head.checked_sub(FIRST_SMALL)?.checked_add(1)?,
        FIRST_CAPITAL..FIRST_SMALL => FIRST_SMALL.checked_sub(head)?,
        _ => return None,
    };
    Some(usize::from(digits))
}

/// How a key lies against the integer it reads as.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Reading {
    /// The key is the integer.
    Whole,
    /// The key is the integer with a fraction after it.
    Fraction,
    /// The key is shorter than its class, and the integer is the key with zeros after it.
    Short,
}

/// An integer: the first digit of a key and the digits that its class holds, kept in a place as long as the longest integer.
#[derive(Clone, Copy, Debug)]
struct Integer {
    /// The digits, of which the first `length` count.
    places: [u8; LONGEST_INTEGER],
    length: usize,
}

impl Integer {
    /// The integer of the class that `head` starts with every digit `value`, or `None` for a head that starts no integer.
    fn filled(head: u8, value: u8) -> Option<Self> {
        let length = class(head)?.checked_add(1)?;
        let mut places = [value; LONGEST_INTEGER];
        *places.first_mut()? = head;
        Some(Self { places, length })
    }

    /// The integer a key reads as and how the key lies against it, or `None` for a key that starts with a decimal digit.
    fn reading(key: &[u8]) -> Option<(Self, Reading)> {
        let mut integer = Self::filled(*key.first()?, 0)?;
        let kept = key.len().min(integer.length);
        integer
            .places
            .get_mut(..kept)?
            .copy_from_slice(key.get(..kept)?);
        let lies = match key.len().cmp(&integer.length) {
            Ordering::Equal => Reading::Whole,
            Ordering::Greater => Reading::Fraction,
            Ordering::Less => Reading::Short,
        };
        Some((integer, lies))
    }

    /// The digits that count.
    fn digits(&self) -> &[u8] {
        self.places.get(..self.length).unwrap_or_default()
    }

    /// Whether the integer ends with zero, which a key never does.
    fn ends_with_zero(&self) -> bool {
        self.digits().last() == Some(&0)
    }

    /// The integer one above, which carries into the next class once every digit is the last, or `None` past the last class.
    fn up(mut self) -> Option<Self> {
        let mut place = self.length;
        while place > 1 {
            place = place.wrapping_sub(1);
            let value = self.places.get_mut(place)?;
            if *value < LAST_DIGIT {
                *value = value.wrapping_add(1);
                return Some(self);
            }
            *value = 0;
        }
        Self::filled(self.places.first()?.checked_add(1)?, 0)
    }

    /// The integer one below, which borrows from the class before once every digit is zero, or `None` before the first class.
    fn down(mut self) -> Option<Self> {
        let mut place = self.length;
        while place > 1 {
            place = place.wrapping_sub(1);
            let value = self.places.get_mut(place)?;
            if *value > 0 {
                *value = value.wrapping_sub(1);
                return Some(self);
            }
            *value = LAST_DIGIT;
        }
        Self::filled(self.places.first()?.checked_sub(1)?, LAST_DIGIT)
    }

    /// The least integer above this one that is a key, so it never ends with zero, or `None` past the last integer.
    fn next(self) -> Option<Self> {
        let mut raised = self.up()?;
        if raised.ends_with_zero() {
            *raised.places.get_mut(raised.length.checked_sub(1)?)? = 1;
        }
        Some(raised)
    }

    /// The greatest integer below this one that is a key, so it never ends with zero, or `None` before the first integer.
    fn previous(self) -> Option<Self> {
        let lowered = self.down()?;
        if lowered.ends_with_zero() {
            lowered.down()
        } else {
            Some(lowered)
        }
    }
}

/// The least integer above a key, or `None` past the last integer or for a key that starts with a decimal digit.
fn above(key: &[u8]) -> Option<Vec<u8>> {
    let (integer, _) = Integer::reading(key)?;
    integer.next().map(|found| found.digits().to_vec())
}

/// The greatest integer below a key, or `None` before the first integer or for a key that starts with a decimal digit.
fn below(key: &[u8]) -> Option<Vec<u8>> {
    let (integer, lies) = Integer::reading(key)?;
    let found = if lies == Reading::Fraction && !integer.ends_with_zero() {
        integer
    } else {
        integer.previous()?
    };
    Some(found.digits().to_vec())
}

/// The digits strictly between `low` and `high`, where an exhausted `low` reads as zeros and `None` is the open end.
///
/// Every step that does not finish consumes a digit of `low`, and the step after the last one finishes, so the walk takes at most two steps more than `low` has digits.
fn midpoint(mut low: &[u8], mut high: Option<&[u8]>) -> Result<Vec<u8>, Unordered> {
    let mut found = Vec::new();
    for _ in 0..low.len().saturating_add(STEPS_PAST_LOW) {
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
    Err(Unordered)
}

/// The digits of a key after `low` with no key above it: the least integer above it and never below [`FIRST`], or the midpoint past the last integer.
fn after(low: &[u8]) -> Result<Vec<u8>, Unordered> {
    if low < FIRST.as_slice() {
        return Ok(FIRST.to_vec());
    }
    above(low).map_or_else(|| midpoint(low, None), Ok)
}

/// The digits of a key before `high` with no key below it: the greatest integer below it and never above [`BEFORE_FIRST`], or the midpoint when no integer lies below it.
fn before(high: &[u8]) -> Result<Vec<u8>, Unordered> {
    if high > BEFORE_FIRST.as_slice() {
        return Ok(BEFORE_FIRST.to_vec());
    }
    below(high).map_or_else(|| midpoint(&[], Some(high)), Ok)
}

/// The digits of a key strictly between `low` and `high`, where `None` is the open end on that side, or the midpoint's refusal of two bounds out of order.
fn place(low: Option<&[u8]>, high: Option<&[u8]>) -> Result<Vec<u8>, Unordered> {
    match (low, high) {
        (None, None) => Ok(FIRST.to_vec()),
        (Some(low), None) => after(low),
        (None, Some(high)) => before(high),
        (Some(low), Some(high)) => midpoint(low, Some(high)),
    }
}

/// A key strictly between `low` and `high`, where `None` is the open end on that side.
///
/// A folder's first key is `a1`, a key after the last is the next integer, a key before the first is the integer before it, and a key between two is their midpoint.
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
    let low_digits = low.map(Key::digits);
    let high_digits = high.map(Key::digits);
    let found = place(low_digits.as_deref(), high_digits.as_deref())?;
    Ok(Key(text(&found)))
}

/// The properties Kani proves: the midpoint over every pair of digit keys of up to [`proofs::DIGIT_BOUND`] digits, the symbol encoding over every byte, and the keys past either end over every key whose class holds at most [`proofs::CLASS_BOUND`] digits.
///
/// `between` decodes its bounds, hands two bounds to the midpoint, one bound to the key past it, and none to `a1`, and encodes the digits it gets, so the proofs of those rules and of the encoding together cover it for keys of those lengths.
#[cfg(any(test, kani))]
mod proofs {
    #[cfg(kani)]
    use super::below;
    use super::{BASE, Unordered, above, after, class, midpoint};
    #[cfg(test)]
    use super::{Key, before, between, place};
    use alloc::vec::Vec;

    /// The longest key the string-level tests draw.
    #[cfg(test)]
    pub(super) const KEY_BOUND: usize = 2;
    /// The longest digit key the harness of the midpoint draws.
    pub(super) const DIGIT_BOUND: usize = 2;
    /// The longest key the harnesses of one bound draw, which reads as an integer of a class of up to two digits or starts one of three.
    #[cfg(kani)]
    const ONE_BOUND: usize = 3;
    /// The most digits of an integer that the class of a fixed letter holds, whose whole integers the harnesses of integers draw.
    #[cfg(test)]
    pub(super) const CLASS_BOUND: usize = 3;

    /// Whether `digits` form a key: nonempty, each below the base, and the last one nonzero.
    pub(super) fn is_digit_key(digits: &[u8]) -> bool {
        !digits.is_empty() && digits.iter().all(|value| *value < BASE) && digits.last() != Some(&0)
    }

    /// How many digits the class of a key's first digit adds to that digit, or none for a decimal digit.
    pub(super) fn class_length(key: &[u8]) -> usize {
        key.first()
            .and_then(|head| class(*head))
            .map_or(0, |digits| digits.saturating_add(1))
    }

    /// Whether a key reads as a whole integer or an integer with a fraction, rather than as a key shorter than its class.
    pub(super) fn reads_whole(key: &[u8]) -> bool {
        class_length(key) > 0 && key.len() >= class_length(key)
    }

    /// What `midpoint` promises: a key strictly between ordered bounds, as digits compare in order, and the refusal of bounds that are not ordered.
    pub(super) fn midpoint_spec(low: &[u8], high: Option<&[u8]>) -> bool {
        let ordered = high.is_none_or(|high| low < high);
        match midpoint(low, high) {
            Ok(middle) => {
                ordered
                    && is_digit_key(&middle)
                    && low < middle.as_slice()
                    && high.is_none_or(|high| middle.as_slice() < high)
            }
            Err(Unordered) => !ordered,
        }
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

    /// What the choice of a key's digits promises: a key strictly inside the bounds whenever they are ordered, and the refusal otherwise.
    #[cfg(test)]
    pub(super) fn place_spec(low: Option<&[u8]>, high: Option<&[u8]>) -> bool {
        let ordered = match (low, high) {
            (Some(low), Some(high)) => low < high,
            _ => true,
        };
        match place(low, high) {
            Ok(found) => {
                ordered
                    && is_digit_key(&found)
                    && low.is_none_or(|low| low < found.as_slice())
                    && high.is_none_or(|high| found.as_slice() < high)
            }
            Err(Unordered) => !ordered,
        }
    }

    /// The digits of a key that a choice found, or why it found none.
    pub(super) type Found = Result<Vec<u8>, Unordered>;

    /// The most symbols a key past a bound may hold: one more than the longer of the bound and its class.
    fn longest_past(bound: &[u8]) -> usize {
        bound.len().max(class_length(bound)).saturating_add(1)
    }

    /// Whether a key found after the last one keeps its promise: a key above it, at most one symbol longer than the longer of the key and its class.
    pub(super) fn after_holds(low: &[u8], found: &Found) -> bool {
        found.as_ref().is_ok_and(|found| {
            is_digit_key(found) && low < found.as_slice() && found.len() <= longest_past(low)
        })
    }

    /// Whether a key found before the first one keeps its promise: a key below it, at most one symbol longer than the longer of the key and its class.
    pub(super) fn before_holds(high: &[u8], found: &Found) -> bool {
        found.as_ref().is_ok_and(|found| {
            is_digit_key(found) && found.as_slice() < high && found.len() <= longest_past(high)
        })
    }

    /// What a key after the last promises.
    #[cfg(test)]
    pub(super) fn after_spec(low: &[u8]) -> bool {
        after_holds(low, &after(low))
    }

    /// What a key before the first promises.
    #[cfg(test)]
    pub(super) fn before_spec(high: &[u8]) -> bool {
        before_holds(high, &before(high))
    }

    /// What the least integer above a key shorter than its class promises: an integer above it as long as its class, below every other integer above it of the same first digit.
    pub(super) fn above_spec(short: &[u8], other: &[u8]) -> bool {
        above(short).is_some_and(|found| {
            let least = !(short < other && other < found.as_slice());
            is_digit_key(&found)
                && short < found.as_slice()
                && found.len() == class_length(short)
                && least
        })
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

    /// The capitals whose classes hold at most three digits, `X` to `Z`, which the harnesses fix one at a time so that each class's length is known.
    #[cfg(kani)]
    fn capitals() -> [u8; 3] {
        let first = super::FIRST_SMALL;
        [
            first.wrapping_sub(3),
            first.wrapping_sub(2),
            first.wrapping_sub(1),
        ]
    }

    /// The small letters whose classes hold at most three digits, `a` to `c`.
    #[cfg(kani)]
    fn smalls() -> [u8; 3] {
        let first = super::FIRST_SMALL;
        [first, first.wrapping_add(1), first.wrapping_add(2)]
    }

    /// The symbols of the longest integer of the classes the harnesses fix: a letter and three digits.
    #[cfg(kani)]
    const LONGEST_DRAWN: usize = 4;

    /// Digits whose first one is `head`, or a drawn decimal digit for `None`, the others drawn.
    #[cfg(kani)]
    fn drawn<const N: usize>(head: Option<u8>) -> [u8; N] {
        let mut values: [u8; N] = kani::any();
        if let Some(first) = values.first_mut() {
            match head {
                Some(head) => *first = head,
                None => kani::assume(*first < super::FIRST_CAPITAL),
            }
        }
        values
    }

    /// Hands `check` a key of one to [`ONE_BOUND`] digits that starts with `head`, or with a drawn decimal digit for `None`, its length and other digits drawn.
    ///
    /// The first digit is fixed before the rest are drawn, so the length of its class is known to the solver.
    #[cfg(kani)]
    fn with_key(head: Option<u8>, check: impl Fn(&[u8])) {
        let values: [u8; ONE_BOUND] = drawn(head);
        let length: usize = kani::any();
        kani::assume(length >= 1 && length <= ONE_BOUND);
        let key = values.get(..length).unwrap_or_default();
        kani::assume(is_digit_key(key));
        check(key);
    }

    /// Whether `found` is a key on one side of `integer`, above it or below it, at most one digit longer.
    #[cfg(kani)]
    fn beside(integer: &super::Integer, found: &super::Integer, above: bool) -> bool {
        let ordered = if above {
            integer.digits() < found.digits()
        } else {
            found.digits() < integer.digits()
        };
        ordered && is_digit_key(found.digits()) && found.length <= integer.length.saturating_add(1)
    }

    /// An integer of the class that `head` starts, each of its digits drawn below the base, or `None` for a head that starts no integer.
    #[cfg(kani)]
    fn any_integer(head: u8) -> Option<super::Integer> {
        let mut integer = super::Integer::filled(head, 0)?;
        let length = integer.length;
        for place in integer.places.iter_mut().take(length).skip(1) {
            let digit: u8 = kani::any();
            kani::assume(digit < BASE);
            *place = digit;
        }
        Some(integer)
    }

    /// Every lower bound that is empty or a key and every upper bound that is open or a key, of up to two digits each and in either order: the midpoint lands strictly between ordered bounds and refuses the others.
    ///
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
        kani::assert(
            midpoint_spec(low, high),
            "the key lands between its bounds, or bounds out of order are refused",
        );
        kani::cover!(
            !low.is_empty() && high.is_some_and(|high| low < high),
            "a key between two keys"
        );
        kani::cover!(high.is_none(), "a key after the last");
        kani::cover!(high.is_some_and(|high| high <= low), "bounds out of order");
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

    /// Every key of up to [`ONE_BOUND`] digits that starts with a letter of [`smalls`], which is how every key at or after `a1` starts: the least integer above it lies above it and grows by one symbol at most past the longer of it and its class.
    ///
    /// A key after the last one is this integer for such a key, `a1` for a key below `a1`, and the midpoint only past the last integer.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(6)]
    fn the_integer_above_a_key_lies_above_and_grows_by_one_symbol_at_most() {
        for head in smalls() {
            with_key(Some(head), |low| {
                let found = above(low).ok_or(Unordered);
                kani::assert(
                    after_holds(low, &found),
                    "the integer above a key lies above it",
                );
                let length = found.as_ref().map_or(0, Vec::len);
                kani::cover!(
                    reads_whole(low) && length > low.len(),
                    "an integer whose digits are spent carries into a longer class"
                );
                kani::cover!(
                    reads_whole(low) && length == low.len(),
                    "an integer steps within its class"
                );
            });
        }
    }

    /// Every key of up to [`ONE_BOUND`] digits that starts with a letter of [`capitals`], which is how every key at or before `Zz` that starts with a letter starts: the greatest integer below it lies below it and grows by one symbol at most past the longer of it and its class.
    ///
    /// A key before the first one is this integer for such a key, `Zz` for a key above `Zz`, and the midpoint below a key that starts with a decimal digit, which the midpoint's harness proves with an empty lower bound.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(6)]
    fn the_integer_below_a_key_lies_below_and_grows_by_one_symbol_at_most() {
        for head in capitals() {
            with_key(Some(head), |high| {
                let found = below(high).ok_or(Unordered);
                kani::assert(
                    before_holds(high, &found),
                    "the integer below a key lies below it",
                );
                let length = found.as_ref().map_or(0, Vec::len);
                kani::cover!(
                    reads_whole(high) && length > high.len(),
                    "an integer whose digits are spent borrows from a longer class"
                );
                kani::cover!(
                    reads_whole(high) && length == high.len(),
                    "an integer steps within its class"
                );
            });
        }
    }

    /// Every integer of a class of up to three digits, its digits drawn: the integer after it is a key above it, at most one digit longer, and the integer before it a key below it, at most one digit longer.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(7)]
    fn the_integers_beside_an_integer_are_keys_on_either_side_of_it() {
        for head in capitals().into_iter().chain(smalls()) {
            let integer = any_integer(head);
            let next = integer.and_then(super::Integer::next);
            let previous = integer.and_then(super::Integer::previous);
            kani::assert(
                integer
                    .zip(next)
                    .is_some_and(|(integer, next)| beside(&integer, &next, true)),
                "the next integer is a key above",
            );
            kani::assert(
                integer
                    .zip(previous)
                    .is_some_and(|(integer, previous)| beside(&integer, &previous, false)),
                "the previous integer is a key below",
            );
            kani::cover!(
                integer
                    .zip(next)
                    .is_some_and(|(integer, next)| next.length > integer.length),
                "an integer carries into a longer class"
            );
            kani::cover!(
                integer
                    .zip(previous)
                    .is_some_and(|(integer, previous)| previous.length > integer.length),
                "an integer borrows from a longer class"
            );
        }
    }

    /// Every key shorter than its class that starts with a letter of [`smalls`], against every integer of that letter: the least integer above the key lies above it, takes its class's length, and passes no integer above the key.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(6)]
    fn the_least_integer_above_a_short_key_takes_its_class_length() {
        for head in smalls() {
            with_key(Some(head), |short| {
                kani::assume(!reads_whole(short));
                let values: [u8; LONGEST_DRAWN] = drawn(Some(head));
                let other = values.get(..class_length(short)).unwrap_or_default();
                kani::assume(is_digit_key(other));
                kani::cover!(other.len() == LONGEST_DRAWN, "an integer of three digits");
                kani::assert(
                    above_spec(short, other),
                    "the least integer above a short key takes its class length",
                );
                kani::cover!(short.len() == 1, "a lone letter");
                kani::cover!(short.len() > 1, "a letter with symbols after it");
            });
        }
    }

    /// A key between two others would leave two siblings unordered at its lower bound; this probe claims one and must fail.
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

    /// The integer `b0z` steps to `b11` within its class, where a midpoint would have lengthened it; this probe claims the append lengthens it and must fail.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(7)]
    fn rejects_an_append_that_lengthens_a_short_integer() {
        let short = [super::FIRST_SMALL.wrapping_add(1), 0, super::LAST_DIGIT];
        kani::assert(
            after(&short).is_ok_and(|found| found.len() > short.len()),
            "NEGATIVE_POSITION_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{
        CLASS_BOUND, DIGIT_BOUND, KEY_BOUND, above_spec, after_spec, before_spec, class_length,
        encoding_spec, is_digit_key, midpoint_spec, place_spec, reads_whole, spec,
    };
    use super::{
        ALPHABET, Integer, Key, LONGEST_INTEGER, Reading, Unordered, above, between, class, digits,
        text,
    };
    use alloc::string::String;
    use alloc::vec::Vec;

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    /// The integer that the digits of a whole integer read as.
    fn integer(digits: &[u8]) -> Integer {
        let (integer, lies) = Integer::reading(digits).unwrap();
        assert_eq!(lies, Reading::Whole, "{}", text(digits));
        integer
    }

    /// The digits of the least integer above a whole integer that is a key.
    fn next(digits: &[u8]) -> Option<Vec<u8>> {
        integer(digits).next().map(|found| found.digits().to_vec())
    }

    /// The digits of the greatest integer below a whole integer that is a key.
    fn previous(digits: &[u8]) -> Option<Vec<u8>> {
        integer(digits)
            .previous()
            .map(|found| found.digits().to_vec())
    }

    fn made(low: Option<&str>, high: Option<&str>) -> Result<String, Unordered> {
        between(low.map(key).as_ref(), high.map(key).as_ref())
            .map(|made| String::from(made.as_str()))
    }

    /// A text of `count` copies of `symbol` after `front` and before `back`.
    fn spelled(front: &str, symbol: char, count: usize, back: &str) -> String {
        let mut text = String::from(front);
        text.extend(core::iter::repeat_n(symbol, count));
        text.push_str(back);
        text
    }

    /// Every key of one to `longest` symbols drawn from `symbols`.
    fn keys_over(symbols: &[u8], longest: usize) -> Vec<Vec<u8>> {
        let mut found: Vec<Vec<u8>> = alloc::vec![Vec::new()];
        let mut keys = Vec::new();
        for _ in 0..longest {
            found = found
                .iter()
                .flat_map(|front| {
                    symbols.iter().map(move |symbol| {
                        let mut longer = front.clone();
                        longer.push(*symbol);
                        longer
                    })
                })
                .collect();
            keys.extend(found.iter().filter(|key| is_digit_key(key)).cloned());
        }
        keys
    }

    /// Digits that hold the ends of every class the proofs draw, the decimal digits, and the letters around them.
    const SAMPLE: [u8; 16] = [0, 1, 2, 9, 10, 11, 32, 33, 34, 35, 36, 37, 38, 39, 60, 61];

    #[test]
    fn a_folder_starts_at_a1_and_runs_through_the_integers() {
        assert_eq!(made(None, None).as_deref(), Ok("a1"));
        assert_eq!(made(Some("a1"), None).as_deref(), Ok("a2"));
        assert_eq!(made(Some("a9"), None).as_deref(), Ok("aA"));
        assert_eq!(made(Some("az"), None).as_deref(), Ok("b01"));
        assert_eq!(
            made(Some("b0z"), None).as_deref(),
            Ok("b11"),
            "an integer that would end in zero steps once more"
        );
        assert_eq!(made(Some("bzz"), None).as_deref(), Ok("c001"));
        assert_eq!(
            made(Some("a5V"), None).as_deref(),
            Ok("a6"),
            "an append leaves the fraction behind"
        );
        assert_eq!(
            made(Some("Y05"), None).as_deref(),
            Ok("a1"),
            "an append below the first key lands on it"
        );
        assert_eq!(made(None, Some("a1")).as_deref(), Ok("Zz"));
        assert_eq!(made(None, Some("Z2")).as_deref(), Ok("Z1"));
        assert_eq!(made(None, Some("Z1")).as_deref(), Ok("Yzz"));
        assert_eq!(made(None, Some("Y01")).as_deref(), Ok("Xzzz"));
        assert_eq!(
            made(None, Some("Z5V")).as_deref(),
            Ok("Z5"),
            "a prepend before a fraction takes its integer"
        );
        assert_eq!(
            made(None, Some("Y50V")).as_deref(),
            Ok("Y4z"),
            "an integer that ends in zero is no key"
        );
        assert_eq!(
            made(None, Some("b05")).as_deref(),
            Ok("Zz"),
            "a prepend above the first key lands just below it"
        );
    }

    #[test]
    fn a_key_between_two_is_their_midpoint() {
        assert_eq!(made(Some("V"), Some("W")).as_deref(), Ok("VV"));
        assert_eq!(made(Some("1"), Some("1V")).as_deref(), Ok("1F"));
        assert_eq!(made(Some("V"), Some("W1")).as_deref(), Ok("W"));
        assert_eq!(made(Some("a1"), Some("a2")).as_deref(), Ok("a1V"));
        assert_eq!(made(Some("Zz"), Some("a1")).as_deref(), Ok("a"));
        assert_eq!(made(Some("a"), Some("a1")).as_deref(), Ok("a0V"));
    }

    #[test]
    fn a_stored_key_goes_on_from_the_integers_beside_it() {
        assert_eq!(made(Some("V"), None).as_deref(), Ok("a1"));
        assert_eq!(made(Some("0V"), None).as_deref(), Ok("a1"));
        assert_eq!(made(Some("a"), None).as_deref(), Ok("a1"));
        assert_eq!(
            made(Some("k"), None),
            Ok(spelled("k", '0', 10, "1")),
            "a key shorter than its class goes on from the least integer that starts with it"
        );
        assert_eq!(made(Some("zV"), None), Ok(spelled("zV", '0', 24, "1")));
        assert_eq!(made(None, Some("V")), Ok(spelled("U", 'z', 6, "")));
        assert_eq!(made(None, Some("VV")), Ok(spelled("VU", 'z', 4, "")));
        assert_eq!(made(None, Some("k")).as_deref(), Ok("Zz"));
        assert_eq!(
            made(None, Some("0V")).as_deref(),
            Ok("0F"),
            "no integer lies below a decimal digit, so the midpoint places the key"
        );
        assert_eq!(made(None, Some("1")).as_deref(), Ok("0V"));
    }

    #[test]
    fn the_ends_of_the_integers_fall_back_to_the_midpoint() {
        assert_eq!(
            class(61).map(|digits| digits.saturating_add(1)),
            Some(LONGEST_INTEGER)
        );
        assert_eq!(
            class(10).map(|digits| digits.saturating_add(1)),
            Some(LONGEST_INTEGER)
        );
        assert!(
            (0..62)
                .filter_map(class)
                .all(|digits| digits < LONGEST_INTEGER)
        );
        let last = spelled("", 'z', 27, "");
        assert_eq!(made(Some(&last), None), Ok(spelled("", 'z', 27, "V")));
        let first = spelled("A", '0', 25, "1");
        assert_eq!(made(None, Some(&first)).as_deref(), Ok("5"));
        assert_eq!(made(None, Some("A")).as_deref(), Ok("5"));
    }

    #[test]
    fn bad_bounds_and_bad_text_are_refused() {
        assert_eq!(made(Some("W"), Some("V")), Err(Unordered));
        assert_eq!(made(Some("V"), Some("V")), Err(Unordered));
        for text in ["V0", "", "V-", "0", "é", "a0", "b10"] {
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

    /// The longest key of `count` appends after `start`, each above the one before and none longer than `bound`.
    ///
    /// Each key meets the bound as it comes, so a run whose keys grow stops at once instead of growing for the rest of the count.
    fn longest_of_appends(start: &str, count: usize, bound: usize) -> usize {
        let mut last = key(start);
        let mut longest = 0;
        for _ in 0..count {
            let made = between(Some(&last), None).unwrap();
            assert!(last < made, "{last:?} {made:?}");
            assert!(made.as_str().len() <= bound, "{made:?}");
            longest = longest.max(made.as_str().len());
            last = made;
        }
        longest
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "a million keys, which the native run covers; Miri checks the same code on the other tests"
    )]
    fn a_million_appends_keep_every_key_within_five_symbols() {
        assert_eq!(
            longest_of_appends("a1", 1_000_000, 5),
            5,
            "61 keys of a, 3,782 of b, and 234,484 of c come first"
        );
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "three hundred thousand keys, which the native run covers; Miri checks the same code on the other tests"
    )]
    fn appends_after_a_stored_key_stay_within_its_class_and_seven_symbols() {
        for (start, expected) in [("V", 4), ("zV", 27), ("0V", 4)] {
            let digits = digits(start).unwrap();
            let held = digits.first().and_then(|head| class(*head)).unwrap_or(0);
            let longest = longest_of_appends(start, 100_000, held.saturating_add(7));
            assert_eq!(longest, expected, "{start}");
        }
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "a hundred thousand keys, which the native run covers; Miri checks the same code on the other tests"
    )]
    fn a_hundred_thousand_prepends_keep_every_key_within_four_symbols() {
        let mut first = key("a1");
        let mut longest = 0;
        for _ in 0..100_000 {
            let made = between(None, Some(&first)).unwrap();
            assert!(made < first, "{made:?} {first:?}");
            assert!(made.as_str().len() <= 4, "{made:?}");
            longest = longest.max(made.as_str().len());
            first = made;
        }
        assert_eq!(longest, 4, "61 keys of Z and 3,782 of Y come first");
    }

    #[test]
    fn the_integers_follow_one_another_in_the_order_of_their_keys() {
        let mut integers: Vec<Vec<u8>> = Vec::new();
        for head in [34_u8, 35, 36, 37] {
            let length = class(head).unwrap();
            for code in 0..62_usize.pow(u32::try_from(length).unwrap()) {
                let mut integer = alloc::vec![head];
                let mut rest = code;
                for _ in 0..length {
                    integer.insert(1, u8::try_from(rest % 62).unwrap());
                    rest /= 62;
                }
                if is_digit_key(&integer) {
                    integers.push(integer);
                }
            }
        }
        integers.sort();
        assert_eq!(integers.len(), 2 * 61 + 2 * 62 * 61);
        for pair in integers.windows(2) {
            assert_eq!(
                next(&pair[0]).as_ref(),
                Some(&pair[1]),
                "{}",
                text(&pair[0])
            );
            assert_eq!(
                previous(&pair[1]).as_ref(),
                Some(&pair[0]),
                "{}",
                text(&pair[1])
            );
        }
        assert_eq!(
            next(integers.last().unwrap())
                .map(|found| text(&found))
                .as_deref(),
            Some("c001")
        );
        assert_eq!(
            previous(&integers[0]).map(|found| text(&found)).as_deref(),
            Some("Xzzz")
        );
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "an exhaustive sweep that Kani and the native run cover; Miri checks the same code on the other tests"
    )]
    fn every_short_key_has_a_key_on_either_side_that_grows_by_one_symbol_at_most() {
        let keys = keys_over(&SAMPLE, CLASS_BOUND + 2);
        for key in &keys {
            assert!(after_spec(key), "{}", text(key));
            assert!(before_spec(key), "{}", text(key));
            if class_length(key) > 0 && !reads_whole(key) {
                let found = above(key).unwrap();
                assert!(
                    previous(&found).is_none_or(|before| before.as_slice() < key.as_slice()),
                    "{}",
                    text(key)
                );
            }
        }
        let alphabet: Vec<u8> = (0..62).collect();
        for key in keys_over(&alphabet, 2) {
            assert!(after_spec(&key), "{}", text(&key));
            assert!(before_spec(&key), "{}", text(&key));
        }
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "an exhaustive sweep that Kani and the native run cover; Miri checks the same code on the other tests"
    )]
    fn the_least_integer_above_a_short_key_passes_no_integer_of_its_class() {
        let shorts: Vec<Vec<u8>> = keys_over(&SAMPLE, CLASS_BOUND)
            .into_iter()
            .filter(|key| class_length(key) > 0 && !reads_whole(key))
            .collect();
        let integers: Vec<Vec<u8>> = keys_over(&SAMPLE, CLASS_BOUND + 1)
            .into_iter()
            .filter(|key| reads_whole(key) && key.len() == class_length(key))
            .collect();
        for short in &shorts {
            for other in integers
                .iter()
                .filter(|other| other.first() == short.first())
            {
                assert!(above_spec(short, other), "{} {}", text(short), text(other));
            }
        }
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "an exhaustive sweep that Kani and the native run cover; Miri checks the same code on the other tests"
    )]
    fn the_integer_reading_agrees_with_the_bytewise_order_of_short_keys() {
        /// The greatest integer below a key and the least above it.
        type Beside = (Option<Vec<u8>>, Option<Vec<u8>>);
        let mut keys = keys_over(&SAMPLE, CLASS_BOUND);
        keys.sort();
        let beside: Vec<Beside> = keys
            .iter()
            .map(|key| (super::below(key), above(key)))
            .collect();
        for (pair, around) in keys.windows(2).zip(beside.windows(2)) {
            let (key, next_key) = (&pair[0], &pair[1]);
            let ((lower, upper), (next_lower, next_upper)) = (&around[0], &around[1]);
            assert!(lower <= next_lower, "{} {}", text(key), text(next_key));
            assert!(upper <= next_upper, "{} {}", text(key), text(next_key));
            assert!(
                lower.as_ref().is_none_or(|found| found < key),
                "{}",
                text(key)
            );
            assert!(
                upper.as_ref().is_none_or(|found| found > key),
                "{}",
                text(key)
            );
        }
    }

    /// Digits that hold the first capital, the last capital, the first two small letters, the last symbol, and a decimal digit.
    const PAIR_SAMPLE: [u8; 6] = [1, 10, 35, 36, 37, 61];

    #[test]
    #[cfg_attr(
        miri,
        ignore = "an exhaustive sweep that Kani and the native run cover; Miri checks the same code on the other tests"
    )]
    fn the_choice_agrees_over_every_pair_of_short_keys() {
        let keys = keys_over(&PAIR_SAMPLE, CLASS_BOUND + 1);
        let bounds: Vec<Option<&[u8]>> = core::iter::once(None)
            .chain(keys.iter().map(|key| Some(key.as_slice())))
            .collect();
        for low in &bounds {
            for high in &bounds {
                assert!(place_spec(*low, *high), "{low:?} {high:?}");
            }
        }
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "an exhaustive sweep that Kani and the native run cover; Miri checks the same code on the other tests"
    )]
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
    #[cfg_attr(
        miri,
        ignore = "an exhaustive sweep that Kani and the native run cover; Miri checks the same code on the other tests"
    )]
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
            for high in &highs {
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
