//! The edits that take a local edit back and repeat it, each applied at a new stamp.

use crate::state::{Edit, Entry, Folder, Presence, Shortcut};

/// The record before an edit, read by the identifier its entry names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Previous<'a> {
    Shortcut(&'a Shortcut),
    Folder(&'a Folder),
}

impl Previous<'_> {
    const fn presence(self) -> Presence {
        match self {
            Self::Shortcut(item) => item.presence.value,
            Self::Folder(item) => item.presence.value,
        }
    }

    const fn names(self, entry: Entry) -> bool {
        matches!(
            (self, entry),
            (Self::Shortcut(_), Entry::Shortcut(_)) | (Self::Folder(_), Entry::Folder(_))
        )
    }

    fn inverse(self, edit: &Edit) -> Option<Edit> {
        Some(match (edit, self) {
            (Edit::Retitle { id, .. }, Self::Shortcut(item)) => Edit::Retitle {
                id: *id,
                title: item.title.value.clone(),
            },
            (Edit::Realias { id, .. }, Self::Shortcut(item)) => Edit::Realias {
                id: *id,
                alias: item.alias.value.clone(),
            },
            (Edit::SetNote { id, .. }, Self::Shortcut(item)) => Edit::SetNote {
                id: *id,
                note: item.note.value.clone(),
            },
            (Edit::RenameFolder { id, .. }, Self::Folder(item)) => Edit::RenameFolder {
                id: *id,
                name: item.name.value.clone(),
            },
            (Edit::MarkFolder { id, .. }, Self::Folder(item)) => Edit::MarkFolder {
                id: *id,
                mark: item.mark.value.clone(),
            },
            (Edit::Move { entry, .. } | Edit::RestorePlace { entry, .. }, Self::Shortcut(item)) => {
                Edit::RestorePlace {
                    entry: *entry,
                    folder: item.folder.value,
                    position: item.position.value.clone(),
                }
            }
            (Edit::Move { entry, .. } | Edit::RestorePlace { entry, .. }, Self::Folder(item)) => {
                Edit::RestorePlace {
                    entry: *entry,
                    folder: item.parent.value,
                    position: item.position.value.clone(),
                }
            }
            (Edit::Trash { entry }, _) => Edit::PutBack { entry: *entry },
            (Edit::PutBack { entry }, _) => Edit::Trash { entry: *entry },
            _ => return None,
        })
    }
}

/// What takes an edit back and repeats it; additions repeat by putting the same entry back.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Reversal {
    pub undo: Edit,
    pub redo: Edit,
}

/// The reversal of an edit of its previous record, or none for an irreversible edit or facts that do not admit it.
#[must_use]
pub fn reversal(edit: &Edit, previous: Option<Previous<'_>>) -> Option<Reversal> {
    let entry = edit.entry()?;
    if matches!(edit, Edit::AddShortcut { .. } | Edit::AddFolder { .. }) {
        return previous.is_none().then_some(Reversal {
            undo: Edit::Trash { entry },
            redo: Edit::PutBack { entry },
        });
    }
    let previous = previous?;
    let needed = if matches!(edit, Edit::PutBack { .. }) {
        Presence::Trashed
    } else {
        Presence::Live
    };
    if !previous.names(entry) || previous.presence() != needed {
        return None;
    }
    Some(Reversal {
        undo: previous.inverse(edit)?,
        redo: edit.clone(),
    })
}

#[cfg(any(test, kani))]
mod proofs {
    use super::{Previous, Reversal, reversal};
    use crate::clock_rules::{DeviceId, Stamp};
    use crate::merge_rules::Register;
    use crate::position_rules::Key;
    use crate::state::{Edit, Entry, Folder, ItemId, Presence, Shortcut, Slot};
    use alloc::string::String;

    const ID: ItemId = ItemId([1; 16]);
    const HOME: ItemId = ItemId([2; 16]);
    const KINDS: u8 = 14;

