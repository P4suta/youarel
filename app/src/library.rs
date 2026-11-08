//! The window's side of the library: the window sends intents, the core decides what shows, and the window draws the view it gets back.
//!
//! Nothing here touches the webview, so the tests drive it as the window would.
//! A view carries one page of the listing the window shows and one page of each column, which the store reads through its indexes.
//! The window asks for more rows as a person scrolls, and the library answers from the listing it laid out while the store stays as it was.

mod build;
mod clipboard;
mod drag;
mod history;
mod jobs;
mod listing;
mod navigation;
mod opening;
mod selection;
mod viewport;

use crate::emoji;
use crate::intent::{Extend, Fields, Intent, TargetName, item, optional};
use crate::view::{
    Choice, EntriesView, Notice, PATH_SEPARATOR, RowsView, View, choice, tree_order,
};
use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use build::{Reader, column_entries, row_views};
use history::{Direction, Step as EditStep};
use jobs::Work;
use listing::{Laid, Shown};
use navigation::Visit;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use youarel_cli::{clock, icons, web};
use youarel_core::history_rules::History;
use youarel_core::library_rules::{
    Grouping, Layout, Move, Period, Scope, column_moved, grouped_moved,
};
use youarel_core::mark_rules;
use youarel_core::page_rules;
use youarel_core::rank_rules::Rank;
use youarel_core::selection_rules::{self, Selection};
use youarel_core::state::{Edit, EditError, Entry, Folder, ItemId, Presence, Shortcut, Slot};
use youarel_core::view as rows;
use youarel_link::Link;
use youarel_store::folders::Folders;
use youarel_store::icons::IconId;
use youarel_store::{Group, Listing, Session, Span, Store, StoreError};

pub use jobs::{Begun, Job};

/// How many rows a view answers with around the place in sight.
pub const PAGE: usize = 200;
/// How many rows before a row the view brings into sight it starts at.
const LEAD: usize = 50;
/// The most rows one call for a page answers with.
pub const MOST_ROWS: usize = 1_000;
/// How many listings the library keeps laid out between views.
const MOST_LAID: usize = 8;
/// The most collections one search of the picker offers.
pub const MOST_CHOICES: usize = 500;

/// The heading of the scope of every shortcut.
const ALL: &str = "Bookmarks";
/// The heading of the shortcuts in no collection.
const UNSORTED: &str = "Unsorted";
/// The heading of the shortcuts added lately.
const RECENT: &str = "Recently added";
/// The heading of what waits in the trash.
const TRASH: &str = "Trash";
/// The heading of the shortcuts in no collection, when the window groups them by collection.
const NO_COLLECTION: &str = "No collection";

/// The heading of the shortcuts added in a period.
const fn period_heading(age: Period) -> &'static str {
    match age {
        Period::Week => "Past 7 days",
        Period::Month => "Past 30 days",
        Period::Earlier => "Earlier",
    }
}

/// The heading a group of shortcuts falls under.
fn heading(group: &Group) -> String {
    match group {
        Group::Period(age) => period_heading(*age).to_owned(),
        Group::Site(site) => site.clone(),
        Group::Folder(None) => NO_COLLECTION.to_owned(),
        Group::Folder(Some(path)) => path.clone(),
    }
}

/// The `data:` address of an icon's PNG, which the window's policy admits as an image and nothing else.
fn data_address(image: &[u8]) -> String {
    format!("data:image/png;base64,{}", BASE64.encode(image))
}

/// Where the window stands: a scope of the library, or the trash.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Place {
    Scope(Scope),
    /// The folders and the shortcuts that wait in the trash themselves.
    Trash,
}

/// What a layout stands in: the first column of the columns, or the folder the cards and the list show.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Root {
    /// The entries of a folder, or of the top for `None`.
    Folder(Option<ItemId>),
    /// The listing of a scope that is no folder.
    Listing,
}

/// A link to add, with the title and the icon its page gave.
#[derive(Clone, Debug)]
pub struct Addition {
    pub link: Link,
    pub title: String,
    pub collection: Option<ItemId>,
    pub icon: Option<(IconId, Vec<u8>)>,
    /// What the page did not give, which the notice mentions.
    pub missing: Option<String>,
}

/// What a page offers a shortcut to add, and what could not be read.
#[derive(Clone, Debug, Default)]
pub struct Fetched {
    pub title: Option<String>,
    pub icon: Option<(IconId, Vec<u8>)>,
    /// Why the page or its icon did not come, which the notice of the addition mentions.
    pub missing: Option<String>,
}

