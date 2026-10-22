//! The replicated state: shortcuts and folders whose fields are registers, the edits a device makes, and the views it reads.
//!
//! An edit writes registers under one stamp, which the shell takes from the device's clock after it observed [`State::latest`].
//! Removal writes a tombstone, so a removal on one device and an edit on another merge to the removal or the edit by their stamps.

use crate::clock_rules::Stamp;
use crate::merge_rules::{Register, merge_sorted};
use crate::position_rules::{self, Key, Unordered};
use crate::tree_rules::{self, Declared};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

/// The identifier of a shortcut or a folder, which the device that made it drew at random.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ItemId(pub [u8; 16]);

/// The version of a UUID that starts with its time, in the high half of its seventh byte (RFC 9562).
const VERSION_7: u8 = 0x70;
/// The bits of the seventh byte below the version.
const BELOW_VERSION: u8 = 0x0f;
/// The RFC 9562 variant, in the two high bits of the ninth byte.
const VARIANT: u8 = 0x80;
/// The bits of the ninth byte below the variant.
const BELOW_VARIANT: u8 = 0x3f;

impl ItemId {
    /// A version 7 UUID: the time in milliseconds, then random bits, with the version and variant marks.
    #[must_use]
    pub const fn v7(now: u64, random: [u8; 10]) -> Self {
        let time = now.to_be_bytes();
        let [_, _, t0, t1, t2, t3, t4, t5] = time;
        let [r0, r1, r2, r3, r4, r5, r6, r7, r8, r9] = random;
        Self([
            t0,
            t1,
            t2,
            t3,
            t4,
            t5,
            VERSION_7 | (r0 & BELOW_VERSION),
            r1,
            VARIANT | (r2 & BELOW_VARIANT),
            r3,
            r4,
            r5,
            r6,
            r7,
            r8,
            r9,
        ])
    }

    /// The time in milliseconds since the Unix epoch that a version 7 identifier starts with, or `None` for an identifier without the version and variant marks.
    #[must_use]
    pub const fn time(self) -> Option<u64> {
        let [t0, t1, t2, t3, t4, t5, version, _, variant, ..] = self.0;
        if version & !BELOW_VERSION != VERSION_7 || variant & !BELOW_VARIANT != VARIANT {
            return None;
        }
        Some(u64::from_be_bytes([0, 0, t0, t1, t2, t3, t4, t5]))
    }
}

/// A link the launcher opens.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Shortcut {
    /// The admitted link in its normalized form.
    pub link: Register<String>,
    pub title: Register<String>,
    /// A short name typed to find the shortcut first, empty when there is none.
    pub alias: Register<String>,
    /// The folder the shortcut sits in, or `None` for the top.
    pub folder: Register<Option<ItemId>>,
    /// Its place among the folder's entries.
    pub position: Register<Key>,
    pub removed: Register<bool>,
    /// The note a person keeps on the shortcut, empty when there is none.
    pub note: Register<String>,
}

/// A folder of shortcuts and folders.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Folder {
    pub name: Register<String>,
    /// What the folder shows in place of its glyph, such as one emoji, or empty for the glyph.
    pub mark: Register<String>,
    /// The folder it declares as its parent, or `None` for the top.
    pub parent: Register<Option<ItemId>>,
    /// Its place among the parent's entries.
    pub position: Register<Key>,
    pub removed: Register<bool>,
}

impl Shortcut {
    /// A shortcut that one write at `stamp` adds, with its link and title, and no alias or note.
    fn new(
        [link, title]: [String; 2],
        folder: Option<ItemId>,
        position: Key,
        stamp: Stamp,
    ) -> Self {
        Self {
            link: register(link, stamp),
            title: register(title, stamp),
            alias: register(String::new(), stamp),
            folder: register(folder, stamp),
            position: register(position, stamp),
            removed: register(false, stamp),
            note: register(String::new(), stamp),
        }
    }

    /// The shortcut after both replicas' writes.
    #[must_use]
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            link: self.link.merge(&other.link),
            title: self.title.merge(&other.title),
            alias: self.alias.merge(&other.alias),
            folder: self.folder.merge(&other.folder),
            position: self.position.merge(&other.position),
            removed: self.removed.merge(&other.removed),
            note: self.note.merge(&other.note),
        }
    }

    fn latest(&self) -> Stamp {
        [
            self.link.stamp,
            self.title.stamp,
            self.alias.stamp,
            self.folder.stamp,
            self.position.stamp,
            self.removed.stamp,
            self.note.stamp,
        ]
        .into_iter()
        .max()
        .unwrap_or(self.link.stamp)
    }
}

