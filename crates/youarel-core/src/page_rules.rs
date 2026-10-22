//! Pages of a listing: a window of places, the part that holds a place, the cursor a next page starts after, and the merge of sorted lists.
//!
//! The merge serves a scope over a folder and the folders inside it, which reads each folder's own list.
//! A listing in places shows the window of a page and the next page starts where the window ends, so pages in turn tile the listing.
//! A listing of parts, such as folders before shortcuts or a group under each heading, finds a place in the one part that holds it.
//! A listing by keys starts each page strictly after the last key of the page before, so a row that another page showed never shows again.

use alloc::vec::Vec;
use core::ops::Range;

/// The places of a page: from `first`, at most `limit` of them, within `total`.
#[must_use]
pub fn window(total: usize, first: usize, limit: usize) -> Range<usize> {
    let start = first.min(total);
    start..start.saturating_add(limit).min(total)
}

/// Where a place falls in a listing of parts in turn, whose sizes `sizes` holds: the part and the place within it, or none past the listing's end.
#[must_use]
pub fn locate(sizes: &[usize], place: usize) -> Option<(usize, usize)> {
    let mut rest = place;
    for (part, size) in sizes.iter().enumerate() {
        match rest.checked_sub(*size) {
            Some(after) => rest = after,
            None => return Some((part, rest)),
        }
    }
    None
}

/// How a view pages a listing: how many rows a page holds, and how many of them come before a row the view brings into sight.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Paging {
    pub limit: usize,
    pub lead: usize,
}

/// Where the page a view answers with starts: where the page in sight starts at `sight`, or `lead` rows before a row the view brings into sight that the page in sight lacks.
///
/// The page never starts so late that it holds fewer rows than the listing could fill.
#[must_use]
pub fn start(total: usize, sight: usize, reveal: Option<usize>, paging: Paging) -> usize {
    let wanted = match reveal {
        Some(place) if !window(total, sight, paging.limit).contains(&place) => {
            place.saturating_sub(paging.lead)
        }
        Some(_) | None => sight,
    };
    wanted.min(total.saturating_sub(paging.limit))
}

/// Where a page of a listing by keys starts: at the start, or strictly after the last key that the page before showed.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Cursor<K> {
    last: Option<K>,
}

impl<K: Ord> Cursor<K> {
    /// The cursor of the first page.
    #[must_use]
    pub const fn start() -> Self {
        Self { last: None }
    }

    /// The cursor of the page after one whose last key is `last`.
    #[must_use]
    pub const fn after(last: K) -> Self {
        Self { last: Some(last) }
    }

    /// Whether a key lies on a page from the cursor on.
    #[must_use]
    pub fn admits(&self, key: &K) -> bool {
        self.last.as_ref().is_none_or(|last| key > last)
    }
}

/// The first `limit` keys from the cursor on of the sorted union of ascending lists, which a scope over a folder and the folders inside it reads from each folder's own list.
///
/// The merge takes one key a round, at most as many rounds as the lists hold keys, and a key both lists hold shows once for each.
#[must_use]
pub fn merged_page<K: Ord + Clone>(lists: &[&[K]], cursor: &Cursor<K>, limit: usize) -> Vec<K> {
    let mut heads: Vec<&[K]> = lists
        .iter()
        .map(|list| {
            let passed = list.partition_point(|key| !cursor.admits(key));
            list.get(passed..).unwrap_or_default()
        })
        .collect();
    let held = heads
        .iter()
        .fold(0_usize, |total, list| total.saturating_add(list.len()));
    let mut page = Vec::new();
    for _ in 0..held.min(limit) {
        let smallest = heads
            .iter()
            .enumerate()
            .filter_map(|(at, list)| list.first().map(|key| (key, at)))
            .min();
        let Some((key, at)) = smallest else {
            break;
        };
        page.push(key.clone());
        if let Some(list) = heads.get_mut(at) {
            *list = list.get(1..).unwrap_or_default();
        }
    }
    page
}

/// The properties Kani proves over small listings: consecutive windows tile a listing, a page in sight keeps what it showed and brings a row into sight, a place falls in the one part that holds it, and a merged page is the start of the sorted union after its cursor.
#[cfg(any(test, kani))]
mod proofs {
    use super::{Cursor, Paging, locate, merged_page, start, window};
    #[cfg(test)]
    use alloc::vec::Vec;

