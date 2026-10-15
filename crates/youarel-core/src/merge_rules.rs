//! The merge of two replicas: every field keeps its latest write, and every item either side holds survives.
//!
//! A field is a last-writer-wins register ordered by its stamp, and by its value when two registers share a stamp, so the merge is a join over a total order.
//! A collection merges by the union of its keys and the merge of the items both sides hold.
//! Either way the merge is commutative, associative, and idempotent, so replicas that saw the same writes hold the same state in any order of merging.

use crate::clock_rules::Stamp;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// One field and the stamp of the write that set it.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Register<T> {
    pub value: T,
    pub stamp: Stamp,
}

impl<T: Ord + Clone> Register<T> {
    /// The register after both writes: the later stamp, and the greater value between equal stamps.
    #[must_use]
    pub fn merge(&self, other: &Self) -> Self {
        match self
            .stamp
            .cmp(&other.stamp)
            .then_with(|| self.value.cmp(&other.value))
        {
            Ordering::Less => other.clone(),
            Ordering::Equal | Ordering::Greater => self.clone(),
        }
    }
}

/// The union of two lists sorted by strictly ascending keys, item by item, merging the values of a key both hold.
#[derive(Clone, Copy, Debug)]
pub struct MergeSorted<'a, K, V, F> {
    left: &'a [(K, V)],
    right: &'a [(K, V)],
    merge: F,
}

impl<K: Ord + Clone, V: Clone, F: Fn(&V, &V) -> V> Iterator for MergeSorted<'_, K, V, F> {
    type Item = (K, V);

    fn next(&mut self) -> Option<(K, V)> {
        match (self.left.split_first(), self.right.split_first()) {
            (None, None) => None,
            (Some((first, rest)), None) => {
                self.left = rest;
                Some(first.clone())
            }
            (None, Some((first, rest))) => {
                self.right = rest;
                Some(first.clone())
            }
            (Some((left, left_rest)), Some((right, right_rest))) => match left.0.cmp(&right.0) {
                Ordering::Less => {
                    self.left = left_rest;
                    Some(left.clone())
                }
                Ordering::Greater => {
                    self.right = right_rest;
                    Some(right.clone())
                }
                Ordering::Equal => {
                    self.left = left_rest;
                    self.right = right_rest;
                    Some((left.0.clone(), (self.merge)(&left.1, &right.1)))
                }
            },
        }
    }
}

/// The union of two key-sorted lists as an iterator, which yields keys in strictly ascending order.
pub const fn merged<'a, K, V, F>(
    left: &'a [(K, V)],
    right: &'a [(K, V)],
    merge: F,
) -> MergeSorted<'a, K, V, F> {
    MergeSorted { left, right, merge }
}

/// The union of two lists sorted by strictly ascending keys, merging the values of a key both hold.
///
/// The result is sorted by strictly ascending keys as well.
#[must_use]
pub fn merge_sorted<K: Ord + Clone, V: Clone>(
    left: &[(K, V)],
    right: &[(K, V)],
    merge: impl Fn(&V, &V) -> V,
) -> Vec<(K, V)> {
    merged(left, right, merge).collect()
}

/// The laws Kani proves: the register merge over every stamp and value, and the list merge over every short sorted list.
#[cfg(any(test, kani))]
mod proofs {
    #[cfg(test)]
    use super::merge_sorted;
    use super::{Register, merged};
    #[cfg(kani)]
    use crate::clock_rules::Stamp;
    #[cfg(test)]
    use alloc::vec::Vec;

    /// The longest list the list harness draws.
    pub(super) const LIST_BOUND: usize = 2;
    /// The keys and values the list harness draws: enough for shared, missing, and ordered keys, and for unequal values.
    #[cfg(kani)]
    const KEY_RANGE: u8 = 3;
    #[cfg(kani)]
    const VALUE_RANGE: u8 = 2;

    /// The join laws of the register merge, and that it keeps one of its inputs.
    pub(super) fn register_laws(
        first: &Register<u8>,
        second: &Register<u8>,
        third: &Register<u8>,
    ) -> bool {
        let commutes = first.merge(second) == second.merge(first);
        let associates = first.merge(second).merge(third) == first.merge(&second.merge(third));
        let idempotent = first.merge(first) == *first;
        let kept = {
            let joined = first.merge(second);
            joined == *first || joined == *second
        };
        let latest = first.merge(second).stamp == first.stamp.max(second.stamp);
        commutes && associates && idempotent && kept && latest
    }