    const fn register<T>(value: T) -> Register<T> {
        Register {
            value,
            stamp: Stamp {
                wall: 1,
                counter: 0,
                device: DeviceId([3; 16]),
            },
        }
    }

    fn shortcut(presence: Presence) -> Option<Shortcut> {
        Some(Shortcut {
            link: register(String::from("https://a.example/")),
            title: register(String::from("t")),
            alias: register(String::from("a")),
            folder: register(Some(HOME)),
            position: register(Key::parse("a4")?),
            presence: register(presence),
            note: register(String::from("n")),
        })
    }

    fn folder(presence: Presence) -> Option<Folder> {
        Some(Folder {
            name: register(String::from("f")),
            mark: register(String::from("m")),
            parent: register(None),
            position: register(Key::parse("a1")?),
            presence: register(presence),
        })
    }

    fn edit(kind: u8) -> Option<Edit> {
        Some(match kind {
            0 => Edit::AddShortcut {
                id: ID,
                link: String::new(),
                title: String::new(),
                folder: None,
            },
            1 => Edit::AddFolder {
                id: ID,
                name: String::new(),
                parent: None,
            },
            2 => Edit::Retitle {
                id: ID,
                title: String::new(),
            },
            3 => Edit::Realias {
                id: ID,
                alias: String::new(),
            },
            4 => Edit::SetNote {
                id: ID,
                note: String::new(),
            },
            5 => Edit::RenameFolder {
                id: ID,
                name: String::new(),
            },
            6 => Edit::MarkFolder {
                id: ID,
                mark: String::new(),
            },
            7 => Edit::Move {
                entry: Entry::Shortcut(ID),
                folder: None,
                slot: Slot::End,
            },
            8 => Edit::Move {
                entry: Entry::Folder(ID),
                folder: Some(HOME),
                slot: Slot::End,
            },
            9 => Edit::Trash {
                entry: Entry::Shortcut(ID),
            },
            10 => Edit::PutBack {
                entry: Entry::Folder(ID),
            },
            11 => Edit::RestorePlace {
                entry: Entry::Shortcut(ID),
                folder: None,
                position: Key::parse("a2")?,
            },
            12 => Edit::RestorePlace {
                entry: Entry::Folder(ID),
                folder: Some(HOME),
                position: Key::parse("a3")?,
            },
            _ => Edit::Remove {
                entry: Entry::Shortcut(ID),
            },
        })
    }

    fn unavailable(edit: &Edit, previous: Option<Previous<'_>>) -> bool {
        let Some(entry) = edit.entry() else {
            return true;
        };
        let adds = matches!(edit, Edit::AddShortcut { .. } | Edit::AddFolder { .. });
        let needed = if matches!(edit, Edit::PutBack { .. }) {
            Presence::Trashed
        } else {
            Presence::Live
        };
        matches!(edit, Edit::Remove { .. })
            || previous.map_or(!adds, |record| {
                adds || !record.names(entry) || record.presence() != needed
            })
    }

