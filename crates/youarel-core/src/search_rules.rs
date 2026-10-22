//! How a search finds the shortcuts it ranks: every shortcut of a small scope, and the shortcuts of the longest word typed in a larger one.
//!
//! A scope of fewer than [`SCAN_LIMIT`] shortcuts reads and scores each of them.
//! A larger scope reads at most [`CANDIDATE_LIMIT`] shortcuts with a word that starts with the longest word typed.
//! The index files each word of a shortcut's texts folded and cut as [`index_rules::query_words`] cuts the words of a query, so the words that start with a typed word lie in one range of the index's keys.
//! Past the scan limit a search therefore finds a shortcut only by a word that starts with what was typed, where a smaller scope finds it by any characters in order.
//! The start of an identifier's spelling bounds the identifiers that it spells, so a lookup by identifier reads those alone.

use crate::index_rules::{self, TERMINATOR, Token};
use alloc::vec::Vec;

/// How many live shortcuts a scope holds at most for a search to read and score each of them.
pub const SCAN_LIMIT: usize = 50_000;
/// The most shortcuts a search reads through the index of words.
pub const CANDIDATE_LIMIT: usize = 2_000;

/// How a search finds the shortcuts it ranks.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Strategy<W> {
    /// Read and score every shortcut of the scope.
    Scan,
    /// Read the shortcuts with a word that starts with this one, those whose word it is first.
    Index(W),
    /// Read nothing, since the query holds no word to look up.
    Nothing,
}

/// How a search over a scope of `live` shortcuts finds what it ranks, given the longest word of its query.
#[must_use]
pub fn plan<W>(live: usize, word: Option<W>) -> Strategy<W> {
    if live < SCAN_LIMIT {
        Strategy::Scan
    } else {
        word.map_or(Strategy::Nothing, Strategy::Index)
    }
}

/// How a search for `query` over a scope of `live` shortcuts finds what it ranks.
#[must_use]
pub fn strategy(live: usize, query: &str) -> Strategy<Token> {
    plan(live, longest(query))
}

/// The place of the first of the longest lengths, or none without a length.
#[must_use]
pub fn first_longest(lengths: &[usize]) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    for (place, length) in lengths.iter().enumerate() {
        if best.is_none_or(|(_, held)| *length > held) {
            best = Some((place, *length));
        }
    }
    best.map(|(place, _)| place)
}

/// The longest word of a query as the index files words, the first of them when several are as long, or none for a query without a word.
#[must_use]
pub fn longest(query: &str) -> Option<Token> {
    let words = index_rules::query_words(query);
    let lengths: Vec<usize> = words.iter().map(|word| word.as_str().len()).collect();
    let at = first_longest(&lengths)?;
    words.into_iter().nth(at)
}

/// The keys that start with a prefix: from the prefix itself, and before the first bytes after every one of them, or to the end when every key after the prefix starts with it.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Bounds {
    pub start: Vec<u8>,
    pub end: Option<Vec<u8>>,
}

impl Bounds {
    /// Whether a key lies within the bounds.
    #[must_use]
    pub fn holds(&self, key: &[u8]) -> bool {
        key >= self.start.as_slice() && self.end.as_ref().is_none_or(|end| key < end.as_slice())
    }
}

/// The bounds of the keys that start with a prefix.
#[must_use]
pub fn starting(prefix: &[u8]) -> Bounds {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if let Some(next) = last.checked_add(1) {
            end.push(next);
            return Bounds {
                start: prefix.to_vec(),
                end: Some(end),
            };
        }
    }
    Bounds {
        start: prefix.to_vec(),
        end: None,
    }
}

/// The bounds of the keys of the words equal to a word, which the terminator ends in each key.
#[must_use]
pub fn equal(word: &[u8]) -> Bounds {
    let mut ended = word.to_vec();
    ended.push(TERMINATOR);
    starting(&ended)
}

/// The bytes of an identifier.
const IDENTIFIER_BYTES: usize = 16;
/// The hexadecimal digits that spell one byte of an identifier.
const DIGITS_PER_BYTE: usize = 2;
/// The hexadecimal digits that spell a whole identifier.
const IDENTIFIER_DIGITS: usize = IDENTIFIER_BYTES * DIGITS_PER_BYTE;
/// The weight of a byte's first hexadecimal digit.
const HIGH_DIGIT: u8 = 16;
/// The value of a byte's second hexadecimal digit at its greatest.
const LOW_DIGITS: u8 = 15;
/// The value of the hexadecimal digit `a`.
const TEN: u8 = 10;

/// The value of a lowercase hexadecimal digit, as an identifier's spelling writes it.
const fn hexadecimal(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit.wrapping_sub(b'0')),
        b'a'..=b'f' => Some(digit.wrapping_sub(b'a').wrapping_add(TEN)),
        _ => None,
    }
}

