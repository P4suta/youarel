//! A session's edits: the facts an edit names, gathered from the store, the decision of `edit_rules`, and the writes it makes.

use super::{Session, write};
use crate::StoreError;
use crate::database::{failed, index, open};
use redb::{AccessGuard, ReadableTable as _, StorageError};
use std::ops::Bound;
use youarel_core::clock_rules::Stamp;
use youarel_core::edit_rules::{self, Facts, Neighbors, Write};
use youarel_core::index_rules::{self, HOME_KEY, TERMINATOR, Table};
use youarel_core::merge_rules::Register;
use youarel_core::position_rules::Key;
use youarel_core::state::{Edit, Entry, Folder, ItemId, Presence, Shortcut, Slot};
use youarel_core::undo_rules::{self, Previous, Reversal};

/// The positions around a slot that a store read, owned until the decision borrows them.
#[derive(Clone, Debug, Default)]
struct Around {
    last: Option<Key>,
    anchor: Option<Key>,
    below: Option<Key>,
    above: Option<Key>,
}

/// Which side of an anchor a neighbor lies on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Side {
    Below,
    Above,
}

/// The position a placement key holds, between its home and its terminator.
fn placed(key: &[u8]) -> Result<Key, StoreError> {
    key.get(HOME_KEY..)
        .and_then(|rest| rest.split(|byte| *byte == TERMINATOR).next())
        .and_then(|text| core::str::from_utf8(text).map_or(None, Key::parse))
        .ok_or(StoreError::Missing("a position"))
}

/// A register written once at `stamp`.
const fn register<T>(value: T, stamp: Stamp) -> Register<T> {
    Register { value, stamp }
}

/// The position of the first placement row that is not the moving shortcut's.
fn first_other<'a>(
    rows: impl Iterator<
        Item = Result<(AccessGuard<'a, &'static [u8]>, AccessGuard<'a, ()>), StorageError>,
    >,
    moving: Option<ItemId>,
) -> Result<Option<Key>, StoreError> {
    for entry in rows {
        let (key, _) = entry.map_err(failed)?;
        let key = key.value();
        if index_rules::identified(key) != moving {
            return placed(key).map(Some);
        }
    }
    Ok(None)
}

impl Session {
    /// Applies an edit and returns the edits that take it back and repeat it.
    ///
    /// # Errors
    ///
    /// Returns an edit or storage error, or a missing reversal for an irreversible edit.
    pub fn edit_remembered(&mut self, edit: Edit) -> Result<Reversal, StoreError> {
        let saved_shortcut = match edit.entry() {
            Some(Entry::Shortcut(id)) => self.record(id)?,
            Some(Entry::Folder(_)) | None => None,
        };
        let saved_folder = match edit.entry() {
            Some(Entry::Folder(id)) => self.folders.records().get(&id),
            Some(Entry::Shortcut(_)) | None => None,
        };
        let previous = saved_shortcut
            .as_ref()
            .map(Previous::Shortcut)
            .or_else(|| saved_folder.map(Previous::Folder));
        let reversed =
            undo_rules::reversal(&edit, previous).ok_or(StoreError::Missing("an undoable edit"))?;
        self.edit(edit)?;
        Ok(reversed)
    }

    /// Applies one edit at the next stamp.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Edit`] when the state refuses the edit, [`StoreError::Clock`] when no stamp is left, and another [`StoreError`] when the store cannot be read or written.
    pub fn edit(&mut self, edit: Edit) -> Result<(), StoreError> {
        let stamp = self.stamp()?;
        let (target, destination, into_itself, around) = self.gather(&edit)?;
        let facts = Facts {
            target,
            destination,
            into_itself,
            neighbors: Neighbors {
                last: around.last.as_ref(),
                anchor: around.anchor.as_ref(),
                below: around.below.as_ref(),
                above: around.above.as_ref(),
            },
        };
        let written = edit_rules::apply(edit, &facts).map_err(StoreError::Edit)?;
        self.carry_out(written, stamp)
    }

    /// The position of an entry that shows, wherever it sits.
    fn position(&self, entry: Entry) -> Result<Option<Key>, StoreError> {
        Ok(match entry {
            Entry::Folder(id) => self
                .folders
                .folder(id)
                .map(|folder| folder.position.value.clone()),
            Entry::Shortcut(id) => self.item(id)?.map(|shortcut| shortcut.position.value),
        })
    }