    fn restores(edit: &Edit, previous: Option<Previous<'_>>, answer: Option<&Reversal>) -> bool {
        let Some(answer) = answer else {
            return unavailable(edit, previous);
        };
        if unavailable(edit, previous) {
            return false;
        }
        if answer.undo.entry() != edit.entry() || answer.redo.entry() != edit.entry() {
            return false;
        }
        if matches!(edit, Edit::AddShortcut { .. } | Edit::AddFolder { .. }) {
            return previous.is_none()
                && matches!(answer.undo, Edit::Trash { .. })
                && matches!(answer.redo, Edit::PutBack { .. });
        }
        if answer.redo != *edit {
            return false;
        }
        match (&answer.undo, previous) {
            (Edit::Retitle { title, .. }, Some(Previous::Shortcut(item))) => {
                matches!(edit, Edit::Retitle { .. }) && *title == item.title.value
            }
            (Edit::Realias { alias, .. }, Some(Previous::Shortcut(item))) => {
                matches!(edit, Edit::Realias { .. }) && *alias == item.alias.value
            }
            (Edit::SetNote { note, .. }, Some(Previous::Shortcut(item))) => {
                matches!(edit, Edit::SetNote { .. }) && *note == item.note.value
            }
            (Edit::RenameFolder { name, .. }, Some(Previous::Folder(item))) => {
                matches!(edit, Edit::RenameFolder { .. }) && *name == item.name.value
            }
            (Edit::MarkFolder { mark, .. }, Some(Previous::Folder(item))) => {
                matches!(edit, Edit::MarkFolder { .. }) && *mark == item.mark.value
            }
            (
                Edit::RestorePlace {
                    folder, position, ..
                },
                Some(Previous::Shortcut(item)),
            ) => {
                matches!(edit, Edit::Move { .. } | Edit::RestorePlace { .. })
                    && *folder == item.folder.value
                    && *position == item.position.value
            }
            (
                Edit::RestorePlace {
                    folder, position, ..
                },
                Some(Previous::Folder(item)),
            ) => {
                matches!(edit, Edit::Move { .. } | Edit::RestorePlace { .. })
                    && *folder == item.parent.value
                    && *position == item.position.value
            }
            (Edit::PutBack { .. }, Some(previous)) => {
                matches!(edit, Edit::Trash { .. }) && previous.presence() == Presence::Live
            }
            (Edit::Trash { .. }, Some(previous)) => {
                matches!(edit, Edit::PutBack { .. }) && previous.presence() == Presence::Trashed
            }
            _ => false,
        }
    }

    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(20)]
    fn a_reversal_restores_only_the_changed_fields_and_keeps_the_identifier() {
        let kind: u8 = kani::any();
        let record: u8 = kani::any();
        kani::assume(kind < KINDS && record < 3);
        let presence = match kani::any::<u8>() {
            0 => Presence::Live,
            1 => Presence::Trashed,
            _ => Presence::Purged,
        };
        let shortcut = shortcut(presence).unwrap();
        let folder = folder(presence).unwrap();
        let previous = match record {
            0 => None,
            1 => Some(Previous::Shortcut(&shortcut)),
            _ => Some(Previous::Folder(&folder)),
        };
        let edit = edit(kind).unwrap();
        let answer = reversal(&edit, previous);
        kani::assert(
            restores(&edit, previous, answer.as_ref()),
            "a reversal restores the changed fields and keeps the identifier",
        );
        kani::cover!(
            matches!(
                answer,
                Some(Reversal {
                    undo: Edit::RestorePlace { .. },
                    ..
                })
            ),
            "a move restores its position"
        );
        kani::cover!(
            matches!(
                answer,
                Some(Reversal {
                    redo: Edit::PutBack { .. },
                    ..
                })
            ),
            "an addition repeats with its identifier"
        );
        kani::cover!(
            answer.is_none(),
            "an irreversible or unadmitted edit has no reversal"
        );
    }

    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(20)]
    fn rejects_an_undo_that_keeps_the_new_title() {
        let before = shortcut(Presence::Live).unwrap();
        let edit = edit(2).unwrap();
        let answer = reversal(&edit, Some(Previous::Shortcut(&before)));
        kani::assert(
            matches!(answer, Some(Reversal { undo: Edit::Retitle { title, .. }, .. }) if title.is_empty()),
            "NEGATIVE_UNDO_CONTROL",
        );
    }

    #[test]
    fn every_edit_and_previous_record_meets_the_reversal_contract() {
        for presence in [Presence::Live, Presence::Trashed, Presence::Purged] {
            let shortcut = shortcut(presence).unwrap();
            let folder = folder(presence).unwrap();
            for previous in [
                None,
                Some(Previous::Shortcut(&shortcut)),
                Some(Previous::Folder(&folder)),
            ] {
                for kind in 0..KINDS {
                    let edit = edit(kind).unwrap();
                    assert!(
                        restores(&edit, previous, reversal(&edit, previous).as_ref()),
                        "{edit:?} {previous:?}"
                    );
                }
            }
        }
    }
}
