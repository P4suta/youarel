//! Fuzzy matching of a typed query against a shortcut's text.
//!
//! A query matches when its characters appear in the text in order, ignoring ASCII case.
//! The match takes each query character at its earliest place, and its score rewards characters that follow one another and characters that start a word.

/// The score of one matched character.
pub const MATCHED: u32 = 16;
/// The bonus of a matched character right after the previous one.
pub const CONSECUTIVE: u32 = 16;
/// The bonus of a matched character at the start of the text or of a word.
pub const BOUNDARY: u32 = 8;

/// The characters after which a word starts.
const SEPARATORS: [char; 10] = [' ', '/', '.', '-', '_', ':', '?', '#', '&', '='];

/// A character with its ASCII case removed.
const fn fold(character: char) -> char {
    character.to_ascii_lowercase()
}

/// Whether `current` starts a word after `previous`.
fn starts_word(previous: Option<char>, current: char) -> bool {
    previous.is_none_or(|previous| {
        SEPARATORS.contains(&previous)
            || (previous.is_ascii_lowercase() && current.is_ascii_uppercase())
    })
}

/// The score of `query` in `text`, or `None` when the query's characters do not appear in order.
///
/// An empty query matches every text with a score of zero.
#[must_use]
pub fn score(query: &str, text: &str) -> Option<u32> {
    score_characters(query.chars(), text.chars())
}

/// The score over the characters of a query and a text, which [`score`] reads from strings.
fn score_characters(
    query: impl Iterator<Item = char>,
    text: impl Iterator<Item = char>,
) -> Option<u32> {
    let mut wanted = query.map(fold).peekable();
    let mut total: u32 = 0;
    let mut previous: Option<char> = None;
    let mut previous_matched = false;
    for character in text {
        let Some(next) = wanted.peek() else {
            break;
        };
        if fold(character) == *next {
            wanted.next();
            let mut gained = MATCHED;
            if previous_matched {
                gained = gained.saturating_add(CONSECUTIVE);
            }
            if starts_word(previous, character) {
                gained = gained.saturating_add(BOUNDARY);
            }
            total = total.saturating_add(gained);
            previous_matched = true;
        } else {
            previous_matched = false;
        }
        previous = Some(character);
    }
    match wanted.peek() {
        Some(_) => None,
        None => Some(total),
    }
}

/// The properties Kani proves over every query and text of up to [`proofs::TEXT_BOUND`] characters from a small alphabet.
#[cfg(any(test, kani))]
mod proofs {
    use super::{BOUNDARY, CONSECUTIVE, MATCHED, fold, score_characters};

    /// The longest query and text the harness draws.
    pub(super) const TEXT_BOUND: usize = 3;

    /// Whether the characters of `query` appear in `text` in order once both lose their ASCII case, by one pass that takes each at its earliest place.
    pub(super) fn in_order(query: &[char], text: &[char]) -> bool {
        let mut remaining = query.iter();
        let mut next = remaining.next();
        for character in text {
            if let Some(wanted) = next
                && fold(*wanted) == fold(*character)
            {
                next = remaining.next();
            }
        }
        next.is_none()
    }

    /// What `score` promises about its value: a score only when the characters appear in order, within the most a match of that length can earn.
    pub(super) fn spec(query: &[char], text: &[char]) -> bool {
        let most = u32::try_from(query.len()).map_or(u32::MAX, |length| {
            length.saturating_mul(MATCHED + CONSECUTIVE + BOUNDARY)
        });
        let matches = in_order(query, text);
        score_characters(query.iter().copied(), text.iter().copied()).map_or(!matches, |found| {
            matches && found <= most && (query.is_empty() || found >= MATCHED)
        })
    }

    /// A character as the harness draws it, from an alphabet that holds case, a separator, and a non-ASCII letter.
    #[cfg(kani)]
    fn character(choice: u8) -> char {
        match choice {
            0 => 'a',
            1 => 'A',
            2 => 'b',
            3 => '/',
            _ => 'é',
        }
    }

    #[cfg(kani)]
    fn draw() -> ([char; TEXT_BOUND], usize) {
        let drawn = [
            character(kani::any()),
            character(kani::any()),
            character(kani::any()),
        ];
        let length: usize = kani::any();
        kani::assume(length <= TEXT_BOUND);
        (drawn, length)
    }