    /// The value a list holds for a key.
    fn lookup(list: &[(u8, u8)], key: u8) -> Option<u8> {
        list.iter()
            .find(|(found, _)| *found == key)
            .map(|(_, value)| *value)
    }

    /// The merge characterized pointwise: an ascending list holding every key of either side, each with the item merge of the values both sides hold, or the one value one side holds.
    ///
    /// The merge's items land in a fixed buffer, so the check needs no allocation of its own.
    /// The join laws of the list merge follow from this characterization and the join laws of the item merge, which the tests also check over three lists.
    pub(super) fn pointwise(left: &[(u8, u8)], right: &[(u8, u8)]) -> bool {
        let larger = |first: &u8, second: &u8| *first.max(second);
        let mut buffer = [(0_u8, 0_u8); 2 * LIST_BOUND];
        let mut count: usize = 0;
        for item in merged(left, right, larger) {
            let Some(slot) = buffer.get_mut(count) else {
                return false;
            };
            *slot = item;
            count = count.saturating_add(1);
        }
        let Some(joined) = buffer.get(..count) else {
            return false;
        };
        let held = left.iter().chain(right).all(|(key, _)| {
            let expected = match (lookup(left, *key), lookup(right, *key)) {
                (Some(first), Some(second)) => Some(larger(&first, &second)),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            };
            lookup(joined, *key) == expected
        });
        let nothing_else = joined
            .iter()
            .all(|(key, _)| lookup(left, *key).is_some() || lookup(right, *key).is_some());
        ascending(joined) && held && nothing_else
    }

    /// The join laws of the list merge over three lists, which the tests check exhaustively over a small domain.
    #[cfg(test)]
    pub(super) fn list_laws(first: &[(u8, u8)], second: &[(u8, u8)], third: &[(u8, u8)]) -> bool {
        let merge = |left: &[(u8, u8)], right: &[(u8, u8)]| {
            merge_sorted(left, right, |left: &u8, right: &u8| *left.max(right))
        };
        let joined = merge(first, second);
        let commutes = joined == merge(second, first);
        let associates = merge(&joined, third) == merge(first, &merge(second, third));
        let idempotent = merge(first, first) == first;
        commutes && associates && idempotent && pointwise(first, second)
    }

    /// Whether a list is sorted by strictly ascending keys, the precondition of the list merge.
    pub(super) fn ascending(list: &[(u8, u8)]) -> bool {
        list.windows(2).all(|pair| match pair {
            [(left, _), (right, _)] => left < right,
            _ => false,
        })
    }

    #[cfg(kani)]
    fn any_register() -> Register<u8> {
        Register {
            value: kani::any(),
            stamp: Stamp {
                wall: kani::any(),
                counter: kani::any(),
                device: crate::clock_rules::DeviceId(kani::any()),
            },
        }
    }

    /// A list of up to [`LIST_BOUND`] entries in a fixed array, and its length.
    #[cfg(kani)]
    fn any_list() -> ([(u8, u8); LIST_BOUND], usize) {
        let entries: [(u8, u8); LIST_BOUND] = kani::any();
        let length: usize = kani::any();
        kani::assume(length <= LIST_BOUND);
        kani::assume(
            entries
                .iter()
                .all(|(key, value)| *key < KEY_RANGE && *value < VALUE_RANGE),
        );
        (entries, length)
    }

    #[cfg(kani)]
    #[kani::proof]
    fn the_register_merge_is_a_join_that_keeps_the_latest_write() {
        let (first, second, third) = (any_register(), any_register(), any_register());
        kani::assert(
            register_laws(&first, &second, &third),
            "the register merge is a join",
        );
        kani::cover!(
            first.merge(&second) == second && first != second,
            "the other side wins"
        );
    }

