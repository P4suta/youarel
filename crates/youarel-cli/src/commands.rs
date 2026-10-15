//! The commands of the `youarel` binary, each one locked session of the store.

use crate::resolve::{self, describe};
use crate::web;
use anyhow::{Context, Result, anyhow};
use std::io::Write as _;
use youarel_core::state::{Edit, Entry, ItemId, Slot, State, Tree};
use youarel_core::ui_rules::{Row, Ui};
use youarel_core::view;
use youarel_html::bookmarks::{self, Item};
use youarel_link::Link;
use youarel_store::{Session, Store, StoreError};

/// The formats `youarel export` writes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum Format {
    /// A bookmark file every browser imports.
    Html,
    /// The folders and shortcuts as plain JSON.
    Json,
}

/// What `add` takes besides the link.
#[derive(Clone, Debug, Default)]
pub struct Addition {
    pub title: Option<String>,
    pub alias: Option<String>,
    pub folder: Option<String>,
    /// Whether to fetch the page's title when none is given.
    pub fetch: bool,
}

/// A store error as the command line reports it.
fn failed(error: StoreError) -> anyhow::Error {
    anyhow!(error)
}

/// Runs `work` in one session at `now`.
fn session<R>(
    store: &Store,
    now: u64,
    work: impl FnOnce(&mut Session) -> Result<R, StoreError>,
) -> Result<R> {
    store.session(now, work).map_err(failed)
}

/// The title a new shortcut gets: the one given, else the page's own, else the link itself.
fn title_for(link: &Link, addition: &Addition) -> (String, Option<String>) {
    if let Some(title) = &addition.title {
        return (title.clone(), None);
    }
    if addition.fetch {
        match web::fetch_title(link) {
            Ok(Some(title)) => return (title, None),
            Ok(None) => {}
            Err(error) => return (link.as_str().to_owned(), Some(format!("{error:#}"))),
        }
    }
    (link.as_str().to_owned(), None)
}

/// Adds a shortcut and reports what it holds; the folders of its path are made when they do not exist.
///
/// # Errors
///
/// Returns an error when the link is not one a shortcut may hold, or the store fails.
pub fn add(store: &Store, now: u64, link: &str, addition: &Addition) -> Result<String> {
    let link = Link::parse(link).with_context(|| format!("cannot add `{link}`"))?;
    let (title, warning) = title_for(&link, addition);
    let message = session(store, now, |session| {
        let folder = match &addition.folder {
            Some(path) => folder_path(session, path)?,
            None => None,
        };
        let id = session.fresh_id()?;
        session.edit(Edit::AddShortcut {
            id,
            link: link.as_str().to_owned(),
            title: title.clone(),
            folder,
        })?;
        if let Some(alias) = &addition.alias {
            session.edit(Edit::Realias {
                id,
                alias: alias.clone(),
            })?;
        }
        let tree = session.state().tree();
        Ok(format!(
            "added {}",
            describe(session.state(), &tree, Entry::Shortcut(id))
        ))
    })?;
    Ok(match warning {
        Some(warning) => format!("{message}\n(the title could not be read: {warning})"),
        None => message,
    })
}

/// The state as it stands, read under the lock.
fn read(store: &Store, now: u64) -> Result<State> {
    session(store, now, |session| Ok(session.state().clone()))
}

/// One line for an entry in a listing.
fn line(state: &State, tree: &Tree, entry: Entry) -> String {
    match entry {
        Entry::Folder(_) => describe(state, tree, entry),
        Entry::Shortcut(id) => state.shortcut(id).map_or_else(String::new, |shortcut| {
            if shortcut.alias.value.is_empty() {
                format!("{}  {}", shortcut.title.value, shortcut.link.value)
            } else {
                format!(
                    "{}  {}  [{}]",
                    shortcut.title.value, shortcut.link.value, shortcut.alias.value
                )
            }
        }),
    }
}

/// The lines of a folder's listing.
///
/// # Errors
///
/// Returns an error when the folder does not exist or the store fails.
pub fn list(store: &Store, now: u64, path: Option<&str>) -> Result<Vec<String>> {
    let state = read(store, now)?;
    let tree = state.tree();
    let folder = resolve::folder(&state, &tree, path.unwrap_or_default())?;
    Ok(state
        .entries(&tree, folder)
        .into_iter()
        .map(|entry| line(&state, &tree, entry))
        .collect())
}

/// The ranked matches of a query.
fn matches(session: &Session, query: &str) -> Vec<Entry> {
    let ui = Ui {
        query: query.to_owned(),
        ..Ui::default()
    };
    view::rows(session.state(), &ui, session.visits(), session.now())
        .into_iter()
        .filter_map(|row| match row {
            Row::Entry(entry) => Some(entry),
            Row::Parent(_) | Row::Add(_) => None,
        })
        .collect()
}