    /// What an edit names: its entry's presence, whether its destination is live, whether a folder moves into itself, and the positions around its slot.
    fn gather(&self, edit: &Edit) -> Result<(Option<Presence>, bool, bool, Around), StoreError> {
        let (entry, place) = match edit {
            Edit::NameDevice { .. } => return Ok((None, true, false, Around::default())),
            Edit::AddShortcut { id, folder, .. } => {
                (Entry::Shortcut(*id), Some((*folder, Slot::End)))
            }
            Edit::AddFolder { id, parent, .. } => (Entry::Folder(*id), Some((*parent, Slot::End))),
            Edit::Move {
                entry,
                folder,
                slot,
            } => (*entry, Some((*folder, *slot))),
            Edit::RestorePlace { entry, folder, .. } => (*entry, Some((*folder, Slot::End))),
            Edit::Retitle { id, .. } | Edit::Realias { id, .. } | Edit::SetNote { id, .. } => {
                (Entry::Shortcut(*id), None)
            }
            Edit::RenameFolder { id, .. } | Edit::MarkFolder { id, .. } => {
                (Entry::Folder(*id), None)
            }
            Edit::Trash { entry } | Edit::PutBack { entry } | Edit::Remove { entry } => {
                (*entry, None)
            }
        };
        let target = self.presence_of(entry)?;
        let Some((folder, slot)) = place else {
            return Ok((target, true, false, Around::default()));
        };
        let destination = folder.is_none_or(|folder| self.folders.live(folder));
        let into_itself = match (entry, folder) {
            (Entry::Folder(moved), Some(folder)) => self.folders.tree().within(folder, moved),
            (Entry::Folder(_) | Entry::Shortcut(_), _) => false,
        };
        let moving = matches!(edit, Edit::Move { .. }).then_some(entry);
        let around = if matches!(edit, Edit::RestorePlace { .. }) {
            Around::default()
        } else {
            self.around(folder, moving, slot)?
        };
        Ok((target, destination, into_itself, around))
    }

    /// The positions around a slot among the entries that show in a folder, the moving one left out: the last for the end, and the sibling's with its neighbors for a slot beside it.
    fn around(
        &self,
        folder: Option<ItemId>,
        moving: Option<Entry>,
        slot: Slot,
    ) -> Result<Around, StoreError> {
        let sibling = match slot {
            Slot::End => {
                return Ok(Around {
                    last: self.neighbor(folder, moving, None)?,
                    ..Around::default()
                });
            }
            Slot::Before(sibling) | Slot::After(sibling) => sibling,
        };
        // A sibling that does not show leaves the decision to refuse the slot.
        let Some(anchor) = self.position(sibling)? else {
            return Ok(Around::default());
        };
        Ok(Around {
            last: None,
            below: self.neighbor(folder, moving, Some((&anchor, Side::Below)))?,
            above: self.neighbor(folder, moving, Some((&anchor, Side::Above)))?,
            anchor: Some(anchor),
        })
    }

    /// The position nearest an anchor on one side among the entries that show in a folder, the moving one left out, or the last of them without an anchor.
    fn neighbor(
        &self,
        folder: Option<ItemId>,
        moving: Option<Entry>,
        anchor: Option<(&Key, Side)>,
    ) -> Result<Option<Key>, StoreError> {
        let wanted = |position: &Key| match anchor {
            None => true,
            Some((anchor, Side::Below)) => position < anchor,
            Some((anchor, Side::Above)) => position > anchor,
        };
        let folders = self
            .folders
            .children(folder)
            .iter()
            .filter(|id| moving != Some(Entry::Folder(**id)))
            .filter_map(|id| self.folders.folder(*id))
            .map(|found| &found.position.value)
            .filter(|position| wanted(position));
        let shortcut = self.placed_neighbor(folder, moving, anchor)?;
        let candidates = folders.cloned().chain(shortcut);
        Ok(match anchor {
            Some((_, Side::Above)) => candidates.min(),
            None | Some((_, Side::Below)) => candidates.max(),
        })
    }

    /// The position nearest an anchor on one side among the shortcuts that show in a folder, the moving one left out, or the last of them without an anchor.
    fn placed_neighbor(
        &self,
        folder: Option<ItemId>,
        moving: Option<Entry>,
        anchor: Option<(&Key, Side)>,
    ) -> Result<Option<Key>, StoreError> {
        let home = index_rules::home_key(folder);
        let at_anchor = |key: &Key, tail: u8| {
            let mut bound = home.to_vec();
            bound.extend_from_slice(key.as_str().as_bytes());
            bound.push(tail);
            bound
        };
        let (start, end) = match anchor {
            None => (home.to_vec(), super::prefix_end(&home)),
            Some((key, Side::Below)) => (home.to_vec(), Some(at_anchor(key, TERMINATOR))),
            Some((key, Side::Above)) => (at_anchor(key, 1), super::prefix_end(&home)),
        };
        let finish = end.as_deref().map_or(Bound::Unbounded, Bound::Excluded);
        let rows = open(&self.transaction, index(Table::Placement))?;
        let range = rows
            .range::<&[u8]>((Bound::Included(start.as_slice()), finish))
            .map_err(failed)?;
        let moving = match moving {
            Some(Entry::Shortcut(id)) => Some(id),
            Some(Entry::Folder(_)) | None => None,
        };
        match anchor {
            Some((_, Side::Above)) => first_other(range, moving),
            None | Some((_, Side::Below)) => first_other(range.rev(), moving),
        }
    }