    /// Each list holds at most two entries, so the merge runs at most five times through its loop and every check at most four.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn the_list_merge_holds_the_union_of_keys_with_merged_values() {
        let (left_entries, left_length) = any_list();
        let (right_entries, right_length) = any_list();
        let (Some(left), Some(right)) = (
            left_entries.get(..left_length),
            right_entries.get(..right_length),
        ) else {
            return;
        };
        kani::assume(ascending(left) && ascending(right));
        kani::assert(
            pointwise(left, right),
            "the list merge is the pointwise union",
        );
        kani::cover!(
            left.len() == LIST_BOUND
                && right.len() == LIST_BOUND
                && left.first().map(|entry| entry.0) == right.first().map(|entry| entry.0),
            "both sides hold a key"
        );
    }

    /// A merge that lets the earlier write win would lose an edit; this probe claims one does and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_an_earlier_write_winning() {
        let device = crate::clock_rules::DeviceId([0; 16]);
        let earlier = Register {
            value: 1_u8,
            stamp: Stamp {
                wall: 1,
                counter: 0,
                device,
            },
        };
        let later = Register {
            value: 0_u8,
            stamp: Stamp {
                wall: 2,
                counter: 0,
                device,
            },
        };
        kani::assert(earlier.merge(&later) == earlier, "NEGATIVE_MERGE_CONTROL");
    }

    #[cfg(test)]
    pub(super) fn lists(values: &[u8]) -> Vec<Vec<(u8, u8)>> {
        let mut found: Vec<Vec<(u8, u8)>> = alloc::vec![Vec::new()];
        for key in values {
            for value in values {
                found.push(alloc::vec![(*key, *value)]);
                for later in values.iter().filter(|later| *later > key) {
                    found.push(alloc::vec![(*key, *value), (*later, *value)]);
                }
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{LIST_BOUND, ascending, list_laws, lists, register_laws};
    use super::{Register, merge_sorted};
    use crate::clock_rules::{DeviceId, Stamp};
    use alloc::vec::Vec;

    fn register(value: u8, wall: u64, device: u8) -> Register<u8> {
        Register {
            value,
            stamp: Stamp {
                wall,
                counter: 0,
                device: DeviceId([device; 16]),
            },
        }
    }

    #[test]
    fn the_later_write_wins_and_equal_stamps_keep_the_greater_value() {
        assert_eq!(
            register(1, 1, 0).merge(&register(0, 2, 0)),
            register(0, 2, 0)
        );
        assert_eq!(
            register(0, 2, 0).merge(&register(1, 1, 0)),
            register(0, 2, 0)
        );
        assert_eq!(
            register(0, 2, 0).merge(&register(5, 2, 1)),
            register(5, 2, 1)
        );
        assert_eq!(
            register(3, 2, 0).merge(&register(7, 2, 0)),
            register(7, 2, 0)
        );
        assert_eq!(
            register(7, 2, 0).merge(&register(3, 2, 0)),
            register(7, 2, 0)
        );
    }

    #[test]
    fn lists_merge_by_the_union_of_their_keys() {
        let larger = |left: &u8, right: &u8| *left.max(right);
        let merged = merge_sorted(&[(1, 5), (3, 1)], &[(2, 2), (3, 4)], larger);
        assert_eq!(merged, [(1, 5), (2, 2), (3, 4)]);
        assert_eq!(merge_sorted::<u8, u8>(&[], &[], larger), Vec::new());
        assert_eq!(merge_sorted(&[(1, 1)], &[], larger), [(1, 1)]);
        assert_eq!(merge_sorted(&[], &[(1, 1)], larger), [(1, 1)]);
    }

    #[test]
    fn the_laws_hold_over_a_drawn_domain() {
        let registers: Vec<Register<u8>> = [0, 1]
            .iter()
            .flat_map(|value| {
                [0, 1, 2]
                    .iter()
                    .flat_map(move |wall| [0, 1].map(|device| register(*value, *wall, device)))
            })
            .collect();
        for first in &registers {
            for second in &registers {
                for third in &registers {
                    assert!(register_laws(first, second, third));
                }
            }
        }
        let drawn = lists(&[1, 2, 3]);
        assert!(
            drawn
                .iter()
                .all(|list| list.len() <= LIST_BOUND && ascending(list))
        );
        for first in &drawn {
            for second in &drawn {
                for third in &drawn {
                    assert!(
                        list_laws(first, second, third),
                        "{first:?} {second:?} {third:?}"
                    );
                }
            }
        }
    }
}
