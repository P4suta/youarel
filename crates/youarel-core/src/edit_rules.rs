//! What an edit writes, decided from the facts that a store gathers about the entries it names, so the store never reads the whole state.
//!
//! The decision keeps the reference model's refusals in its order: an edit that names an entry not in the presence it needs, adds one that exists, puts an entry into a folder that is not live, moves a folder into itself, names a sibling that does not show, or finds no position.
//! A folder's removal for good removes every folder inside it and every shortcut those folders hold, which [`removal_plan`] names folder by folder for the store to remove in batches.

use crate::position_rules::{Key, Unordered, between};
use crate::state::{Edit, EditError, Entry, ItemId, Presence, Slot, Tree};
use alloc::string::String;
use alloc::vec::Vec;

/// The positions around the slot of an edit that places an entry, among the entries that show at the destination with the moving one left out.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Neighbors<'a> {
    /// The last position among the destination's entries.
    pub last: Option<&'a Key>,
    /// The position of the sibling that the slot names, while it shows, wherever it sits.
    pub anchor: Option<&'a Key>,
    /// The greatest position below the anchor among the destination's entries.
    pub below: Option<&'a Key>,
    /// The least position above the anchor among the destination's entries.
    pub above: Option<&'a Key>,
}

/// What a store gathered about the entries that an edit names.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Facts<'a> {
    /// The presence of the entry that the edit names, or adds, or `None` when the store holds no record of it.
    pub target: Option<Presence>,
    /// Whether the folder the edit puts an entry into is live itself; the top always is.
    pub destination: bool,
    /// Whether the folder that a move names is the destination or holds it.
    pub into_itself: bool,
    pub neighbors: Neighbors<'a>,
}

/// What an edit writes at its stamp.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Write {
    DeviceName {
        id: crate::device_rules::MemberId,
        name: String,
    },
    /// A new shortcut, every register at the stamp.
    AddShortcut {
        id: ItemId,
        link: String,
        title: String,
        folder: Option<ItemId>,
        position: Key,
    },
    /// A new folder, every register at the stamp.
    AddFolder {
        id: ItemId,
        name: String,
        parent: Option<ItemId>,
        position: Key,
    },
    Title {
        id: ItemId,
        title: String,
    },
    Alias {
        id: ItemId,
        alias: String,
    },
    Note {
        id: ItemId,
        note: String,
    },
    Name {
        id: ItemId,
        name: String,
    },
    Mark {
        id: ItemId,
        mark: String,
    },
    /// An entry's folder, or a folder's parent, and its position there.
    Place {
        entry: Entry,
        folder: Option<ItemId>,
        position: Key,
    },
    /// An entry's presence: into the trash, or back out of it.
    Presence {
        entry: Entry,
        presence: Presence,
    },
    /// A shortcut's removal for good, or a folder's with everything inside it.
    Remove {
        entry: Entry,
    },
}

/// The entry that an edit names in the presence it needs, as an error otherwise.
fn require(facts: &Facts<'_>, entry: Entry, needed: &[Presence]) -> Result<(), EditError> {
    match facts.target {
        Some(presence) if needed.contains(&presence) => Ok(()),
        Some(_) | None => Err(EditError::Unknown(entry)),
    }
}

/// The presence an edit of most kinds needs: live.
const LIVE: &[Presence] = &[Presence::Live];
/// The presence a put back needs: in the trash.
const TRASHED: &[Presence] = &[Presence::Trashed];
/// The presences a removal for good takes: live, or in the trash.
const NOT_GONE: &[Presence] = &[Presence::Live, Presence::Trashed];

/// The destination folder of an edit, as an error when it is gone.
const fn require_destination(facts: &Facts<'_>, folder: Option<ItemId>) -> Result<(), EditError> {
    match folder {
        Some(id) if !facts.destination => Err(EditError::Unknown(Entry::Folder(id))),
        Some(_) | None => Ok(()),
    }
}

/// The position of a new entry after every entry of its folder, refusing an identifier that exists and a folder that is gone.
fn appended<'a>(
    facts: &Facts<'a>,
    entry: Entry,
    folder: Option<ItemId>,
    place: &impl Fn(Option<&'a Key>, Option<&'a Key>) -> Result<Key, Unordered>,
) -> Result<Key, EditError> {
    if facts.target.is_some() {
        return Err(EditError::Exists(entry));
    }
    require_destination(facts, folder)?;
    place(facts.neighbors.last, None).map_err(EditError::Position)
}

