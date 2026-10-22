//! The commands of the `youarel` binary, each one locked session of the store.

use crate::resolve::{self, describe};
use crate::{browsers, icons, web};
use anyhow::{Context, Result, anyhow};
use std::io::Write as _;
use youarel_core::browser_rules::Browser;
use youarel_core::mark_rules;
use youarel_core::state::{Edit, Entry, ItemId, Slot, State, Tree};
use youarel_core::ui_rules::{Row, Ui};
use youarel_core::view;
use youarel_html::bookmarks::{self, Item};
use youarel_link::Link;
use youarel_store::icons::IconId;
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
    /// Whether to fetch the page for its title, when none is given, and for its icon.
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

/// What a new shortcut gets from its page: its title, its icon, and what could not be read.
struct Fetched {
    title: String,
    icon: Option<(IconId, Vec<u8>)>,
    notes: Vec<String>,
}

/// The title a new shortcut gets, the one given, else the page's own, else the link itself, and the icon its page offers.
fn fetched(link: &Link, addition: &Addition) -> Fetched {
    let mut notes = Vec::new();
    let page = if addition.fetch {
        match web::fetch_page(link) {
            Ok(page) => Some(page),
            Err(error) => {
                notes.push(format!("the page could not be read: {error:#}"));
                None
            }
        }
    } else {
        None
    };
    let title = addition
        .title
        .clone()
        .or_else(|| page.as_ref().and_then(|page| page.title.clone()))
        .unwrap_or_else(|| link.as_str().to_owned());
    let icon = page
        .as_ref()
        .and_then(|page| match icons::fetch(link, page) {
            Ok(icon) => Some(icon),
            Err(error) => {
                notes.push(format!("no icon: {error:#}"));
                None
            }
        });
    Fetched { title, icon, notes }
}

