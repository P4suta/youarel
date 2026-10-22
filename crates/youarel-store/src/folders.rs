//! The folders of a store, which a session holds whole with the tree every replica draws from them.
//!
//! A store assumes at most [`MOST_FOLDERS_IN_MEMORY`] folders, so every session reads them all.
//! The tree resolves once a session first asks for it after a change, so a merge or an import of many folders resolves it once.

use std::cell::OnceCell;
use std::collections::BTreeMap;
use youarel_core::index_rules::{self, Standing};
use youarel_core::library_rules;
use youarel_core::position_rules::Key;
use youarel_core::state::{Folder, ItemId, Presence, Shortcut, Tree, tree_of};

/// The most folders a store expects to hold, which every session reads whole.
pub const MOST_FOLDERS_IN_MEMORY: usize = 100_000;

/// The tree of the folders not gone for good, and each folder's folders that show, in their order and by their names.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Resolved {
    tree: Tree,
    children: BTreeMap<Option<ItemId>, Vec<ItemId>>,
    named: BTreeMap<Option<ItemId>, Vec<ItemId>>,
}

/// Every folder record of a store, those in the trash and those gone for good included, with the tree of the others and each folder's folders that show, in their order.
#[derive(Clone, Debug, Default)]
pub struct Folders {
    records: BTreeMap<ItemId, Folder>,
    resolved: OnceCell<Resolved>,
}

impl PartialEq for Folders {
    fn eq(&self, other: &Self) -> bool {
        self.records == other.records
    }
}

impl Eq for Folders {}

impl Folders {
    /// The folders of these records.
    #[must_use]
    pub const fn new(records: BTreeMap<ItemId, Folder>) -> Self {
        Self {
            records,
            resolved: OnceCell::new(),
        }
    }

    /// Takes a folder's record in place of the one the store held, which the next question about the tree resolves again.
    pub(crate) fn set(&mut self, id: ItemId, folder: Folder) {
        self.records.insert(id, folder);
        self.resolved = OnceCell::new();
    }

    /// The tree and the children, resolved once after each change.
    fn resolved(&self) -> &Resolved {
        self.resolved.get_or_init(|| {
            let tree = tree_of(&self.records);
            let mut placed: BTreeMap<Option<ItemId>, Vec<(&Key, ItemId)>> = BTreeMap::new();
            for (id, parent) in tree.folders() {
                if let Some(folder) = self.records.get(&id).filter(|_| tree.shows(id)) {
                    placed
                        .entry(parent)
                        .or_default()
                        .push((&folder.position.value, id));
                }
            }
            let children: BTreeMap<Option<ItemId>, Vec<ItemId>> = placed
                .into_iter()
                .map(|(parent, mut inside)| {
                    inside.sort();
                    (parent, inside.into_iter().map(|(_, id)| id).collect())
                })
                .collect();
            let named = children
                .iter()
                .map(|(parent, inside)| {
                    let mut by_name: Vec<(String, String, ItemId)> = inside
                        .iter()
                        .filter_map(|id| {
                            self.records
                                .get(id)
                                .map(|folder| library_rules::by_name(&folder.name.value, *id))
                        })
                        .collect();
                    by_name.sort_unstable();
                    (*parent, by_name.into_iter().map(|(_, _, id)| id).collect())
                })
                .collect();
            Resolved {
                tree,
                children,
                named,
            }
        })
    }

    /// The tree of the folders not gone for good.
    #[must_use]
    pub fn tree(&self) -> &Tree {
        &self.resolved().tree
    }

    /// Every folder record, those in the trash and those gone for good included.
    #[must_use]
    pub const fn records(&self) -> &BTreeMap<ItemId, Folder> {
        &self.records
    }

    /// A folder that shows.
    #[must_use]
    pub fn folder(&self, id: ItemId) -> Option<&Folder> {
        self.records.get(&id).filter(|_| self.shows(id))
    }

    /// Whether the tree holds the folder: it shows or the trash hides it, and it is not gone for good.
    #[must_use]
    pub fn holds(&self, id: ItemId) -> bool {
        self.tree().holds(id)
    }

    /// Whether the folder shows: neither it nor a folder over it waits in the trash, and it is not gone for good.
    #[must_use]
    pub fn shows(&self, id: ItemId) -> bool {
        self.tree().shows(id)
    }

    /// Whether the folder's own presence is live, which a folder an edit puts an entry into needs, whether or not a folder in the trash hides it.
    #[must_use]
    pub fn live(&self, id: ItemId) -> bool {
        self.records
            .get(&id)
            .is_some_and(|folder| folder.presence.value == Presence::Live)
    }