    /// The most keys of each list the merge harness draws, and the most it pages.
    pub(super) const KEYS: usize = 2;
    pub(super) const LIMIT: usize = 4;
    /// The keys the merge harness draws lie below this.
    pub(super) const VALUES: usize = 4;
    /// The most parts the harness of the places draws.
    #[cfg(kani)]
    const PARTS: usize = 3;

    /// The sum of sizes, or none when it passes every `usize`.
    fn sum(sizes: &[usize]) -> Option<usize> {
        sizes
            .iter()
            .try_fold(0_usize, |sum, size| sum.checked_add(*size))
    }

    /// What locating promises: a place within the listing falls in a part that holds it, at its place after the parts before, and a place past the listing's end falls in none.
    pub(super) fn locate_spec(sizes: &[usize], place: usize) -> bool {
        match locate(sizes, place) {
            Some((part, offset)) => {
                let before = sizes.get(..part).and_then(sum);
                sizes.get(part).is_some_and(|size| offset < *size)
                    && before.and_then(|before| before.checked_add(offset)) == Some(place)
            }
            None => sum(sizes).is_some_and(|total| place >= total),
        }
    }

    /// How often each key below [`VALUES`] occurs.
    fn counts<'a>(keys: impl Iterator<Item = &'a u8>) -> [usize; VALUES] {
        let mut counts = [0_usize; VALUES];
        for key in keys {
            if let Some(count) = counts.get_mut(usize::from(*key)) {
                *count = count.saturating_add(1);
            }
        }
        counts
    }

    /// What windows promise: a window lies within the listing and holds at most its limit, and the next window starts where it ends, so together they hold the window of both limits.
    pub(super) fn window_spec(total: usize, first: usize, limit: usize) -> bool {
        let page = window(total, first, limit);
        let within = page.start <= page.end && page.end <= total && page.len() <= limit;
        let tiled = first.checked_add(limit).is_none_or(|next| {
            let following = window(total, next, limit);
            page.end == following.start
                && limit
                    .checked_mul(2)
                    .is_none_or(|both| window(total, first, both) == (page.start..following.end))
        });
        within && tiled
    }

    /// What the start of a page promises: the page holds as many rows as the listing can fill, the row brought into sight when a lead shorter than a page leaves room for it, and every row of the page in sight when no row is brought into sight from outside it.
    pub(super) fn start_spec(
        total: usize,
        sight: usize,
        reveal: Option<usize>,
        paging: Paging,
    ) -> bool {
        let page = window(total, start(total, sight, reveal, paging), paging.limit);
        let in_sight = window(total, sight, paging.limit);
        let full = page.len() == total.min(paging.limit);
        let revealed = reveal.is_none_or(|place| {
            place >= total || paging.lead >= paging.limit || page.contains(&place)
        });
        let kept = reveal.is_some_and(|place| !in_sight.contains(&place))
            || in_sight.is_empty()
            || (page.start <= in_sight.start && in_sight.end <= page.end);
        full && revealed && kept
    }

    /// What a merged page promises over two ascending lists of keys below [`VALUES`]: it holds, in ascending order, the first keys after the cursor of both lists together, up to the limit.
    ///
    /// The page holds the smallest of those keys when it holds no key more often than the lists do, and every key below its last as often as the lists do.
    pub(super) fn merged_spec(left: &[u8], right: &[u8], last: Option<u8>, limit: usize) -> bool {
        let cursor = last.map_or_else(Cursor::start, Cursor::after);
        let page = merged_page(&[left, right], &cursor, limit);
        let union = counts(left.iter().chain(right).filter(|key| cursor.admits(key)));
        let held = union
            .iter()
            .fold(0_usize, |sum, count| sum.saturating_add(*count));
        let shown = counts(page.iter());
        let top = page.last().map_or(0, |key| usize::from(*key));
        page.len() == held.min(limit)
            && page.windows(2).all(|pair| pair.first() <= pair.last())
            && (0..VALUES)
                .zip(union.iter().zip(&shown))
                .all(|(key, (all, some))| some <= all && (key >= top || some == all))
    }

    /// Every total, first place, and limit: windows tile the listing.
    #[cfg(kani)]
    #[kani::proof]
    fn consecutive_windows_tile_a_listing_without_a_skip_or_a_repeat() {
        let total: usize = kani::any();
        let first: usize = kani::any();
        let limit: usize = kani::any();
        kani::assert(window_spec(total, first, limit), "windows tile a listing");
        kani::cover!(
            first < total && window(total, first, limit).end == total,
            "a last page that the total cuts short"
        );
    }

    /// Every total, place in sight, row to bring into sight or none, limit, and lead: the page holds what the listing fills, the row brought into sight, and what was in sight.
    #[cfg(kani)]
    #[kani::proof]
    fn a_page_keeps_what_was_in_sight_and_brings_a_row_into_sight() {
        let total: usize = kani::any();
        let sight: usize = kani::any();
        let reveal: Option<usize> = kani::any();
        let paging = Paging {
            limit: kani::any(),
            lead: kani::any(),
        };
        kani::assert(
            start_spec(total, sight, reveal, paging),
            "a page keeps what was in sight and brings a row into sight",
        );
        kani::cover!(
            paging.lead < paging.limit
                && reveal.is_some_and(|place| {
                    place < total && place > sight.saturating_add(paging.limit)
                }),
            "a row brought into sight from far below"
        );
        kani::cover!(
            reveal.is_none()
                && paging.limit < total
                && sight < total
                && total < sight.saturating_add(paging.limit),
            "a page that the listing's end pulls back"
        );
    }

    /// Every list of up to three parts of any size and every place: a place falls in the one part that holds it.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn a_place_falls_in_the_one_part_that_holds_it() {
        let sizes: [usize; PARTS] = kani::any();
        let count: usize = kani::any();
        kani::assume(count <= PARTS);
        let place: usize = kani::any();
        kani::assert(
            locate_spec(&sizes[..count], place),
            "a place falls in the one part that holds it",
        );
        kani::cover!(
            count == PARTS
                && sizes[1] == 0
                && locate(&sizes, place).is_some_and(|(part, _)| part == 2),
            "a place past an empty part"
        );
        kani::cover!(
            count > 0 && locate(&sizes[..count], place).is_none(),
            "a place past the listing's end"
        );
    }

    /// Every two ascending lists of up to two keys below four, every cursor, and every limit up to four: a merged page is the start of the sorted union after the cursor.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(6)]
    fn a_merged_page_is_the_start_of_the_sorted_union_after_its_cursor() {
        let left: [u8; KEYS] = kani::any();
        let right: [u8; KEYS] = kani::any();
        let (left_length, right_length): (usize, usize) = (kani::any(), kani::any());
        kani::assume(left_length <= KEYS && right_length <= KEYS);
        let (left, right) = (&left[..left_length], &right[..right_length]);
        kani::assume(left.windows(2).all(|pair| pair[0] < pair[1]));
        kani::assume(right.windows(2).all(|pair| pair[0] < pair[1]));
        kani::assume(
            left.iter()
                .chain(right)
                .all(|key| usize::from(*key) < VALUES),
        );
        let last: Option<u8> = kani::any();
        let limit: usize = kani::any();
        kani::assume(limit <= LIMIT);
        kani::assert(
            merged_spec(left, right, last, limit),
            "a merged page is the start of the sorted union after its cursor",
        );
        kani::cover!(
            left_length == KEYS && right_length == KEYS && last.is_some() && limit == 2,
            "a page from a cursor across both lists"
        );
    }

    /// The next page starting before the last one ended would show a row twice; this probe claims two pages in turn overlap and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_page_that_repeats_a_row() {
        let total: usize = kani::any();
        kani::assume(total > 3);
        kani::assert(
            window(total, 0, 3).end > window(total, 3, 3).start,
            "NEGATIVE_PAGE_CONTROL",
        );
    }

    /// The ascending lists of up to two keys below four, for the tests.
    #[cfg(test)]
    pub(super) fn lists() -> Vec<Vec<u8>> {
        let mut lists: Vec<Vec<u8>> = alloc::vec![Vec::new()];
        for key in (0_u8..).take(VALUES) {
            let longer: Vec<Vec<u8>> = lists
                .iter()
                .filter(|list| list.len() < KEYS)
                .map(|list| [list.as_slice(), &[key]].concat())
                .collect();
            lists.extend(longer);
        }
        lists
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{LIMIT, lists, locate_spec, merged_spec, start_spec, window_spec};
    use super::{Cursor, Paging, locate, merged_page, start, window};

    #[test]
    fn a_page_starts_in_sight_or_around_the_row_it_brings_into_sight() {
        let paging = Paging { limit: 10, lead: 3 };
        assert_eq!(start(100, 20, None, paging), 20);
        assert_eq!(start(100, 20, Some(25), paging), 20, "a row in sight");
        assert_eq!(start(100, 20, Some(29), paging), 20);
        assert_eq!(start(100, 20, Some(30), paging), 27, "a row past the page");
        assert_eq!(
            start(100, 20, Some(19), paging),
            16,
            "a row before the page"
        );
        assert_eq!(start(100, 20, Some(1), paging), 0);
        assert_eq!(
            start(100, 95, None, paging),
            90,
            "the end pulls the page back"
        );
        assert_eq!(start(100, 20, Some(99), paging), 90);
        assert_eq!(start(5, 3, Some(4), paging), 0);
        assert_eq!(start(0, 0, None, paging), 0);
        let edges = [0, 1, 2, 3, 7, usize::MAX - 1, usize::MAX];
        let reveals = [None, Some(0), Some(2), Some(6), Some(usize::MAX)];
        let pagings = [(0, 0), (3, 1), (3, 3), (7, 2), (usize::MAX, 1)];
        for total in edges {
            for sight in edges {
                for reveal in reveals {
                    for (limit, lead) in pagings {
                        let drawn = Paging { limit, lead };
                        assert!(
                            start_spec(total, sight, reveal, drawn),
                            "{total} {sight} {reveal:?} {drawn:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_place_falls_in_the_part_that_holds_it() {
        assert_eq!(locate(&[2, 0, 3], 0), Some((0, 0)));
        assert_eq!(locate(&[2, 0, 3], 1), Some((0, 1)));
        assert_eq!(
            locate(&[2, 0, 3], 2),
            Some((2, 0)),
            "an empty part holds no place"
        );
        assert_eq!(locate(&[2, 0, 3], 4), Some((2, 2)));
        assert_eq!(locate(&[2, 0, 3], 5), None);
        assert_eq!(locate(&[], 0), None);
        assert_eq!(locate(&[usize::MAX, 1], usize::MAX), Some((1, 0)));
        let edges = [0, 1, 2, 3, usize::MAX - 1, usize::MAX];
        for first in edges {
            for second in edges {
                let sizes = [first, second, 1];
                for count in 0..=sizes.len() {
                    for place in edges {
                        assert!(
                            locate_spec(&sizes[..count], place),
                            "{sizes:?} {count} {place}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_window_stays_within_the_listing() {
        assert_eq!(window(10, 0, 3), 0..3);
        assert_eq!(window(10, 3, 3), 3..6);
        assert_eq!(window(10, 9, 3), 9..10);
        assert_eq!(window(10, 12, 3), 10..10);
        assert_eq!(window(10, 2, usize::MAX), 2..10);
        assert_eq!(window(0, 0, 0), 0..0);
    }

    #[test]
    fn a_merged_page_starts_after_its_cursor_and_keeps_its_limit() {
        let left = [1, 4, 6];
        let right = [2, 3, 7];
        assert_eq!(
            merged_page(&[&left, &right], &Cursor::start(), usize::MAX),
            [1, 2, 3, 4, 6, 7]
        );
        assert_eq!(merged_page(&[&left, &right], &Cursor::after(3), 2), [4, 6]);
        assert_eq!(merged_page(&[&left, &[4]], &Cursor::after(3), 3), [4, 4, 6]);
        assert_eq!(merged_page::<u8>(&[], &Cursor::start(), 5), []);
        assert!(Cursor::after(3).admits(&4) && !Cursor::after(3).admits(&3));
        assert!(Cursor::<u8>::start().admits(&0));
    }

    #[test]
    fn the_specifications_agree_over_a_drawn_domain() {
        let edges = [0, 1, 2, 3, 7, usize::MAX - 1, usize::MAX];
        for total in edges {
            for first in edges {
                for limit in edges {
                    assert!(window_spec(total, first, limit), "{total} {first} {limit}");
                }
            }
        }
        let drawn = lists();
        assert_eq!(drawn.len(), 11);
        for left in &drawn {
            for right in &drawn {
                for last in [None, Some(0), Some(1), Some(2), Some(3)] {
                    for limit in 0..=LIMIT {
                        assert!(
                            merged_spec(left, right, last, limit),
                            "{left:?} {right:?} {last:?} {limit}"
                        );
                    }
                }
            }
        }
    }
}
