//! What a person types on the command line to name a folder or an entry.
//!
//! A folder is a path of names joined by slashes from the top, where an empty path or a lone slash is the top.
//! An entry is a shortcut's alias, title, or link, a prefix of its identifier, or a folder path that ends with a slash.

use anyhow::{Result, bail};
use youarel_core::state::{Entry, ItemId, Shortcut, State, Tree};
use youarel_link::Link;
use youarel_store::codec::hex;

/// The shortest identifier prefix that names an entry, which keeps a short word from matching by accident.
pub const SHORTEST_PREFIX: usize = 6;

/// The folder a path names, or `None` for the top.
///
/// # Errors
///
/// Returns an error naming the segment that no folder or more than one folder answers.
pub fn folder(state: &State, tree: &Tree, path: &str) -> Result<Option<ItemId>> {
    let mut current: Option<ItemId> = None;
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        let children: Vec<ItemId> = state
            .entries(tree, current)
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Folder(id) => state
                    .folder(id)
                    .filter(|folder| folder.name.value == segment)
                    .map(|_| id),
                Entry::Shortcut(_) => None,
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

/// The text that names an entry in a message.
#[must_use]
pub fn describe(state: &State, tree: &Tree, entry: Entry) -> String {
    match entry {
        Entry::Folder(id) => {
            let mut path = state.path(tree, Some(id)).join("/");
            path.push('/');
            path
        }
        Entry::Shortcut(id) => state.shortcut(id).map_or_else(
            || hex(&id.0),
            |shortcut| format!("{} <{}>", shortcut.title.value, shortcut.link.value),
        ),
    }
}

/// One kind of name a shortcut answers to, which the resolution tries in order.
type Tier<'a> = &'a dyn Fn(ItemId, &Shortcut) -> bool;

/// The entry a reference names.
///
/// # Errors
///
/// Returns an error when nothing matches, or when several entries match the first kind of name that matches at all.
pub fn entry(state: &State, tree: &Tree, reference: &str) -> Result<Entry> {
    let normalized = Link::parse(reference).map(|link| link.as_str().to_owned());
    if reference.ends_with('/') && normalized.is_err() {
        return match folder(state, tree, reference)? {
            Some(id) => Ok(Entry::Folder(id)),
            None => bail!("the top is not an entry; name a folder or a shortcut"),
        };
    }
    let live: Vec<(ItemId, &Shortcut)> = state
        .shortcuts
        .keys()
        .filter_map(|id| state.shortcut(*id).map(|shortcut| (*id, shortcut)))
        .collect();
    let tiers: [Tier<'_>; 4] = [
        &|_, shortcut| !shortcut.alias.value.is_empty() && shortcut.alias.value == reference,
        &|_, shortcut| shortcut.title.value == reference,
        &|_, shortcut| {
            shortcut.link.value == reference
                || normalized
                    .as_ref()
                    .is_ok_and(|link| *link == shortcut.link.value)
        },
        &|id, _| reference.len() >= SHORTEST_PREFIX && hex(&id.0).starts_with(reference),
    ];
    for tier in tiers {
        let found: Vec<Entry> = live
            .iter()
            .filter(|(id, shortcut)| tier(*id, shortcut))
            .map(|(id, _)| Entry::Shortcut(*id))
            .collect();
        match found.as_slice() {
            [only] => return Ok(*only),
            [] => {}
            [_, ..] => {
                let names: Vec<String> = found
                    .iter()
                    .map(|entry| format!("  {}", describe(state, tree, *entry)))
                    .collect();
                bail!(
                    "`{reference}` names more than one shortcut:\n{}\nname one by its link or its identifier",
                    names.join("\n")
                );
            }
        }
    }
    match folder(state, tree, reference) {
        Ok(Some(id)) => Ok(Entry::Folder(id)),
        Ok(None) | Err(_) => bail!("nothing is named `{reference}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::{entry, folder};
    use youarel_core::clock_rules::{DeviceId, Stamp};
    use youarel_core::state::{Edit, Entry, ItemId, State};

    const fn id(byte: u8) -> ItemId {
        ItemId([byte; 16])
    }

    fn sample() -> State {
        let mut state = State::default();
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
        for (wall, edit) in (1..).zip(edits) {
            let stamp = Stamp {
                wall,
                counter: 0,
                device: DeviceId([9; 16]),
            };
            state.apply(edit, stamp).unwrap();
        }
        state
    }

    #[test]
    fn folder_paths_walk_from_the_top() {
        let state = sample();
        let tree = state.tree();
        assert_eq!(folder(&state, &tree, "").unwrap(), None);
        assert_eq!(folder(&state, &tree, "/").unwrap(), None);
        assert_eq!(folder(&state, &tree, "Work/Infra").unwrap(), Some(id(2)));
        assert_eq!(folder(&state, &tree, "/Work/Infra/").unwrap(), Some(id(2)));
        let missing = folder(&state, &tree, "Work/Nope").unwrap_err().to_string();
        assert!(missing.contains("no folder `Nope`"), "{missing}");
        let twin = folder(&state, &tree, "Twin").unwrap_err().to_string();
        assert!(twin.contains("more than one folder"), "{twin}");
    }

    #[test]
    fn entries_resolve_by_alias_title_link_identifier_or_folder() {
        let state = sample();
        let tree = state.tree();
        let shortcut = Entry::Shortcut(id(3));
        assert_eq!(entry(&state, &tree, "gh").unwrap(), shortcut);
        assert_eq!(entry(&state, &tree, "GitHub").unwrap(), shortcut);
        assert_eq!(
            entry(&state, &tree, "https://github.com/").unwrap(),
            shortcut
        );
        assert_eq!(
            entry(&state, &tree, "HTTPS://GitHub.com").unwrap(),
            shortcut
        );
        assert_eq!(entry(&state, &tree, "030303").unwrap(), shortcut);
        assert_eq!(
            entry(&state, &tree, "Work/Infra/").unwrap(),
            Entry::Folder(id(2))
        );
        assert_eq!(entry(&state, &tree, "Work").unwrap(), Entry::Folder(id(1)));
        let ambiguous = entry(&state, &tree, "Same").unwrap_err().to_string();
        assert!(ambiguous.contains("more than one shortcut"), "{ambiguous}");
        assert!(ambiguous.contains("gitlab.com"), "{ambiguous}");
        let nothing = entry(&state, &tree, "0303").unwrap_err().to_string();
        assert!(nothing.contains("nothing is named"), "{nothing}");
        entry(&state, &tree, "/").unwrap_err();
    }
}