    /// The folders that show directly in a folder, or at the top, in their order, which the terminal launcher keeps.
    #[must_use]
    pub fn children(&self, parent: Option<ItemId>) -> &[ItemId] {
        self.resolved()
            .children
            .get(&parent)
            .map_or(&[], Vec::as_slice)
    }

    /// The folders that show directly in a folder, or at the top, by their names without their case, then their names, then their identifiers, as an explorer lists them.
    #[must_use]
    pub fn named(&self, parent: Option<ItemId>) -> &[ItemId] {
        self.resolved()
            .named
            .get(&parent)
            .map_or(&[], Vec::as_slice)
    }

    /// The names of the folders from the top down to `folder`, shown or hidden by the trash.
    #[must_use]
    pub fn path(&self, folder: Option<ItemId>) -> Vec<&str> {
        self.tree()
            .lineage(folder)
            .into_iter()
            .filter_map(|id| self.records.get(&id).map(|found| found.name.value.as_str()))
            .collect()
    }

    /// The standing of the folder a shortcut declares, which a shortcut that declares none passes over.
    #[must_use]
    pub fn declared(&self, shortcut: &Shortcut) -> Standing {
        shortcut
            .folder
            .value
            .map_or(Standing::Shown, |folder| self.tree().standing(folder))
    }

    /// Where a shortcut stands: gone for good, hidden by the trash, or shown.
    #[must_use]
    pub fn standing(&self, shortcut: &Shortcut) -> Standing {
        let declared = shortcut
            .folder
            .value
            .map(|folder| self.tree().standing(folder));
        index_rules::filing(shortcut.presence.value, declared).standing
    }

    /// The folder a shortcut sits in, as every replica draws it: the one it declares while the tree holds it, and the top otherwise.
    #[must_use]
    pub fn home(&self, shortcut: &Shortcut) -> Option<ItemId> {
        index_rules::home(
            shortcut.folder.value,
            shortcut
                .folder
                .value
                .is_some_and(|folder| self.holds(folder)),
        )
    }

    /// Where each folder of a record stands, which tells the folders whose standing a change moved.
    #[must_use]
    pub fn standings(&self) -> BTreeMap<ItemId, Standing> {
        let tree = self.tree();
        self.records
            .keys()
            .map(|id| (*id, tree.standing(*id)))
            .collect()
    }

