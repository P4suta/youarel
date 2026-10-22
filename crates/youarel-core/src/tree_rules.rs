//! The folder tree that replicas agree on, resolved from the parents each folder declares.
//!
//! Each folder declares one parent, so the declared graph is a set of chains that end at the top or run into a cycle.
//! Two devices that move folders into each other at the same time declare a cycle that neither made alone.
//! The resolution breaks each cycle at the folder whose declaration is the most recent, which moves to the top, and keeps every other declaration.
//! A parent that is gone, or out of range, leaves its folder at the top.
//! The result depends on the declarations alone, so every replica that merged the same writes draws the same tree.
//! A folder in the trash hides itself and every folder inside it, which [`hidden`] reads from the resolved tree.

use alloc::vec::Vec;
use core::cmp::Ordering;

/// What each folder declares, by its index in a list of folders.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Declared<K> {
    /// The index of the parent folder, or `None` for the top.
    pub parent: Option<usize>,
    /// How recent the declaration is, where a greater key is more recent.
    pub recency: K,
}

/// The effective parent of every folder, by index into `declared`.
///
/// The effective parents never form a cycle, and each names a folder of the list or the top.
/// Each folder walks its own chain of declarations, so the work grows with the folders times the depth of their chains, and with the square of the folders while a cycle stands.
#[must_use]
pub fn resolve<K: Ord>(declared: &[Declared<K>]) -> Vec<Option<usize>> {
    let count = declared.len();
    (0..count)
        .map(|index| {
            let parent = declared
                .get(index)?
                .parent
                .filter(|parent| *parent < count)?;
            (!breaks_cycle(declared, index)).then_some(parent)
        })
        .collect()
}

/// Whether `index` lies on a cycle of declarations and its declaration is the most recent there.
///
/// A walk from a folder on a cycle stays on the cycle, so a more recent folder that the walk meets before it returns is a member, and the walk stops there.
fn breaks_cycle<K: Ord>(declared: &[Declared<K>], index: usize) -> bool {
    let mut current = declared.get(index).and_then(|folder| folder.parent);
    for _ in 0..declared.len() {
        let Some(node) = current else {
            return false;
        };
        if node == index {
            return true;
        }
        // A parent out of range ends the walk at the top.
        let Some(folder) = declared.get(node) else {
            return false;
        };
        if recency(declared, node, index) == Ordering::Greater {
            return false;
        }
        current = folder.parent;
    }
    false
}

/// The order of two folders' declarations, the more recent last, and the greater index last between equal keys.
fn recency<K: Ord>(declared: &[Declared<K>], left: usize, right: usize) -> Ordering {
    let key = |index: usize| declared.get(index).map(|folder| &folder.recency);
    key(left).cmp(&key(right)).then(left.cmp(&right))
}

/// Which folders the trash hides, by index into `parents`: each folder in the trash, as `trashed` marks it, and each folder inside one.
///
/// A walk up from each folder passes at most as many folders as the list holds, so parents that run into a cycle or out of range end the walk too.
#[must_use]
pub fn hidden(parents: &[Option<usize>], trashed: &[bool]) -> Vec<bool> {
    let count = parents.len();
    (0..count)
        .map(|index| {
            let mut current = Some(index);
            for _ in 0..count {
                let Some(node) = current else {
                    return false;
                };
                if trashed.get(node).copied().unwrap_or_default() {
                    return true;
                }
                current = parents.get(node).copied().flatten();
            }
            false
        })
        .collect()
}

/// The properties Kani proves over every declaration of up to [`proofs::FOLDER_BOUND`] folders, and over every tree of that many folders with every set of them in the trash.
#[cfg(any(test, kani))]
mod proofs {
    #[cfg(kani)]
    use super::hidden;
    use super::{Declared, recency, resolve};

    /// The most folders the harness draws.
    pub(super) const FOLDER_BOUND: usize = 4;

    /// What `hidden` promises for a tree without a cycle: a folder hides exactly when it waits in the trash or its parent hides, and the answer names every folder once.
    ///
    /// A tree without a cycle reaches the top from every folder, so the folders nearest the top settle first and the rule decides every other folder in turn.
    pub(super) fn hiding_holds(
        parents: &[Option<usize>],
        trashed: &[bool],
        hides: &[bool],
    ) -> bool {
        hides.len() == parents.len()
            && parents.iter().enumerate().all(|(index, parent)| {
                let own = trashed.get(index).copied().unwrap_or_default();
                let over = parent
                    .and_then(|parent| hides.get(parent))
                    .copied()
                    .unwrap_or_default();
                hides.get(index).copied() == Some(own || over)
            })
    }

    /// Whether following `parents` from `start` reaches the top within as many steps as there are folders.
    pub(super) fn reaches_top(parents: &[Option<usize>], start: usize) -> bool {
        let mut current = Some(start);
        for _ in 0..parents.len() {
            current = current.and_then(|node| parents.get(node).copied().flatten());
        }
        current.is_none()
    }