/// The position a move's slot names among the destination's entries, refusing an entry or a destination that is not live, a folder moved into itself, and a sibling that does not show.
fn moved<'a>(
    facts: &Facts<'a>,
    (entry, folder, slot): (Entry, Option<ItemId>, Slot),
    place: &impl Fn(Option<&'a Key>, Option<&'a Key>) -> Result<Key, Unordered>,
) -> Result<Key, EditError> {
    require_move(facts, entry, folder)?;
    let Neighbors {
        last,
        anchor,
        below,
        above,
    } = facts.neighbors;
    let (low, high) = match slot {
        Slot::End => (last, None),
        Slot::Before(sibling) => (below, Some(anchor.ok_or(EditError::Unknown(sibling))?)),
        Slot::After(sibling) => (Some(anchor.ok_or(EditError::Unknown(sibling))?), above),
    };
    place(low, high).map_err(EditError::Position)
}

fn require_move(facts: &Facts<'_>, entry: Entry, folder: Option<ItemId>) -> Result<(), EditError> {
    require(facts, entry, LIVE)?;
    require_destination(facts, folder)?;
    if let (Entry::Folder(id), Some(_)) = (entry, folder)
        && facts.into_itself
    {
        return Err(EditError::IntoItself(id));
    }
    Ok(())
}

fn device_name(id: crate::device_rules::MemberId, name: String) -> Result<Write, EditError> {
    if !crate::device_rules::admitted_name(&name) {
        return Err(EditError::DeviceName);
    }
    Ok(Write::DeviceName { id, name })
}

/// What one edit writes, from the facts a store gathered about the entries it names.
///
/// # Errors
///
/// Returns the [`EditError`] of the reference model: an entry not in the presence its edit needs, a destination that is not live, a slot's sibling that does not show, an identifier added again, a folder moved into itself or a folder it holds, or no position between the neighbors.
pub fn apply(edit: Edit, facts: &Facts<'_>) -> Result<Write, EditError> {
    decide(edit, facts, &between)
}

/// What one edit writes, with `place` choosing the position strictly between the slot's bounds for an edit that places its entry, as [`between`] chooses it.
fn decide<'a>(
    edit: Edit,
    facts: &Facts<'a>,
    place: &impl Fn(Option<&'a Key>, Option<&'a Key>) -> Result<Key, Unordered>,
) -> Result<Write, EditError> {
    Ok(match edit {
        Edit::NameDevice { id, name } => device_name(id, name)?,
        Edit::AddShortcut {
            id,
            link,
            title,
            folder,
        } => Write::AddShortcut {
            position: appended(facts, Entry::Shortcut(id), folder, place)?,
            id,
            link,
            title,
            folder,
        },
        Edit::AddFolder { id, name, parent } => Write::AddFolder {
            position: appended(facts, Entry::Folder(id), parent, place)?,
            id,
            name,
            parent,
        },
        Edit::Retitle { id, title } => {
            require(facts, Entry::Shortcut(id), LIVE)?;
            Write::Title { id, title }
        }
        Edit::Realias { id, alias } => {
            require(facts, Entry::Shortcut(id), LIVE)?;
            Write::Alias { id, alias }
        }
        Edit::SetNote { id, note } => {
            require(facts, Entry::Shortcut(id), LIVE)?;
            Write::Note { id, note }
        }
        Edit::RenameFolder { id, name } => {
            require(facts, Entry::Folder(id), LIVE)?;
            Write::Name { id, name }
        }
        Edit::MarkFolder { id, mark } => {
            require(facts, Entry::Folder(id), LIVE)?;
            Write::Mark { id, mark }
        }
        Edit::Move {
            entry,
            folder,
            slot,
        } => Write::Place {
            position: moved(facts, (entry, folder, slot), place)?,
            entry,
            folder,
        },
        Edit::RestorePlace {
            entry,
            folder,
            position,
        } => {
            require_move(facts, entry, folder)?;
            Write::Place {
                entry,
                folder,
                position,
            }
        }
        Edit::Trash { entry } => {
            require(facts, entry, LIVE)?;
            Write::Presence {
                entry,
                presence: Presence::Trashed,
            }
        }
        Edit::PutBack { entry } => {
            require(facts, entry, TRASHED)?;
            Write::Presence {
                entry,
                presence: Presence::Live,
            }
        }
        Edit::Remove { entry } => {
            require(facts, entry, NOT_GONE)?;
            Write::Remove { entry }
        }
    })
}

/// The folders that the removal for good of `folder` removes: the folder and every folder inside it that the tree holds, shown or in the trash, which the store removes with every shortcut they hold.
#[must_use]
pub fn removal_plan(tree: &Tree, folder: ItemId) -> Vec<ItemId> {
    tree.folders()
        .map(|(id, _)| id)
        .filter(|id| tree.within(*id, folder))
        .collect()
}