impl Folder {
    /// The folder after both replicas' writes.
    #[must_use]
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            name: self.name.merge(&other.name),
            mark: self.mark.merge(&other.mark),
            parent: self.parent.merge(&other.parent),
            position: self.position.merge(&other.position),
            removed: self.removed.merge(&other.removed),
        }
    }

    fn latest(&self) -> Stamp {
        [
            self.name.stamp,
            self.mark.stamp,
            self.parent.stamp,
            self.position.stamp,
            self.removed.stamp,
        ]
        .into_iter()
        .max()
        .unwrap_or(self.name.stamp)
    }
}

/// Every shortcut and folder a replica holds, removed ones included.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct State {
    pub shortcuts: BTreeMap<ItemId, Shortcut>,
    pub folders: BTreeMap<ItemId, Folder>,
}

/// An entry of a folder or of a search result.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Entry {
    Folder(ItemId),
    Shortcut(ItemId),
}

/// Where an entry goes among its new siblings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Slot {
    /// After every sibling.
    End,
    /// Right before the sibling.
    Before(Entry),
    /// Right after the sibling.
    After(Entry),
}

/// One change a device makes.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Edit {
    AddShortcut {
        id: ItemId,
        link: String,
        title: String,
        folder: Option<ItemId>,
    },
    Retitle {
        id: ItemId,
        title: String,
    },
    Realias {
        id: ItemId,
        alias: String,
    },
    AddFolder {
        id: ItemId,
        name: String,
        parent: Option<ItemId>,
    },
    RenameFolder {
        id: ItemId,
        name: String,
    },
    /// Gives a folder a mark to show in place of its glyph, or clears it with an empty one.
    MarkFolder {
        id: ItemId,
        mark: String,
    },
    /// Moves an entry into a folder, or the top, at a slot among its entries.
    Move {
        entry: Entry,
        folder: Option<ItemId>,
        slot: Slot,
    },
    /// Removes a shortcut, or a folder with everything it holds.
    Remove {
        entry: Entry,
    },
    /// Writes a shortcut's note, or clears it with an empty one.
    SetNote {
        id: ItemId,
        note: String,
    },
}

/// Why an edit was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum EditError {
    /// The edit names an entry the state does not hold, or holds as removed.
    Unknown(Entry),
    /// The edit adds an identifier the state already holds.
    Exists(Entry),
    /// The edit would move a folder into itself or into a folder it holds.
    IntoItself(ItemId),
    /// No position fits the slot.
    Position(Unordered),
}

/// The folder tree every replica agrees on: the effective parent of every live folder.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Tree {
    parents: BTreeMap<ItemId, Option<ItemId>>,
}

impl Tree {
    /// Whether the folder is live.
    #[must_use]
    pub fn holds(&self, folder: ItemId) -> bool {
        self.parents.contains_key(&folder)
    }

    /// The effective parent of a live folder, or `None` for the top and for a folder the tree does not hold.
    #[must_use]
    pub fn parent(&self, folder: ItemId) -> Option<ItemId> {
        self.parents.get(&folder).copied().flatten()
    }

    /// The live folders from the top down to `folder`, which is last, or nothing for the top.
    #[must_use]
    pub fn lineage(&self, folder: Option<ItemId>) -> Vec<ItemId> {
        let mut lineage = Vec::new();
        let mut current = folder.filter(|folder| self.holds(*folder));
        // The tree has no cycle, so a walk up meets each live folder at most once.
        for _ in 0..self.parents.len() {
            let Some(node) = current else {
                break;
            };
            lineage.push(node);
            current = self.parent(node);
        }
        lineage.reverse();
        lineage
    }

    /// Whether `folder` is `ancestor` or lies inside it.
    #[must_use]
    pub fn within(&self, folder: ItemId, ancestor: ItemId) -> bool {
        self.lineage(Some(folder)).contains(&ancestor)
    }
}

const fn register<T>(value: T, stamp: Stamp) -> Register<T> {
    Register { value, stamp }
}

/// Overwrites a register with a write at `stamp`, as the merge with that write would.
fn write<T: Ord + Clone>(target: &mut Register<T>, value: T, stamp: Stamp) {
    *target = target.merge(&register(value, stamp));
}