    /// Whether `index` lies on a declared cycle and no other member's declaration is more recent.
    ///
    /// A walk of as many steps as there are folders from a member of a cycle visits every member and returns, and from any other folder never returns.
    fn most_recent_on_its_cycle<K: Ord>(declared: &[Declared<K>], index: usize) -> bool {
        let mut current = Some(index);
        let mut returns = false;
        let mut most_recent = true;
        for _ in 0..declared.len() {
            current = current
                .and_then(|node| declared.get(node))
                .and_then(|folder| folder.parent)
                .filter(|parent| *parent < declared.len());
            if let Some(member) = current {
                returns |= member == index;
                most_recent &= recency(declared, member, index) != core::cmp::Ordering::Greater;
            }
        }
        returns && most_recent
    }

    /// What `resolve` promises for the declarations it read.
    #[cfg(test)]
    pub(super) fn spec<K: Ord>(declared: &[Declared<K>]) -> bool {
        holds(declared, &resolve(declared))
    }

    /// The resolution restated: the top for a parent out of range and for the most recent folder of its declared cycle, and the declared parent otherwise; then no cycle and only parents in range.
    pub(super) fn holds<K: Ord>(declared: &[Declared<K>], effective: &[Option<usize>]) -> bool {
        let expected = declared.iter().enumerate().all(|(index, folder)| {
            let in_range = folder.parent.filter(|parent| *parent < declared.len());
            let wanted = if most_recent_on_its_cycle(declared, index) {
                None
            } else {
                in_range
            };
            effective.get(index).copied().flatten() == wanted
        });
        let acyclic = (0..declared.len()).all(|start| reaches_top(effective, start));
        let in_range = effective
            .iter()
            .flatten()
            .all(|parent| *parent < declared.len());
        effective.len() == declared.len() && expected && acyclic && in_range
    }

    /// A parent as the harness draws it: the top, a folder, or an index out of range.
    #[cfg(kani)]
    fn parent(choice: u8) -> Option<usize> {
        match choice {
            0 => None,
            1..=4 => Some(usize::from(choice) - 1),
            _ => Some(9),
        }
    }

    /// Proves the resolution of every declaration of exactly `N` folders, and hands back the declarations and their tree.
    ///
    /// `resolve` reads recencies only through comparisons, so recencies below the folder count realize every order of the folders' keys, ties included.
    #[cfg(kani)]
    fn resolves_every_declaration_of<const N: usize>()
    -> ([Declared<u8>; N], alloc::vec::Vec<Option<usize>>) {
        let drawn: [(u8, u8); N] = kani::any();
        kani::assume(drawn.iter().all(|(_, recency)| usize::from(*recency) < N));
        let declared = drawn.map(|(choice, recency)| Declared {
            parent: parent(choice),
            recency,
        });
        let effective = resolve(&declared);
        kani::assert(
            holds(&declared, &effective),
            "the tree keeps every declaration but one per cycle",
        );
        (declared, effective)
    }

    /// Every walk visits each folder at most once, so no loop runs more than five times.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(6)]
    fn the_tree_of_four_folders_breaks_each_cycle_at_its_most_recent_move() {
        let (declared, effective) = resolves_every_declaration_of::<FOLDER_BOUND>();
        kani::cover!(
            declared
                .iter()
                .zip(&effective)
                .any(|(folder, resolved)| folder.parent.is_some() && resolved.is_none()),
            "a declaration is dropped"
        );
        kani::cover!(
            effective.iter().filter(|parent| parent.is_some()).count() == 3,
            "a chain of four"
        );
    }