/// The properties Kani proves over every fact of a tiny domain: an edit is refused exactly as the reference model refuses it, in its order, and an edit that places its entry asks for a position between exactly the slot's bounds, which `between` chooses under its own proofs.
#[cfg(any(test, kani))]
mod proofs {
    #[cfg(kani)]
    use super::Neighbors;
    use super::{Facts, Presence, Write, apply, decide};
    use crate::position_rules::{Key, Unordered};
    use crate::state::{Edit, EditError, Entry, ItemId, Slot};
    use alloc::string::String;
    use core::cell::Cell;

    /// The position that the harness's choice answers between ordered bounds.
    pub(super) const CHOSEN: &str = "V";
    /// The kinds of edits the harness draws.
    pub(super) const KINDS: u8 = 14;
    /// The kinds of slots the harness draws.
    pub(super) const SLOTS: u8 = 3;
    /// The entry an edit names, its folder, and the sibling its slot names.
    pub(super) const ID: ItemId = ItemId([2; 16]);
    pub(super) const FOLDER: ItemId = ItemId([1; 16]);
    pub(super) const SIBLING: Entry = Entry::Shortcut(ItemId([3; 16]));

    /// The edit of a kind, with empty texts, into the top or [`FOLDER`].
    pub(super) fn edit(kind: u8, slot: u8, folder: Option<ItemId>, position: Key) -> Edit {
        let slot = match slot {
            0 => Slot::End,
            1 => Slot::Before(SIBLING),
            _ => Slot::After(SIBLING),
        };
        match kind {
            0 => Edit::AddShortcut {
                id: ID,
                link: String::new(),
                title: String::new(),
                folder,
            },
            1 => Edit::AddFolder {
                id: ID,
                name: String::new(),
                parent: folder,
            },
            2 => Edit::Retitle {
                id: ID,
                title: String::new(),
            },
            3 => Edit::MarkFolder {
                id: ID,
                mark: String::new(),
            },
            4 => Edit::Move {
                entry: Entry::Folder(ID),
                folder,
                slot,
            },
            5 => Edit::Move {
                entry: Entry::Shortcut(ID),
                folder,
                slot,
            },
            6 => Edit::Remove {
                entry: Entry::Folder(ID),
            },
            7 => Edit::Trash {
                entry: Entry::Shortcut(ID),
            },
            8 => Edit::PutBack {
                entry: Entry::Folder(ID),
            },
            9 => Edit::SetNote {
                id: ID,
                note: String::new(),
            },
            10 | 11 => Edit::RestorePlace {
                entry: if kind == 10 {
                    Entry::Shortcut(ID)
                } else {
                    Entry::Folder(ID)
                },
                folder,
                position,
            },
            12 => Edit::Realias {
                id: ID,
                alias: String::new(),
            },
            _ => Edit::RenameFolder {
                id: ID,
                name: String::new(),
            },
        }
    }

    /// Whether a target of a presence, or none for one the store does not hold, is in the presence the edit needs: in the trash for a put back, not gone for good for a removal, and live otherwise.
    fn fits(edit: &Edit, target: Option<Presence>) -> bool {
        match (edit, target) {
            (_, None) => false,
            (Edit::PutBack { .. }, Some(presence)) => presence == Presence::Trashed,
            (Edit::Remove { .. }, Some(presence)) => presence != Presence::Purged,
            (_, Some(presence)) => presence == Presence::Live,
        }
    }

    /// The bounds that a placed entry's position lies strictly between, where `None` is an open end.
    type Bounds<'a> = (Option<&'a Key>, Option<&'a Key>);

