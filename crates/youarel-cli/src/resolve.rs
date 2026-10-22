//! What a person types on the command line to name a folder or an entry.
//!
//! A folder is a path of names joined by slashes from the top, where an empty path or a lone slash is the top.
//! An entry is a shortcut's alias, title, or link, a prefix of its identifier, or a folder path that ends with a slash.
//! Each kind of name reads the store's indexes, so naming one shortcut reads the shortcuts that share its name.
//! Only an alias of separators alone, which no index files, reads every shortcut.

use anyhow::{Result, bail};
use youarel_core::state::{Entry, ItemId, Shortcut};
use youarel_link::{Link, LinkError};
use youarel_store::codec::hex;
use youarel_store::folders::Folders;
use youarel_store::{Named, Session, StoreError};

/// The shortest identifier prefix that names an entry, which keeps a short word from matching by accident.
pub const SHORTEST_PREFIX: usize = 6;

/// The folder a path names, or `None` for the top.
///
/// # Errors
///
/// Returns an error naming the segment that no folder or more than one folder answers.
pub fn folder(folders: &Folders, path: &str) -> Result<Option<ItemId>> {
    let mut current: Option<ItemId> = None;
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        let children: Vec<ItemId> = folders
            .children(current)
            .iter()
            .copied()
            .filter(|id| {
                folders
                    .folder(*id)
                    .is_some_and(|folder| folder.name.value == segment)
            })
            .collect();
        match children.as_slice() {
            [found] => current = Some(*found),
            [] => bail!("no folder `{segment}` in `{path}`"),
            [_, ..] => bail!("more than one folder is named `{segment}` in `{path}`"),
        }
    }
    Ok(current)
}

/// The text that names a folder, or a shortcut the store holds as `shortcut`, in a message.
#[must_use]
pub fn named(folders: &Folders, entry: Entry, shortcut: Option<&Shortcut>) -> String {
    match entry {
        Entry::Folder(id) => {
            let mut path = folders.path(Some(id)).join("/");
            path.push('/');
            path
        }
        Entry::Shortcut(id) => shortcut.map_or_else(
            || hex(&id.0),
            |shortcut| format!("{} <{}>", shortcut.title.value, shortcut.link.value),
        ),
    }
}

/// The text that names an entry in a message, as the store holds it.
///
/// # Errors
///
/// Returns an error when the store cannot be read.
pub fn describe(session: &Session, entry: Entry) -> Result<String> {
    let shortcut = match entry {
        Entry::Shortcut(id) => session.item(id)?,
        Entry::Folder(_) => None,
    };
    Ok(named(session.folders(), entry, shortcut.as_ref()))
}

/// One kind of name a shortcut answers to, which the resolution tries in order: the live shortcuts that answer to it.
type Tier<'a> = &'a dyn Fn() -> Result<Named, StoreError>;

/// The live shortcuts that hold a link as typed or as its normalized form, in the order of their identifiers.
///
/// A shortcut holds one link, so the two lookups never find the same shortcut.
fn linked(
    session: &Session,
    reference: &str,
    normalized: &Result<String, LinkError>,
) -> Result<Named, StoreError> {
    let mut found = session.holders(reference)?;
    if let Ok(link) = normalized
        && link != reference
    {
        found.extend(session.holders(link)?);
        found.sort_by_key(|(id, _)| *id);
    }
    Ok(found)
}