    /// Every walk visits each folder at most once, so no loop runs more than four times.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn the_tree_of_fewer_folders_breaks_each_cycle_at_its_most_recent_move() {
        resolves_every_declaration_of::<0>();
        resolves_every_declaration_of::<1>();
        resolves_every_declaration_of::<2>();
        let (declared, effective) = resolves_every_declaration_of::<3>();
        kani::cover!(
            declared
                .iter()
                .map(|folder| folder.parent)
                .take(2)
                .eq([Some(1), Some(0)])
                && effective.get(1) == Some(&None),
            "a cycle of two breaks at its most recent folder"
        );
    }

    /// Every tree of four folders without a cycle, each folder at the top, under a folder, or under one out of range, and every set of them in the trash: a folder hides exactly when it or a folder over it waits there.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(6)]
    fn a_folder_hides_exactly_when_it_or_a_folder_over_it_waits_in_the_trash() {
        let drawn: [u8; FOLDER_BOUND] = kani::any();
        let trashed: [bool; FOLDER_BOUND] = kani::any();
        let parents = drawn.map(parent);
        kani::assume((0..FOLDER_BOUND).all(|start| reaches_top(&parents, start)));
        let hides = hidden(&parents, &trashed);
        kani::assert(
            hiding_holds(&parents, &trashed, &hides),
            "a folder hides exactly when it or its parent does",
        );
        kani::cover!(
            hides
                .iter()
                .zip(trashed)
                .filter(|(hides, trashed)| **hides && !*trashed)
                .count()
                == 3,
            "a trashed folder hides a chain of three"
        );
        kani::cover!(
            trashed.iter().all(|trashed| *trashed) && hides.iter().all(|hides| *hides),
            "every folder in the trash"
        );
    }

    /// A folder inside one in the trash must hide with it; this probe claims it shows and must fail.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(4)]
    fn rejects_a_folder_shown_inside_the_trash() {
        let hides = hidden(&[None, Some(0)], &[true, false]);
        kani::assert(hides.get(1) == Some(&false), "NEGATIVE_TREE_CONTROL");
    }

    /// Two folders that declare each other must not stay a cycle; this probe claims they do and must fail.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(6)]
    fn rejects_a_cycle_left_in_place() {
        let declared = [
            Declared {
                parent: Some(1),
                recency: 1_u8,
            },
            Declared {
                parent: Some(0),
                recency: 2_u8,
            },
        ];
        let effective = resolve(&declared);
        kani::assert(effective == [Some(1), Some(0)], "NEGATIVE_TREE_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{FOLDER_BOUND, hiding_holds, reaches_top, spec};
    use super::{Declared, hidden, resolve};
    use alloc::vec::Vec;

    #[test]
    fn a_folder_in_the_trash_hides_every_folder_inside_it() {
        let parents = [None, Some(0), Some(1), None, Some(9)];
        assert_eq!(
            hidden(&parents, &[false, true, false, false, false]),
            [false, true, true, false, false]
        );
        assert_eq!(
            hidden(&parents, &[false; 5]),
            [false; 5],
            "nothing in the trash hides nothing"
        );
        assert_eq!(hidden(&[], &[]), Vec::<bool>::new());
        assert_eq!(
            hidden(&[Some(1), Some(0)], &[false, false]),
            [false, false],
            "a cycle that no trash reaches ends its walk"
        );
        assert_eq!(hidden(&[Some(1), Some(0)], &[false, true]), [true, true]);
        assert_eq!(
            hidden(&[None], &[]),
            [false],
            "a folder the marks leave out waits in no trash"
        );
    }

    #[test]
    fn the_hiding_specification_agrees_over_every_small_tree() {
        let choices = [None, Some(0), Some(1), Some(2), Some(9)];
        let mut trees: Vec<Vec<Option<usize>>> = alloc::vec![Vec::new()];
        for _ in 0..3 {
            let longer: Vec<Vec<Option<usize>>> = trees
                .iter()
                .flat_map(|tree| {
                    choices.iter().map(move |parent| {
                        let mut next = tree.clone();
                        next.push(*parent);
                        next
                    })
                })
                .collect();
            trees.extend(longer);
        }
        let mut acyclic = 0_usize;
        for parents in &trees {
            if !(0..parents.len()).all(|start| reaches_top(parents, start)) {
                continue;
            }
            acyclic += 1;
            for mask in 0..8_u8 {
                let trashed: Vec<bool> = (0..parents.len())
                    .map(|place| mask >> place & 1 == 1)
                    .collect();
                let hides = hidden(parents, &trashed);
                assert!(
                    hiding_holds(parents, &trashed, &hides),
                    "{parents:?} {trashed:?}"
                );
            }
        }
        assert!(acyclic > 1 && trees.iter().all(|tree| tree.len() <= FOLDER_BOUND));
    }

    const fn folder(parent: Option<usize>, recency: u8) -> Declared<u8> {
        Declared { parent, recency }
    }

    #[test]
    fn declarations_without_a_cycle_stand() {
        assert_eq!(
            resolve(&[folder(None, 0), folder(Some(0), 0), folder(Some(1), 0)]),
            [None, Some(0), Some(1)]
        );
        assert_eq!(resolve::<u8>(&[]), Vec::new());
        assert_eq!(resolve(&[folder(Some(7), 0)]), [None]);
    }

    #[test]
    fn a_cycle_breaks_at_its_most_recent_declaration() {
        assert_eq!(
            resolve(&[folder(Some(1), 1), folder(Some(0), 2)]),
            [Some(1), None]
        );
        assert_eq!(
            resolve(&[
                folder(Some(1), 5),
                folder(Some(2), 2),
                folder(Some(0), 3),
                folder(Some(0), 9)
            ]),
            [None, Some(2), Some(0), Some(0)]
        );
        assert_eq!(resolve(&[folder(Some(0), 0)]), [None]);
        assert_eq!(
            resolve(&[folder(Some(1), 4), folder(Some(0), 4)]),
            [Some(1), None]
        );
    }

    #[test]
    fn the_specification_agrees_over_every_small_declaration() {
        let parents = [None, Some(0), Some(1), Some(2), Some(9)];
        let mut lists: Vec<Vec<Declared<u8>>> = alloc::vec![Vec::new()];
        for _ in 0..3 {
            let longer: Vec<Vec<Declared<u8>>> = lists
                .iter()
                .flat_map(|list| {
                    parents.iter().flat_map(move |parent| {
                        [0, 1].map(|recency| {
                            let mut next = list.clone();
                            next.push(folder(*parent, recency));
                            next
                        })
                    })
                })
                .collect();
            lists.extend(longer);
        }
        assert!(lists.iter().all(|list| list.len() <= FOLDER_BOUND));
        for list in &lists {
            assert!(spec(list), "{list:?}");
        }
    }
}
