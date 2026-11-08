//! The commands of the `youarel` binary, each one locked session of the store.

use crate::resolve::{self, describe};
use crate::{icons, web};
use anyhow::{Context, Result, anyhow};
use std::io::Write as _;
use youarel_core::mark_rules;
use youarel_core::state::{Edit, Entry, ItemId, Slot};
use youarel_link::Link;
use youarel_store::icons::IconId;
use youarel_store::{Session, Store, StoreError};

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

/// Runs `work`, which reports any failure, in one session at `now`.
fn reading<R>(store: &Store, now: u64, work: impl FnOnce(&mut Session) -> Result<R>) -> Result<R> {
    let mut outcome = None;
    session(store, now, |session| {
        outcome = Some(work(session));
        Ok(())
    })?;
    outcome.context("the session ran no work")?
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
    let link = Link::from_input(link).with_context(|| format!("cannot add `{link}`"))?;
    let Fetched { title, icon, notes } = fetched(&link, addition);
    let message = reading(store, now, |session| {
        let folder = match &addition.folder {
            Some(path) => folder_path(session, path)?,
            None => None,
        };
        if let Some(held) = session.holder(folder, link.as_str())? {
            let named = describe(session, Entry::Shortcut(held))?;
            anyhow::bail!("its folder holds it already: {named}");
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
        Ok(format!("added {}", describe(session, Entry::Shortcut(id))?))
    })?;
    Ok(notes
        .iter()
        .fold(message, |message, note| format!("{message}\n({note})")))
}

/// One line for an entry in a listing.
fn line(session: &Session, entry: Entry) -> Result<String> {
    match entry {
        Entry::Folder(_) => describe(session, entry),
        Entry::Shortcut(id) => Ok(session.item(id)?.map_or_else(String::new, |shortcut| {
            if shortcut.alias.value.is_empty() {
                format!("{}  {}", shortcut.title.value, shortcut.link.value)
            } else {
                format!(
                    "{}  {}  [{}]",
                    shortcut.title.value, shortcut.link.value, shortcut.alias.value
                )
            }
        })),
    }
}

/// The lines of the first entries of a folder's listing, at most `limit` of them.
///
/// # Errors
///
/// Returns an error when the folder does not exist or the store fails.
pub fn list(store: &Store, now: u64, path: Option<&str>, limit: usize) -> Result<Vec<String>> {
    reading(store, now, |session| {
        let folder = resolve::folder(session.folders(), path.unwrap_or_default())?;
        session
            .entries_page(folder, 0, limit)?
            .into_iter()
            .map(|entry| line(session, entry))
            .collect()
    })
}

/// The lines of the best matches of a search, at most `limit` of them.
///
/// # Errors
///
/// Returns an error when the store fails.
pub fn find(store: &Store, now: u64, query: &str, limit: usize) -> Result<Vec<String>> {
    reading(store, now, |session| {
        session
            .find(query)?
            .into_iter()
            .take(limit)
            .map(|entry| line(session, entry))
            .collect()
    })
}

/// Opens the best shortcut for a query and counts the visit, then gathers its site's favicon when this device keeps no icon for its host.
///
/// # Errors
///
/// Returns an error when no shortcut matches, the browser cannot start, or the store fails.
pub fn open(store: &Store, now: u64, query: &str) -> Result<String> {
    let found = session(store, now, |session| {
        let mut best = None;
        for entry in session.find(query)? {
            if let Entry::Shortcut(id) = entry
                && let Some(shortcut) = session.item(id)?
            {
                best = Some((id, shortcut.link.value, shortcut.title.value));
                break;
            }
        }
        if let Some((id, _, _)) = &best {
            session.visit(*id)?;
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
        Ok(format!(
            "folder {}/",
            session.folders().path(current).join("/")
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
}

/// The folder named `name` in `parent`, created when there is none.
fn folder_named(
    session: &mut Session,
    parent: Option<ItemId>,
    name: &str,
) -> Result<Named, StoreError> {
    let folders = session.folders();
    let existing = folders.children(parent).iter().copied().find(|id| {
        folders
            .folder(*id)
            .is_some_and(|folder| folder.name.value == name)
    });
    if let Some(id) = existing {
        return Ok(Named { id });
    }
    let id = session.fresh_id()?;
    session.edit(Edit::AddFolder {
        id,
        name: name.to_owned(),
        parent,
    })?;
    Ok(Named { id })
}

/// Runs an edit of the entry a reference names, and reports it with the verb `done`.
///
/// An entry another process removes between the resolution and the edit then makes the edit refuse it.
fn edit_named(
    store: &Store,
    now: u64,
    reference: &str,
    edit: impl FnOnce(&Session, Entry) -> Result<Edit>,
) -> Result<String> {
    reading(store, now, |session| {
        let entry = resolve::entry(session, reference)?;
        let made = edit(session, entry)?;
        let before = describe(session, entry)?;
        session.edit(made)?;
        let after = describe(session, entry)?;
        Ok(match entry {
            Entry::Shortcut(_) | Entry::Folder(_)
                if session.item(entry_id(entry))?.is_none()
                    && !session.folders().holds(entry_id(entry)) =>
            {
                before
            }
            Entry::Shortcut(_) | Entry::Folder(_) => after,
        })
    })
}

/// The identifier of a folder or a shortcut.
const fn entry_id(entry: Entry) -> ItemId {
    match entry {
        Entry::Folder(id) | Entry::Shortcut(id) => id,
    }
}

/// Moves an entry to the end of a folder.
///
/// # Errors
///
/// Returns an error when a name resolves to nothing, the move would put a folder into itself, or the store fails.
pub fn move_entry(store: &Store, now: u64, reference: &str, path: &str) -> Result<String> {
    let moved = edit_named(store, now, reference, |session, entry| {
        Ok(Edit::Move {
            entry,
            folder: resolve::folder(session.folders(), path)?,
            slot: Slot::End,
        })
    })?;
    Ok(format!("moved {moved}"))
}

/// Gives a shortcut a new title or a folder a new name.
///
/// # Errors
///
/// Returns an error when the name resolves to nothing or the store fails.
pub fn rename(store: &Store, now: u64, reference: &str, name: &str) -> Result<String> {
    let renamed = edit_named(store, now, reference, |_, entry| {
        Ok(match entry {
            Entry::Folder(id) => Edit::RenameFolder {
                id,
                name: name.to_owned(),
            },
            Entry::Shortcut(id) => Edit::Retitle {
                id,
                title: name.to_owned(),
            },
        })
    })?;
    Ok(format!("renamed {renamed}"))
}

/// Gives a shortcut an alias, or removes its alias when the alias is empty.
///
/// # Errors
///
/// Returns an error when the name resolves to nothing or to a folder, or the store fails.
pub fn alias(store: &Store, now: u64, reference: &str, alias: &str) -> Result<String> {
    let aliased = edit_named(store, now, reference, |_, entry| match entry {
        Entry::Shortcut(id) => Ok(Edit::Realias {
            id,
            alias: alias.to_owned(),
        }),
        Entry::Folder(_) => anyhow::bail!("only shortcuts have aliases"),
    })?;
    Ok(format!("aliased {aliased}"))
}

/// Writes a shortcut's note, or clears it with an empty one.
///
/// # Errors
///
/// Returns an error when the name resolves to nothing or to a folder, or the store fails.
pub fn note(store: &Store, now: u64, reference: &str, note: &str) -> Result<String> {
    let noted = edit_named(store, now, reference, |_, entry| match entry {
        Entry::Shortcut(id) => Ok(Edit::SetNote {
            id,
            note: note.to_owned(),
        }),
        Entry::Folder(_) => anyhow::bail!("only shortcuts have notes"),
    })?;
    Ok(format!("noted {noted}"))
}

/// Gives a folder a mark to show in place of its glyph, such as one emoji, or clears it with an empty one.
///
/// # Errors
///
/// Returns an error when the name resolves to nothing or to a shortcut, the mark is refused, or the store fails.
pub fn mark(store: &Store, now: u64, reference: &str, mark: &str) -> Result<String> {
    let marked = edit_named(store, now, reference, |_, entry| match entry {
        Entry::Folder(id) => {
            let mark = mark_rules::admit(mark).map_err(|refused| anyhow::anyhow!("{refused}"))?;
            Ok(Edit::MarkFolder {
                id,
                mark: mark.to_owned(),
            })
        }
        Entry::Shortcut(_) => anyhow::bail!("only folders have marks"),
    })?;
    Ok(format!("marked {marked}"))
}

/// Removes a shortcut, or a folder with everything it holds.
///
/// # Errors
///
/// Returns an error when the name resolves to nothing or the store fails.
pub fn remove(store: &Store, now: u64, reference: &str) -> Result<String> {
    let removed = edit_named(store, now, reference, |_, entry| Ok(Edit::Remove { entry }))?;
    Ok(format!("removed {removed}"))
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
    use super::{Addition, add, find, list, make_folder, mark, note};
    use youarel_store::Store;

    /// A time for every session of the tests.
    const NOW: u64 = 1_700_000_000_000;
    const MDN: &str = "https://developer.mozilla.org/";
    const HTML_STANDARD: &str = "https://html.spec.whatwg.org/";

    #[test]
    fn a_typed_domain_is_stored_canonically_and_matches_an_explicit_duplicate() {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_path_buf());
        add(&store, NOW, "google.com", &Addition::default()).unwrap();
        assert_eq!(
            list(&store, NOW, None, usize::MAX).unwrap(),
            ["https://google.com/  https://google.com/"]
        );
        let duplicate = add(&store, NOW, "https://google.com/", &Addition::default());
        assert!(
            duplicate
                .unwrap_err()
                .to_string()
                .contains("its folder holds it already")
        );
    }

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
            list(&store, NOW, None, usize::MAX).unwrap(),
            ["URL Standard  https://url.spec.whatwg.org/  [url]"]
        );
        let noted = note(&store, NOW, "url", "the parser of record").unwrap();
        assert!(noted.starts_with("noted URL Standard"), "{noted}");
        assert_eq!(find(&store, NOW, "record", usize::MAX).unwrap().len(), 1);
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
        assert_eq!(find(&store, NOW, "mozilla", usize::MAX).unwrap().len(), 2);
    }

    #[test]
    fn a_listing_and_a_search_stop_at_their_limit() {
        let home = tempfile::tempdir().unwrap();
        let store = Store::new(home.path().to_path_buf());
        for link in [MDN, HTML_STANDARD] {
            add(&store, NOW, link, &Addition::default()).unwrap();
        }
        assert_eq!(list(&store, NOW, None, 1).unwrap().len(), 1);
        assert_eq!(list(&store, NOW, None, usize::MAX).unwrap().len(), 2);
        assert_eq!(find(&store, NOW, "https", 1).unwrap().len(), 1);
        assert_eq!(find(&store, NOW, "https", 0).unwrap(), Vec::<String>::new());
        assert_eq!(find(&store, NOW, "https", usize::MAX).unwrap().len(), 2);
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
        let kept = store
            .session(NOW, |session| {
                Ok(session
                    .folders()
                    .records()
                    .values()
                    .any(|folder| folder.mark.value == "📚"))
            })
            .unwrap();
        assert!(kept);
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
}
