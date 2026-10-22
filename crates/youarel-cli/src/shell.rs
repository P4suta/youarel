//! The launcher's shell around the core's interface machine, which the terminal launcher drives.
//!
//! The launcher reads its own input, asks [`step`](youarel_core::ui_rules::step) what it means, and hands the effect to [`carry_out`], which acts through the store and the browser.
//! The texts here are what the launcher shows for the interface; it draws them and decides nothing the machine decides.

use crate::resolve::named;
use crate::web::Page;
use crate::{clock, icons, web};
use anyhow::Result;
use std::collections::BTreeMap;
use youarel_core::state::{Edit, Entry, ItemId, Slot};
use youarel_core::ui_rules::{Effect, Field, Mode, Row, Ui};
use youarel_link::Link;
use youarel_store::{Session, Store, StoreError};

/// The key hints a launcher shows while browsing.
pub const HINTS: &str = "Enter open · → ← folders · ^E rename · ^L alias · ^D remove · ^O new folder · ^X ^V move · Shift ↑↓ reorder · Esc quit";
/// The notice while the page of a link to add is read.
pub const READING_TITLE: &str = "reading the page's title and icon…";
/// The most entries a launcher shows: the first of a folder's entries, or the best of a search's matches.
pub const SHOWN: usize = 200;

/// What a launcher shows: one page of rows for the interface with the texts of each, the path of the folder browsed, and the names of the entries the header and the prompt name.
#[derive(Debug)]
pub struct Screen {
    pub rows: Vec<Row>,
    /// The texts of each row, in the rows' order.
    pub texts: Vec<RowText>,
    /// The names of the folders from the top down to the one browsed, joined by slashes.
    pub path: String,
    /// The text that names the carried entry and the one waiting for confirmation.
    pub names: BTreeMap<Entry, String>,
}

/// What a launcher does after an effect.
#[derive(Debug, Eq, PartialEq)]
pub enum Outcome {
    /// It stays, with a notice for the person or none.
    Continue(Option<String>),
    /// It leaves, and the terminal launcher ends.
    Leave,
}

/// The rows for the interface: one page of a folder's entries or of a search's matches.
///
/// While the query is empty, the first [`SHOWN`] entries of the folder follow a way up, and otherwise the best [`SHOWN`] matches.
/// An offer to add a typed link the store does not hold comes first.
///
/// # Errors
///
/// Returns a [`StoreError`] when the store cannot be read.
pub fn rows(session: &Session, ui: &Ui) -> Result<Vec<Row>, StoreError> {
    let mut rows = Vec::new();
    if ui.query.is_empty() {
        let folders = session.folders();
        let folder = ui.folder.filter(|folder| folders.shows(*folder));
        if let Some(folder) = folder {
            rows.push(Row::Parent(folders.tree().parent(folder)));
        }
        rows.extend(
            session
                .entries_page(folder, 0, SHOWN)?
                .into_iter()
                .map(Row::Entry),
        );
    } else {
        rows.extend(
            session
                .find(&ui.query)?
                .into_iter()
                .take(SHOWN)
                .map(Row::Entry),
        );
    }
    if let Ok(link) = Link::parse(ui.query.trim())
        && !session.holds_link(link.as_str())?
    {
        rows.insert(0, Row::Add(link.as_str().to_owned()));
    }
    Ok(rows)
}