/// The lines of a search.
///
/// # Errors
///
/// Returns an error when the store fails.
pub fn find(store: &Store, now: u64, query: &str) -> Result<Vec<String>> {
    session(store, now, |session| {
        let tree = session.state().tree();
        Ok(matches(session, query)
            .into_iter()
            .map(|entry| line(session.state(), &tree, entry))
            .collect())
    })
}

/// Opens the best shortcut for a query and counts the visit.
///
/// # Errors
///
/// Returns an error when no shortcut matches, the browser cannot start, or the store fails.
pub fn open(store: &Store, now: u64, query: &str) -> Result<String> {
    let found = session(store, now, |session| {
        let best = matches(session, query)
            .into_iter()
            .find_map(|entry| match entry {
                Entry::Shortcut(id) => session.state().shortcut(id).map(|shortcut| {
                    (
                        id,
                        shortcut.link.value.clone(),
                        shortcut.title.value.clone(),
                    )
                }),
                Entry::Folder(_) => None,
            });
        if let Some((id, _, _)) = &best {
            session.visit(*id);
        }
        Ok(best)
    })?;
    let (_, link, title) = found.with_context(|| format!("no shortcut matches `{query}`"))?;
    web::open(&link)?;
    Ok(format!("opened {title} <{link}>"))
}

/// Creates the folders of a path that do not exist yet.
///
/// # Errors
///
/// Returns an error when a segment is ambiguous or the store fails.
pub fn make_folder(store: &Store, now: u64, path: &str) -> Result<String> {
    session(store, now, |session| {
        let current = folder_path(session, path)?;
        let tree = session.state().tree();
        Ok(format!(
            "folder {}/",
            session.state().path(&tree, current).join("/")
        ))
    })
}

/// The folder a path names, made with every folder above it that does not exist, or `None` for the top.
fn folder_path(session: &mut Session, path: &str) -> Result<Option<ItemId>, StoreError> {
    let mut current = None;
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        current = Some(folder_named(session, current, segment)?.id);
    }
    Ok(current)
}

/// A folder found or made by name.
#[derive(Clone, Copy, Debug)]
struct Named {
    id: ItemId,
    created: bool,
}

/// The folder named `name` in `parent`, created when there is none.
fn folder_named(
    session: &mut Session,
    parent: Option<ItemId>,
    name: &str,
) -> Result<Named, StoreError> {
    let tree = session.state().tree();
    let existing =
        session
            .state()
            .entries(&tree, parent)
            .into_iter()
            .find_map(|entry| match entry {
                Entry::Folder(id) => session
                    .state()
                    .folder(id)
                    .filter(|folder| folder.name.value == name)
                    .map(|_| id),
                Entry::Shortcut(_) => None,
            });
    if let Some(id) = existing {
        return Ok(Named { id, created: false });
    }
    let id = session.fresh_id()?;
    session.edit(Edit::AddFolder {
        id,
        name: name.to_owned(),
        parent,
    })?;
    Ok(Named { id, created: true })
}

/// The entry a reference names in the state as it stands.
///
/// An entry another process removes before the edit then makes the edit refuse it.
fn resolved(store: &Store, now: u64, reference: &str) -> Result<Entry> {
    let state = read(store, now)?;
    resolve::entry(&state, &state.tree(), reference)
}

/// Moves an entry to the end of a folder.
///
/// # Errors
///
/// Returns an error when a name resolves to nothing, the move would put a folder into itself, or the store fails.
pub fn move_entry(store: &Store, now: u64, reference: &str, path: &str) -> Result<String> {
    let entry = resolved(store, now, reference)?;
    let state = read(store, now)?;
    let folder = resolve::folder(&state, &state.tree(), path)?;
    session(store, now, |session| {
        session.edit(Edit::Move {
            entry,
            folder,
            slot: Slot::End,
        })?;
        let tree = session.state().tree();
        Ok(format!("moved {}", describe(session.state(), &tree, entry)))
    })
}

/// Gives a shortcut a new title or a folder a new name.
///
/// # Errors
///
/// Returns an error when the name resolves to nothing or the store fails.
pub fn rename(store: &Store, now: u64, reference: &str, name: &str) -> Result<String> {
    let entry = resolved(store, now, reference)?;
    session(store, now, |session| {
        session.edit(match entry {
            Entry::Folder(id) => Edit::RenameFolder {
                id,
                name: name.to_owned(),
            },
            Entry::Shortcut(id) => Edit::Retitle {
                id,
                title: name.to_owned(),
            },
        })?;
        let tree = session.state().tree();
        Ok(format!(
            "renamed {}",
            describe(session.state(), &tree, entry)
        ))
    })
}

/// Gives a shortcut an alias, or removes its alias when the alias is empty.
///
/// # Errors
///
/// Returns an error when the name resolves to nothing or to a folder, or the store fails.
pub fn alias(store: &Store, now: u64, reference: &str, alias: &str) -> Result<String> {
    let Entry::Shortcut(id) = resolved(store, now, reference)? else {
        anyhow::bail!("only shortcuts have aliases");
    };
    session(store, now, |session| {
        session.edit(Edit::Realias {
            id,
            alias: alias.to_owned(),
        })?;
        let tree = session.state().tree();
        Ok(format!(
            "aliased {}",
            describe(session.state(), &tree, Entry::Shortcut(id))
        ))
    })
}