fn merge_maps<K: Ord + Copy, V: Clone>(
    left: &BTreeMap<K, V>,
    right: &BTreeMap<K, V>,
    merge: impl Fn(&V, &V) -> V,
) -> BTreeMap<K, V> {
    let left: Vec<(K, V)> = left.iter().map(|(id, item)| (*id, item.clone())).collect();
    let right: Vec<(K, V)> = right.iter().map(|(id, item)| (*id, item.clone())).collect();
    merge_sorted(&left, &right, merge).into_iter().collect()
}

impl State {
    /// The state after both replicas' writes.
    #[must_use]
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            shortcuts: merge_maps(&self.shortcuts, &other.shortcuts, Shortcut::merge),
            folders: merge_maps(&self.folders, &other.folders, Folder::merge),
        }
    }

    /// The latest stamp of any write, which the device's clock observes before its next edit.
    #[must_use]
    pub fn latest(&self) -> Option<Stamp> {
        let shortcuts = self.shortcuts.values().map(Shortcut::latest);
        let folders = self.folders.values().map(Folder::latest);
        shortcuts.chain(folders).max()
    }

    /// A shortcut that is not removed.
    #[must_use]
    pub fn shortcut(&self, id: ItemId) -> Option<&Shortcut> {
        self.shortcuts
            .get(&id)
            .filter(|shortcut| !shortcut.removed.value)
    }

    /// A folder that is not removed.
    #[must_use]
    pub fn folder(&self, id: ItemId) -> Option<&Folder> {
        self.folders.get(&id).filter(|folder| !folder.removed.value)
    }

    /// The folder tree of the live folders.
    #[must_use]
    pub fn tree(&self) -> Tree {
        let live: Vec<(ItemId, &Folder)> = self
            .folders
            .iter()
            .filter(|(_, folder)| !folder.removed.value)
            .map(|(id, folder)| (*id, folder))
            .collect();
        let index: BTreeMap<ItemId, usize> = live
            .iter()
            .enumerate()
            .map(|(place, (id, _))| (*id, place))
            .collect();
        let declared: Vec<Declared<Stamp>> = live
            .iter()
            .map(|(_, folder)| Declared {
                parent: folder
                    .parent
                    .value
                    .and_then(|parent| index.get(&parent).copied()),
                recency: folder.parent.stamp,
            })
            .collect();
        let resolved = tree_rules::resolve(&declared);
        let parents = live
            .iter()
            .zip(resolved)
            .map(|((child, _), parent)| {
                let parent = parent.and_then(|place| live.get(place).map(|(id, _)| *id));
                (*child, parent)
            })
            .collect();
        Tree { parents }
    }

    /// The live entries of a folder, or of the top, in their order.
    #[must_use]
    pub fn entries(&self, tree: &Tree, folder: Option<ItemId>) -> Vec<Entry> {
        let mut placed: Vec<(&Key, Entry)> = Vec::new();
        for (id, child) in &self.folders {
            if !child.removed.value && tree.holds(*id) && tree.parent(*id) == folder {
                placed.push((&child.position.value, Entry::Folder(*id)));
            }
        }
        for (id, shortcut) in &self.shortcuts {
            if !shortcut.removed.value && home(tree, shortcut) == folder {
                placed.push((&shortcut.position.value, Entry::Shortcut(*id)));
            }
        }
        placed.sort();
        placed.into_iter().map(|(_, entry)| entry).collect()
    }

    /// The position of a live entry.
    fn position(&self, entry: Entry) -> Option<&Key> {
        match entry {
            Entry::Folder(id) => self.folder(id).map(|folder| &folder.position.value),
            Entry::Shortcut(id) => self.shortcut(id).map(|shortcut| &shortcut.position.value),
        }
    }

    /// The live folder or shortcut an entry names, as an error when it is gone.
    fn require(&self, entry: Entry) -> Result<(), EditError> {
        self.position(entry)
            .map(|_| ())
            .ok_or(EditError::Unknown(entry))
    }

    /// A position for an entry at a destination, leaving `moving` out of its new siblings.
    fn place(&self, tree: &Tree, to: Destination, moving: Option<Entry>) -> Result<Key, EditError> {
        let mut positions: Vec<&Key> = self
            .entries(tree, to.folder)
            .into_iter()
            .filter(|entry| Some(*entry) != moving)
            .filter_map(|entry| self.position(entry))
            .collect();
        positions.sort_unstable();
        let (low, high) = match to.slot {
            Slot::End => (positions.last().copied(), None),
            Slot::Before(sibling) => {
                let anchor = self.position(sibling).ok_or(EditError::Unknown(sibling))?;
                let below = positions.iter().rev().find(|position| **position < anchor);
                (below.copied(), Some(anchor))
            }
            Slot::After(sibling) => {
                let anchor = self.position(sibling).ok_or(EditError::Unknown(sibling))?;
                let above = positions.iter().find(|position| **position > anchor);
                (Some(anchor), above.copied())
            }
        };
        position_rules::between(low, high).map_err(EditError::Position)
    }

    fn require_folder(&self, folder: Option<ItemId>) -> Result<(), EditError> {
        folder.map_or(Ok(()), |id| self.require(Entry::Folder(id)))
    }

    /// Applies one edit, written at `stamp`.
    ///
    /// # Errors
    ///
    /// Returns an [`EditError`] when the edit names an entry that is gone, adds one that exists, moves a folder into itself, or finds no position; the state is then unchanged.
    pub fn apply(&mut self, edit: Edit, stamp: Stamp) -> Result<(), EditError> {
        let tree = self.tree();
        let at_end = |folder| Destination {
            folder,
            slot: Slot::End,
        };
        match edit {
            Edit::AddShortcut {
                id,
                link,
                title,
                folder,
            } => {
                if self.shortcuts.contains_key(&id) {
                    return Err(EditError::Exists(Entry::Shortcut(id)));
                }
                self.require_folder(folder)?;
                let position = self.place(&tree, at_end(folder), None)?;
                let added = Shortcut::new([link, title], folder, position, stamp);
                self.shortcuts.insert(id, added);
            }
            Edit::AddFolder { id, name, parent } => {
                if self.folders.contains_key(&id) {
                    return Err(EditError::Exists(Entry::Folder(id)));
                }
                self.require_folder(parent)?;
                let position = self.place(&tree, at_end(parent), None)?;
                self.folders.insert(
                    id,
                    Folder {
                        name: register(name, stamp),
                        mark: register(String::new(), stamp),
                        parent: register(parent, stamp),
                        position: register(position, stamp),
                        removed: register(false, stamp),
                    },
                );
            }
            Edit::Retitle { id, title } => {
                self.change(id, |shortcut| write(&mut shortcut.title, title, stamp))?;
            }
            Edit::Realias { id, alias } => {
                self.change(id, |shortcut| write(&mut shortcut.alias, alias, stamp))?;
            }
            Edit::RenameFolder { id, name } => {
                self.require(Entry::Folder(id))?;
                if let Some(folder) = self.folders.get_mut(&id) {
                    write(&mut folder.name, name, stamp);
                }
            }
            Edit::MarkFolder { id, mark } => {
                self.require(Entry::Folder(id))?;
                if let Some(folder) = self.folders.get_mut(&id) {
                    write(&mut folder.mark, mark, stamp);
                }
            }
            Edit::Move {
                entry,
                folder,
                slot,
            } => {
                let context = Context { tree: &tree, stamp };
                self.move_entry(&context, entry, Destination { folder, slot })?;
            }
            Edit::Remove { entry } => self.remove(&tree, entry, stamp)?,
            Edit::SetNote { id, note } => {
                self.change(id, |shortcut| write(&mut shortcut.note, note, stamp))?;
            }
        }
        Ok(())
    }

    /// Changes a live shortcut, or refuses a shortcut that is gone.
    fn change(&mut self, id: ItemId, change: impl FnOnce(&mut Shortcut)) -> Result<(), EditError> {
        self.require(Entry::Shortcut(id))?;
        if let Some(shortcut) = self.shortcuts.get_mut(&id) {
            change(shortcut);
        }
        Ok(())
    }

    fn move_entry(
        &mut self,
        context: &Context<'_>,
        entry: Entry,
        to: Destination,
    ) -> Result<(), EditError> {
        self.require(entry)?;
        self.require_folder(to.folder)?;
        if let (Entry::Folder(id), Some(target)) = (entry, to.folder)
            && context.tree.within(target, id)
        {
            return Err(EditError::IntoItself(id));
        }
        let position = self.place(context.tree, to, Some(entry))?;
        match entry {
            Entry::Folder(id) => {
                if let Some(moved) = self.folders.get_mut(&id) {
                    write(&mut moved.parent, to.folder, context.stamp);
                    write(&mut moved.position, position, context.stamp);
                }
            }
            Entry::Shortcut(id) => {
                if let Some(moved) = self.shortcuts.get_mut(&id) {
                    write(&mut moved.folder, to.folder, context.stamp);
                    write(&mut moved.position, position, context.stamp);
                }
            }
        }
        Ok(())
    }

    fn remove(&mut self, tree: &Tree, entry: Entry, stamp: Stamp) -> Result<(), EditError> {
        self.require(entry)?;
        match entry {
            Entry::Shortcut(id) => {
                if let Some(shortcut) = self.shortcuts.get_mut(&id) {
                    write(&mut shortcut.removed, true, stamp);
                }
            }
            Entry::Folder(removed) => {
                let doomed: BTreeSet<ItemId> = self
                    .folders
                    .keys()
                    .copied()
                    .filter(|folder| tree.within(*folder, removed))
                    .collect();
                let held: Vec<ItemId> = self
                    .shortcuts
                    .iter()
                    .filter(|(_, shortcut)| {
                        !shortcut.removed.value
                            && home(tree, shortcut).is_some_and(|folder| doomed.contains(&folder))
                    })
                    .map(|(shortcut, _)| *shortcut)
                    .collect();
                for shortcut in held {
                    if let Some(doomed_shortcut) = self.shortcuts.get_mut(&shortcut) {
                        write(&mut doomed_shortcut.removed, true, stamp);
                    }
                }
                for folder in doomed {
                    if let Some(doomed_folder) = self.folders.get_mut(&folder) {
                        write(&mut doomed_folder.removed, true, stamp);
                    }
                }
            }
        }
        Ok(())
    }

    /// The names of the live folders from the top down to `folder`.
    #[must_use]
    pub fn path(&self, tree: &Tree, folder: Option<ItemId>) -> Vec<&str> {
        tree.lineage(folder)
            .into_iter()
            .filter_map(|id| self.folder(id).map(|found| found.name.value.as_str()))
            .collect()
    }

    /// The folder a live shortcut sits in, as every replica draws it.
    #[must_use]
    pub fn folder_of(&self, tree: &Tree, id: ItemId) -> Option<ItemId> {
        self.shortcut(id).and_then(|shortcut| home(tree, shortcut))
    }

    /// The live shortcut that holds `link` in `folder`, or the top for `None`, which a folder holds at most once.
    #[must_use]
    pub fn holding(&self, tree: &Tree, folder: Option<ItemId>, link: &str) -> Option<ItemId> {
        self.shortcuts
            .iter()
            .find(|(_, shortcut)| {
                !shortcut.removed.value
                    && shortcut.link.value == link
                    && home(tree, shortcut) == folder
            })
            .map(|(id, _)| *id)
    }
}