/// Reads the page of a link for its title and its icon, outside any lock, since a page can take seconds.
#[must_use]
pub fn fetch(link: &Link) -> Fetched {
    let page = match web::fetch_page(link) {
        Ok(page) => page,
        Err(error) => {
            return Fetched {
                missing: Some(format!("the page could not be read: {error:#}")),
                ..Fetched::default()
            };
        }
    };
    // A shortcut without an icon still opens, so an icon that does not come leaves none, and the notice says so.
    let (icon, missing) = match icons::fetch(link, &page) {
        Ok(icon) => (Some(icon), None),
        Err(error) => (None, Some(format!("no icon: {error:#}"))),
    };
    Fetched {
        title: page.title,
        icon,
        missing,
    }
}

/// The identifier of a folder or a shortcut.
const fn entry_id(entry: Entry) -> ItemId {
    match entry {
        Entry::Folder(id) | Entry::Shortcut(id) => id,
    }
}

/// An identifier as the folder or the shortcut it names, for a selection that may hold both.
fn entry_of(folders: &Folders, id: ItemId) -> Entry {
    if folders.holds(id) {
        Entry::Folder(id)
    } else {
        Entry::Shortcut(id)
    }
}

/// Each identifier as the folder or the shortcut it names.
fn kinds(folders: &Folders, ids: &[ItemId]) -> Vec<Entry> {
    ids.iter().map(|id| entry_of(folders, *id)).collect()
}

/// A folder that waits in the trash itself.
fn trashed_folder(folders: &Folders, id: ItemId) -> Option<&Folder> {
    folders
        .records()
        .get(&id)
        .filter(|folder| folder.presence.value == Presence::Trashed)
}

/// The name of the folder or the title of the shortcut that waits in the trash itself, or none for an item that does not.
fn trashed_name(session: &Session, id: ItemId) -> Result<Option<String>, StoreError> {
    if let Some(folder) = trashed_folder(session.folders(), id) {
        return Ok(Some(folder.name.value.clone()));
    }
    Ok(session.trashed(id)?.map(|shortcut| shortcut.title.value))
}

/// A shortcut that shows, or that waits in the trash itself.
fn listed_shortcut(session: &Session, id: ItemId) -> Result<Option<Shortcut>, StoreError> {
    session
        .item(id)?
        .map_or_else(|| session.trashed(id), |shown| Ok(Some(shown)))
}

/// Whether a window can list an item: it shows, or it waits in the trash itself.
fn listable(session: &Session, place: Place, id: ItemId) -> Result<bool, StoreError> {
    let folders = session.folders();
    match place {
        Place::Scope(_) => Ok(folders.shows(id) || session.item(id)?.is_some()),
        Place::Trash => Ok(trashed_folder(folders, id).is_some() || session.trashed(id)?.is_some()),
    }
}

/// The place of an entry in a listing: the place the page named when the row there is that entry, and otherwise the place the listing holds it at.
fn checked_place(
    session: &Session,
    laid: &Laid,
    entry: Entry,
    hint: Option<usize>,
) -> Result<Option<usize>, StoreError> {
    if let Some(place) = hint
        && laid
            .row(session, place)?
            .is_some_and(|row| row.entry == entry)
    {
        return Ok(Some(place));
    }
    laid.place(session, entry)
}

/// A listing as a selection reads it: which one, and how many rows it holds.
fn selection_shown(laid: &Laid) -> selection_rules::Shown {
    selection_rules::Shown {
        listing: laid.shown.fingerprint(),
        total: laid.total(),
    }
}

/// What an intent comes to: what the notice says, or a job that runs in batches.
enum Answer {
    Said(Option<String>),
    Job(Box<Job>),
}

/// The library behind the window: the store, the window's choices, the selection, the place in sight, and the last notice.
#[derive(Debug)]
pub struct Library {
    store: Store,
    place: Place,
    layout: Layout,
    grouping: Grouping,
    selection: Selection<ItemId>,
    pins: Option<selection_rules::Pinned<ItemId>>,
    settled_generation: Option<u64>,
    focus: Option<ItemId>,
    /// Where the focused row stood when the library last looked, which it tries first.
    focus_place: Option<usize>,
    anchor: Option<ItemId>,
    places: History<Visit>,
    history: History<EditStep>,
    cut: BTreeSet<Entry>,
    drag: Option<drag::Drag>,
    typed: Option<youarel_core::typeahead_rules::Typed>,
    /// The entry selected in each column from the left, with the place it stood at.
    path: Vec<(Entry, usize)>,
    notice: Option<Notice>,
    /// Each icon the window has asked for, drawn again by youarel's own decoder, or `None` for one that does not draw.
    drawn: BTreeMap<IconId, Option<String>>,
    /// The links a person opened whose hosts show no icon, which the window asks their sites' favicons for.
    wanted: Vec<Link>,
    /// The first place the page showed of the listing, which the next view answers around.
    viewport: usize,
    /// The first place the page showed of each column.
    column_viewports: Vec<(selection_rules::Listing, usize)>,
    /// Whether the next view brings the focused row into sight, after an intent that moved the focus.
    revealing: bool,
    /// Whether the listing changed since the last view to another place, so the page starts from its top.
    fresh: bool,
    /// The listing the last view showed, which tells when the listing changed.
    shown: Option<Shown>,
    /// The listings laid out lately, which a view reads again while the store stays as it was.
    laid: Vec<Arc<Laid>>,
}

