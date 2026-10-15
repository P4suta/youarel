//! The launcher's shell around the core's interface machine, which the terminal launcher and the window launcher both drive.
//!
//! A launcher reads its own input, asks [`step`](youarel_core::ui_rules::step) what it means, and hands the effect to [`carry_out`], which acts through the store and the browser.
//! The texts here are what either launcher shows for the interface; each draws them its own way and decides nothing the machine decides.

use crate::web::Page;
use crate::{clock, icons, web};
use anyhow::Result;
use std::collections::BTreeMap;
use youarel_core::rank_rules::Visits;
use youarel_core::state::{Edit, Entry, IconId, ItemId, Slot, State};
use youarel_core::ui_rules::{Effect, Field, Mode, Row, Ui};
use youarel_core::view;
use youarel_link::Link;
use youarel_store::Store;

/// The key hints a launcher shows while browsing.
pub const HINTS: &str = "Enter open · → ← folders · ^E rename · ^L alias · ^D remove · ^O new folder · ^X ^V move · Shift ↑↓ reorder · Esc quit";
/// The notice while the page of a link to add is read.
pub const READING_TITLE: &str = "reading the page's title and icon…";

/// What a launcher shows: the state as read, and the rows for the interface.
#[derive(Debug)]
pub struct Screen {
    pub state: State,
    pub rows: Vec<Row>,
}

/// What a launcher does after an effect.
#[derive(Debug, Eq, PartialEq)]
pub enum Outcome {
    /// It stays, with a notice for the person or none.
    Continue(Option<String>),
    /// It leaves: the terminal launcher ends, and the window launcher hides.
    Leave,
}

/// The rows for the interface: the core's rows, after an offer to add a typed link the store does not hold.
#[must_use]
pub fn rows(state: &State, ui: &Ui, visits: &BTreeMap<ItemId, Visits>, now: u64) -> Vec<Row> {
    let mut rows = view::rows(state, ui, visits, now);
    if let Ok(link) = Link::parse(ui.query.trim()) {
        let held = state
            .shortcuts
            .values()
            .any(|shortcut| !shortcut.removed.value && shortcut.link.value == link.as_str());
        if !held {
            rows.insert(0, Row::Add(link.as_str().to_owned()));
        }
    }
    rows
}

/// The state and the rows the interface shows now.
///
/// # Errors
///
/// Returns an error when the clock or the store cannot be read.
pub fn load(store: &Store, ui: &Ui) -> Result<Screen> {
    let now = clock::now()?;
    let (state, visits) = store
        .session(now, |session| {
            Ok((session.state().clone(), session.visits().clone()))
        })
        .map_err(anyhow::Error::from)?;
    let rows = rows(&state, ui, &visits, now);
    Ok(Screen { state, rows })
}

/// Runs one edit in a session now, and the notice it leaves.
fn edit(store: &Store, edit: Edit, done: &str) -> Outcome {
    let result = clock::now().and_then(|now| {
        store
            .session(now, |session| session.edit(edit))
            .map_err(anyhow::Error::from)
    });
    Outcome::Continue(Some(match result {
        Ok(()) => done.to_owned(),
        Err(error) => format!("{error:#}"),
    }))
}

/// Carries out one effect of the machine through the store and the browser.
#[must_use]
pub fn carry_out(store: &Store, ui: &Ui, screen: &Screen, effect: Effect) -> Outcome {
    match effect {
        Effect::None => Outcome::Continue(None),
        Effect::Quit => Outcome::Leave,
        Effect::Open(id) => open(store, id),
        Effect::Add(link) => add(store, ui.folder, &link),
        Effect::Rename { target, text } => edit(
            store,
            match target {
                Entry::Folder(id) => Edit::RenameFolder { id, name: text },
                Entry::Shortcut(id) => Edit::Retitle { id, title: text },
            },
            "renamed",
        ),
        Effect::Realias { id, alias } => edit(store, Edit::Realias { id, alias }, "alias set"),
        Effect::CreateFolder { name, parent } => match fresh_id(store) {
            Ok(id) => edit(
                store,
                Edit::AddFolder { id, name, parent },
                "folder created",
            ),
            Err(error) => Outcome::Continue(Some(format!("{error:#}"))),
        },
        Effect::Remove(entry) => edit(store, Edit::Remove { entry }, "removed"),
        Effect::Move { entry, folder } => edit(
            store,
            Edit::Move {
                entry,
                folder,
                slot: Slot::End,
            },
            "moved",
        ),
        Effect::Reorder {
            entry,
            neighbor,
            before,
        } => {
            let tree = screen.state.tree();
            let folder = match entry {
                Entry::Folder(id) => tree.parent(id),
                Entry::Shortcut(id) => screen.state.folder_of(&tree, id),
            };
            let slot = if before {
                Slot::Before(neighbor)
            } else {
                Slot::After(neighbor)
            };
            edit(
                store,
                Edit::Move {
                    entry,
                    folder,
                    slot,
                },
                "moved",
            )
        }
    }
}