/// Where an entry goes: a folder, or the top, and a slot among its entries.
#[derive(Clone, Copy, Debug)]
struct Destination {
    folder: Option<ItemId>,
    slot: Slot,
}

/// What every write of one edit shares: the tree before it and its stamp.
#[derive(Clone, Copy, Debug)]
struct Context<'a> {
    tree: &'a Tree,
    stamp: Stamp,
}

/// The folder a shortcut effectively sits in: its declared folder while that is live, and the top otherwise.
fn home(tree: &Tree, shortcut: &Shortcut) -> Option<ItemId> {
    shortcut.folder.value.filter(|folder| tree.holds(*folder))
}

#[cfg(test)]
mod tests {
    use super::{Edit, EditError, Entry, ItemId, Slot, State};
    use crate::clock_rules::{DeviceId, Stamp};
    use alloc::string::String;
    use alloc::vec::Vec;

    const DEVICE: DeviceId = DeviceId([1; 16]);
    const OTHER: DeviceId = DeviceId([2; 16]);

    const fn id(byte: u8) -> ItemId {
        ItemId([byte; 16])
    }

    const fn at(wall: u64, device: DeviceId) -> Stamp {
        Stamp {
            wall,
            counter: 0,
            device,
        }
    }

    fn add(state: &mut State, byte: u8, folder: Option<ItemId>, wall: u64) {
        state
            .apply(
                Edit::AddShortcut {
                    id: id(byte),
                    link: alloc::format!("https://example.com/{byte}"),
                    title: alloc::format!("page {byte}"),
                    folder,
                },
                at(wall, DEVICE),
            )
            .unwrap();
    }