/// The bounds of the identifiers whose lowercase hexadecimal spelling starts with `spelled`, or `None` when no identifier's spelling does.
///
/// No spelling starts with a character other than a lowercase hexadecimal digit, or with more digits than an identifier holds.
/// An odd digit at the end bounds the sixteen bytes whose first digit it is.
#[must_use]
pub fn identifiers(spelled: &[u8]) -> Option<Bounds> {
    if spelled.len() > IDENTIFIER_DIGITS {
        return None;
    }
    let mut start = Vec::with_capacity(IDENTIFIER_BYTES);
    for pair in spelled.chunks(DIGITS_PER_BYTE) {
        match pair {
            [high, low] => {
                let first = hexadecimal(*high)?.wrapping_mul(HIGH_DIGIT);
                start.push(first.wrapping_add(hexadecimal(*low)?));
            }
            [high] => {
                let first = hexadecimal(*high)?.wrapping_mul(HIGH_DIGIT);
                let mut last = start.clone();
                last.push(first.wrapping_add(LOW_DIGITS));
                start.push(first);
                return Some(Bounds {
                    start,
                    end: starting(&last).end,
                });
            }
            _ => return None,
        }
    }
    Some(starting(&start))
}

/// The properties Kani proves: a search scans exactly below the limit, the longest word is the first that no other passes, a word's bounds hold exactly the keys that start with it, and a spelling's bounds hold exactly the identifiers it starts.
#[cfg(any(test, kani))]
mod proofs {
    use super::{
        Bounds, DIGITS_PER_BYTE, HIGH_DIGIT, IDENTIFIER_BYTES, IDENTIFIER_DIGITS, SCAN_LIMIT,
        Strategy, first_longest, hexadecimal, identifiers, plan, starting,
    };

    /// The longest prefix and key the harness of the bounds draws.
    #[cfg(kani)]
    const PREFIX: usize = 2;
    #[cfg(kani)]
    const KEY: usize = 3;
    /// The most lengths the harness of the longest word draws.
    #[cfg(kani)]
    const WORDS: usize = 3;

    /// What a plan promises: a scan exactly below the limit, and otherwise the word's range when there is a word and nothing without one.
    pub(super) fn plan_spec(live: usize, word: Option<u8>) -> bool {
        match plan(live, word) {
            Strategy::Scan => live < SCAN_LIMIT,
            Strategy::Index(found) => live >= SCAN_LIMIT && word == Some(found),
            Strategy::Nothing => live >= SCAN_LIMIT && word.is_none(),
        }
    }

    /// What the first longest length promises: none without lengths, and otherwise a length that no other passes and that every one before it falls short of.
    pub(super) fn longest_spec(lengths: &[usize]) -> bool {
        first_longest(lengths).map_or(lengths.is_empty(), |at| {
            lengths.get(at).is_some_and(|best| {
                lengths.iter().all(|length| length <= best)
                    && lengths.iter().take(at).all(|length| length < best)
            })
        })
    }

    /// What a prefix's bounds promise: they hold a key exactly when it starts with the prefix.
    pub(super) fn bounds_spec(prefix: &[u8], key: &[u8]) -> bool {
        starting(prefix).holds(key) == key.starts_with(prefix)
    }

    /// The hexadecimal digit at a place of an identifier's spelling: the first or the second digit of one of its bytes.
    fn spelled_digit(id: &[u8; IDENTIFIER_BYTES], place: usize) -> Option<u8> {
        let byte = id.get(place.checked_div(DIGITS_PER_BYTE)?)?;
        if place.checked_rem(DIGITS_PER_BYTE)? == 0 {
            byte.checked_div(HIGH_DIGIT)
        } else {
            byte.checked_rem(HIGH_DIGIT)
        }
    }

    /// Whether the bounds that a spelling gives keep its promise: they hold an identifier exactly when its lowercase hexadecimal spelling starts with the spelling.
    pub(super) fn identifiers_hold(
        spelled: &[u8],
        id: &[u8; IDENTIFIER_BYTES],
        bounds: Option<&Bounds>,
    ) -> bool {
        let spells = spelled.len() <= IDENTIFIER_DIGITS
            && spelled.iter().enumerate().all(|(place, digit)| {
                let value = spelled_digit(id, place);
                value.is_some() && hexadecimal(*digit) == value
            });
        bounds.is_some_and(|bounds| bounds.holds(id)) == spells
    }

    /// What a spelling's bounds promise.
    #[cfg(test)]
    pub(super) fn identifiers_spec(spelled: &[u8], id: &[u8; IDENTIFIER_BYTES]) -> bool {
        identifiers_hold(spelled, id, identifiers(spelled).as_ref())
    }