/// The texts of one row as the store holds its entry.
fn text_of(session: &Session, row: &Row) -> Result<RowText, StoreError> {
    let text = |kind, name: &str, detail: &str, alias: &str| RowText {
        kind,
        name: name.to_owned(),
        detail: detail.to_owned(),
        alias: alias.to_owned(),
    };
    Ok(match row {
        Row::Parent(_) => text(Kind::Parent, "..", "", ""),
        Row::Add(link) => text(Kind::Add, "add", link, ""),
        Row::Entry(Entry::Folder(id)) => text(
            Kind::Folder,
            session
                .folders()
                .folder(*id)
                .map_or("", |folder| folder.name.value.as_str()),
            "",
            "",
        ),
        Row::Entry(Entry::Shortcut(id)) => session.item(*id)?.map_or_else(
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
    })
}

/// The texts that name the carried entry and the entry waiting for confirmation.
fn names(session: &Session, ui: &Ui) -> Result<BTreeMap<Entry, String>, StoreError> {
    let confirmed = match &ui.mode {
        Mode::Confirm { target } => Some(*target),
        Mode::Browse | Mode::Typing { .. } => None,
    };
    let mut found = BTreeMap::new();
    for entry in ui.carried.into_iter().chain(confirmed) {
        let shortcut = match entry {
            Entry::Shortcut(id) => session.item(id)?,
            Entry::Folder(_) => None,
        };
        found.insert(entry, named(session.folders(), entry, shortcut.as_ref()));
    }
    Ok(found)
}

/// The rows the interface shows now, with their texts and the names the header and the prompt show.
///
/// # Errors
///
/// Returns an error when the clock or the store cannot be read.
pub fn load(store: &Store, ui: &Ui) -> Result<Screen> {
    let now = clock::now()?;
    store
        .session(now, |session| {
            let rows = rows(session, ui)?;
            let texts = rows
                .iter()
                .map(|row| text_of(session, row))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Screen {
                texts,
                path: session.folders().path(ui.folder).join("/"),
                names: names(session, ui)?,
                rows,
            })
        })
        .map_err(anyhow::Error::from)
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

/// Moves an entry beside a neighbor in the folder it sits in, which the store tells.
fn reorder(store: &Store, entry: Entry, slot: Slot) -> Outcome {
    let result = clock::now().and_then(|now| {
        store
            .session(now, |session| {
                let folder = match entry {
                    Entry::Folder(id) => session.folders().tree().parent(id),
                    Entry::Shortcut(id) => session
                        .item(id)?
                        .and_then(|shortcut| session.folders().home(&shortcut)),
                };
                session.edit(Edit::Move {
                    entry,
                    folder,
                    slot,
                })
            })
            .map_err(anyhow::Error::from)
    });
    Outcome::Continue(Some(match result {
        Ok(()) => "moved".to_owned(),
        Err(error) => format!("{error:#}"),
    }))
}

/// Carries out one effect of the machine through the store and the browser.
#[must_use]
pub fn carry_out(store: &Store, ui: &Ui, effect: Effect) -> Outcome {
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
            let slot = if before {
                Slot::Before(neighbor)
            } else {
                Slot::After(neighbor)
            };
            reorder(store, entry, slot)
        }
    }
}

fn fresh_id(store: &Store) -> Result<ItemId> {
    let now = clock::now()?;
    store
        .session(now, |session| session.fresh_id())
        .map_err(anyhow::Error::from)
}