    fn folder(state: &mut State, byte: u8, parent: Option<ItemId>, wall: u64) {
        state
            .apply(
                Edit::AddFolder {
                    id: id(byte),
                    name: alloc::format!("folder {byte}"),
                    parent,
                },
                at(wall, DEVICE),
            )
            .unwrap();
    }

    #[test]
    fn a_version_7_identifier_carries_the_time_and_its_marks() {
        let ItemId(bytes) = ItemId::v7(0x0123_4567_89ab, [0xff; 10]);
        assert_eq!(
            bytes,
            [
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0x7f, 0xff, 0xbf, 0xff, 0xff, 0xff, 0xff, 0xff,
                0xff, 0xff
            ]
        );
        let ItemId(zero) = ItemId::v7(0, [0; 10]);
        assert_eq!(zero[6] & 0xf0, 0x70);
        assert_eq!(zero[8] & 0xc0, 0x80);
        assert!(ItemId::v7(1, [0; 10]) < ItemId::v7(2, [0; 10]));
    }

    #[test]
    fn a_version_7_identifier_gives_its_time_back() {
        for now in [0, 1, 0x0123_4567_89ab, (1 << 48) - 1] {
            for random in [[0; 10], [0xff; 10], [0x5a; 10]] {
                assert_eq!(ItemId::v7(now, random).time(), Some(now));
            }
        }
        assert_eq!(id(2).time(), None, "no version mark");
        let mut unmarked = ItemId::v7(5, [0; 10]);
        unmarked.0[8] = 0x3f;
        assert_eq!(unmarked.time(), None, "no variant mark");
        let mut other_version = ItemId::v7(5, [0; 10]);
        other_version.0[6] = 0x40;
        assert_eq!(other_version.time(), None, "version 4");
        let mut high_variant = ItemId::v7(5, [0; 10]);
        high_variant.0[8] = 0xc0;
        assert_eq!(high_variant.time(), None, "another variant");
    }

