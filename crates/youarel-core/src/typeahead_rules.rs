//! What a person types into a listing to select a row by its name: the letters typed within a moment of each other, and whether a name starts with them.
//!
//! A key typed within [`TYPE_AHEAD_WINDOW`] of the one before it joins the letters typed so far, and a key typed later starts them anew.
//! The letters keep their folded form, as the index of titles folds a title, and at most as many bytes of it as a key of that index keeps.
//! A seek in that index then finds the first title that starts with them, since a title starts with them exactly when the start its key keeps does.

use crate::index_rules::{self, SORT_PREFIX};
use crate::sync_rules::SECOND;
use alloc::string::String;
use alloc::vec::Vec;

/// How long after a key the next one still joins it, in milliseconds.
pub const TYPE_AHEAD_WINDOW: u64 = SECOND;
/// The most bytes the folded letters hold, as many as a key of the index of titles keeps of a title.
pub const LONGEST_TYPED: usize = SORT_PREFIX;

/// The letters typed so far in their folded form, and when the last key came.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct Typed {
    pub letters: String,
    /// When the last key came, in milliseconds since the Unix epoch.
    pub at: u64,
}

/// What a key does to the letters typed so far.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Typing {
    /// The key joins the letters, which came within the window and leave room for it.
    Joins,
    /// The letters stay as they are, since they came within the window and leave no room for the key.
    Holds,
    /// The key starts the letters anew: none came before, the window passed, or the clock moved back.
    Starts,
}

/// What a key of `key` folded bytes that comes at `now` does to the letters typed so far, which `before` names by how many folded bytes they hold and when their last key came.
#[must_use]
pub const fn typing(before: Option<(usize, u64)>, key: usize, now: u64) -> Typing {
    let Some((held, at)) = before else {
        return Typing::Starts;
    };
    match now.checked_sub(at) {
        Some(gap) if gap <= TYPE_AHEAD_WINDOW => match held.checked_add(key) {
            Some(total) if total <= LONGEST_TYPED => Typing::Joins,
            Some(_) | None => Typing::Holds,
        },
        Some(_) | None => Typing::Starts,
    }
}

/// The letters typed once `key` comes at `now`, as [`typing`] decides.
#[must_use]
pub fn typed(before: Option<&Typed>, key: char, now: u64) -> Typed {
    let folded: String = key.to_lowercase().collect();
    let held = before.map(|before| (before.letters.len(), before.at));
    let letters = match (typing(held, folded.len(), now), before) {
        (Typing::Joins, Some(before)) => [before.letters.as_str(), &folded].concat(),
        (Typing::Holds, Some(before)) => before.letters.clone(),
        (Typing::Starts, _) | (Typing::Joins | Typing::Holds, None) => folded,
    };
    Typed { letters, at: now }
}

/// Whether a name starts with the folded letters, once folded as the index of titles folds it.
#[must_use]
pub fn starts(name: &str, letters: &str) -> bool {
    name.to_lowercase().starts_with(letters)
}

/// The start of the keys of the index of titles from which the titles that start with the folded letters file, as far as a key keeps a title.
#[must_use]
pub fn seek(letters: &str) -> Vec<u8> {
    index_rules::prefix::<SORT_PREFIX>(letters.as_bytes()).to_vec()
}

/// The properties Kani proves over every key of any size at every reading of the clock after letters of any size typed at any time: a key joins the letters typed within the window that leave room for it, holds them when they leave none, and starts them anew otherwise.
#[cfg(any(test, kani))]
mod proofs {
    use super::{LONGEST_TYPED, TYPE_AHEAD_WINDOW, Typing, typing};

    /// Whether a key decides as the letters promise, read without the arithmetic the decision does: the window holds a key at or after the last one by at most its span, and the room holds a key whose bytes the bound leaves after the letters'.
    pub(super) fn typing_holds(before: Option<(usize, u64)>, key: usize, now: u64) -> bool {
        let within =
            before.is_some_and(|(_, at)| at <= now && now.abs_diff(at) <= TYPE_AHEAD_WINDOW);
        let room = before
            .is_some_and(|(held, _)| held <= LONGEST_TYPED && key <= LONGEST_TYPED.abs_diff(held));
        let expected = match (within, room) {
            (false, _) => Typing::Starts,
            (true, true) => Typing::Joins,
            (true, false) => Typing::Holds,
        };
        typing(before, key, now) == expected
    }