impl Library {
    /// The bookmark library, opening at its top in a list ordered by title.
    #[must_use]
    pub const fn new(store: Store) -> Self {
        Self {
            store,
            place: Place::Scope(Scope::All),
            layout: Layout::List,
            grouping: Grouping::None,
            selection: Selection::none(),
            pins: None,
            settled_generation: None,
            focus: None,
            focus_place: None,
            anchor: None,
            places: History::new(),
            history: History::new(),
            cut: BTreeSet::new(),
            drag: None,
            typed: None,
            path: Vec::new(),
            notice: None,
            drawn: BTreeMap::new(),
            wanted: Vec::new(),
            viewport: 0,
            column_viewports: Vec::new(),
            revealing: false,
            fresh: false,
            shown: None,
            laid: Vec::new(),
        }
    }

    /// Runs `work` in one session now, which keeps what it wrote only when it succeeds.
    fn session<R>(&self, work: impl FnOnce(&mut Session) -> Result<R, StoreError>) -> Result<R> {
        let now = clock::now()?;
        Ok(self.store.session(now, work)?)
    }

    /// Runs `work` with the library and one session of the store now, for what reads the store and changes the library's own choices.
    fn reading<R>(&mut self, work: impl FnOnce(&mut Self, &mut Session) -> Result<R>) -> Result<R> {
        let store = self.store.clone();
        let now = clock::now()?;
        let mut outcome = None;
        store.session(now, |session| {
            outcome = Some(work(self, session));
            Ok(())
        })?;
        outcome.context("the session ran no work")?
    }

    /// The images of icons the window lacks, by their names, drawn again first, so the webview's decoders only ever read an image youarel encoded.
    ///
    /// # Errors
    ///
    /// Returns an error when a name names no image or the store cannot be read.
    pub fn icons(&mut self, wanted: &[String]) -> Result<BTreeMap<String, String>> {
        let named = wanted
            .iter()
            .map(|name| Ok((name, IconId::parse(name)?)))
            .collect::<Result<Vec<_>>>()?;
        let unread: Vec<IconId> = named
            .iter()
            .map(|(_, image)| *image)
            .filter(|image| !self.drawn.contains_key(image))
            .collect();
        for (image, bytes) in unread.iter().zip(self.store.images(&unread)?) {
            // An image the device no longer keeps shows as no icon, and a later view may name it again.
            let Some(bytes) = bytes else {
                continue;
            };
            // An image that does not draw shows as no icon, as a shortcut without one does.
            let drawn = youarel_icon::normalize(&bytes)
                .map_or_else(|_undrawable| None, |redrawn| Some(data_address(&redrawn)));
            self.drawn.insert(*image, drawn);
        }
        let mut found = BTreeMap::new();
        for (name, image) in named {
            if let Some(Some(address)) = self.drawn.get(&image) {
                found.insert(name.clone(), address.clone());
            }
        }
        Ok(found)
    }

    /// Asks for the favicon of the site of a link a person opened, when this device keeps no icon for its host, since the browser reaches the site at that moment anyway.
    fn want_icon(&mut self, link: &str) -> Result<()> {
        let link = Link::parse(link)?;
        if icons::lacking(&self.store, clock::now()?, &link)? {
            self.wanted.push(link);
        }
        Ok(())
    }

    /// The links whose sites' favicons the window asks for, each handed over once.
    pub fn icons_wanted(&mut self) -> Vec<Link> {
        core::mem::take(&mut self.wanted)
    }

    /// Keeps the icon a site offered for a link a person opened.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be written or the image is larger than the device keeps.
    pub fn keep_icon(&self, link: &Link, icon: (IconId, Vec<u8>)) -> Result<()> {
        self.session(|session| icons::keep(session, link, icon))
    }

    /// Carries out one intent and answers with the view; a failure becomes the notice, and a move or a removal runs every batch it holds.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read for the view.
    pub fn act(&mut self, intent: Intent) -> Result<View> {
        match self.begin(intent)? {
            Begun::Answered(view) => Ok(*view),
            Begun::Working(mut job) => {
                while self.advance(&mut job).is_some() {}
                self.finish(&mut job)
            }
        }
    }

    /// Begins one intent: a move or a removal reads what it acts on and answers with a job that runs in batches, and any other intent is carried out and answered with the view, a failure as its notice.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read for the view.
    pub fn begin(&mut self, intent: Intent) -> Result<Begun> {
        let said = match self.answer(intent) {
            Ok(Answer::Job(job)) => return Ok(Begun::Working(job)),
            Ok(Answer::Said(said)) => Ok(said),
            Err(error) => Err(error),
        };
        self.notice = match said {
            Ok(said) => said.map(|text| Notice { text, error: false }),
            Err(error) => Some(Notice {
                text: format!("{error:#}"),
                error: true,
            }),
        };
        Ok(Begun::Answered(Box::new(self.view()?)))
    }