fn fresh_id(store: &Store) -> Result<ItemId> {
    let now = clock::now()?;
    store
        .session(now, |session| session.fresh_id())
        .map_err(anyhow::Error::from)
}

/// Counts the visit and opens the shortcut, leaving the launcher once the browser has it.
fn open(store: &Store, id: ItemId) -> Outcome {
    let link = clock::now().and_then(|now| {
        store
            .session(now, |session| {
                session.visit(id);
                Ok(session
                    .state()
                    .shortcut(id)
                    .map(|shortcut| shortcut.link.value.clone()))
            })
            .map_err(anyhow::Error::from)
    });
    match link {
        Ok(Some(link)) => match web::open(&link) {
            Ok(()) => Outcome::Leave,
            Err(error) => Outcome::Continue(Some(format!("{error:#}"))),
        },
        Ok(None) => Outcome::Continue(Some("the shortcut is gone".to_owned())),
        Err(error) => Outcome::Continue(Some(format!("{error:#}"))),
    }
}

/// Adds a link to the browsed folder, titled by its page when the page names one.
fn add(store: &Store, folder: Option<ItemId>, text: &str) -> Outcome {
    let link = match Link::parse(text) {
        Ok(link) => link,
        Err(error) => return Outcome::Continue(Some(format!("cannot add {text}: {error:#}"))),
    };
    let page = web::fetch_page(&link);
    let (title, warning) = match &page {
        Ok(Page {
            title: Some(title), ..
        }) => (title.clone(), None),
        Ok(Page { title: None, .. }) => (link.as_str().to_owned(), None),
        Err(error) => (
            link.as_str().to_owned(),
            Some(format!(" (no title: {error:#})")),
        ),
    };
    // A shortcut without an icon still opens, so the notice only mentions a missing icon.
    let (icon, missing) = match page.as_ref().map(|page| icons::fetch(&link, page)) {
        Ok(Ok(icon)) => (Some(icon), ""),
        Ok(Err(_)) | Err(_) => (None, " (no icon)"),
    };
    let added = clock::now().and_then(|now| {
        store
            .session(now, |session| {
                let id = session.fresh_id()?;
                session.edit(Edit::AddShortcut {
                    id,
                    link: link.as_str().to_owned(),
                    title: title.clone(),
                    folder,
                })?;
                if let Some(icon) = icon {
                    session.edit(Edit::SetIcon {
                        id,
                        icon: Some(icon),
                    })?;
                }
                Ok(())
            })
            .map_err(anyhow::Error::from)
    });
    Outcome::Continue(Some(match added {
        Ok(()) => format!("added {title}{}{missing}", warning.unwrap_or_default()),
        Err(error) => format!("{error:#}"),
    }))
}

/// The link as the address bar shows it.
#[must_use]
pub fn bare(link: &str) -> &str {
    link.strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))
        .unwrap_or(link)
}

/// What kind of row a launcher draws.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Parent,
    Add,
    Folder,
    Shortcut,
}

/// The texts of one row: its name, the link or the hint beside it, and its alias.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RowText {
    pub kind: Kind,
    pub name: String,
    pub detail: String,
    pub alias: String,
}

/// The texts of one row.
#[must_use]
pub fn row_text(state: &State, row: &Row) -> RowText {
    let text = |kind, name: &str, detail: &str, alias: &str| RowText {
        kind,
        name: name.to_owned(),
        detail: detail.to_owned(),
        alias: alias.to_owned(),
    };
    match row {
        Row::Parent(_) => text(Kind::Parent, "..", "", ""),
        Row::Add(link) => text(Kind::Add, "add", link, ""),
        Row::Entry(Entry::Folder(id)) => text(
            Kind::Folder,
            state
                .folder(*id)
                .map_or("", |folder| folder.name.value.as_str()),
            "",
            "",
        ),
        Row::Entry(Entry::Shortcut(id)) => state.shortcut(*id).map_or_else(
            || text(Kind::Shortcut, "", "", ""),
            |shortcut| {
                text(
                    Kind::Shortcut,
                    &shortcut.title.value,
                    bare(&shortcut.link.value),
                    &shortcut.alias.value,
                )
            },
        ),
    }
}