    /// Every key of any size at every reading of the clock, after no letters or after letters of any size whose last key came at any time.
    #[cfg(kani)]
    #[kani::proof]
    fn a_key_joins_the_letters_typed_within_the_window_and_starts_anew_after_it() {
        let before: Option<(usize, u64)> = kani::any();
        let key: usize = kani::any();
        let now: u64 = kani::any();
        kani::assert(
            typing_holds(before, key, now),
            "a key joins the letters typed within the window that leave room for it and starts them anew after it",
        );
        kani::cover!(
            before.is_some_and(|(held, at)| now.checked_sub(at) == Some(TYPE_AHEAD_WINDOW)
                && held.checked_add(key) == Some(LONGEST_TYPED)),
            "a key at the end of the window fills the letters"
        );
        kani::cover!(
            before.is_some_and(|(_, at)| now < at),
            "a clock that moved back"
        );
        kani::cover!(
            typing(before, key, now) == Typing::Holds,
            "letters that leave no room hold"
        );
    }

    /// A key typed after the window must start the letters anew; this probe claims it joins them and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_letter_joined_after_the_window() {
        let decided = typing(Some((1, 0)), 1, TYPE_AHEAD_WINDOW.saturating_add(1));
        kani::assert(decided == Typing::Joins, "NEGATIVE_TYPEAHEAD_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::typing_holds;
    use super::{LONGEST_TYPED, TYPE_AHEAD_WINDOW, Typed, Typing, seek, starts, typed, typing};
    use crate::index_rules::{self, SORT_PREFIX};
    use crate::state::ItemId;
    use alloc::string::String;

    #[test]
    fn letters_join_within_the_window_and_start_anew_after_it() {
        let first = typed(None, 'G', 10);
        assert_eq!(
            first,
            Typed {
                letters: String::from("g"),
                at: 10,
            }
        );
        let second = typed(Some(&first), 'i', 10 + TYPE_AHEAD_WINDOW);
        assert_eq!(second.letters, "gi");
        let later = typed(Some(&second), 't', 10 + 3 * TYPE_AHEAD_WINDOW);
        assert_eq!(later.letters, "t");
        assert_eq!(
            typed(Some(&second), 'x', 9).letters,
            "x",
            "a clock that moved back"
        );
        assert_eq!(typed(None, 'İ', 0).letters, "i\u{307}", "a key folds");
    }

    #[test]
    fn held_letters_stop_at_what_a_key_of_the_index_keeps() {
        let mut held = typed(None, 'a', 0);
        for _ in 0..LONGEST_TYPED + 5 {
            held = typed(Some(&held), 'a', held.at);
        }
        assert_eq!(held.letters.len(), LONGEST_TYPED);
        let full = typed(Some(&held), 'é', held.at + 1);
        assert_eq!(
            full.letters.len(),
            LONGEST_TYPED,
            "a key without room holds"
        );
        assert_eq!(full.at, held.at + 1, "and the window starts again from it");
        assert_eq!(typing(Some((usize::MAX, 0)), 1, 0), Typing::Holds);
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "an exhaustive sweep that Kani and the native run cover"
    )]
    fn the_typing_specification_agrees_over_a_drawn_domain() {
        let times = [0, 1, TYPE_AHEAD_WINDOW, TYPE_AHEAD_WINDOW + 1, u64::MAX];
        let sizes = [0, 1, LONGEST_TYPED - 1, LONGEST_TYPED, usize::MAX];
        for now in times {
            for key in sizes {
                assert!(typing_holds(None, key, now));
                for at in times {
                    for held in sizes {
                        assert!(
                            typing_holds(Some((held, at)), key, now),
                            "{held} {at} {key} {now}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_name_starts_with_the_folded_letters_whatever_its_case() {
        assert!(starts("GitHub", "git"));
        assert!(starts("École", "éc"));
        assert!(starts("anything", ""));
        assert!(!starts("GitLab", "gith"));
        assert!(!starts("", "a"));
    }

    #[test]
    fn a_seek_starts_where_the_index_of_titles_files_the_letters() {
        let id = ItemId([3; 16]);
        let key = index_rules::title_key("GitHub Docs", id);
        let part = index_rules::sort_part(&key);
        assert!(part >= seek("gith").as_slice());
        assert!(part < seek("gitl").as_slice());
        assert!(index_rules::sort_part(&index_rules::title_key("Gh", id)) < seek("git").as_slice());
        assert!(index_rules::sort_part(&index_rules::title_key("Gi", id)) < seek("git").as_slice());
        let long = "x".repeat(SORT_PREFIX + 3);
        assert_eq!(
            seek(&long).len(),
            SORT_PREFIX,
            "a key keeps no more of a title"
        );
    }
}