    #[test]
    fn entries_keep_the_order_they_were_added_in() {
        let mut state = State::default();
        add(&mut state, 1, None, 1);
        folder(&mut state, 2, None, 2);
        add(&mut state, 3, None, 3);
        add(&mut state, 4, Some(id(2)), 4);
        let tree = state.tree();
        assert_eq!(
            state.entries(&tree, None),
            [
                Entry::Shortcut(id(1)),
                Entry::Folder(id(2)),
                Entry::Shortcut(id(3))
            ]
        );
        assert_eq!(state.entries(&tree, Some(id(2))), [Entry::Shortcut(id(4))]);
        assert_eq!(state.path(&tree, Some(id(2))), ["folder 2"]);
        assert_eq!(state.folder_of(&tree, id(4)), Some(id(2)));
        assert_eq!(state.latest(), Some(at(4, DEVICE)));
    }

    #[test]
    fn moves_land_before_or_after_a_sibling_or_at_the_end() {
        let mut state = State::default();
        for byte in 1..=3 {
            add(&mut state, byte, None, u64::from(byte));
        }
        let first = Entry::Shortcut(id(1));
        let third = Entry::Shortcut(id(3));
        state
            .apply(
                Edit::Move {
                    entry: third,
                    folder: None,
                    slot: Slot::Before(first),
                },
                at(4, DEVICE),
            )
            .unwrap();
        let order = |current: &State| current.entries(&current.tree(), None);
        assert_eq!(order(&state), [third, first, Entry::Shortcut(id(2))]);
        state
            .apply(
                Edit::Move {
                    entry: third,
                    folder: None,
                    slot: Slot::After(first),
                },
                at(5, DEVICE),
            )
            .unwrap();
        assert_eq!(order(&state), [first, third, Entry::Shortcut(id(2))]);
        state
            .apply(
                Edit::Move {
                    entry: first,
                    folder: None,
                    slot: Slot::End,
                },
                at(6, DEVICE),
            )
            .unwrap();
        assert_eq!(order(&state), [third, Entry::Shortcut(id(2)), first]);
    }