    /// Carries out an intent, or begins the job of a move or a removal.
    fn answer(&mut self, intent: Intent) -> Result<Answer> {
        let said = match intent {
            Intent::File { target, collection } => {
                let collection = optional(collection.as_deref())?;
                return self.begin_job(&target, Work::File(collection));
            }
            Intent::Trash { target } => return self.begin_job(&target, Work::Trash),
            Intent::PutBack { target } => return self.begin_job(&target, Work::PutBack),
            Intent::Remove { target } => return self.begin_job(&target, Work::Remove),
            Intent::EmptyTrash => {
                return self.empty_trash().map(|job| Answer::Job(Box::new(job)));
            }
            Intent::Show => Ok(None),
            Intent::Undo => self.walk_history(Direction::Undo),
            Intent::Redo => self.walk_history(Direction::Redo),
            Intent::Cut => self.cut_selection(),
            Intent::CancelCut => {
                self.cut.clear();
                Ok(None)
            }
            Intent::Cancel => Ok(self.cancel()),
            Intent::Paste { text } => return self.paste(&text),
            Intent::EndDrag { token } => Ok(self.end_drag(item(&token)?)),
            Intent::Spring { token, folder } => return self.spring(item(&token)?, item(&folder)?),
            Intent::Scope { scope } => Ok(self.choose_place(scope.place()?)),
            Intent::Back => Ok(self.back()),
            Intent::Forward => Ok(self.forward()),
            Intent::ShowInFolder { id } => self.show_in_folder(optional(id.as_deref())?),
            Intent::Type { key } => self.type_ahead(key),
            Intent::Select { id, extend, place } => self.select(item(&id)?, extend, place),
            Intent::SelectAll => self.select_all(),
            Intent::Step {
                key,
                across,
                extend,
            } => self.step(key.into(), across, extend),
            Intent::Open { id } => self.open(optional(id.as_deref())?),
            Intent::Edit { fields } => self.save_fields(&fields),
            Intent::Retitle { id, title } => self.retitle(item(&id)?, &title),
            Intent::NewCollection { name, parent } => {
                self.new_collection(&name, optional(parent.as_deref())?)
            }
            Intent::RenameCollection { id, name } => self.rename_collection(item(&id)?, &name),
            Intent::MarkCollection { id, mark } => self.mark_collection(item(&id)?, &mark),
        };
        said.map(Answer::Said)
    }

    /// Begins the job of a move to the trash, a put back, or a deletion.
    fn begin_job(&mut self, target: &TargetName, work: Work) -> Result<Answer> {
        self.job(target, work).map(|job| Answer::Job(Box::new(job)))
    }

    fn choose_place(&mut self, place: Place) -> Option<String> {
        if self.place != place {
            self.places.push(Visit {
                before: self.place,
                after: place,
            });
        }
        self.place = place;
        self.forget_selection();
        None
    }

    fn forget_selection(&mut self) {
        self.pins = None;
        self.settled_generation = None;
        self.typed = None;
        self.selection = Selection::none();
        self.focus = None;
        self.focus_place = None;
        self.anchor = None;
        self.path.clear();
    }

    /// The folder the cards and the list stand in, as a file explorer shows one folder at a time.
    ///
    /// A scope that is no folder and the columns, which walk the folders themselves, show a listing instead.
    const fn browsed(&self) -> Root {
        if !matches!(self.layout, Layout::Cards | Layout::List) {
            return Root::Listing;
        }
        self.root_folder()
    }

    /// What the first column shows, and what the cards and the list show.
    const fn root_folder(&self) -> Root {
        match self.place {
            Place::Scope(Scope::All) => Root::Folder(None),
            Place::Scope(Scope::Collection(id)) => Root::Folder(Some(id)),
            Place::Scope(Scope::Unsorted | Scope::Recent) | Place::Trash => Root::Listing,
        }
    }

    /// The listing of a scope or the trash, as the window orders and gathers it.
    const fn flat(&self) -> Shown {
        match self.place {
            Place::Scope(scope) => Shown::Flat {
                listing: Listing {
                    span: Span::Scope(scope),
                    sort: youarel_core::library_rules::Sort::Title,
                    search: String::new(),
                    grouping: self.grouping,
                },
            },
            Place::Trash => Shown::Trash {
                search: String::new(),
            },
        }
    }

    /// The current folder's direct contents or the Trash listing.
    const fn main_shown(&self) -> Shown {
        match self.browsed() {
            Root::Folder(folder) => Shown::Folder {
                folder,
                sort: youarel_core::library_rules::Sort::Title,
                grouping: self.grouping,
            },
            Root::Listing => self.flat(),
        }
    }