    /// Every count of live shortcuts, with a word and without: a search scans exactly below the limit.
    #[cfg(kani)]
    #[kani::proof]
    fn a_search_scans_exactly_below_the_limit() {
        let live: usize = kani::any();
        let word: Option<u8> = kani::any();
        kani::assert(
            plan_spec(live, word),
            "a search scans exactly below the limit",
        );
        kani::cover!(
            matches!(plan(live, word), Strategy::Index(_)),
            "a search through the index"
        );
        kani::cover!(
            matches!(plan(live, word), Strategy::Nothing),
            "a query without a word"
        );
    }

    /// Every list of up to three lengths: the longest word is the first that no other passes.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn the_longest_word_is_the_first_that_no_other_passes() {
        let lengths: [usize; WORDS] = kani::any();
        let count: usize = kani::any();
        kani::assume(count <= WORDS);
        kani::assert(
            longest_spec(&lengths[..count]),
            "the longest word is the first that no other passes",
        );
        kani::cover!(
            count == WORDS && lengths[0] == lengths[2] && lengths[1] < lengths[0],
            "two words as long, apart"
        );
    }

    /// Every prefix of up to two bytes and every key of up to three, each byte any value: a word's bounds hold exactly the keys that start with it.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn a_word_s_bounds_hold_exactly_the_keys_that_start_with_it() {
        let prefix: [u8; PREFIX] = kani::any();
        let key: [u8; KEY] = kani::any();
        let (prefix_length, key_length): (usize, usize) = (kani::any(), kani::any());
        kani::assume(prefix_length <= PREFIX && key_length <= KEY);
        let (prefix, key) = (&prefix[..prefix_length], &key[..key_length]);
        kani::assert(
            bounds_spec(prefix, key),
            "a word's bounds hold exactly the keys that start with it",
        );
        kani::cover!(
            prefix_length == PREFIX && prefix.iter().all(|byte| *byte == u8::MAX),
            "a prefix of the last bytes, which no end bounds"
        );
        kani::cover!(
            key_length == KEY && key.starts_with(prefix) && prefix_length > 0,
            "a key that starts with the prefix"
        );
    }

    /// The most bytes of a spelling the harness of identifiers draws.
    #[cfg(kani)]
    const SPELLED: usize = 3;

    /// Every spelling of up to three bytes, each any value, against every identifier with its first two bytes drawn: the bounds hold exactly the identifiers whose spelling starts with it.
    ///
    /// Each length is fixed in turn before the bytes are drawn, so the bounds grow to a known size.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn a_spelling_s_bounds_hold_exactly_the_identifiers_it_starts() {
        for length in 0..=SPELLED {
            let spelled: [u8; SPELLED] = kani::any();
            let first: [u8; 2] = kani::any();
            let mut id = [0_u8; IDENTIFIER_BYTES];
            id[0] = first[0];
            id[1] = first[1];
            let spelling = &spelled[..length];
            let bounds = identifiers(spelling);
            kani::assert(
                identifiers_hold(spelling, &id, bounds.as_ref()),
                "a spelling's bounds hold exactly the identifiers it starts",
            );
            let odd = length == SPELLED;
            kani::cover!(
                odd && bounds.as_ref().is_some_and(|bounds| bounds.holds(&id)),
                "an odd spelling that starts an identifier"
            );
            kani::cover!(
                odd && bounds.as_ref().is_some_and(|bounds| bounds.end.is_none()),
                "a spelling of the last digits, which no end bounds"
            );
        }
    }

    /// A small scope must scan every shortcut; this probe claims one just below the limit reads the index and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_an_index_search_below_the_limit() {
        kani::assert(
            plan(SCAN_LIMIT - 1, Some(0_u8)) == Strategy::Index(0),
            "NEGATIVE_SEARCH_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{bounds_spec, identifiers_spec, longest_spec, plan_spec};
    use super::{
        Bounds, CANDIDATE_LIMIT, IDENTIFIER_BYTES, SCAN_LIMIT, Strategy, equal, first_longest,
        identifiers, longest, plan, starting, strategy,
    };
    use alloc::vec::Vec;

    #[test]
    fn a_spelling_bounds_the_identifiers_it_starts() {
        assert_eq!(
            identifiers(b"0a"),
            Some(Bounds {
                start: alloc::vec![0x0a],
                end: Some(alloc::vec![0x0b]),
            })
        );
        assert_eq!(
            identifiers(b"0a3"),
            Some(Bounds {
                start: alloc::vec![0x0a, 0x30],
                end: Some(alloc::vec![0x0a, 0x40]),
            }),
            "an odd digit bounds the bytes it starts"
        );
        assert_eq!(identifiers(b"fff").and_then(|bounds| bounds.end), None);
        assert_eq!(
            identifiers(b"").map(|bounds| bounds.start),
            Some(Vec::new())
        );
        for refused in [&b"0A"[..], b"0g", b"0 ", b"\xff"] {
            assert_eq!(identifiers(refused), None, "{refused:?}");
        }
        assert!(identifiers(&[b'0'; 32]).is_some());
        assert_eq!(identifiers(&[b'0'; 33]), None, "longer than an identifier");
        let digits = *b"09af-A";
        let mut spellings: Vec<Vec<u8>> = alloc::vec![Vec::new()];
        for _ in 0..3 {
            let longer: Vec<Vec<u8>> = spellings
                .iter()
                .flat_map(|spelling| {
                    digits.iter().map(move |digit| {
                        let mut next = spelling.clone();
                        next.push(*digit);
                        next
                    })
                })
                .collect();
            spellings.extend(longer);
            spellings.sort();
            spellings.dedup();
        }
        for first in [0x00_u8, 0x09, 0x0a, 0x90, 0x9f, 0xa0, 0xaf, 0xf0, 0xff] {
            for second in [0x00_u8, 0x0a, 0x9f, 0xf0, 0xff] {
                let mut id = [0x55_u8; IDENTIFIER_BYTES];
                id[0] = first;
                id[1] = second;
                for spelling in &spellings {
                    assert!(identifiers_spec(spelling, &id), "{spelling:?} {id:?}");
                }
            }
        }
    }

    #[test]
    fn a_search_scans_below_the_limit_and_reads_the_longest_word_past_it() {
        assert_eq!(strategy(SCAN_LIMIT - 1, "c.ex"), Strategy::Scan);
        let indexed = strategy(SCAN_LIMIT, "c.Example");
        assert!(
            matches!(&indexed, Strategy::Index(word) if word.as_str() == "example"),
            "{indexed:?}"
        );
        assert_eq!(strategy(SCAN_LIMIT, "/ .."), Strategy::Nothing);
        assert_eq!(plan(SCAN_LIMIT, Some(4_u8)), Strategy::Index(4));
        assert_eq!(plan::<u8>(usize::MAX, None), Strategy::Nothing);
        const { assert!(CANDIDATE_LIMIT < SCAN_LIMIT) };
        for live in [0, 1, SCAN_LIMIT - 1, SCAN_LIMIT, SCAN_LIMIT + 1, usize::MAX] {
            for word in [None, Some(0_u8), Some(9)] {
                assert!(plan_spec(live, word), "{live} {word:?}");
            }
        }
    }

    #[test]
    fn the_longest_word_is_the_first_of_the_longest() {
        assert_eq!(first_longest(&[]), None);
        assert_eq!(first_longest(&[2, 5, 5, 1]), Some(1));
        assert_eq!(first_longest(&[0]), Some(0));
        assert_eq!(first_longest(&[3, 1, 3]), Some(0));
        let found = longest("go to the Cathedral cathedrals").unwrap();
        assert_eq!(found.as_str(), "cathedrals");
        assert_eq!(longest("ab cd").unwrap().as_str(), "ab");
        assert_eq!(longest("extraordinarily"), longest("extraordinar"));
        assert_eq!(longest(" / "), None);
        let values = [0, 1, 2, 3];
        for first in values {
            for second in values {
                for third in values {
                    let lengths = [first, second, third];
                    for count in 0..=lengths.len() {
                        assert!(longest_spec(&lengths[..count]), "{lengths:?} {count}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_word_s_bounds_hold_the_keys_that_start_with_it() {
        assert_eq!(
            starting(b"ab"),
            Bounds {
                start: b"ab".to_vec(),
                end: Some(b"ac".to_vec())
            }
        );
        assert_eq!(starting(&[1, 0xff]).end, Some(alloc::vec![2]));
        assert_eq!(starting(&[0xff, 0xff]).end, None);
        assert_eq!(starting(&[]).end, None);
        assert_eq!(
            equal(b"ab"),
            Bounds {
                start: b"ab\0".to_vec(),
                end: Some(b"ab\x01".to_vec())
            }
        );
        assert!(equal(b"ab").holds(b"ab\0id"));
        assert!(!equal(b"ab").holds(b"abc\0id"));
        let bytes = [0_u8, 1, 0xfe, 0xff];
        let mut texts: Vec<Vec<u8>> = alloc::vec![Vec::new()];
        for _ in 0..3 {
            let longer: Vec<Vec<u8>> = texts
                .iter()
                .flat_map(|text| {
                    bytes.iter().map(move |byte| {
                        let mut next = text.clone();
                        next.push(*byte);
                        next
                    })
                })
                .collect();
            texts.extend(longer);
            texts.sort();
            texts.dedup();
        }
        for prefix in texts.iter().filter(|text| text.len() <= 2) {
            for key in &texts {
                assert!(bounds_spec(prefix, key), "{prefix:?} {key:?}");
            }
        }
    }
}