/// Removes a shortcut, or a folder with everything it holds.
///
/// # Errors
///
/// Returns an error when the name resolves to nothing or the store fails.
pub fn remove(store: &Store, now: u64, reference: &str) -> Result<String> {
    let entry = resolved(store, now, reference)?;
    session(store, now, |session| {
        let tree = session.state().tree();
        let described = describe(session.state(), &tree, entry);
        session.edit(Edit::Remove { entry })?;
        Ok(format!("removed {described}"))
    })
}

/// What an import added and skipped.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Imported {
    pub folders: usize,
    pub shortcuts: usize,
    /// Links already held, which the import leaves alone.
    pub duplicates: usize,
    /// Links that are no web links, such as bookmarklets.
    pub refused: usize,
}

/// An import in progress: the links the store holds, and what the import did so far.
#[derive(Debug)]
struct Importing {
    held: std::collections::BTreeSet<String>,
    counts: Imported,
}

fn import_items(
    session: &mut Session,
    items: &[Item],
    parent: Option<ItemId>,
    importing: &mut Importing,
) -> Result<(), StoreError> {
    for item in items {
        match item {
            Item::Folder { name, items: inner } => {
                let folder = folder_named(session, parent, name)?;
                if folder.created {
                    importing.counts.folders = importing.counts.folders.saturating_add(1);
                }
                import_items(session, inner, Some(folder.id), importing)?;
            }
            Item::Link { href, title } => {
                let counts = &mut importing.counts;
                let Ok(link) = Link::parse(href) else {
                    counts.refused = counts.refused.saturating_add(1);
                    continue;
                };
                if !importing.held.insert(link.as_str().to_owned()) {
                    counts.duplicates = counts.duplicates.saturating_add(1);
                    continue;
                }
                let id = session.fresh_id()?;
                let title = if title.is_empty() {
                    link.as_str().to_owned()
                } else {
                    title.clone()
                };
                session.edit(Edit::AddShortcut {
                    id,
                    link: link.as_str().to_owned(),
                    title,
                    folder: parent,
                })?;
                importing.counts.shortcuts = importing.counts.shortcuts.saturating_add(1);
            }
        }
    }
    Ok(())
}

/// Imports a bookmark file: its folders merge into folders of the same name, and links already held stay single.
///
/// # Errors
///
/// Returns an error when the store fails.
pub fn import(store: &Store, now: u64, text: &str) -> Result<Imported> {
    let items = bookmarks::parse(text);
    session(store, now, |session| {
        let held = session
            .state()
            .shortcuts
            .values()
            .filter(|shortcut| !shortcut.removed.value)
            .map(|shortcut| shortcut.link.value.clone())
            .collect();
        let mut importing = Importing {
            held,
            counts: Imported::default(),
        };
        import_items(session, &items, None, &mut importing)?;
        Ok(importing.counts)
    })
}

/// The live entries of a folder as bookmark items.
fn items(state: &State, tree: &Tree, folder: Option<ItemId>) -> Vec<Item> {
    state
        .entries(tree, folder)
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Folder(id) => state.folder(id).map(|found| Item::Folder {
                name: found.name.value.clone(),
                items: items(state, tree, Some(id)),
            }),
            Entry::Shortcut(id) => state.shortcut(id).map(|shortcut| Item::Link {
                href: shortcut.link.value.clone(),
                title: shortcut.title.value.clone(),
            }),
        })
        .collect()
}

/// A bookmark item as plain JSON.
fn json(item: &Item) -> serde_json::Value {
    match item {
        Item::Folder { name, items } => serde_json::json!({
            "folder": name,
            "items": items.iter().map(json).collect::<Vec<_>>(),
        }),
        Item::Link { href, title } => serde_json::json!({ "title": title, "link": href }),
    }
}

/// Every live folder and shortcut in a format another program reads.
///
/// # Errors
///
/// Returns an error when the store fails.
pub fn export(store: &Store, now: u64, format: Format) -> Result<String> {
    let state = read(store, now)?;
    let tree = state.tree();
    let exported = items(&state, &tree, None);
    Ok(match format {
        Format::Html => bookmarks::write(&exported),
        Format::Json => {
            let document: Vec<serde_json::Value> = exported.iter().map(json).collect();
            let mut text = serde_json::to_string_pretty(&document)?;
            text.push('\n');
            text
        }
    })
}

/// Writes lines to standard output.
///
/// # Errors
///
/// Returns an error when standard output is closed.
pub fn print(lines: &[String]) -> Result<()> {
    let mut out = std::io::stdout().lock();
    for text in lines {
        writeln!(out, "{text}")?;
    }
    Ok(())
}