    /// What the first column shows: a folder's entries, or a scope's listing.
    const fn root_column(&self) -> Shown {
        match self.root_folder() {
            Root::Folder(folder) => self.column_of(folder),
            Root::Listing => self.flat(),
        }
    }

    /// The column of a folder, or of the top, in the order and under the grouping the settings name.
    const fn column_of(&self, folder: Option<ItemId>) -> Shown {
        Shown::Column {
            folder,
            sort: youarel_core::library_rules::Sort::Title,
            grouping: self.grouping,
        }
    }

    /// A listing laid out as the store holds it now, read again only once the store changed.
    fn laid(&mut self, session: &Session, shown: Shown) -> Result<Arc<Laid>, StoreError> {
        let generation = session.generation();
        if let Some(held) = self
            .laid
            .iter()
            .find(|held| held.shown == shown && held.generation == generation)
        {
            return Ok(Arc::clone(held));
        }
        let laid = Arc::new(listing::lay(session, shown, &heading)?);
        self.laid.retain(|held| held.generation == generation);
        self.laid.push(Arc::clone(&laid));
        if self.laid.len() > MOST_LAID {
            self.laid.remove(0);
        }
        Ok(laid)
    }

    /// The listing the cards or the list show, laid out, with the selection carried over when the listing changed.
    ///
    /// A selection of every row stays every row when the listing only orders or gathers the same rows anew, and a selection of places gives way to the rows it added by identifier.
    fn main_laid(&mut self, session: &Session) -> Result<Arc<Laid>, StoreError> {
        let shown = self.main_shown();
        self.restore_pins(selection_rules::Version {
            listing: shown.fingerprint(),
            generation: session.generation(),
        });
        if let Some(before) = self.shown.as_ref()
            && *before != shown
        {
            let same = before.same_rows(&shown);
            let selection = core::mem::take(&mut self.selection);
            self.selection = if same {
                selection.reordered(shown.fingerprint())
            } else {
                Selection::of(selection.added().iter().copied())
            };
            self.focus_place = None;
            self.viewport = 0;
            self.fresh = true;
        }
        self.shown = Some(shown.clone());
        self.laid(session, shown)
    }

    /// The place of the focused row in a listing: where it stood when it still stands there, and where the listing holds it otherwise.
    fn focus_place(&mut self, session: &Session, laid: &Laid) -> Result<Option<usize>, StoreError> {
        let Some(focus) = self.focus else {
            return Ok(None);
        };
        let entry = entry_of(session.folders(), focus);
        let place = checked_place(session, laid, entry, self.focus_place)?;
        self.focus_place = place;
        Ok(place)
    }

    fn select(
        &mut self,
        id: ItemId,
        extend: Extend,
        hint: Option<usize>,
    ) -> Result<Option<String>> {
        self.reading(|library, session| {
            let laid = library.selection_laid(session)?;
            library.select_in(session, &laid, (id, extend, hint))?;
            library.typed = None;
            Ok(None)
        })
    }

    /// Selects every row of the listing or the active column without naming the rows.
    fn select_all(&mut self) -> Result<Option<String>> {
        self.reading(|library, session| {
            let laid = library.selection_laid(session)?;
            library.set_selection(session, &laid, Selection::all(laid.shown.fingerprint()))?;
            if library.focus.is_none()
                && let Some(row) = laid.row(session, 0)?
            {
                library.focus = Some(entry_id(row.entry));
                library.focus_place = Some(0);
            }
            Ok(None)
        })
    }

    fn step(&mut self, key: Move, across: usize, extend: bool) -> Result<Option<String>> {
        self.reading(|library, session| {
            if library.layout == Layout::Columns {
                library.column_step(session, key, extend)?;
                return Ok(None);
            }
            let laid = library.main_laid(session)?;
            let current = library.focus_place(session, &laid)?;
            // The list shows one entry to a row, whatever the page counts.
            let across = if library.layout == Layout::List {
                1
            } else {
                across
            };
            let next = grouped_moved(current, &laid.sizes(), across, key);
            let row = match next {
                Some(place) => laid.row(session, place)?,
                None => None,
            };
            let anchor = library.range_anchor(session, &laid)?.or(current).or(next);
            if extend {
                library.anchor = library.anchor.or(library.focus);
            }
            library.focus = row.map(|row| entry_id(row.entry));
            library.focus_place = row.and(next);
            let selection = match (extend, anchor, next) {
                (true, Some(anchor), Some(next)) => {
                    Selection::between(laid.shown.fingerprint(), anchor, next)
                }
                _ => Selection::of(library.focus),
            };
            library.set_selection(session, &laid, selection)?;
            if !extend {
                library.anchor = library.focus;
            }
            library.revealing = true;
            Ok(None)
        })
    }