/// Adds a shortcut and reports what it holds; the folders of its path are made when they do not exist.
///
/// # Errors
///
/// Returns an error when the link is not one a shortcut may hold, its folder holds it already, or the store fails.
pub fn add(store: &Store, now: u64, link: &str, addition: &Addition) -> Result<String> {
    let link = Link::parse(link).with_context(|| format!("cannot add `{link}`"))?;
    let Fetched { title, icon, notes } = fetched(&link, addition);
    // The session answers with the shortcut that already holds the link, which refuses the addition.
    let outcome = session(store, now, |session| {
        let folder = match &addition.folder {
            Some(path) => folder_path(session, path)?,
            None => None,
        };
        let before = session.state().tree();
        if let Some(held) = session.state().holding(&before, folder, link.as_str()) {
            return Ok(Err(describe(
                session.state(),
                &before,
                Entry::Shortcut(held),
            )));
        }
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
        if let Some(icon) = icon {
            icons::keep(session, &link, icon)?;
        }
        let tree = session.state().tree();
        Ok(Ok(format!(
            "added {}",
            describe(session.state(), &tree, Entry::Shortcut(id))
        )))
    })?;
    let message = outcome.map_err(|held| anyhow!("its folder holds it already: {held}"))?;
    Ok(notes
        .iter()
        .fold(message, |message, note| format!("{message}\n({note})")))
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

/// Opens the best shortcut for a query and counts the visit, then gathers its site's favicon when this device keeps no icon for its host.
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
    let opened = format!("opened {title} <{link}>");
    // The browser reaches the site now anyway, so a host this device keeps no icon for may give it one.
    let gathered = Link::parse(&link)
        .map_err(|error| anyhow!(error))
        .and_then(|link| icons::opened(store, now, &link));
    Ok(match gathered {
        Ok(()) => opened,
        Err(error) => format!("{opened}\n(no icon kept: {error:#})"),
    })
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

/// The shortcut a reference names, refusing a folder for what only shortcuts have.
fn shortcut_named(store: &Store, now: u64, reference: &str, what: &str) -> Result<ItemId> {
    match resolved(store, now, reference)? {
        Entry::Shortcut(id) => Ok(id),
        Entry::Folder(_) => anyhow::bail!("only shortcuts have {what}"),
    }
}

/// Writes a shortcut's note, or clears it with an empty one.
///
/// # Errors
///
/// Returns an error when the name resolves to nothing or to a folder, or the store fails.
pub fn note(store: &Store, now: u64, reference: &str, note: &str) -> Result<String> {
    let id = shortcut_named(store, now, reference, "notes")?;
    session(store, now, |session| {
        session.edit(Edit::SetNote {
            id,
            note: note.to_owned(),
        })?;
        let tree = session.state().tree();
        Ok(format!(
            "noted {}",
            describe(session.state(), &tree, Entry::Shortcut(id))
        ))
    })
}

/// Gives a folder a mark to show in place of its glyph, such as one emoji, or clears it with an empty one.
///
/// # Errors
///
/// Returns an error when the name resolves to nothing or to a shortcut, the mark is refused, or the store fails.
pub fn mark(store: &Store, now: u64, reference: &str, mark: &str) -> Result<String> {
    let Entry::Folder(id) = resolved(store, now, reference)? else {
        anyhow::bail!("only folders have marks");
    };
    let mark = mark_rules::admit(mark).map_err(|refused| anyhow::anyhow!("{refused}"))?;
    session(store, now, |session| {
        session.edit(Edit::MarkFolder {
            id,
            mark: mark.to_owned(),
        })?;
        let tree = session.state().tree();
        Ok(format!(
            "marked {}",
            describe(session.state(), &tree, Entry::Folder(id))
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

impl core::fmt::Display for Imported {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "imported {} shortcuts and {} folders; {} were already here, and {} were no web links",
            self.shortcuts, self.folders, self.duplicates, self.refused
        )
    }
}

/// An import in progress: the links the store holds, and what the import did so far.
#[derive(Debug)]
struct Importing {
    held: std::collections::BTreeSet<String>,
    counts: Imported,
}

/// What an import adds: a folder that holds something new, or a web link the store holds nowhere yet.
#[derive(Debug)]
enum Fresh {
    Folder { name: String, items: Vec<Self> },
    Link { link: Link, title: String },
}

/// The items an import adds, counting what falls away: links already held, links that are no web links, and every folder left holding nothing.
fn fresh(items: &[Item], importing: &mut Importing) -> Vec<Fresh> {
    let mut kept = Vec::new();
    for item in items {
        match item {
            Item::Folder { name, items: inner } => {
                let held = fresh(inner, importing);
                if !held.is_empty() {
                    kept.push(Fresh::Folder {
                        name: name.clone(),
                        items: held,
                    });
                }
            }
            Item::Link { href, title } => {
                let counts = &mut importing.counts;
                let Ok(link) = Link::parse(href) else {
                    counts.refused = counts.refused.saturating_add(1);
                    continue;
                };
                if importing.held.insert(link.as_str().to_owned()) {
                    kept.push(Fresh::Link {
                        link,
                        title: title.clone(),
                    });
                } else {
                    counts.duplicates = counts.duplicates.saturating_add(1);
                }
            }
        }
    }
    kept
}

fn import_items(
    session: &mut Session,
    items: &[Fresh],
    parent: Option<ItemId>,
    counts: &mut Imported,
) -> Result<(), StoreError> {
    for item in items {
        match item {
            Fresh::Folder { name, items: inner } => {
                let folder = folder_named(session, parent, name)?;
                if folder.created {
                    counts.folders = counts.folders.saturating_add(1);
                }
                import_items(session, inner, Some(folder.id), counts)?;
            }
            Fresh::Link { link, title } => {
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
                counts.shortcuts = counts.shortcuts.saturating_add(1);
            }
        }
    }
    Ok(())
}

/// Imports a file of bookmarks that a browser exported or keeps, read as its first bytes name it.
///
/// The file lands under the folder path `folder` such as `Work/Infra`, or at the top.
/// Its folders merge into folders of the same name, and links already held stay single.
///
/// # Errors
///
/// Returns an error when the file is not the kind its first bytes name, or the store fails.
pub fn import(store: &Store, now: u64, bytes: &[u8], folder: Option<&str>) -> Result<Imported> {
    import_tree(store, now, &nested(folder, browsers::read_file(bytes)?))
}

/// Imports the bookmarks a browser keeps on this device, in a folder named after the browser, under the folder path `folder` or at the top.
///
/// # Errors
///
/// Returns an error when the browser's bookmarks cannot be found or read, or the store fails.
pub fn import_browser(
    store: &Store,
    now: u64,
    browser: Browser,
    folder: Option<&str>,
) -> Result<Imported> {
    import_tree(store, now, &nested(folder, vec![browsers::read(browser)?]))
}

/// Items inside the folders of a path such as `Work/Infra`, the outermost first, which the import merges like any folder.
fn nested(path: Option<&str>, items: Vec<Item>) -> Vec<Item> {
    path.unwrap_or_default()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .rev()
        .fold(items, |inner, name| {
            vec![Item::Folder {
                name: name.to_owned(),
                items: inner,
            }]
        })
}

/// Imports folders and links: folders merge into folders of the same name, links already held stay single, and a folder with nothing new stays away.
///
/// # Errors
///
/// Returns an error when the store fails.
pub fn import_tree(store: &Store, now: u64, items: &[Item]) -> Result<Imported> {
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
        let adding = fresh(items, &mut importing);
        import_items(session, &adding, None, &mut importing.counts)?;
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

#[cfg(test)]
mod tests {
    use super::{Addition, Imported, Item, add, find, import, list, make_folder, mark, note, read};
    use youarel_html::bookmarks;
    use youarel_import::{chrome, firefox};
    use youarel_store::Store;

    /// A time for every session of the tests.
    const NOW: u64 = 1_700_000_000_000;
    /// A page the import tests file.
    const MDN: &str = "https://developer.mozilla.org/";
    /// Another page the import tests file.
    const HTML_STANDARD: &str = "https://html.spec.whatwg.org/";

    #[test]
    fn a_note_goes_on_a_shortcut_and_a_search_finds_it() {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_path_buf());
        let addition = Addition {
            title: Some("URL Standard".to_owned()),
            alias: Some("url".to_owned()),
            ..Addition::default()
        };
        add(&store, NOW, "https://url.spec.whatwg.org/", &addition).unwrap();
        assert_eq!(
            list(&store, NOW, None).unwrap(),
            ["URL Standard  https://url.spec.whatwg.org/  [url]"]
        );
        let noted = note(&store, NOW, "url", "the parser of record").unwrap();
        assert!(noted.starts_with("noted URL Standard"), "{noted}");
        assert_eq!(find(&store, NOW, "record").unwrap().len(), 1);
    }

    #[test]
    fn a_folder_holds_a_link_once_and_another_folder_may_hold_it_too() {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_path_buf());
        let link = "https://developer.mozilla.org/";
        add(&store, NOW, link, &Addition::default()).unwrap();
        let again = add(&store, NOW, link, &Addition::default());
        assert!(
            again
                .unwrap_err()
                .to_string()
                .contains("its folder holds it already"),
        );
        let elsewhere = Addition {
            folder: Some("MDN".to_owned()),
            ..Addition::default()
        };
        add(&store, NOW, link, &elsewhere).unwrap();
        assert_eq!(find(&store, NOW, "mozilla").unwrap().len(), 2);
    }

    #[test]
    fn a_folder_takes_no_note() {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_path_buf());
        make_folder(&store, NOW, "Work").unwrap();
        let noted = note(&store, NOW, "Work/", "x");
        assert!(
            noted
                .unwrap_err()
                .to_string()
                .contains("only shortcuts have notes")
        );
    }

    #[test]
    fn a_folder_takes_a_mark_and_a_shortcut_or_a_spaced_mark_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_path_buf());
        make_folder(&store, NOW, "Work").unwrap();
        let marked = mark(&store, NOW, "Work/", " 📚 ").unwrap();
        assert!(marked.starts_with("marked"), "{marked}");
        let state = read(&store, NOW).unwrap();
        assert!(
            state
                .folders
                .values()
                .any(|folder| folder.mark.value == "📚")
        );
        let spaced = mark(&store, NOW, "Work/", "a b").unwrap_err();
        assert!(spaced.to_string().contains("no space"), "{spaced}");
        let named = Addition {
            title: Some("MDN".to_owned()),
            alias: Some("mdn".to_owned()),
            ..Addition::default()
        };
        add(&store, NOW, MDN, &named).unwrap();
        let shortcut = mark(&store, NOW, "mdn", "📚").unwrap_err();
        assert!(shortcut.to_string().contains("only folders"), "{shortcut}");
    }

    #[test]
    fn a_file_of_any_browser_lands_in_the_folder_named_and_its_links_stay_single() {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_path_buf());
        let tree = [Item::Folder {
            name: "Docs".to_owned(),
            items: vec![
                Item::Link {
                    href: "https://developer.mozilla.org/".to_owned(),
                    title: "MDN".to_owned(),
                },
                Item::Link {
                    href: "javascript:void(0)".to_owned(),
                    title: "A bookmarklet".to_owned(),
                },
            ],
        }];
        let chromium = chrome::write(&tree);
        let imported = import(&store, NOW, chromium.as_bytes(), Some("/Edge//Study/")).unwrap();
        assert_eq!(
            (imported.shortcuts, imported.folders, imported.refused),
            (1, 3, 1),
            "Edge, Study, and Docs, which the file keeps as its bookmarks bar"
        );
        assert_eq!(
            list(&store, NOW, Some("Edge/Study/Docs")).unwrap(),
            ["MDN  https://developer.mozilla.org/"]
        );
        let again = import(&store, NOW, &firefox::encode(&tree).unwrap(), Some("Edge")).unwrap();
        assert_eq!((again.shortcuts, again.duplicates), (0, 1));
        import(&store, NOW, b"mozLz40\0", None).unwrap_err();
    }

    #[test]
    fn an_import_makes_no_folder_whose_links_the_library_holds_already() {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_path_buf());
        let docs = |links: &[&str]| {
            [Item::Folder {
                name: "Docs".to_owned(),
                items: links
                    .iter()
                    .map(|href| Item::Link {
                        href: (*href).to_owned(),
                        title: String::new(),
                    })
                    .collect(),
            }]
        };
        let first = bookmarks::write(&docs(&[MDN]));
        import(&store, NOW, first.as_bytes(), Some("Brave")).unwrap();
        let held = import(
            &store,
            NOW,
            chrome::write(&docs(&[MDN])).as_bytes(),
            Some("Edge"),
        )
        .unwrap();
        assert_eq!(
            (held.shortcuts, held.folders, held.duplicates),
            (0, 0, 1),
            "neither Edge nor its Docs holds anything new"
        );
        list(&store, NOW, Some("Edge")).unwrap_err();
        let both = chrome::write(&docs(&[MDN, HTML_STANDARD]));
        let added = import(&store, NOW, both.as_bytes(), Some("Brave")).unwrap();
        assert_eq!(
            (added.shortcuts, added.folders, added.duplicates),
            (1, 0, 1),
            "a folder of the same name takes the new link"
        );
        assert_eq!(
            list(&store, NOW, Some("Brave/Docs")).unwrap(),
            [
                format!("{MDN}  {MDN}"),
                format!("{HTML_STANDARD}  {HTML_STANDARD}")
            ],
            "a link without a title takes the link"
        );
    }

    #[test]
    fn an_import_reports_what_it_added_and_skipped() {
        let counts = Imported {
            folders: 1,
            shortcuts: 2,
            duplicates: 3,
            refused: 4,
        };
        assert_eq!(
            counts.to_string(),
            "imported 2 shortcuts and 1 folders; 3 were already here, and 4 were no web links"
        );
    }
}