    /// Every loop runs at most once per drawn character, and the separator check once per separator.
    ///
    /// The value's properties hold for every drawn pair, and then any placement of the query's characters at increasing places of the text must score.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(11)]
    fn a_query_scores_exactly_when_its_characters_appear_in_order() {
        let (query, query_length) = draw();
        let (text, text_length) = draw();
        let (Some(query), Some(text)) = (query.get(..query_length), text.get(..text_length)) else {
            return;
        };
        kani::assert(spec(query, text), "a score only for characters in order");
        let found = score_characters(query.iter().copied(), text.iter().copied());
        kani::cover!(found.is_some() && !query.is_empty(), "a match");
        kani::cover!(found.is_none(), "no match");
        let places: [usize; TEXT_BOUND] = kani::any();
        for (index, wanted) in query.iter().enumerate() {
            let place = places.get(index).copied().unwrap_or(usize::MAX);
            kani::assume(place < text.len());
            kani::assume(index == 0 || places.get(index - 1).is_some_and(|before| *before < place));
            kani::assume(
                text.get(place)
                    .is_some_and(|character| fold(*character) == fold(*wanted)),
            );
        }
        kani::assert(found.is_some(), "characters in order always score");
    }

    /// A query whose characters appear out of order must not match; this probe claims it does and must fail.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn rejects_a_query_out_of_order() {
        kani::assert(
            score_characters(['b', 'a'].into_iter(), ['a', 'b'].into_iter()).is_some(),
            "NEGATIVE_MATCH_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{TEXT_BOUND, in_order, spec};
    use super::{BOUNDARY, CONSECUTIVE, MATCHED, fold, score};
    use alloc::vec::Vec;

    /// Whether `query` is a subsequence of `text` once both lose their ASCII case, by a search over every placement.
    fn subsequence(query: &[char], text: &[char]) -> bool {
        match (query.split_first(), text.split_first()) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some((first, rest)), Some((head, tail))) => {
                (fold(*first) == fold(*head) && subsequence(rest, tail)) || subsequence(query, tail)
            }
        }
    }

    #[test]
    fn characters_in_order_match_regardless_of_ascii_case() {
        assert_eq!(score("", "anything"), Some(0));
        assert_eq!(score("gh", "GitHub"), Some(2 * MATCHED + 2 * BOUNDARY));
        assert_eq!(
            score("git", "github.com"),
            Some(3 * MATCHED + 2 * CONSECUTIVE + BOUNDARY)
        );
        assert_eq!(score("hg", "GitHub"), None);
        assert_eq!(score("x", ""), None);
        assert_eq!(
            score("ドキュ", "社内ドキュメント"),
            Some(3 * MATCHED + 2 * CONSECUTIVE)
        );
    }

    #[test]
    fn a_word_start_and_a_run_score_above_scattered_characters() {
        let start = score("doc", "rust docs").unwrap();
        let scattered = score("doc", "dashboard of cats").unwrap();
        assert!(start > scattered, "{start} {scattered}");
        assert!(score("hub", "/hub").unwrap() > score("hub", "h-u-b").unwrap());
        assert_eq!(score("a", "x/a"), Some(MATCHED + BOUNDARY));
        assert_eq!(score("b", "aB"), Some(MATCHED + BOUNDARY));
        assert_eq!(score("b", "ab"), Some(MATCHED));
    }

    #[test]
    fn the_specification_agrees_over_every_short_pair() {
        let alphabet = ['a', 'A', 'b', '/', 'é'];
        let mut texts: Vec<Vec<char>> = alloc::vec![Vec::new()];
        for _ in 0..TEXT_BOUND {
            let longer: Vec<Vec<char>> = texts
                .iter()
                .flat_map(|text| {
                    alphabet.iter().map(move |character| {
                        let mut next = text.clone();
                        next.push(*character);
                        next
                    })
                })
                .collect();
            texts.extend(longer);
            texts.sort();
            texts.dedup();
        }
        for query in &texts {
            for text in &texts {
                assert!(spec(query, text), "{query:?} {text:?}");
                assert_eq!(
                    in_order(query, text),
                    subsequence(query, text),
                    "{query:?} {text:?}"
                );
            }
        }
    }
}