    #[test]
    fn removing_a_folder_removes_what_it_holds() {
        let mut state = State::default();
        folder(&mut state, 1, None, 1);
        folder(&mut state, 2, Some(id(1)), 2);
        add(&mut state, 3, Some(id(2)), 3);
        add(&mut state, 4, None, 4);
        folder(&mut state, 6, None, 4);
        add(&mut state, 7, Some(id(6)), 4);
        state
            .apply(
                Edit::Remove {
                    entry: Entry::Folder(id(1)),
                },
                at(5, DEVICE),
            )
            .unwrap();
        let tree = state.tree();
        assert_eq!(
            state.entries(&tree, None),
            [Entry::Shortcut(id(4)), Entry::Folder(id(6))]
        );
        assert_eq!(state.entries(&tree, Some(id(6))), [Entry::Shortcut(id(7))]);
        assert!(state.shortcut(id(3)).is_none());
        assert!(state.folder(id(2)).is_none());
        assert_eq!(
            state.apply(
                Edit::Retitle {
                    id: id(3),
                    title: String::from("x")
                },
                at(6, DEVICE)
            ),
            Err(EditError::Unknown(Entry::Shortcut(id(3))))
        );
    }

    #[test]
    fn invalid_edits_change_nothing() {
        let mut state = State::default();
        folder(&mut state, 1, None, 1);
        folder(&mut state, 2, Some(id(1)), 2);
        add(&mut state, 3, None, 3);
        let before = state.clone();
        let refusals = [
            (
                Edit::Move {
                    entry: Entry::Folder(id(1)),
                    folder: Some(id(2)),
                    slot: Slot::End,
                },
                EditError::IntoItself(id(1)),
            ),
            (
                Edit::Move {
                    entry: Entry::Folder(id(1)),
                    folder: Some(id(1)),
                    slot: Slot::End,
                },
                EditError::IntoItself(id(1)),
            ),
            (
                Edit::AddFolder {
                    id: id(1),
                    name: String::new(),
                    parent: None,
                },
                EditError::Exists(Entry::Folder(id(1))),
            ),
            (
                Edit::AddShortcut {
                    id: id(3),
                    link: String::new(),
                    title: String::new(),
                    folder: None,
                },
                EditError::Exists(Entry::Shortcut(id(3))),
            ),
            (
                Edit::AddShortcut {
                    id: id(9),
                    link: String::new(),
                    title: String::new(),
                    folder: Some(id(8)),
                },
                EditError::Unknown(Entry::Folder(id(8))),
            ),
            (
                Edit::Move {
                    entry: Entry::Shortcut(id(3)),
                    folder: None,
                    slot: Slot::Before(Entry::Shortcut(id(7))),
                },
                EditError::Unknown(Entry::Shortcut(id(7))),
            ),
            (
                Edit::RenameFolder {
                    id: id(7),
                    name: String::new(),
                },
                EditError::Unknown(Entry::Folder(id(7))),
            ),
            (
                Edit::Realias {
                    id: id(7),
                    alias: String::new(),
                },
                EditError::Unknown(Entry::Shortcut(id(7))),
            ),
        ];
        for (edit, refusal) in refusals {
            assert_eq!(state.apply(edit, at(9, DEVICE)), Err(refusal));
            assert_eq!(state, before);
        }
    }

    #[test]
    fn concurrent_edits_merge_to_the_same_state_in_either_order() {
        let mut base = State::default();
        folder(&mut base, 1, None, 1);
        folder(&mut base, 2, None, 2);
        add(&mut base, 3, None, 3);
        let mut left = base.clone();
        let mut right = base.clone();
        left.apply(
            Edit::Move {
                entry: Entry::Folder(id(1)),
                folder: Some(id(2)),
                slot: Slot::End,
            },
            at(10, DEVICE),
        )
        .unwrap();
        right
            .apply(
                Edit::Move {
                    entry: Entry::Folder(id(2)),
                    folder: Some(id(1)),
                    slot: Slot::End,
                },
                at(11, OTHER),
            )
            .unwrap();
        left.apply(
            Edit::Retitle {
                id: id(3),
                title: String::from("left"),
            },
            at(12, DEVICE),
        )
        .unwrap();
        right
            .apply(
                Edit::Remove {
                    entry: Entry::Shortcut(id(3)),
                },
                at(13, OTHER),
            )
            .unwrap();
        let merged = left.merge(&right);
        assert_eq!(merged, right.merge(&left));
        assert_eq!(merged.merge(&merged), merged);
        let tree = merged.tree();
        assert_eq!(tree.parent(id(1)), Some(id(2)));
        assert_eq!(tree.parent(id(2)), None);
        assert!(merged.shortcut(id(3)).is_none());
        assert_eq!(
            merged
                .shortcuts
                .get(&id(3))
                .map(|shortcut| shortcut.title.value.as_str()),
            Some("left")
        );
    }