/// The icon of a row and its image, for a launcher that draws pictures.
#[must_use]
pub fn row_icon<'a>(state: &'a State, row: &Row) -> Option<(IconId, &'a [u8])> {
    match row {
        Row::Entry(Entry::Shortcut(id)) => {
            let icon = state.shortcut(*id)?.icon.value?;
            state.icon(*id).map(|image| (icon, image))
        }
        Row::Entry(Entry::Folder(_)) | Row::Parent(_) | Row::Add(_) => None,
    }
}

/// The prompt: what typing fills now, and the text typed so far.
#[must_use]
pub fn prompt(ui: &Ui, state: &State) -> String {
    match &ui.mode {
        Mode::Browse => format!("> {}", ui.query),
        Mode::Typing {
            target,
            field,
            text,
        } => {
            let label = match (field, target) {
                (Field::Title, _) => "new title",
                (Field::Alias, _) => "alias (empty removes it)",
                (Field::Name, Some(_)) => "new name",
                (Field::Name, None) => "new folder",
            };
            format!("{label}: {text}")
        }
        Mode::Confirm { target } => {
            let tree = state.tree();
            let named = crate::resolve::describe(state, &tree, *target);
            format!("remove {named}? y / n")
        }
    }
}

/// The header: the browsed folder's path, and the entry the person carries.
#[must_use]
pub fn header(ui: &Ui, state: &State) -> String {
    let tree = state.tree();
    let mut title = format!("youarel  /{}", state.path(&tree, ui.folder).join("/"));
    if let Some(carried) = ui.carried {
        title.push_str("  · carrying ");
        title.push_str(&crate::resolve::describe(state, &tree, carried));
    }
    title
}

#[cfg(test)]
mod tests {
    use super::{Kind, Outcome, bare, carry_out, header, load, prompt, row_text};
    use youarel_core::state::Entry;
    use youarel_core::ui_rules::{Effect, Field, Mode, Row, Ui};
    use youarel_store::Store;

    #[test]
    fn a_launcher_shows_and_edits_the_store_through_the_shell() {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_path_buf());
        let ui = Ui::default();
        let empty = load(&store, &ui).unwrap();
        assert_eq!(empty.rows, []);
        let created = carry_out(
            &store,
            &ui,
            &empty,
            Effect::CreateFolder {
                name: "Work".to_owned(),
                parent: None,
            },
        );
        assert_eq!(
            created,
            Outcome::Continue(Some("folder created".to_owned()))
        );
        let screen = load(&store, &ui).unwrap();
        assert_eq!(screen.rows.len(), 1);
        let folder = screen
            .rows
            .iter()
            .find_map(|row| match row {
                Row::Entry(Entry::Folder(id)) => Some(*id),
                Row::Entry(Entry::Shortcut(_)) | Row::Parent(_) | Row::Add(_) => None,
            })
            .unwrap();
        let shown = row_text(&screen.state, &Row::Entry(Entry::Folder(folder)));
        assert_eq!((shown.kind, shown.name.as_str()), (Kind::Folder, "Work"));
        let carrying = Ui {
            carried: Some(Entry::Folder(folder)),
            ..Ui::default()
        };
        assert!(header(&carrying, &screen.state).contains("carrying"));
        let naming = Ui {
            mode: Mode::Typing {
                target: None,
                field: Field::Name,
                text: "Ho".to_owned(),
            },
            ..Ui::default()
        };
        assert_eq!(prompt(&naming, &screen.state), "new folder: Ho");
        assert_eq!(
            carry_out(&store, &ui, &screen, Effect::Quit),
            Outcome::Leave
        );
        let typed = Ui {
            query: "https://example.com/".to_owned(),
            ..Ui::default()
        };
        let offered = load(&store, &typed).unwrap();
        assert_eq!(
            offered.rows.first(),
            Some(&Row::Add("https://example.com/".to_owned()))
        );
        assert_eq!(bare("https://example.com/"), "example.com/");
        assert_eq!(bare("http://example.com/"), "example.com/");
    }
}