/// The entry a reference names.
///
/// # Errors
///
/// Returns an error when nothing matches, when several entries match the first kind of name that matches at all, or when the store cannot be read.
pub fn entry(session: &Session, reference: &str) -> Result<Entry> {
    let normalized = Link::parse(reference).map(|link| link.as_str().to_owned());
    let folders = session.folders();
    if reference.ends_with('/') && normalized.is_err() {
        return match folder(folders, reference)? {
            Some(id) => Ok(Entry::Folder(id)),
            None => bail!("the top is not an entry; name a folder or a shortcut"),
        };
    }
    let tiers: [Tier<'_>; 4] = [
        &|| session.aliased(reference),
        &|| session.titled(reference),
        &|| linked(session, reference, &normalized),
        &|| {
            if reference.len() >= SHORTEST_PREFIX {
                session.spelled(reference)
            } else {
                Ok(Vec::new())
            }
        },
    ];
    for tier in tiers {
        let found = tier()?;
        match found.as_slice() {
            [(only, _)] => return Ok(Entry::Shortcut(*only)),
            [] => {}
            [_, ..] => {
                let names: Vec<String> = found
                    .iter()
                    .map(|(id, shortcut)| {
                        format!("  {}", named(folders, Entry::Shortcut(*id), Some(shortcut)))
                    })
                    .collect();
                bail!(
                    "`{reference}` names more than one shortcut:\n{}\nname one by its link or its identifier",
                    names.join("\n")
                );
            }
        }
    }
    match folder(folders, reference) {
        Ok(Some(id)) => Ok(Entry::Folder(id)),
        Ok(None) | Err(_) => bail!("nothing is named `{reference}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::{entry, folder};
    use youarel_core::state::{Edit, Entry, ItemId};
    use youarel_store::Store;

    const NOW: u64 = 1_700_000_000_000;

    const fn id(byte: u8) -> ItemId {
        ItemId([byte; 16])
    }

    /// A store with Work holding Infra holding GitHub, two folders named Twin, and two shortcuts titled Same.
    fn sample() -> (tempfile::TempDir, Store) {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_path_buf());
        let edits = [
            Edit::AddFolder {
                id: id(1),
                name: "Work".to_owned(),
                parent: None,
            },
            Edit::AddFolder {
                id: id(2),
                name: "Infra".to_owned(),
                parent: Some(id(1)),
            },
            Edit::AddFolder {
                id: id(5),
                name: "Twin".to_owned(),
                parent: None,
            },
            Edit::AddFolder {
                id: id(6),
                name: "Twin".to_owned(),
                parent: None,
            },
            Edit::AddShortcut {
                id: id(3),
                link: "https://github.com/".to_owned(),
                title: "GitHub".to_owned(),
                folder: Some(id(2)),
            },
            Edit::AddShortcut {
                id: id(4),
                link: "https://gitlab.com/".to_owned(),
                title: "Same".to_owned(),
                folder: None,
            },
            Edit::AddShortcut {
                id: id(7),
                link: "https://example.com/".to_owned(),
                title: "Same".to_owned(),
                folder: None,
            },
            Edit::Realias {
                id: id(3),
                alias: "gh".to_owned(),
            },
        ];
        store
            .session(NOW, |session| {
                edits.into_iter().try_for_each(|edit| session.edit(edit))
            })
            .unwrap();
        (home, store)
    }

    #[test]
    fn folder_paths_walk_from_the_top() {
        let (_home, store) = sample();
        store
            .session(NOW, |session| {
                let folders = session.folders();
                assert_eq!(folder(folders, "").unwrap(), None);
                assert_eq!(folder(folders, "/").unwrap(), None);
                assert_eq!(folder(folders, "Work/Infra").unwrap(), Some(id(2)));
                assert_eq!(folder(folders, "/Work/Infra/").unwrap(), Some(id(2)));
                let missing = folder(folders, "Work/Nope").unwrap_err().to_string();
                assert!(missing.contains("no folder `Nope`"), "{missing}");
                let twin = folder(folders, "Twin").unwrap_err().to_string();
                assert!(twin.contains("more than one folder"), "{twin}");
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn entries_resolve_by_alias_title_link_identifier_or_folder() {
        let (_home, store) = sample();
        store
            .session(NOW, |session| {
                let named = |reference: &str| entry(session, reference);
                let shortcut = Entry::Shortcut(id(3));
                assert_eq!(named("gh").unwrap(), shortcut);
                assert_eq!(named("GitHub").unwrap(), shortcut);
                assert_eq!(named("https://github.com/").unwrap(), shortcut);
                assert_eq!(named("HTTPS://GitHub.com").unwrap(), shortcut);
                assert_eq!(named("030303").unwrap(), shortcut);
                assert_eq!(named("Work/Infra/").unwrap(), Entry::Folder(id(2)));
                assert_eq!(named("Work").unwrap(), Entry::Folder(id(1)));
                let ambiguous = named("Same").unwrap_err().to_string();
                assert!(ambiguous.contains("more than one shortcut"), "{ambiguous}");
                assert!(ambiguous.contains("gitlab.com"), "{ambiguous}");
                let nothing = named("0303").unwrap_err().to_string();
                assert!(nothing.contains("nothing is named"), "{nothing}");
                named("/").unwrap_err();
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn a_name_passes_removed_shortcuts_and_finds_an_alias_without_a_word() {
        let (_home, store) = sample();
        store
            .session(NOW, |session| {
                let edits = [
                    Edit::Realias {
                        id: id(4),
                        alias: "-".to_owned(),
                    },
                    Edit::AddShortcut {
                        id: id(8),
                        link: "https://github.com/".to_owned(),
                        title: "Mirror".to_owned(),
                        folder: None,
                    },
                    Edit::AddShortcut {
                        id: id(9),
                        link: "https://gone.example/".to_owned(),
                        title: "Gone".to_owned(),
                        folder: None,
                    },
                    Edit::Realias {
                        id: id(9),
                        alias: "gone".to_owned(),
                    },
                    Edit::Remove {
                        entry: Entry::Shortcut(id(9)),
                    },
                ];
                edits.into_iter().try_for_each(|edit| session.edit(edit))
            })
            .unwrap();
        store
            .session(NOW, |session| {
                let named = |reference: &str| entry(session, reference);
                assert_eq!(
                    named("-").unwrap(),
                    Entry::Shortcut(id(4)),
                    "an alias of separators alone"
                );
                let twice = named("https://github.com/").unwrap_err().to_string();
                assert!(twice.contains("more than one shortcut"), "{twice}");
                assert!(twice.contains("Mirror"), "{twice}");
                let gone = named("gone").unwrap_err().to_string();
                assert!(gone.contains("nothing is named"), "{gone}");
                assert_eq!(
                    named("0303030").unwrap(),
                    Entry::Shortcut(id(3)),
                    "an odd start of an identifier"
                );
                Ok(())
            })
            .unwrap();
    }
}