    /// The folder that new folders and links go into, as a file explorer's current folder: the deepest folder the columns open, or the collection the scope shows, and the top from the trash.
    fn here(&self) -> Option<ItemId> {
        let Place::Scope(scope) = self.place else {
            return None;
        };
        let opened = if self.layout == Layout::Columns {
            self.path.iter().rev().find_map(|(entry, _)| match entry {
                Entry::Folder(id) => Some(*id),
                Entry::Shortcut(_) => None,
            })
        } else {
            None
        };
        opened.or(match scope {
            Scope::Collection(id) => Some(id),
            Scope::All | Scope::Unsorted | Scope::Recent => None,
        })
    }

    /// The columns the path opens, laid out, and the place of each entry of the path in its column; the path stops where it leaves them.
    fn walk(&mut self, session: &Session) -> Result<(Vec<Arc<Laid>>, Vec<usize>), StoreError> {
        let mut columns = Vec::new();
        let mut kept = Vec::new();
        let mut next = Some(self.root_column());
        for (entry, hint) in self.path.clone() {
            let Some(shown) = next.take() else {
                break;
            };
            let laid = self.laid(session, shown)?;
            let place = checked_place(session, &laid, entry, Some(hint))?;
            columns.push(laid);
            let Some(place) = place else {
                break;
            };
            kept.push((entry, place));
            // A folder in the trash opens no column, since it opens once it is put back.
            next = match entry {
                Entry::Folder(id) if self.place != Place::Trash => Some(self.column_of(Some(id))),
                Entry::Folder(_) | Entry::Shortcut(_) => None,
            };
        }
        if let Some(shown) = next {
            columns.push(self.laid(session, shown)?);
        }
        let places = kept.iter().map(|(_, place)| *place).collect();
        self.path = kept;
        Ok((columns, places))
    }

    fn column_step(
        &mut self,
        session: &Session,
        key: Move,
        extend: bool,
    ) -> Result<(), StoreError> {
        if extend && !matches!(key, Move::Left | Move::Right) {
            return self.extend_column(session, key);
        }
        let (columns, places) = self.walk(session)?;
        let lengths: Vec<usize> = columns
            .iter()
            .take(places.len().max(1))
            .map(|column| column.total())
            .collect();
        let opens = matches!(self.path.last(), Some((Entry::Folder(_), _)))
            && columns
                .get(self.path.len())
                .is_some_and(|column| column.total() > 0);
        let mut path = Vec::new();
        for (column, place) in columns
            .iter()
            .zip(column_moved(&places, &lengths, opens, key))
        {
            let Some(row) = column.row(session, place)? else {
                break;
            };
            path.push((row.entry, place));
        }
        self.path = path;
        self.focus_path();
        self.revealing = true;
        Ok(())
    }

    /// Focuses and selects the last entry of the column path.
    fn focus_path(&mut self) {
        self.focus = self.path.last().map(|(entry, _)| entry_id(*entry));
        self.anchor = self.focus;
        self.focus_place = None;
        self.select_ids(self.focus);
    }

    /// Where the page showed the column at `depth` before, while that column shows the same listing, or its top.
    fn column_sight(&self, depth: usize, column: &Laid) -> usize {
        match self.column_viewports.get(depth) {
            Some((listing, first)) if *listing == column.shown.fingerprint() => *first,
            Some(_) | None => 0,
        }
    }

    fn keep_column_sight(&mut self, depth: usize, column: &Laid, first: usize) {
        let held = (column.shown.fingerprint(), first);
        if let Some(sight) = self.column_viewports.get_mut(depth) {
            *sight = held;
        } else {
            self.column_viewports
                .resize(depth, (selection_rules::Listing::default(), 0));
            self.column_viewports.push(held);
        }
    }

    /// Opens a shortcut, or the focused one, and counts the visit; a folder opens in the window, as an explorer steps into it.
    fn open_one(
        &mut self,
        id: Option<ItemId>,
        opener: &impl Fn(&str) -> Result<()>,
    ) -> Result<Option<String>> {
        let Some(id) = id.or(self.focus) else {
            return Ok(None);
        };
        // What waits in the trash opens once it is put back, as an explorer keeps what its trash holds closed.
        if let Some(name) = self.session(|session| trashed_name(session, id))? {
            bail!("{name} is in the Trash; put it back to open it");
        }
        let folder = self.reading(|library, session| {
            if !session.folders().shows(id) {
                return Ok(false);
            }
            library.choose_place(Place::Scope(Scope::Collection(id)));
            Ok(true)
        })?;
        if folder {
            return Ok(None);
        }
        let found = self.session(|session| {
            let Some(shortcut) = session.item(id)? else {
                return Ok(None);
            };
            Ok(Some((shortcut.link.value, shortcut.title.value)))
        })?;
        let (link, title) = found.context("the shortcut is gone")?;
        let link = Link::parse(&link)?;
        opener(link.as_str())?;
        self.session(|session| session.visit(id))?;
        self.want_icon(link.as_str())?;
        Ok(Some(format!("Opened {title}")))
    }