/// Counts the visit and opens the shortcut, gathers its site's favicon when this device keeps no icon for its host, and leaves the launcher.
fn open(store: &Store, id: ItemId) -> Outcome {
    let link = clock::now().and_then(|now| {
        store
            .session(now, |session| {
                session.visit(id)?;
                Ok(session.item(id)?.map(|shortcut| shortcut.link.value))
            })
            .map_err(anyhow::Error::from)
    });
    match link {
        Ok(Some(link)) => match web::open(&link) {
            Ok(()) => {
                // The browser reaches the site now anyway, and an icon that cannot be kept waits for the next opening, since the launcher leaves.
                drop(
                    Link::parse(&link)
                        .map_err(anyhow::Error::from)
                        .and_then(|link| icons::opened(store, clock::now()?, &link)),
                );
                Outcome::Leave
            }
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
                    icons::keep(session, &link, icon)?;
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

/// The text that names an entry the screen names, or none for one it does not.
fn describe(screen: &Screen, entry: Entry) -> &str {
    screen.names.get(&entry).map_or("", String::as_str)
}

/// The prompt: what typing fills now, and the text typed so far.
#[must_use]
pub fn prompt(ui: &Ui, screen: &Screen) -> String {
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
        Mode::Confirm { target } => format!("remove {}? y / n", describe(screen, *target)),
    }
}

/// The header: the browsed folder's path, and the entry the person carries.
#[must_use]
pub fn header(ui: &Ui, screen: &Screen) -> String {
    let mut title = format!("youarel  /{}", screen.path);
    if let Some(carried) = ui.carried {
        title.push_str("  · carrying ");
        title.push_str(describe(screen, carried));
    }
    title
}

#[cfg(test)]
mod tests {
    use super::{Kind, Outcome, SHOWN, bare, carry_out, header, load, prompt};
    use youarel_core::state::{Edit, Entry, ItemId};
    use youarel_core::ui_rules::{Effect, Field, Mode, Row, Ui};
    use youarel_store::Store;

    /// A time for the sessions the tests run.
    const NOW: u64 = 1_700_000_000_000;

    #[test]
    fn a_launcher_shows_and_edits_the_store_through_the_shell() {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_path_buf());
        let ui = Ui::default();
        let empty = load(&store, &ui).unwrap();
        assert_eq!((empty.rows.len(), empty.texts.len()), (0, 0));
        let created = carry_out(
            &store,
            &ui,
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
        let shown = &screen.texts[0];
        assert_eq!((shown.kind, shown.name.as_str()), (Kind::Folder, "Work"));
        let carrying = Ui {
            carried: Some(Entry::Folder(folder)),
            ..Ui::default()
        };
        let carried = load(&store, &carrying).unwrap();
        assert!(header(&carrying, &carried).contains("carrying Work/"));
        let naming = Ui {
            mode: Mode::Typing {
                target: None,
                field: Field::Name,
                text: "Ho".to_owned(),
            },
            ..Ui::default()
        };
        assert_eq!(prompt(&naming, &screen), "new folder: Ho");
        let confirming = Ui {
            mode: Mode::Confirm {
                target: Entry::Folder(folder),
            },
            ..Ui::default()
        };
        let asked = load(&store, &confirming).unwrap();
        assert_eq!(prompt(&confirming, &asked), "remove Work/? y / n");
        assert_eq!(carry_out(&store, &ui, Effect::Quit), Outcome::Leave);
        let inside = Ui {
            folder: Some(folder),
            ..Ui::default()
        };
        let opened = load(&store, &inside).unwrap();
        assert_eq!(opened.rows, [Row::Parent(None)]);
        assert_eq!(header(&inside, &opened), "youarel  /Work");
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

    #[test]
    fn a_launcher_shows_one_page_and_reorders_in_the_folder_the_store_names() {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_path_buf());
        let ids: Vec<ItemId> = store
            .session(NOW, |session| {
                let mut ids = Vec::new();
                for place in 0..=SHOWN {
                    let id = session.fresh_id()?;
                    session.edit(Edit::AddShortcut {
                        id,
                        link: format!("https://example.com/{place}"),
                        title: format!("Page {place}"),
                        folder: None,
                    })?;
                    ids.push(id);
                }
                Ok(ids)
            })
            .unwrap();
        let screen = load(&store, &Ui::default()).unwrap();
        assert_eq!((screen.rows.len(), screen.texts.len()), (SHOWN, SHOWN));
        assert_eq!(screen.texts[1].detail, "example.com/1");
        let searched = Ui {
            query: "page".to_owned(),
            ..Ui::default()
        };
        assert_eq!(load(&store, &searched).unwrap().rows.len(), SHOWN);
        let moved = carry_out(
            &store,
            &Ui::default(),
            Effect::Reorder {
                entry: Entry::Shortcut(ids[1]),
                neighbor: Entry::Shortcut(ids[0]),
                before: true,
            },
        );
        assert_eq!(moved, Outcome::Continue(Some("moved".to_owned())));
        let reordered = load(&store, &Ui::default()).unwrap();
        assert_eq!(
            reordered.rows[..2],
            [
                Row::Entry(Entry::Shortcut(ids[1])),
                Row::Entry(Entry::Shortcut(ids[0]))
            ]
        );
    }
}