    #[test]
    fn an_entry_added_to_a_folder_removed_elsewhere_surfaces_at_the_top() {
        let mut base = State::default();
        folder(&mut base, 1, None, 1);
        let mut left = base.clone();
        let mut right = base;
        left.apply(
            Edit::Remove {
                entry: Entry::Folder(id(1)),
            },
            at(5, DEVICE),
        )
        .unwrap();
        right
            .apply(
                Edit::AddShortcut {
                    id: id(2),
                    link: String::from("https://example.com/"),
                    title: String::from("kept"),
                    folder: Some(id(1)),
                },
                at(6, OTHER),
            )
            .unwrap();
        let merged = left.merge(&right);
        let tree = merged.tree();
        let top: Vec<Entry> = merged.entries(&tree, None);
        assert_eq!(top, [Entry::Shortcut(id(2))]);
        assert_eq!(merged.folder_of(&tree, id(2)), None);
    }

    #[test]
    fn a_folder_holds_a_link_once_and_another_folder_may_hold_it_too() {
        let mut state = State::default();
        folder(&mut state, 10, None, 1);
        add(&mut state, 1, None, 2);
        add(&mut state, 2, Some(id(10)), 3);
        let tree = state.tree();
        assert_eq!(
            state.holding(&tree, None, "https://example.com/1"),
            Some(id(1))
        );
        assert_eq!(
            state.holding(&tree, Some(id(10)), "https://example.com/1"),
            None
        );
        assert_eq!(
            state.holding(&tree, Some(id(10)), "https://example.com/2"),
            Some(id(2))
        );
        state
            .apply(
                Edit::Remove {
                    entry: Entry::Shortcut(id(1)),
                },
                at(4, DEVICE),
            )
            .unwrap();
        assert_eq!(
            state.holding(&state.tree(), None, "https://example.com/1"),
            None,
            "a removed shortcut holds nothing"
        );
    }

    #[test]
    fn a_note_is_written_cleared_and_counted_in_the_latest_stamp() {
        let mut state = State::default();
        add(&mut state, 1, None, 1);
        assert_eq!(state.shortcut(id(1)).unwrap().note.value, "");
        state
            .apply(
                Edit::SetNote {
                    id: id(1),
                    note: String::from("read the errata first"),
                },
                at(2, DEVICE),
            )
            .unwrap();
        assert_eq!(
            state.shortcut(id(1)).unwrap().note.value,
            "read the errata first"
        );
        assert_eq!(state.latest(), Some(at(2, DEVICE)));
        state
            .apply(
                Edit::SetNote {
                    id: id(1),
                    note: String::new(),
                },
                at(3, DEVICE),
            )
            .unwrap();
        assert_eq!(state.shortcut(id(1)).unwrap().note.value, "");
        assert_eq!(
            state.apply(
                Edit::SetNote {
                    id: id(9),
                    note: String::from("x"),
                },
                at(4, DEVICE),
            ),
            Err(EditError::Unknown(Entry::Shortcut(id(9))))
        );
    }

    #[test]
    fn concurrent_notes_merge_to_the_same_state_in_either_order() {
        let mut base = State::default();
        add(&mut base, 1, None, 1);
        let mut left = base.clone();
        let mut right = base;
        left.apply(
            Edit::SetNote {
                id: id(1),
                note: String::from("left"),
            },
            at(5, DEVICE),
        )
        .unwrap();
        right
            .apply(
                Edit::SetNote {
                    id: id(1),
                    note: String::from("right"),
                },
                at(6, OTHER),
            )
            .unwrap();
        let merged = left.merge(&right);
        assert_eq!(merged, right.merge(&left));
        assert_eq!(merged.merge(&merged), merged);
        assert_eq!(merged.merge(&left), merged);
        assert_eq!(merged.shortcut(id(1)).unwrap().note.value, "right");
        assert_eq!(merged.latest(), Some(at(6, OTHER)));
    }
}