    /// The shortcut that holds `link` in `collection` already, which a new link there would repeat.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn holder(&self, link: &Link, collection: Option<ItemId>) -> Result<Option<ItemId>> {
        self.session(|session| session.holder(collection, link.as_str()))
    }

    /// Reveals an entry's ancestors in the list or column path.
    fn open_to(&mut self, session: &Session, id: ItemId) -> Result<()> {
        let entry = entry_of(session.folders(), id);
        let home = match entry {
            Entry::Folder(folder) => session.folders().tree().parent(folder),
            Entry::Shortcut(shortcut) => session
                .item(shortcut)?
                .and_then(|held| session.folders().home(&held)),
        };
        let lineage = session.folders().tree().lineage(home);
        if self.layout == Layout::List {
            self.choose_place(Place::Scope(home.map_or(Scope::All, Scope::Collection)));
            self.focus_on(id);
            return Ok(());
        }
        let Root::Folder(root) = self.root_folder() else {
            return Ok(());
        };
        if self.layout != Layout::Columns {
            return Ok(());
        }
        let below = root.map_or(Some(lineage.as_slice()), |root| {
            lineage
                .iter()
                .position(|folder| *folder == root)
                .and_then(|at| lineage.get(at.saturating_add(1)..))
        });
        if let Some(below) = below {
            self.path = below
                .iter()
                .map(|folder| (Entry::Folder(*folder), 0))
                .chain([(entry, 0)])
                .collect();
        }
        Ok(())
    }

    /// Focuses an entry, selects it alone, and brings it into sight.
    fn focus_on(&mut self, id: ItemId) {
        self.focus = Some(id);
        self.anchor = Some(id);
        self.focus_place = None;
        self.select_ids([id]);
        self.revealing = true;
    }

    /// Selects the shortcut that holds a link already, and says where it sits.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn reveal(&mut self, id: ItemId) -> Result<View> {
        self.reading(|library, session| {
            let home = session
                .item(id)?
                .and_then(|shortcut| session.folders().home(&shortcut));
            let place = session.folders().path(home).join(PATH_SEPARATOR);
            let title = session
                .item(id)?
                .map_or_else(String::new, |shortcut| shortcut.title.value);
            library.focus_on(id);
            library.open_to(session, id)?;
            library.notice = Some(Notice {
                text: if place.is_empty() {
                    format!("Already here: {title}")
                } else {
                    format!("Already in {place}: {title}")
                },
                error: false,
            });
            Ok(())
        })?;
        self.view()
    }

    /// Adds a shortcut and focuses it, or selects the one its collection holds already.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read for the view.
    pub fn add(&mut self, addition: Addition) -> Result<View> {
        let added = self.session(|session| {
            let link = addition.link.as_str();
            // The store may have gained the link while its page was read.
            if let Some(holder) = session.holder(addition.collection, link)? {
                return Ok(Err(holder));
            }
            let id = session.fresh_id()?;
            let reversal = session.edit_remembered(Edit::AddShortcut {
                id,
                link: link.to_owned(),
                title: addition.title.clone(),
                folder: addition.collection,
            })?;
            if let Some(icon) = addition.icon.clone() {
                icons::keep(session, &addition.link, icon)?;
            }
            Ok(Ok((id, reversal)))
        });
        self.notice = Some(match added {
            Ok(Ok((id, reversal))) => {
                self.remember("New Link", vec![reversal]);
                self.focus_on(id);
                // The view names a failure to read the store, so the columns stay as they were.
                drop(self.reading(|library, session| library.open_to(session, id)));
                Notice {
                    text: addition.missing.map_or_else(
                        || format!("Added {}", addition.title),
                        |missing| format!("Added {} ({missing})", addition.title),
                    ),
                    error: false,
                }
            }
            Ok(Err(holder)) => return self.reveal(holder),
            Err(error) => Notice {
                text: format!("{error:#}"),
                error: true,
            },
        });
        self.view()
    }

    fn retitle(&mut self, id: ItemId, title: &str) -> Result<Option<String>> {
        let title = title.trim().to_owned();
        if title.is_empty() {
            bail!("a shortcut needs a title");
        }
        self.edit_remembered("Rename", Edit::Retitle { id, title })?;
        Ok(None)
    }

    fn save_fields(&mut self, fields: &Fields) -> Result<Option<String>> {
        let id = item(&fields.id)?;
        let collection = optional(fields.collection.as_deref())?;
        if fields.title.trim().is_empty() {
            bail!("a shortcut needs a title");
        }
        let changes = self.session(|session| {
            let shortcut = session
                .item(id)?
                .ok_or(StoreError::Edit(EditError::Unknown(Entry::Shortcut(id))))?;
            let mut edits = Vec::new();
            if shortcut.title.value != fields.title {
                edits.push(Edit::Retitle {
                    id,
                    title: fields.title.clone(),
                });
            }
            if shortcut.alias.value != fields.alias {
                edits.push(Edit::Realias {
                    id,
                    alias: fields.alias.clone(),
                });
            }
            if shortcut.note.value != fields.note {
                edits.push(Edit::SetNote {
                    id,
                    note: fields.note.clone(),
                });
            }
            if session.folders().home(&shortcut) != collection {
                edits.push(Edit::Move {
                    entry: Entry::Shortcut(id),
                    folder: collection,
                    slot: Slot::End,
                });
            }
            edits
                .into_iter()
                .map(|edit| session.edit_remembered(edit))
                .collect::<Result<Vec<_>, StoreError>>()
        })?;
        self.remember("Edit", changes);
        Ok(Some(format!("Saved {}", fields.title)))
    }

    fn new_collection(&mut self, name: &str, parent: Option<ItemId>) -> Result<Option<String>> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            bail!("a folder needs a name");
        }
        let (id, reversal) = self.session(|session| {
            let id = session.fresh_id()?;
            let reversal = session.edit_remembered(Edit::AddFolder {
                id,
                name: name.clone(),
                parent,
            })?;
            Ok((id, reversal))
        })?;
        self.remember("New Folder", vec![reversal]);
        self.choose_place(Place::Scope(parent.map_or(Scope::All, Scope::Collection)));
        self.focus_on(id);
        Ok(Some(format!("Created {name}")))
    }

    fn rename_collection(&mut self, id: ItemId, name: &str) -> Result<Option<String>> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            bail!("a folder needs a name");
        }
        self.edit_remembered("Rename", Edit::RenameFolder { id, name })?;
        Ok(None)
    }

    /// Gives a collection the mark a person typed, each `:name:` written as its emoji, as the core admits it, or clears it.
    fn mark_collection(&mut self, id: ItemId, typed: &str) -> Result<Option<String>> {
        let expanded = emoji::expand(typed);
        let mark = mark_rules::admit(&expanded).map_err(|refused| anyhow::anyhow!("{refused}"))?;
        self.edit_remembered(
            "Set Mark",
            Edit::MarkFolder {
                id,
                mark: mark.to_owned(),
            },
        )?;
        Ok(None)
    }

    /// The collections a picker offers from a place on, at most `limit` of them: those whose names a search finds, best first, or every collection in the order of the tree for an empty search.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn choices(&self, query: &str, first: usize, limit: usize) -> Result<Vec<Choice>> {
        let limit = limit.min(MOST_CHOICES);
        self.session(|session| {
            let folders = session.folders();
            let every = tree_order(folders);
            let found: Vec<ItemId> = if query.trim().is_empty() {
                every
            } else {
                let mut ranked: Vec<Rank<usize>> = every
                    .iter()
                    .enumerate()
                    .filter_map(|(place, id)| {
                        let folder = folders.folder(*id)?;
                        let path = folders.path(Some(*id));
                        rows::score_folder(query, folder, &path)
                            .map(|score| Rank::new(score, 0, place))
                    })
                    .collect();
                ranked.sort();
                ranked
                    .into_iter()
                    .filter_map(|rank| every.get(rank.into_id()).copied())
                    .collect()
            };
            Ok(found
                .into_iter()
                .skip(first)
                .take(limit)
                .map(|id| choice(folders, id))
                .collect())
        })
    }

    /// Rows of the listing the cards or the list show, from a place on, which the page asks for as a person scrolls; the next view answers around them.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn page(&mut self, first: usize, limit: usize) -> Result<RowsView> {
        self.reading(|library, session| {
            let laid = library.main_laid(session)?;
            library.viewport = first;
            let total = laid.total();
            let reader = Reader {
                session,
                counts: None,
                selection: &library.selection,
                cut: &library.cut,
            };
            let window = page_rules::window(total, first, limit.min(MOST_ROWS));
            Ok(RowsView {
                key: laid.key(),
                total,
                first,
                rows: row_views(&reader, &laid, window)?,
            })
        })
    }

    /// Entries of a column of the folder walk, from a place on, which the page asks for as a person scrolls the column.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read or the walk holds no such column.
    pub fn column_page(&mut self, depth: usize, first: usize, limit: usize) -> Result<EntriesView> {
        self.reading(|library, session| {
            library.refresh_pins(session.generation());
            let (columns, _) = library.walk(session)?;
            let column = columns
                .get(depth)
                .context("the walk holds no such column")?;
            library.keep_column_sight(depth, column, first);
            let total = column.total();
            let window = page_rules::window(total, first, limit.min(MOST_ROWS));
            Ok(EntriesView {
                key: column.key(),
                total,
                first,
                entries: column_entries(
                    &Reader {
                        session,
                        counts: None,
                        selection: &library.selection,
                        cut: &library.cut,
                    },
                    column,
                    window,
                )?,
            })
        })
    }

    /// The view of the library as the store holds it now.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn view(&mut self) -> Result<View> {
        self.reading(|library, session| library.build(session))
    }
}

#[cfg(test)]
mod tests;