    /// Writes what an edit decided, at its stamp.
    fn carry_out(&mut self, written: Write, stamp: Stamp) -> Result<(), StoreError> {
        match written {
            Write::DeviceName { id, name } => self.put_device_name(id, &register(name, stamp)),
            Write::AddShortcut {
                id,
                link,
                title,
                folder,
                position,
            } => {
                let added = Shortcut {
                    link: register(link, stamp),
                    title: register(title, stamp),
                    alias: register(String::new(), stamp),
                    folder: register(folder, stamp),
                    position: register(position, stamp),
                    presence: register(Presence::Live, stamp),
                    note: register(String::new(), stamp),
                };
                self.rewrite(id, None, &added)
            }
            Write::AddFolder {
                id,
                name,
                parent,
                position,
            } => self.put_folder(
                id,
                Folder {
                    name: register(name, stamp),
                    mark: register(String::new(), stamp),
                    parent: register(parent, stamp),
                    position: register(position, stamp),
                    presence: register(Presence::Live, stamp),
                },
            ),
            Write::Title { id, title } => {
                self.change(id, |shortcut| write(&mut shortcut.title, title, stamp))
            }
            Write::Alias { id, alias } => {
                self.change(id, |shortcut| write(&mut shortcut.alias, alias, stamp))
            }
            Write::Note { id, note } => {
                self.change(id, |shortcut| write(&mut shortcut.note, note, stamp))
            }
            Write::Name { id, name } => {
                self.change_folder(id, |folder| write(&mut folder.name, name, stamp))
            }
            Write::Mark { id, mark } => {
                self.change_folder(id, |folder| write(&mut folder.mark, mark, stamp))
            }
            Write::Place {
                entry: Entry::Shortcut(id),
                folder,
                position,
            } => self.change(id, |shortcut| {
                write(&mut shortcut.folder, folder, stamp);
                write(&mut shortcut.position, position, stamp);
            }),
            Write::Place {
                entry: Entry::Folder(id),
                folder,
                position,
            } => self.change_folder(id, |moved| {
                write(&mut moved.parent, folder, stamp);
                write(&mut moved.position, position, stamp);
            }),
            Write::Presence { entry, presence } => self.set_presence(entry, presence, stamp),
            Write::Remove {
                entry: Entry::Shortcut(id),
            } => self.set_presence(Entry::Shortcut(id), Presence::Purged, stamp),
            Write::Remove {
                entry: Entry::Folder(id),
            } => self.remove_folder(id, stamp),
        }
    }

    /// Writes an entry's presence, which refiles a shortcut's rows, or every row a folder's standing moves.
    fn set_presence(
        &mut self,
        entry: Entry,
        presence: Presence,
        stamp: Stamp,
    ) -> Result<(), StoreError> {
        match entry {
            Entry::Shortcut(id) => self.change(id, |shortcut| {
                write(&mut shortcut.presence, presence, stamp);
            }),
            Entry::Folder(id) => {
                self.change_folder(id, |folder| write(&mut folder.presence, presence, stamp))
            }
        }
    }

    /// Changes a shortcut's record and refiles its rows.
    fn change(&mut self, id: ItemId, change: impl FnOnce(&mut Shortcut)) -> Result<(), StoreError> {
        let before =
            self.record(id)?
                .ok_or(StoreError::Edit(youarel_core::state::EditError::Unknown(
                    Entry::Shortcut(id),
                )))?;
        let mut after = before.clone();
        change(&mut after);
        self.rewrite(id, Some(&before), &after)
    }

    /// Changes a folder's record.
    fn change_folder(
        &mut self,
        id: ItemId,
        change: impl FnOnce(&mut Folder),
    ) -> Result<(), StoreError> {
        let mut folder = self
            .folders
            .records()
            .get(&id)
            .cloned()
            .ok_or(StoreError::Edit(youarel_core::state::EditError::Unknown(
                Entry::Folder(id),
            )))?;
        change(&mut folder);
        self.put_folder(id, folder)
    }

    /// Removes a folder for good, with every folder inside it and every shortcut they hold, whether they show or the trash hides them, the shortcuts of each folder read in batches.
    ///
    /// Every shortcut goes before any folder, so a folder that moves to the top for a moment once its parent went takes nothing with it.
    fn remove_folder(&mut self, folder: ItemId, stamp: Stamp) -> Result<(), StoreError> {
        let doomed = edit_rules::removal_plan(self.folders.tree(), folder);
        for id in &doomed {
            let home = index_rules::home_key(Some(*id));
            for table in [Table::Placement, Table::Hidden] {
                self.in_batches(table, &home, |session, ids| {
                    for shortcut in ids {
                        session.set_presence(Entry::Shortcut(shortcut), Presence::Purged, stamp)?;
                    }
                    Ok(())
                })?;
            }
        }
        for id in doomed {
            self.set_presence(Entry::Folder(id), Presence::Purged, stamp)?;
        }
        Ok(())
    }
}
