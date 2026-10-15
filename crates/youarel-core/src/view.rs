//! The rows the interface shows: a folder's entries while the search is empty, and the ranked matches otherwise.

use crate::match_rules::score;
use crate::rank_rules::{Rank, Visits, frecency};
use crate::state::{Entry, ItemId, State, Tree};
use crate::ui_rules::{Row, Ui};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// The bonus of a shortcut whose alias is the whole query, which puts it first.
pub const ALIAS_EXACT: u32 = 10_000;

/// The link without its scheme, which a person types the way the address bar shows it.
fn bare(link: &str) -> &str {
    link.strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))
        .unwrap_or(link)
}

/// The folder path of an entry as text, joined by slashes.
fn joined(names: &[&str]) -> String {
    let mut text = String::new();
    for name in names {
        if !text.is_empty() {
            text.push('/');
        }
        text.push_str(name);
    }
    text
}

/// The best score of `query` over a list of texts, if any matches.
fn best<'a>(query: &str, texts: impl IntoIterator<Item = &'a str>) -> Option<u32> {
    texts
        .into_iter()
        .filter_map(|text| score(query, text))
        .max()
}

/// The search score of a live shortcut.
fn shortcut_score(state: &State, tree: &Tree, query: &str, id: ItemId) -> Option<u32> {
    let shortcut = state.shortcut(id)?;
    let path = joined(&state.path(tree, state.folder_of(tree, id)));
    let located = alloc::format!("{path}/{}", shortcut.title.value);
    let found = best(
        query,
        [
            shortcut.alias.value.as_str(),
            shortcut.title.value.as_str(),
            bare(&shortcut.link.value),
            located.as_str(),
        ],
    )?;
    let alias = &shortcut.alias.value;
    let exact = !alias.is_empty() && alias.eq_ignore_ascii_case(query);
    Some(if exact {
        found.saturating_add(ALIAS_EXACT)
    } else {
        found
    })
}

/// The search score of a live folder.
fn folder_score(state: &State, tree: &Tree, query: &str, id: ItemId) -> Option<u32> {
    let folder = state.folder(id)?;
    let path = joined(&state.path(tree, Some(id)));
    best(query, [folder.name.value.as_str(), path.as_str()])
}

/// The rows of the interface for the state.
#[must_use]
pub fn rows(state: &State, ui: &Ui, visits: &BTreeMap<ItemId, Visits>, now: u64) -> Vec<Row> {
    let tree = state.tree();
    if ui.query.is_empty() {
        let folder = ui.folder.filter(|folder| tree.holds(*folder));
        let mut rows = Vec::new();
        if let Some(folder) = folder {
            rows.push(Row::Parent(tree.parent(folder)));
        }
        rows.extend(state.entries(&tree, folder).into_iter().map(Row::Entry));
        return rows;
    }
    let mut ranked: Vec<Rank<Entry>> = Vec::new();
    for id in state.shortcuts.keys() {
        if let Some(found) = shortcut_score(state, &tree, &ui.query, *id) {
            let visited = visits.get(id).copied().unwrap_or_default();
            ranked.push(Rank::new(
                found,
                frecency(visited, now),
                Entry::Shortcut(*id),
            ));
        }
    }
    for id in state.folders.keys() {
        if let Some(found) = folder_score(state, &tree, &ui.query, *id) {
            ranked.push(Rank::new(found, 0, Entry::Folder(*id)));
        }
    }
    ranked.sort();
    ranked
        .into_iter()
        .map(|rank| Row::Entry(rank.into_id()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ALIAS_EXACT, rows};
    use crate::clock_rules::{DeviceId, Stamp};
    use crate::rank_rules::Visits;
    use crate::state::{Edit, Entry, ItemId, State};
    use crate::ui_rules::{Row, Ui};
    use alloc::collections::BTreeMap;
    use alloc::string::String;

    const fn id(byte: u8) -> ItemId {
        ItemId([byte; 16])
    }

    fn sample() -> State {
        let mut state = State::default();
        let stamp = |wall| Stamp {
            wall,
            counter: 0,
            device: DeviceId([9; 16]),
        };
        let edits = [
            Edit::AddFolder {
                id: id(1),
                name: String::from("Work"),
                parent: None,
            },
            Edit::AddShortcut {
                id: id(2),
                link: String::from("https://github.com/"),
                title: String::from("GitHub"),
                folder: Some(id(1)),
            },
            Edit::AddShortcut {
                id: id(3),
                link: String::from("https://docs.rs/"),
                title: String::from("Docs"),
                folder: None,
            },
            Edit::Realias {
                id: id(3),
                alias: String::from("gh"),
            },
        ];
        for (wall, edit) in (1..).zip(edits) {
            state.apply(edit, stamp(wall)).unwrap();
        }
        state
    }

    fn search(query: &str) -> Ui {
        Ui {
            query: String::from(query),
            ..Ui::default()
        }
    }

    #[test]
    fn an_empty_search_lists_the_folder_with_a_way_up() {
        let state = sample();
        let visits = BTreeMap::new();
        assert_eq!(
            rows(&state, &Ui::default(), &visits, 0),
            [
                Row::Entry(Entry::Folder(id(1))),
                Row::Entry(Entry::Shortcut(id(3)))
            ]
        );
        let inside = Ui {
            folder: Some(id(1)),
            ..Ui::default()
        };
        assert_eq!(
            rows(&state, &inside, &visits, 0),
            [Row::Parent(None), Row::Entry(Entry::Shortcut(id(2)))]
        );
        let gone = Ui {
            folder: Some(id(7)),
            ..Ui::default()
        };
        assert_eq!(rows(&state, &gone, &visits, 0).len(), 2);
    }

    #[test]
    fn an_exact_alias_ranks_first_and_visits_break_ties() {
        let state = sample();
        let mut visits = BTreeMap::new();
        let found = rows(&state, &search("gh"), &visits, 0);
        assert_eq!(
            found,
            [
                Row::Entry(Entry::Shortcut(id(3))),
                Row::Entry(Entry::Shortcut(id(2)))
            ]
        );
        const { assert!(ALIAS_EXACT > crate::match_rules::MATCHED * 100) };
        assert_eq!(
            rows(&state, &search("work"), &visits, 0),
            [
                Row::Entry(Entry::Folder(id(1))),
                Row::Entry(Entry::Shortcut(id(2)))
            ]
        );
        assert_eq!(
            rows(&state, &search("docs.rs"), &visits, 0),
            [Row::Entry(Entry::Shortcut(id(3)))]
        );
        assert_eq!(rows(&state, &search("zzz"), &visits, 0), []);
        let tie = rows(&state, &search("o"), &visits, 0);
        assert_eq!(tie.first(), Some(&Row::Entry(Entry::Folder(id(1)))));
        visits.insert(id(3), Visits { count: 5, last: 0 });
        let visited = rows(&state, &search("o"), &visits, 0);
        assert_eq!(tie.len(), visited.len());
        assert_eq!(visited.first(), Some(&Row::Entry(Entry::Shortcut(id(3)))));
    }
}