    /// The folders in the trash themselves, by their names.
    #[must_use]
    pub fn trashed(&self) -> Vec<ItemId> {
        let mut named: Vec<(String, String, ItemId)> = self
            .records
            .iter()
            .filter(|(_, folder)| folder.presence.value == Presence::Trashed)
            .map(|(id, folder)| library_rules::by_name(&folder.name.value, *id))
            .collect();
        named.sort_unstable();
        named.into_iter().map(|(_, _, id)| id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Folders, MOST_FOLDERS_IN_MEMORY};
    use std::collections::BTreeMap;
    use youarel_core::clock_rules::{DeviceId, Stamp};
    use youarel_core::index_rules::Standing;
    use youarel_core::state::{Edit, Entry, ItemId, Presence, Slot, State};

    const fn id(byte: u8) -> ItemId {
        ItemId([byte; 16])
    }

    fn stamp(wall: u64) -> Stamp {
        Stamp {
            wall,
            counter: 0,
            device: DeviceId([1; 16]),
        }
    }

    /// Work holds Docs before Infra, Infra holds a shortcut, Gone is gone for good, and Bin and Attic wait in the trash, Attic inside Bin.
    fn sample() -> State {
        let mut state = State::default();
        let folder = |byte, name: &str, parent| Edit::AddFolder {
            id: id(byte),
            name: name.to_owned(),
            parent,
        };
        let edits = [
            folder(1, "Work", None),
            folder(2, "Infra", Some(id(1))),
            folder(3, "Docs", Some(id(1))),
            Edit::Move {
                entry: Entry::Folder(id(3)),
                folder: Some(id(1)),
                slot: Slot::Before(Entry::Folder(id(2))),
            },
            Edit::AddShortcut {
                id: id(9),
                link: "https://example.com/".to_owned(),
                title: "Example".to_owned(),
                folder: Some(id(2)),
            },
            folder(4, "Gone", None),
            Edit::Remove {
                entry: Entry::Folder(id(4)),
            },
            folder(5, "bin", None),
            folder(6, "Attic", Some(id(5))),
            Edit::Trash {
                entry: Entry::Folder(id(6)),
            },
            Edit::Trash {
                entry: Entry::Folder(id(5)),
            },
        ];
        for (wall, edit) in (1..).zip(edits) {
            state.apply(edit, stamp(wall)).unwrap();
        }
        state
    }

    #[test]
    fn the_folders_hold_their_tree_children_paths_and_homes() {
        let state = sample();
        let folders = Folders::new(state.folders.clone());
        assert_eq!(folders.tree(), &state.tree());
        assert_eq!(folders.records(), &state.folders);
        assert_eq!(folders.children(None), [id(1)]);
        assert_eq!(folders.children(Some(id(1))), [id(3), id(2)]);
        assert_eq!(folders.children(Some(id(2))), []);
        assert_eq!(
            folders.children(Some(id(5))),
            [],
            "a folder the trash hides shows nowhere"
        );
        assert_eq!(folders.path(Some(id(2))), ["Work", "Infra"]);
        assert_eq!(folders.path(Some(id(6))), ["bin", "Attic"]);
        assert_eq!(folders.path(None), Vec::<&str>::new());
        assert_eq!(Folders::default().children(None), []);
        assert_eq!(MOST_FOLDERS_IN_MEMORY, 100_000);
    }

    #[test]
    fn a_folder_shows_hides_in_the_trash_or_is_gone() {
        let state = sample();
        let folders = Folders::new(state.folders.clone());
        assert!(folders.holds(id(1)) && !folders.holds(id(4)) && !folders.holds(id(8)));
        assert!(folders.holds(id(6)) && !folders.shows(id(6)) && folders.shows(id(1)));
        assert!(folders.live(id(1)) && !folders.live(id(5)) && !folders.live(id(4)));
        assert!(folders.folder(id(4)).is_none() && folders.folder(id(3)).is_some());
        assert!(folders.folder(id(6)).is_none());
        assert_eq!(folders.trashed(), [id(6), id(5)], "Attic, then bin");
        let standings = folders.standings();
        assert_eq!(
            standings.get(&id(4)),
            Some(&Standing::Gone),
            "every record stands, the ones gone for good too"
        );
        assert_eq!(standings.get(&id(6)), Some(&Standing::Hidden));
        assert_eq!(standings.len(), state.folders.len());
    }

    #[test]
    fn a_shortcut_stands_and_sits_as_the_folder_it_declares_stands() {
        let state = sample();
        let folders = Folders::new(state.folders.clone());
        let shortcut = state.shortcuts.get(&id(9)).unwrap();
        assert_eq!(folders.declared(shortcut), Standing::Shown);
        assert_eq!(folders.standing(shortcut), Standing::Shown);
        assert_eq!(folders.home(shortcut), Some(id(2)));
        let mut lost = shortcut.clone();
        lost.folder.value = Some(id(4));
        assert_eq!(folders.declared(&lost), Standing::Gone);
        assert_eq!(
            folders.home(&lost),
            None,
            "a gone folder's shortcut sits at the top"
        );
        lost.folder.value = Some(id(6));
        assert_eq!(folders.standing(&lost), Standing::Hidden);
        assert_eq!(folders.home(&lost), Some(id(6)));
        lost.presence.value = Presence::Purged;
        assert_eq!(folders.standing(&lost), Standing::Gone);
        lost.folder.value = None;
        assert_eq!(folders.home(&lost), None);
    }

    #[test]
    fn a_changed_folder_resolves_the_tree_again() {
        let state = sample();
        let mut folders = Folders::new(state.folders.clone());
        assert_eq!(folders.children(Some(id(1))), [id(3), id(2)]);
        let mut moved = state.folders.get(&id(2)).unwrap().clone();
        moved.parent.value = None;
        folders.set(id(2), moved.clone());
        assert_eq!(folders.children(Some(id(1))), [id(3)]);
        assert_eq!(folders.children(None), [id(1), id(2)]);
        assert_eq!(
            folders.named(None),
            [id(2), id(1)],
            "Infra before Work by name, whatever their places"
        );
        assert_eq!(folders.named(Some(id(1))), [id(3)]);
        assert_eq!(folders.named(Some(id(7))), []);
        assert_eq!(folders.records().get(&id(2)), Some(&moved));
        assert_eq!(folders, folders.clone());
        assert_ne!(folders, Folders::new(state.folders));
        assert_ne!(folders, Folders::new(BTreeMap::new()));
    }
}