    /// The reference model's answer to an edit of the drawn shape, restated from `State::apply`: the refusals in its order, then the slot's bounds for an edit that places its entry.
    pub(super) fn expected<'a>(
        edit: &Edit,
        facts: &Facts<'a>,
    ) -> Result<Option<Bounds<'a>>, EditError> {
        let neighbors = facts.neighbors;
        let (entry, folder, slot, adds) = match edit {
            Edit::NameDevice { name, .. } => {
                return if crate::device_rules::admitted_name(name) {
                    Ok(None)
                } else {
                    Err(EditError::DeviceName)
                };
            }
            Edit::AddShortcut { id, folder, .. } => {
                (Entry::Shortcut(*id), *folder, Slot::End, true)
            }
            Edit::AddFolder { id, parent, .. } => (Entry::Folder(*id), *parent, Slot::End, true),
            Edit::Move {
                entry,
                folder,
                slot,
            } => (*entry, *folder, *slot, false),
            Edit::RestorePlace { entry, folder, .. } => (*entry, *folder, Slot::End, false),
            Edit::Retitle { id, .. } | Edit::Realias { id, .. } | Edit::SetNote { id, .. } => {
                (Entry::Shortcut(*id), None, Slot::End, false)
            }
            Edit::RenameFolder { id, .. } | Edit::MarkFolder { id, .. } => {
                (Entry::Folder(*id), None, Slot::End, false)
            }
            Edit::Trash { entry } | Edit::PutBack { entry } | Edit::Remove { entry } => {
                (*entry, None, Slot::End, false)
            }
        };
        let places = adds || matches!(edit, Edit::Move { .. } | Edit::RestorePlace { .. });
        if adds && facts.target.is_some() {
            return Err(EditError::Exists(entry));
        }
        if !adds && !fits(edit, facts.target) {
            return Err(EditError::Unknown(entry));
        }
        if let Some(id) = folder
            && places
            && !facts.destination
        {
            return Err(EditError::Unknown(Entry::Folder(id)));
        }
        if let (Entry::Folder(id), Some(_), false) = (entry, folder, adds)
            && places
            && facts.into_itself
        {
            return Err(EditError::IntoItself(id));
        }
        if !places || matches!(edit, Edit::RestorePlace { .. }) {
            return Ok(None);
        }
        Ok(Some(match slot {
            Slot::End => (neighbors.last, None),
            Slot::Before(sibling) => (
                neighbors.below,
                Some(neighbors.anchor.ok_or(EditError::Unknown(sibling))?),
            ),
            Slot::After(sibling) => (
                Some(neighbors.anchor.ok_or(EditError::Unknown(sibling))?),
                neighbors.above,
            ),
        }))
    }

    /// Whether a low bound lies below a high one, which an open end always does.
    fn ordered((low, high): Bounds<'_>) -> bool {
        match (low, high) {
            (Some(low), Some(high)) => low < high,
            (None, _) | (_, None) => true,
        }
    }

    /// A choice of positions that answers [`CHOSEN`] between ordered bounds, refuses unordered ones as `between` does, and remembers the bounds it was asked for last.
    pub(super) struct Chooser<'a> {
        chosen: Key,
        asked: Cell<Option<Bounds<'a>>>,
    }

    impl<'a> Chooser<'a> {
        pub(super) const fn new(chosen: Key) -> Self {
            Self {
                chosen,
                asked: Cell::new(None),
            }
        }

        fn place(&self, low: Option<&'a Key>, high: Option<&'a Key>) -> Result<Key, Unordered> {
            self.asked.set(Some((low, high)));
            if ordered((low, high)) {
                Ok(self.chosen.clone())
            } else {
                Err(Unordered)
            }
        }

        /// What `decide` answers with this choice of positions.
        pub(super) fn decide(&self, edit: Edit, facts: &Facts<'a>) -> Result<Write, EditError> {
            decide(edit, facts, &|low, high| self.place(low, high))
        }
    }

    /// What `decide` promises: the reference model's refusal before it asks for a position, or the edit's own write, at the position chosen between the slot's bounds, which it asks for exactly when the edit places its entry.
    pub(super) fn decided<'a>(
        edit: &Edit,
        facts: &Facts<'a>,
        answer: &Result<Write, EditError>,
        chooser: &Chooser<'a>,
    ) -> bool {
        let asked = chooser.asked.get();
        match (expected(edit, facts), answer) {
            (Err(wanted), Err(found)) => wanted == *found && asked.is_none(),
            (Ok(None), Ok(write)) => asked.is_none() && unplaced(edit, write),
            (Ok(Some(bounds)), Ok(write)) => {
                asked == Some(bounds)
                    && ordered(bounds)
                    && placed(write) == Some(&chooser.chosen)
                    && own(edit, write)
            }
            (Ok(Some(bounds)), Err(found)) => {
                asked == Some(bounds)
                    && !ordered(bounds)
                    && *found == EditError::Position(Unordered)
            }
            (Ok(None), Err(_)) | (Err(_), Ok(_)) => false,
        }
    }

    /// Whether a position lies strictly between its bounds.
    #[cfg(test)]
    fn within(position: &Key, (low, high): Bounds<'_>) -> bool {
        low.is_none_or(|low| low < position) && high.is_none_or(|high| position < high)
    }

    /// What `apply` promises in its answer: the reference model's refusal, or the edit's own write, placed strictly between the slot's bounds when they are ordered and refused for want of a position when they are not.
    #[cfg(test)]
    pub(super) fn holds(edit: &Edit, facts: &Facts<'_>, answer: &Result<Write, EditError>) -> bool {
        match (expected(edit, facts), answer) {
            (Err(wanted), Err(found)) => wanted == *found,
            (Ok(None), Ok(write)) => unplaced(edit, write),
            (Ok(Some(bounds)), Ok(write)) => {
                ordered(bounds)
                    && placed(write).is_some_and(|position| within(position, bounds))
                    && own(edit, write)
            }
            (Ok(Some(bounds)), Err(found)) => {
                !ordered(bounds) && *found == EditError::Position(Unordered)
            }
            (Ok(None), Err(_)) | (Err(_), Ok(_)) => false,
        }
    }

    /// What `apply` promises for an edit and its facts.
    #[cfg(test)]
    pub(super) fn spec(edit: &Edit, facts: &Facts<'_>) -> bool {
        holds(edit, facts, &apply(edit.clone(), facts))
    }

    /// A write that needs no generated position, including one that restores its saved position.
    fn unplaced(edit: &Edit, write: &Write) -> bool {
        own(edit, write)
            && match edit {
                Edit::RestorePlace { position, .. } => placed(write) == Some(position),
                Edit::AddShortcut { .. }
                | Edit::AddFolder { .. }
                | Edit::Retitle { .. }
                | Edit::Realias { .. }
                | Edit::SetNote { .. }
                | Edit::RenameFolder { .. }
                | Edit::MarkFolder { .. }
                | Edit::Move { .. }
                | Edit::Trash { .. }
                | Edit::PutBack { .. }
                | Edit::Remove { .. }
                | Edit::NameDevice { .. } => placed(write).is_none(),
            }
    }

    /// The position a write places its entry at, if it places one.
    const fn placed(write: &Write) -> Option<&Key> {
        match write {
            Write::AddShortcut { position, .. }
            | Write::AddFolder { position, .. }
            | Write::Place { position, .. } => Some(position),
            Write::DeviceName { .. }
            | Write::Title { .. }
            | Write::Alias { .. }
            | Write::Note { .. }
            | Write::Name { .. }
            | Write::Mark { .. }
            | Write::Presence { .. }
            | Write::Remove { .. } => None,
        }
    }

    /// Whether a write is the edit's own: the same entry, folder, and texts.
    fn own(edit: &Edit, write: &Write) -> bool {
        added(edit, write) || changed(edit, write)
    }

    fn added(edit: &Edit, write: &Write) -> bool {
        match (edit, write) {
            (
                Edit::AddShortcut {
                    id,
                    link,
                    title,
                    folder,
                },
                Write::AddShortcut {
                    id: written,
                    link: linked,
                    title: titled,
                    folder: filed,
                    ..
                },
            ) => id == written && link == linked && title == titled && folder == filed,
            (
                Edit::AddFolder { id, name, parent },
                Write::AddFolder {
                    id: written,
                    name: named,
                    parent: placed,
                    ..
                },
            ) => id == written && name == named && parent == placed,
            _ => false,
        }
    }

    fn changed(edit: &Edit, write: &Write) -> bool {
        match (edit, write) {
            (
                Edit::NameDevice { id, name },
                Write::DeviceName {
                    id: written,
                    name: saved,
                },
            ) => id == written && name == saved,
            (
                Edit::Retitle { id, title },
                Write::Title {
                    id: written,
                    title: titled,
                },
            ) => id == written && title == titled,
            (
                Edit::Realias { id, alias },
                Write::Alias {
                    id: written,
                    alias: saved,
                },
            ) => id == written && alias == saved,
            (
                Edit::RenameFolder { id, name },
                Write::Name {
                    id: written,
                    name: saved,
                },
            ) => id == written && name == saved,
            (
                Edit::SetNote { id, note },
                Write::Note {
                    id: written,
                    note: noted,
                },
            ) => id == written && note == noted,
            (
                Edit::MarkFolder { id, mark },
                Write::Mark {
                    id: written,
                    mark: marked,
                },
            ) => id == written && mark == marked,
            (
                Edit::Move { entry, folder, .. },
                Write::Place {
                    entry: written,
                    folder: filed,
                    ..
                },
            ) => entry == written && folder == filed,
            (
                Edit::RestorePlace {
                    entry,
                    folder,
                    position,
                },
                Write::Place {
                    entry: written,
                    folder: filed,
                    position: saved,
                },
            ) => entry == written && folder == filed && position == saved,
            (Edit::Remove { entry }, Write::Remove { entry: written })
            | (
                Edit::Trash { entry },
                Write::Presence {
                    entry: written,
                    presence: Presence::Trashed,
                },
            )
            | (
                Edit::PutBack { entry },
                Write::Presence {
                    entry: written,
                    presence: Presence::Live,
                },
            ) => entry == written,
            _ => false,
        }
    }

    /// The positions the harness draws its neighbors from.
    #[cfg(kani)]
    const POSITIONS: [&str; 3] = ["1", "V", "z"];

    /// A neighbor of the drawn positions, or none.
    #[cfg(kani)]
    fn any_neighbor(keys: &[Key; 3]) -> Option<&Key> {
        let choice: u8 = kani::any();
        kani::assume(choice <= 3);
        keys.get(usize::from(choice))
    }

    /// Every kind of edit, every slot, the top or a folder, every presence or none, every liveness and containment, and neighbors drawn from three positions, with a choice of positions in place of `between`, whose own proofs cover it.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(17)]
    fn an_edit_is_refused_as_the_reference_refuses_it_and_lands_between_its_neighbors() {
        let keys = POSITIONS.map(|text| Key::parse(text).unwrap());
        let target = match kani::any::<u8>() {
            0 => None,
            1 => Some(Presence::Trashed),
            2 => Some(Presence::Purged),
            _ => Some(Presence::Live),
        };
        let facts = Facts {
            target,
            destination: kani::any(),
            into_itself: kani::any(),
            neighbors: Neighbors {
                last: any_neighbor(&keys),
                anchor: any_neighbor(&keys),
                below: any_neighbor(&keys),
                above: any_neighbor(&keys),
            },
        };
        let kind: u8 = kani::any();
        let slot: u8 = kani::any();
        kani::assume(kind < KINDS && slot < SLOTS);
        let folder = if kani::any() { Some(FOLDER) } else { None };
        let edit = edit(kind, slot, folder, Key::parse(CHOSEN).unwrap());
        let chooser = Chooser::new(Key::parse(CHOSEN).unwrap());
        let answer = chooser.decide(edit.clone(), &facts);
        kani::assert(
            decided(&edit, &facts, &answer, &chooser),
            "an edit is refused as the reference refuses it and lands between its neighbors",
        );
        kani::cover!(matches!(answer, Ok(Write::Place { .. })), "a move lands");
        kani::cover!(
            matches!(answer, Err(EditError::IntoItself(_))),
            "a folder moved into itself"
        );
        kani::cover!(
            matches!(answer, Err(EditError::Position(_))),
            "no position between the neighbors"
        );
        kani::cover!(
            matches!(
                answer,
                Ok(Write::Presence {
                    presence: Presence::Live,
                    ..
                })
            ),
            "an entry comes back out of the trash"
        );
    }

    /// A move into a folder that is not live would leave its entry where no listing reaches; this probe claims such a move lands and must fail.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(5)]
    fn rejects_a_move_into_a_dead_folder() {
        let facts = Facts {
            target: Some(Presence::Live),
            destination: false,
            into_itself: false,
            neighbors: Neighbors::default(),
        };
        let moved = apply(
            Edit::Move {
                entry: Entry::Shortcut(ID),
                folder: Some(FOLDER),
                slot: Slot::End,
            },
            &facts,
        );
        kani::assert(moved.is_ok(), "NEGATIVE_EDIT_CONTROL");
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{CHOSEN, Chooser, FOLDER, KINDS, SLOTS, decided, edit, spec};
    use super::{Facts, Neighbors, Presence, Write, apply, removal_plan};
    use crate::clock_rules::{DeviceId, Stamp};
    use crate::position_rules::{Key, Unordered};
    use crate::state::{Edit, EditError, Entry, ItemId, Slot, State};
    use alloc::string::String;
    use alloc::vec::Vec;

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    const fn id(byte: u8) -> ItemId {
        ItemId([byte; 16])
    }

    const fn live(neighbors: Neighbors<'_>) -> Facts<'_> {
        Facts {
            target: Some(Presence::Live),
            destination: true,
            into_itself: false,
            neighbors,
        }
    }

    #[test]
    fn the_trash_takes_a_live_entry_and_gives_back_one_it_holds() {
        let facts = live(Neighbors::default());
        let shortcut = Entry::Shortcut(id(1));
        assert_eq!(
            apply(Edit::Trash { entry: shortcut }, &facts),
            Ok(Write::Presence {
                entry: shortcut,
                presence: Presence::Trashed,
            })
        );
        assert_eq!(
            apply(Edit::PutBack { entry: shortcut }, &facts),
            Err(EditError::Unknown(shortcut)),
            "a live entry is in no trash"
        );
        let trashed = Facts {
            target: Some(Presence::Trashed),
            ..facts
        };
        assert_eq!(
            apply(Edit::PutBack { entry: shortcut }, &trashed),
            Ok(Write::Presence {
                entry: shortcut,
                presence: Presence::Live,
            })
        );
        assert_eq!(
            apply(Edit::Trash { entry: shortcut }, &trashed),
            Err(EditError::Unknown(shortcut))
        );
        assert_eq!(
            apply(Edit::Remove { entry: shortcut }, &trashed),
            Ok(Write::Remove { entry: shortcut }),
            "the trash empties for good"
        );
        let gone = Facts {
            target: Some(Presence::Purged),
            ..facts
        };
        assert_eq!(
            apply(Edit::Remove { entry: shortcut }, &gone),
            Err(EditError::Unknown(shortcut))
        );
    }

    #[test]
    fn each_edit_writes_its_own_registers() {
        let facts = live(Neighbors {
            last: None,
            anchor: None,
            below: None,
            above: None,
        });
        let text = String::from;
        let cases = [
            (
                Edit::Retitle {
                    id: id(1),
                    title: text("t"),
                },
                Write::Title {
                    id: id(1),
                    title: text("t"),
                },
            ),
            (
                Edit::Realias {
                    id: id(1),
                    alias: text("a"),
                },
                Write::Alias {
                    id: id(1),
                    alias: text("a"),
                },
            ),
            (
                Edit::SetNote {
                    id: id(1),
                    note: text("n"),
                },
                Write::Note {
                    id: id(1),
                    note: text("n"),
                },
            ),
            (
                Edit::RenameFolder {
                    id: id(2),
                    name: text("f"),
                },
                Write::Name {
                    id: id(2),
                    name: text("f"),
                },
            ),
            (
                Edit::MarkFolder {
                    id: id(2),
                    mark: text("m"),
                },
                Write::Mark {
                    id: id(2),
                    mark: text("m"),
                },
            ),
            (
                Edit::Remove {
                    entry: Entry::Folder(id(2)),
                },
                Write::Remove {
                    entry: Entry::Folder(id(2)),
                },
            ),
        ];
        for (edit, write) in cases {
            assert_eq!(apply(edit, &facts), Ok(write));
        }
    }

    #[test]
    fn an_addition_lands_after_the_last_entry_of_its_folder() {
        let last = key("a5");
        let facts = Facts {
            target: None,
            ..live(Neighbors {
                last: Some(&last),
                anchor: None,
                below: None,
                above: None,
            })
        };
        assert_eq!(
            apply(
                Edit::AddShortcut {
                    id: id(1),
                    link: String::from("l"),
                    title: String::from("t"),
                    folder: Some(id(2)),
                },
                &facts
            ),
            Ok(Write::AddShortcut {
                id: id(1),
                link: String::from("l"),
                title: String::from("t"),
                folder: Some(id(2)),
                position: key("a6"),
            })
        );
        assert_eq!(
            apply(
                Edit::AddFolder {
                    id: id(3),
                    name: String::from("f"),
                    parent: None,
                },
                &facts
            ),
            Ok(Write::AddFolder {
                id: id(3),
                name: String::from("f"),
                parent: None,
                position: key("a6"),
            })
        );
        let purged = Facts {
            target: Some(Presence::Purged),
            ..facts
        };
        assert_eq!(
            apply(
                Edit::AddFolder {
                    id: id(3),
                    name: String::new(),
                    parent: None,
                },
                &purged
            ),
            Err(EditError::Exists(Entry::Folder(id(3)))),
            "an identifier gone for good exists"
        );
        let gone = Facts {
            destination: false,
            ..facts
        };
        assert_eq!(
            apply(
                Edit::AddShortcut {
                    id: id(1),
                    link: String::new(),
                    title: String::new(),
                    folder: Some(id(2)),
                },
                &gone
            ),
            Err(EditError::Unknown(Entry::Folder(id(2))))
        );
    }

    /// What a move of `entry` into `folder` at `slot` writes, given `given`.
    fn moved(
        entry: Entry,
        folder: Option<ItemId>,
        slot: Slot,
        given: &Facts<'_>,
    ) -> Result<Write, EditError> {
        apply(
            Edit::Move {
                entry,
                folder,
                slot,
            },
            given,
        )
    }

    /// The write of an entry that lands at the top at `position`.
    fn placed(entry: Entry, position: &str) -> Write {
        Write::Place {
            entry,
            folder: None,
            position: key(position),
        }
    }

    #[test]
    fn a_move_lands_beside_its_sibling_or_at_the_end() {
        let (below, anchor, above, last) = (key("a1"), key("a5"), key("a9"), key("b01"));
        let facts = live(Neighbors {
            last: Some(&last),
            anchor: Some(&anchor),
            below: Some(&below),
            above: Some(&above),
        });
        let shortcut = Entry::Shortcut(id(1));
        assert_eq!(
            moved(shortcut, None, Slot::End, &facts),
            Ok(placed(shortcut, "b02")),
            "the integer after the last"
        );
        let sibling = Entry::Shortcut(id(5));
        assert_eq!(
            moved(shortcut, None, Slot::Before(sibling), &facts),
            Ok(placed(shortcut, "a3")),
            "midway between the one below and the sibling"
        );
        assert_eq!(
            moved(shortcut, None, Slot::After(sibling), &facts),
            Ok(placed(shortcut, "a7")),
            "midway between the sibling and the one above"
        );
        let anchorless = Facts {
            neighbors: Neighbors {
                anchor: None,
                ..facts.neighbors
            },
            ..facts
        };
        assert_eq!(
            moved(
                shortcut,
                None,
                Slot::After(Entry::Folder(id(5))),
                &anchorless
            ),
            Err(EditError::Unknown(Entry::Folder(id(5))))
        );
        let unordered = live(Neighbors {
            last: None,
            anchor: Some(&below),
            below: Some(&anchor),
            above: None,
        });
        assert_eq!(
            moved(shortcut, None, Slot::Before(sibling), &unordered),
            Err(EditError::Position(Unordered))
        );
    }

    #[test]
    fn a_folder_moves_into_no_folder_it_holds() {
        let last = key("b01");
        let inside = Facts {
            into_itself: true,
            ..live(Neighbors {
                last: Some(&last),
                anchor: None,
                below: None,
                above: None,
            })
        };
        let folder = Entry::Folder(id(1));
        assert_eq!(
            moved(folder, Some(FOLDER), Slot::End, &inside),
            Err(EditError::IntoItself(id(1)))
        );
        assert_eq!(
            moved(folder, None, Slot::End, &inside),
            Ok(placed(folder, "b02")),
            "the top lies inside no folder"
        );
        let shortcut = Entry::Shortcut(id(1));
        assert_eq!(
            moved(shortcut, Some(FOLDER), Slot::End, &inside),
            Ok(Write::Place {
                entry: shortcut,
                folder: Some(FOLDER),
                position: key("b02"),
            }),
            "a shortcut moves into any live folder"
        );
    }

    #[test]
    fn an_edit_of_a_gone_entry_is_refused() {
        for target in [None, Some(Presence::Purged)] {
            let facts = Facts {
                target,
                ..live(Neighbors::default())
            };
            assert_eq!(
                apply(
                    Edit::Remove {
                        entry: Entry::Shortcut(id(1))
                    },
                    &facts
                ),
                Err(EditError::Unknown(Entry::Shortcut(id(1))))
            );
            assert_eq!(
                apply(
                    Edit::MarkFolder {
                        id: id(2),
                        mark: String::new()
                    },
                    &facts
                ),
                Err(EditError::Unknown(Entry::Folder(id(2))))
            );
        }
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "an exhaustive sweep that Kani and the native run cover; Miri checks the same code on the other tests"
    )]
    fn the_specification_agrees_over_a_drawn_domain() {
        let keys = [key("1"), key("V"), key("z")];
        let neighbor = |choice: usize| keys.get(choice);
        let flags = [(false, false), (false, true), (true, false), (true, true)];
        let targets = [
            None,
            Some(Presence::Live),
            Some(Presence::Trashed),
            Some(Presence::Purged),
        ];
        for target in targets {
            for (destination, into_itself) in flags {
                for code in 0..256_usize {
                    let facts = Facts {
                        target,
                        destination,
                        into_itself,
                        neighbors: Neighbors {
                            last: neighbor(code % 4),
                            anchor: neighbor(code / 4 % 4),
                            below: neighbor(code / 16 % 4),
                            above: neighbor(code / 64 % 4),
                        },
                    };
                    for kind in 0..KINDS {
                        for slot in 0..SLOTS {
                            for folder in [None, Some(FOLDER)] {
                                let edit = edit(kind, slot, folder, key(CHOSEN));
                                assert!(spec(&edit, &facts), "{edit:?} {facts:?}");
                                let chooser = Chooser::new(Key::parse(CHOSEN).unwrap());
                                let answer = chooser.decide(edit.clone(), &facts);
                                assert!(
                                    decided(&edit, &facts, &answer, &chooser),
                                    "{edit:?} {facts:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_removal_plan_holds_the_folder_and_every_live_folder_inside_it() {
        let mut state = State::default();
        let stamp = |wall| Stamp {
            wall,
            counter: 0,
            device: DeviceId([1; 16]),
        };
        let folders = [
            (1, None),
            (2, Some(1)),
            (3, Some(2)),
            (4, None),
            (5, Some(1)),
        ];
        for (wall, (byte, parent)) in (1..).zip(folders) {
            state
                .apply(
                    Edit::AddFolder {
                        id: id(byte),
                        name: String::new(),
                        parent: parent.map(id),
                    },
                    stamp(wall),
                )
                .unwrap();
        }
        state
            .apply(
                Edit::Remove {
                    entry: Entry::Folder(id(5)),
                },
                stamp(9),
            )
            .unwrap();
        state
            .apply(
                Edit::Trash {
                    entry: Entry::Folder(id(2)),
                },
                stamp(10),
            )
            .unwrap();
        let tree = state.tree();
        assert_eq!(
            removal_plan(&tree, id(1)),
            [id(1), id(2), id(3)],
            "the folders in the trash and those they hide"
        );
        assert_eq!(removal_plan(&tree, id(3)), [id(3)]);
        assert_eq!(removal_plan(&tree, id(5)), Vec::<ItemId>::new());
    }
}
