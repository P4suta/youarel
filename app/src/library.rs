//! The window's side of the library: the window sends intents, the core decides what shows, and the window draws the view it gets back.
//!
//! Nothing here touches the webview, so the tests drive it as the window would.

use crate::emoji;
use crate::intent::{
    BrowserChoice, ColumnName, Extend, Fields, GroupName, Intent, SortName, item, items, optional,
    text,
};
use crate::syncing::Syncing;
use crate::themes::{self, ThemeOption};
use crate::view::{
    CollectionCount, ColumnEntry, Context, EntryKind, GroupView, Item, Notice, OutlineRow,
    PATH_SEPARATOR, Sidebar, Step, View, choices, folders_in, site, url,
};
use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use youarel_cli::browsers;
use youarel_cli::{clock, commands, icons, web};
use youarel_core::folder_rules::Seen;
use youarel_core::library_rules::{
    self, Counts, Facts, Grouping, Layout, Move, Period, Query, Scope, added, column_moved,
    grouped, grouped_moved, period,
};
use youarel_core::mark_rules;
use youarel_core::rank_rules::Visits;
use youarel_core::state::{Edit, EditError, Entry, ItemId, Slot, State, Tree};
use youarel_core::sync_rules;
use youarel_link::Link;
use youarel_store::icons::{Icon, IconId};
use youarel_store::settings::{Column, HIDEABLE, Imported, Settings, ThemeChoice};
use youarel_store::{Store, StoreError};

/// The heading of the scope of every shortcut.
const ALL: &str = "All";
/// The heading of the shortcuts in no collection.
const UNSORTED: &str = "Unsorted";
/// The heading of the shortcuts added lately.
const RECENT: &str = "Recently added";
/// The heading of the shortcuts in no collection, when the window groups them by collection.
const NO_COLLECTION: &str = "No collection";
/// How many headings a column must hold before it shows them, since one heading over all tells nothing.
const HEADINGS_WORTH_SHOWING: usize = 2;

/// The heading of the shortcuts added in a period.
const fn heading(age: Period) -> &'static str {
    match age {
        Period::Week => "Past 7 days",
        Period::Month => "Past 30 days",
        Period::Earlier => "Earlier",
    }
}

/// The `data:` address of an icon's PNG, which the window's policy admits as an image and nothing else.
fn data_address(image: &[u8]) -> String {
    format!("data:image/png;base64,{}", BASE64.encode(image))
}

/// What a layout stands in: the first column of the columns, or the folder the cards and the list show.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Root {
    /// The entries of a folder, or of the top for `None`.
    Folder(Option<ItemId>),
    /// The listing of a scope that is no folder, or of a search.
    Listing,
}

/// The state, this device's visits and icons, and the time, as one session read them.
#[derive(Debug)]
struct Snapshot {
    state: State,
    visits: BTreeMap<ItemId, Visits>,
    icons: BTreeMap<String, Icon>,
    now: u64,
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

/// Whether the store holds a write or an operation of this device that no publish to the sync target carried, for a window that syncs.
///
/// A store that cannot tell leaves it to the first merge, which reads the same facts and reports what failed.
fn unpublished(store: &Store, settings: &Settings) -> bool {
    let sync = &settings.sync;
    if !sync.enabled || sync.target.is_empty() {
        return false;
    }
    matches!(
        clock::now().and_then(|now| youarel_cli::sync::unpublished(store, now, &sync.target)),
        Ok(true)
    )
}

/// The library behind the window: the store, the window's choices, the selection, and the last notice.
#[derive(Debug)]
pub struct Library {
    store: Store,
    scope: Scope,
    layout: Layout,
    /// The window's preferences: the layout it opens with, the order, and the themes.
    settings: Settings,
    search: String,
    selected: BTreeSet<ItemId>,
    focus: Option<ItemId>,
    /// The entry selected in each column from the left.
    path: Vec<Entry>,
    notice: Option<Notice>,
    /// Each icon the window has asked for, drawn again by youarel's own decoder, or `None` for one that does not draw.
    drawn: BTreeMap<IconId, Option<String>>,
    /// The links a person opened whose hosts show no icon, which the window asks their sites' favicons for.
    wanted: Vec<Link>,
    /// The folders the list's outline holds open.
    outlined: BTreeSet<ItemId>,
    /// The folders the sidebar's tree holds open.
    unfolded: BTreeSet<ItemId>,
    /// The folder whose place the sidebar's tree opened last, so a folder a person closes stays closed.
    revealed: Option<ItemId>,
    /// The browsers that keep bookmarks on this kind of host, which the settings offer to import.
    browsers: Vec<BrowserChoice>,
    syncing: Syncing,
}

impl Library {
    /// The library over a store, opening with the layout and the order its settings name.
    ///
    /// When the window syncs, the store says at once whether this device holds what no publish to the target carried, which the first sync then publishes.
    #[must_use]
    pub fn new(store: Store) -> Self {
        let (settings, notice) = match store.settings() {
            Ok(settings) => (settings, None),
            Err(error) => (
                Settings::default(),
                Some(Notice {
                    text: format!("{:#}", anyhow::Error::from(error)),
                    error: true,
                }),
            ),
        };
        let mut syncing = Syncing::default();
        if unpublished(&store, &settings) {
            syncing.unpublished();
        }
        Self {
            store,
            scope: Scope::All,
            layout: settings.layout,
            settings,
            search: String::new(),
            selected: BTreeSet::new(),
            focus: None,
            path: Vec::new(),
            notice,
            drawn: BTreeMap::new(),
            wanted: Vec::new(),
            outlined: BTreeSet::new(),
            unfolded: BTreeSet::new(),
            revealed: None,
            // A variable that is no UTF-8 offers no browser, and an import names the variable.
            browsers: browsers::available().map_or_else(
                |_unreadable| Vec::new(),
                |found| found.into_iter().map(BrowserChoice::from).collect(),
            ),
            syncing,
        }
    }

    /// The store, for a sync that runs outside the library's lock.
    #[must_use]
    pub fn store(&self) -> Store {
        self.store.clone()
    }

    /// Whether this device belongs to an account; files that cannot be read count as none, and the sync that follows names them.
    fn has_account(&self) -> bool {
        matches!(youarel_cli::sync::has_account(&self.store), Ok(true))
    }

    /// The sync due at `now` and its target, which this call marks begun, or none.
    pub fn begin_sync(&mut self, now: u64) -> Option<(sync_rules::Sync, String)> {
        let has_account = self.has_account();
        let sync = self.syncing.begin(&self.settings.sync, has_account, now)?;
        Some((sync, self.settings.sync.target.clone()))
    }

    /// Notes what the watch of the shared folder saw at `now`, and answers whether it may bring another device's bundle, which wakes the window to merge.
    pub fn noticed(&mut self, seen: Seen<'_>, now: u64) -> bool {
        self.syncing.noticed(seen, now)
    }

    /// Learns the name of this device's own bundle, whose changes the watch then passes over.
    pub fn learned_own(&mut self, own: String) {
        self.syncing.learned(own);
    }

    /// Records whether the window watches the shared folder, which the settings show.
    pub const fn watching(&mut self, watched: bool) {
        self.syncing.watching(watched);
    }

    /// Whether a sync failed since the last call, so the window starts its watch again.
    pub const fn rewatch(&mut self) -> bool {
        self.syncing.rewatch()
    }

    /// The folder the window watches: the shared folder while the window syncs through one, and none while it syncs through a relay or not at all.
    #[must_use]
    pub fn watch_wanted(&self) -> Option<PathBuf> {
        let sync = &self.settings.sync;
        (sync.enabled && !sync.target.is_empty() && !youarel_cli::sync::is_relay(&sync.target))
            .then(|| PathBuf::from(&sync.target))
    }

    /// Records how a sync that began at `began` ended: whether this device still has something new to say at the target, which a merge reads from the store, or why it failed.
    pub fn end_sync(&mut self, began: u64, outcome: Result<bool>) {
        let succeeded = self
            .syncing
            .end(outcome.map_err(|error| format!("{error:#}")));
        if succeeded {
            self.settings.sync.last = Some(began);
            if let Err(error) = self.save_settings() {
                self.notice = Some(Notice {
                    text: format!("{error:#}"),
                    error: true,
                });
            }
        }
    }

    fn choose_sync(&mut self, enabled: bool, target: &str) -> Result<Option<String>> {
        let target = target.trim();
        if enabled && target.is_empty() {
            bail!("a sync needs a shared folder or a relay's address");
        }
        self.settings.sync.enabled = enabled;
        target.clone_into(&mut self.settings.sync.target);
        self.save_settings()?;
        self.syncing.now();
        Ok(None)
    }

    fn snapshot(&self) -> Result<Snapshot> {
        let now = clock::now()?;
        Ok(self.store.session(now, |session| {
            Ok(Snapshot {
                state: session.state().clone(),
                visits: session.visits().clone(),
                icons: session.icons().clone(),
                now,
            })
        })?)
    }

    /// Runs `work` in one session now.
    fn session<R>(
        &self,
        work: impl FnOnce(&mut youarel_store::Session) -> Result<R, StoreError>,
    ) -> Result<R> {
        let now = clock::now()?;
        Ok(self.store.session(now, work)?)
    }

    fn save_settings(&self) -> Result<()> {
        Ok(self.store.save_settings(&self.settings)?)
    }

    /// Every theme the window can wear: the bundled ones, then the imported ones.
    #[must_use]
    pub fn themes(&self) -> Vec<ThemeOption> {
        themes::all(&self.settings)
    }

    fn choose_theme(
        &mut self,
        follow: bool,
        light: String,
        dark: String,
    ) -> Result<Option<String>> {
        for id in [&light, &dark] {
            if !themes::all(&self.settings)
                .iter()
                .any(|theme| &theme.id == id)
            {
                bail!("no theme is named `{id}`");
            }
        }
        self.settings.theme = ThemeChoice {
            follow,
            light,
            dark,
        };
        self.save_settings()?;
        Ok(None)
    }

    fn import_theme(&mut self, text: &str) -> Result<Option<String>> {
        let theme = youarel_theme::parse(text)?;
        let name = theme.name.clone();
        self.settings.imported.retain(|held| held.id != theme.id);
        self.settings.imported.push(Imported {
            id: theme.id,
            name: theme.name,
            kind: theme.kind,
            palette: theme.palette,
        });
        self.save_settings()?;
        Ok(Some(format!("Imported {name}")))
    }

    fn remove_theme(&mut self, id: &str) -> Result<Option<String>> {
        self.settings.imported.retain(|held| held.id != id);
        let defaults = ThemeChoice::default();
        if self.settings.theme.light == id {
            self.settings.theme.light = defaults.light;
        }
        if self.settings.theme.dark == id {
            self.settings.theme.dark = defaults.dark;
        }
        self.save_settings()?;
        Ok(None)
    }

    /// The images of icons the window lacks, by their names, drawn again first, so the webview's decoders only ever read an image youarel encoded.
    ///
    /// # Errors
    ///
    /// Returns an error when a name names no image or the store cannot be read.
    pub fn icons(&mut self, wanted: &[String]) -> Result<BTreeMap<String, String>> {
        let mut found = BTreeMap::new();
        for name in wanted {
            let image = IconId::parse(name)?;
            let drawn = if let Some(drawn) = self.drawn.get(&image) {
                drawn.clone()
            } else {
                // An image the device no longer keeps shows as no icon, and a later view may name it again.
                let Some(bytes) = self.store.image(image)? else {
                    continue;
                };
                // An image that does not draw shows as no icon, as a shortcut without one does.
                let drawn = youarel_icon::normalize(&bytes)
                    .map_or_else(|_undrawable| None, |redrawn| Some(data_address(&redrawn)));
                self.drawn.insert(image, drawn.clone());
                drawn
            };
            if let Some(address) = drawn {
                found.insert(name.clone(), address);
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

    /// Carries out one intent and answers with the view; a failure becomes the notice.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read for the view.
    pub fn act(&mut self, intent: Intent) -> Result<View> {
        let edits = intent.edits();
        let carried_out = self.carry_out(intent);
        // Every edit runs in one session, so an edit that failed wrote nothing for a publish to carry.
        let edited = edits && carried_out.is_ok();
        self.notice = match carried_out {
            Ok(said) => said.map(|text| Notice { text, error: false }),
            Err(error) => Some(Notice {
                text: format!("{error:#}"),
                error: true,
            }),
        };
        if edited {
            self.syncing.edited(clock::now()?);
        }
        self.view()
    }

    fn carry_out(&mut self, intent: Intent) -> Result<Option<String>> {
        match intent {
            Intent::Show => Ok(None),
            Intent::Scope { scope } => {
                self.scope = scope.scope()?;
                self.forget_selection();
                Ok(None)
            }
            Intent::Layout { layout } => {
                self.layout = layout.into();
                self.prefer(|settings| settings.layout = layout.into())
            }
            Intent::Sort { sort } => self.prefer(|settings| settings.sort = sort.into()),
            Intent::Group { grouping } => {
                self.prefer(|settings| settings.grouping = grouping.into())
            }
            Intent::ShowColumn { column, shown } => {
                self.prefer(|settings| set(&mut settings.hidden, column.into(), !shown))
            }
            Intent::Search { text } => {
                self.search = text;
                self.path.clear();
                Ok(None)
            }
            Intent::Select { id, extend } => self.select(item(&id)?, extend),
            Intent::SelectAll => self.select_all(),
            Intent::Disclose { id, open } => {
                set(&mut self.outlined, item(&id)?, open);
                Ok(None)
            }
            Intent::DiscloseSidebar { id, open } => {
                set(&mut self.unfolded, item(&id)?, open);
                Ok(None)
            }
            Intent::Step { key, across } => self.step(key.into(), across),
            Intent::Column { depth, entry } => {
                self.column(depth, entry.entry()?);
                Ok(None)
            }
            Intent::Open { id } => self.open(optional(id.as_deref())?),
            Intent::Edit { fields } => self.save_fields(&fields),
            Intent::Retitle { id, title } => self.retitle(item(&id)?, &title),
            Intent::File { ids, collection } => {
                self.file(&items(&ids)?, optional(collection.as_deref())?)
            }
            Intent::Remove { ids } => self.remove(&items(&ids)?),
            Intent::NewCollection { name, parent } => {
                self.new_collection(&name, optional(parent.as_deref())?)
            }
            Intent::RenameCollection { id, name } => self.rename_collection(item(&id)?, &name),
            Intent::MarkCollection { id, mark } => self.mark_collection(item(&id)?, &mark),
            Intent::RemoveCollection { id } => self.remove_collection(item(&id)?),
            Intent::MoveFolders { ids, collection } => {
                self.move_folders(&items(&ids)?, optional(collection.as_deref())?)
            }
            Intent::Import { bytes } => {
                let now = clock::now()?;
                Ok(Some(
                    commands::import(&self.store, now, &bytes, None)?.to_string(),
                ))
            }
            Intent::Theme {
                follow,
                light,
                dark,
            } => self.choose_theme(follow, light, dark),
            Intent::ImportTheme { text } => self.import_theme(&text),
            Intent::RemoveTheme { id } => self.remove_theme(&id),
            Intent::ImportBrowser { browser } => {
                let now = clock::now()?;
                let imported = commands::import_browser(&self.store, now, browser.into(), None)?;
                Ok(Some(imported.to_string()))
            }
            Intent::SyncSettings { enabled, target } => self.choose_sync(enabled, &target),
            Intent::SyncNow => {
                self.syncing.now();
                Ok(None)
            }
        }
    }

    /// Changes a preference and saves it.
    fn prefer(&mut self, change: impl FnOnce(&mut Settings)) -> Result<Option<String>> {
        change(&mut self.settings);
        self.save_settings()?;
        Ok(None)
    }

    fn forget_selection(&mut self) {
        self.selected.clear();
        self.focus = None;
        self.path.clear();
    }

    /// The folder the cards and the list stand in, as a file explorer shows one folder at a time.
    ///
    /// A search, a scope that is no folder, and the columns, which walk the folders themselves, show a listing instead.
    const fn browsed(&self) -> Root {
        if !matches!(self.layout, Layout::Cards | Layout::List) || !self.search.is_empty() {
            return Root::Listing;
        }
        match self.scope {
            Scope::All => Root::Folder(None),
            Scope::Collection(id) => Root::Folder(Some(id)),
            Scope::Unsorted | Scope::Recent => Root::Listing,
        }
    }

    /// The folders the cards and the list show before the shortcuts, in their order, with those the list's outline opens.
    fn folders(&self, snapshot: &Snapshot, tree: &Tree) -> Vec<ItemId> {
        if let Some(rows) = self.outline(snapshot, tree) {
            return rows
                .into_iter()
                .filter_map(|(entry, _)| match entry {
                    Entry::Folder(id) => Some(id),
                    Entry::Shortcut(_) => None,
                })
                .collect();
        }
        match self.browsed() {
            Root::Folder(folder) => folders_in(&snapshot.state, tree, folder),
            Root::Listing => Vec::new(),
        }
    }

    /// The shortcuts directly in a folder, or at the top, in the window's order.
    fn direct(&self, snapshot: &Snapshot, tree: &Tree, folder: Option<ItemId>) -> Vec<ItemId> {
        let facts = Facts {
            state: &snapshot.state,
            tree,
            visits: &snapshot.visits,
            now: snapshot.now,
        };
        let scope = folder.map_or(Scope::All, Scope::Collection);
        let mut listed = library_rules::listing(
            facts,
            Query {
                scope: &scope,
                search: "",
                sort: self.settings.sort,
            },
        );
        listed.retain(|id| snapshot.state.folder_of(tree, *id) == folder);
        listed
    }

    /// The rows of the list's outline while the list stands in a folder and gathers nothing, each with its depth.
    ///
    /// Each folder shows its folders, each open one with its own rows beneath it, then its shortcuts, as an explorer's list does.
    fn outline(&self, snapshot: &Snapshot, tree: &Tree) -> Option<Vec<(Entry, usize)>> {
        let Root::Folder(top) = self.browsed() else {
            return None;
        };
        if self.layout != Layout::List || self.gathers() {
            return None;
        }
        let level = |folder: Option<ItemId>| -> Vec<Entry> {
            folders_in(&snapshot.state, tree, folder)
                .into_iter()
                .map(Entry::Folder)
                .chain(
                    self.direct(snapshot, tree, folder)
                        .into_iter()
                        .map(Entry::Shortcut),
                )
                .collect()
        };
        let mut rows = Vec::new();
        let mut pending: Vec<(Entry, usize)> = level(top)
            .into_iter()
            .rev()
            .map(|entry| (entry, 0))
            .collect();
        while let Some((entry, depth)) = pending.pop() {
            rows.push((entry, depth));
            if let Entry::Folder(id) = entry
                && self.outlined.contains(&id)
            {
                let below = depth.saturating_add(1);
                pending.extend(
                    level(Some(id))
                        .into_iter()
                        .rev()
                        .map(|inside| (inside, below)),
                );
            }
        }
        Some(rows)
    }

    /// The folder cards of the cards and the list, each with the shortcuts inside it.
    fn folder_counts(
        &self,
        snapshot: &Snapshot,
        tree: &Tree,
        counts: &Counts,
    ) -> Vec<CollectionCount> {
        self.folders(snapshot, tree)
            .into_iter()
            .filter_map(|id| {
                snapshot.state.folder(id).map(|folder| CollectionCount {
                    id: text(id),
                    name: folder.name.value.clone(),
                    mark: folder.mark.value.clone(),
                    count: counts.collections.get(&id).copied().unwrap_or_default(),
                })
            })
            .collect()
    }

    /// The rows of the list's outline as the window draws them, or none while the list shows no outline.
    fn outline_rows(&self, snapshot: &Snapshot, tree: &Tree) -> Vec<OutlineRow> {
        self.outline(snapshot, tree)
            .unwrap_or_default()
            .into_iter()
            .map(|(entry, depth)| OutlineRow {
                kind: match entry {
                    Entry::Folder(_) => EntryKind::Folder,
                    Entry::Shortcut(_) => EntryKind::Shortcut,
                },
                id: text(entry_id(entry)),
                depth,
                open: self.outlined.contains(&entry_id(entry)),
            })
            .collect()
    }

    /// The columns the list draws after the name: those a person keeps, without the collection while every row sits in the one folder shown.
    fn list_columns(&self) -> Vec<ColumnName> {
        let browsing = matches!(self.browsed(), Root::Folder(_));
        HIDEABLE
            .into_iter()
            .filter(|column| !self.settings.hidden.contains(column))
            .filter(|column| !(browsing && *column == Column::Collection))
            .map(ColumnName::from)
            .collect()
    }

    /// The count beside the heading: every shortcut inside the folder the cards and the list stand in, as the sidebar counts it, or the listing's length.
    fn shown(&self, counts: &Counts, listed: usize) -> usize {
        match self.browsed() {
            Root::Folder(None) => counts.all,
            Root::Folder(Some(id)) => counts.collections.get(&id).copied().unwrap_or_default(),
            Root::Listing => listed,
        }
    }

    /// Every entry the cards and the list show, the folders first, with how many entries each part holds in turn: the folders, then each heading's shortcuts.
    fn entries(&self, snapshot: &Snapshot, tree: &Tree) -> (Vec<ItemId>, Vec<usize>) {
        if let Some(rows) = self.outline(snapshot, tree) {
            let ids: Vec<ItemId> = rows.into_iter().map(|(entry, _)| entry_id(entry)).collect();
            let size = ids.len();
            return (ids, vec![size]);
        }
        let folders = self.folders(snapshot, tree);
        let listing = self.listing(snapshot, tree);
        let mut sizes = vec![folders.len()];
        // Each heading begins a row of the cards, so a row ends where its group does.
        if self.gathers() {
            sizes.extend(
                self.gathered(snapshot, tree, &listing)
                    .iter()
                    .map(|(_, ids)| ids.len()),
            );
        } else {
            sizes.push(listing.len());
        }
        (folders.into_iter().chain(listing).collect(), sizes)
    }

    /// The shortcuts the cards and the list show, in their order: those directly in the folder they stand in, or the whole listing of the scope or the search.
    fn listing(&self, snapshot: &Snapshot, tree: &Tree) -> Vec<ItemId> {
        if let Some(rows) = self.outline(snapshot, tree) {
            return rows
                .into_iter()
                .filter_map(|(entry, _)| match entry {
                    Entry::Shortcut(id) => Some(id),
                    Entry::Folder(_) => None,
                })
                .collect();
        }
        let facts = Facts {
            state: &snapshot.state,
            tree,
            visits: &snapshot.visits,
            now: snapshot.now,
        };
        let mut listed = library_rules::listing(
            facts,
            Query {
                scope: &self.scope,
                search: &self.search,
                sort: self.settings.sort,
            },
        );
        if let Root::Folder(folder) = self.browsed() {
            listed.retain(|id| snapshot.state.folder_of(tree, *id) == folder);
        }
        // The keys move through the shortcuts in the order the headings show them.
        if self.gathers() {
            self.gathered(snapshot, tree, &listed)
                .into_iter()
                .flat_map(|(_, ids)| ids)
                .collect()
        } else {
            listed
        }
    }

    /// Whether the cards and the list gather the listing under headings; the columns walk the folders instead.
    fn gathers(&self) -> bool {
        self.settings.grouping != Grouping::None
    }

    /// Marks the first shortcut of each heading in a column, when its shortcuts fall under more than one.
    fn headed(
        &self,
        (snapshot, tree): (&Snapshot, &Tree),
        column: &[Entry],
        shown: &mut [ColumnEntry],
    ) {
        if !self.gathers() {
            return;
        }
        let ids: Vec<ItemId> = column
            .iter()
            .filter_map(|entry| match entry {
                Entry::Shortcut(id) => Some(*id),
                Entry::Folder(_) => None,
            })
            .collect();
        let groups = self.gathered(snapshot, tree, &ids);
        // A column whose shortcuts all fall under one heading needs none.
        if groups.len() < HEADINGS_WORTH_SHOWING {
            return;
        }
        for (label, members) in groups {
            let first = members.first().map(|id| text(*id));
            if let Some(entry) = shown
                .iter_mut()
                .find(|entry| Some(&entry.id) == first.as_ref())
            {
                entry.heading = Some(label);
            }
        }
    }

    /// The headings of the listing, each with its shortcuts in the listing's order.
    fn gathered(
        &self,
        snapshot: &Snapshot,
        tree: &Tree,
        listed: &[ItemId],
    ) -> Vec<(String, Vec<ItemId>)> {
        let state = &snapshot.state;
        match self.settings.grouping {
            Grouping::None => vec![(String::new(), listed.to_vec())],
            Grouping::Site => grouped(listed, |id| {
                state
                    .shortcut(id)
                    .map_or_else(String::new, |shortcut| site(&shortcut.link.value))
            }),
            // The shortcuts in no collection come first, then the collections by their paths.
            Grouping::Collection => grouped(listed, |id| {
                let folder = state.folder_of(tree, id);
                (
                    folder.is_some(),
                    state.path(tree, folder).join(PATH_SEPARATOR),
                )
            })
            .into_iter()
            .map(|((filed, path), ids)| {
                (
                    if filed {
                        path
                    } else {
                        NO_COLLECTION.to_owned()
                    },
                    ids,
                )
            })
            .collect(),
            Grouping::Added => grouped(listed, |id| {
                period(
                    state.shortcut(id).map_or(0, |shortcut| added(id, shortcut)),
                    snapshot.now,
                )
            })
            .into_iter()
            .map(|(age, ids)| (heading(age).to_owned(), ids))
            .collect(),
        }
    }

    fn select(&mut self, id: ItemId, extend: Extend) -> Result<Option<String>> {
        match extend {
            Extend::Replace => {
                self.selected = BTreeSet::from([id]);
            }
            Extend::Toggle => {
                if !self.selected.remove(&id) {
                    self.selected.insert(id);
                }
            }
            Extend::Range => {
                let snapshot = self.snapshot()?;
                let (listing, _) = self.entries(&snapshot, &snapshot.state.tree());
                let place = |wanted: Option<ItemId>| {
                    wanted.and_then(|wanted| listing.iter().position(|listed| *listed == wanted))
                };
                let to = place(Some(id)).unwrap_or_default();
                let from = place(self.focus).unwrap_or(to);
                let (low, high) = (from.min(to), from.max(to));
                self.selected = listing
                    .iter()
                    .skip(low)
                    .take(high.saturating_sub(low).saturating_add(1))
                    .copied()
                    .collect();
            }
        }
        self.focus = Some(id);
        Ok(None)
    }

    /// Selects every entry the cards or the list show, as an explorer's select all does; the columns select one entry at a time.
    fn select_all(&mut self) -> Result<Option<String>> {
        if self.layout == Layout::Columns {
            return Ok(None);
        }
        let snapshot = self.snapshot()?;
        let (entries, _) = self.entries(&snapshot, &snapshot.state.tree());
        self.focus = self.focus.or_else(|| entries.first().copied());
        self.selected = entries.into_iter().collect();
        Ok(None)
    }

    /// Opens or closes the focused folder of the outline, or steps from a row up to its folder, as an explorer's list does with the arrow keys.
    fn disclose_step(&mut self, rows: &[(Entry, usize)], key: Move) {
        let Some(place) = self
            .focus
            .and_then(|focus| rows.iter().position(|(entry, _)| entry_id(*entry) == focus))
        else {
            return;
        };
        let Some(&(entry, depth)) = rows.get(place) else {
            return;
        };
        match (entry, key) {
            (Entry::Folder(id), Move::Right) => {
                self.outlined.insert(id);
            }
            (Entry::Folder(id), Move::Left) if self.outlined.contains(&id) => {
                self.outlined.remove(&id);
            }
            (_, Move::Left) => {
                // The folder over a row is the nearest row before it one level up.
                let parent = rows
                    .iter()
                    .take(place)
                    .rev()
                    .find(|(_, over)| over.saturating_add(1) == depth)
                    .map(|(found, _)| entry_id(*found));
                if let Some(parent) = parent {
                    self.focus = Some(parent);
                    self.selected = BTreeSet::from([parent]);
                }
            }
            (Entry::Shortcut(_), Move::Right)
            | (_, Move::Up | Move::Down | Move::First | Move::Last) => {}
        }
    }

    fn step(&mut self, key: Move, across: usize) -> Result<Option<String>> {
        let snapshot = self.snapshot()?;
        let tree = snapshot.state.tree();
        if self.layout == Layout::Columns {
            self.column_step(&snapshot, &tree, key);
            return Ok(None);
        }
        if matches!(key, Move::Left | Move::Right)
            && let Some(rows) = self.outline(&snapshot, &tree)
        {
            self.disclose_step(&rows, key);
            return Ok(None);
        }
        let (listing, sizes) = self.entries(&snapshot, &tree);
        let current = self
            .focus
            .and_then(|focus| listing.iter().position(|listed| *listed == focus));
        // The list shows one entry to a row, whatever the page counts.
        let across = if self.layout == Layout::List {
            1
        } else {
            across
        };
        let next = grouped_moved(current, &sizes, across, key).and_then(|place| listing.get(place));
        self.focus = next.copied();
        self.selected = next.copied().into_iter().collect();
        Ok(None)
    }

    /// What the first column shows.
    const fn root_folder(&self) -> Root {
        match self.scope {
            _ if !self.search.is_empty() => Root::Listing,
            Scope::All => Root::Folder(None),
            Scope::Collection(id) => Root::Folder(Some(id)),
            Scope::Unsorted | Scope::Recent => Root::Listing,
        }
    }

    /// The folder that new folders and links go into, as a file explorer's current folder: the deepest folder the columns open, or the collection the scope shows.
    fn here(&self) -> Option<ItemId> {
        let opened = if self.layout == Layout::Columns {
            self.path.iter().rev().find_map(|entry| match entry {
                Entry::Folder(id) => Some(*id),
                Entry::Shortcut(_) => None,
            })
        } else {
            None
        };
        opened.or(match self.scope {
            Scope::Collection(id) => Some(id),
            Scope::All | Scope::Unsorted | Scope::Recent => None,
        })
    }

    /// Opens the columns down to a shortcut, when the folder the first column shows holds it.
    fn open_to(&mut self, state: &State, tree: &Tree, id: ItemId) {
        let Root::Folder(root) = self.root_folder() else {
            return;
        };
        if self.layout != Layout::Columns {
            return;
        }
        let lineage = tree.lineage(state.folder_of(tree, id));
        let below = root.map_or(Some(lineage.as_slice()), |root| {
            lineage
                .iter()
                .position(|folder| *folder == root)
                .and_then(|at| lineage.get(at.saturating_add(1)..))
        });
        if let Some(below) = below {
            self.path = below
                .iter()
                .copied()
                .map(Entry::Folder)
                .chain([Entry::Shortcut(id)])
                .collect();
        }
    }

    /// The entries of the first column: the scope's folder or the top, or the listing when the scope or the search is no folder.
    fn root(&self, snapshot: &Snapshot, tree: &Tree) -> Vec<Entry> {
        match self.root_folder() {
            Root::Folder(folder) => self.opened(snapshot, tree, folder),
            Root::Listing => self
                .listing(snapshot, tree)
                .into_iter()
                .map(Entry::Shortcut)
                .collect(),
        }
    }

    /// The entries a folder shows in its column: in its own order, or, while the window gathers, its folders first and then its shortcuts under their headings.
    fn opened(&self, snapshot: &Snapshot, tree: &Tree, folder: Option<ItemId>) -> Vec<Entry> {
        let entries = snapshot.state.entries(tree, folder);
        if self.settings.grouping == Grouping::None {
            return entries;
        }
        let (folders, shortcuts): (Vec<Entry>, Vec<Entry>) = entries
            .into_iter()
            .partition(|entry| matches!(entry, Entry::Folder(_)));
        let ids: Vec<ItemId> = shortcuts
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Shortcut(id) => Some(id),
                Entry::Folder(_) => None,
            })
            .collect();
        folders
            .into_iter()
            .chain(
                self.gathered(snapshot, tree, &ids)
                    .into_iter()
                    .flat_map(|(_, ids)| ids)
                    .map(Entry::Shortcut),
            )
            .collect()
    }

    /// The columns of the walk and the path cut where it leaves them, each folder's column arranged as it shows.
    fn walked(&self, snapshot: &Snapshot, tree: &Tree) -> (Vec<Vec<Entry>>, Vec<Entry>) {
        walk(self.root(snapshot, tree), &self.path, |folder| {
            self.opened(snapshot, tree, Some(folder))
        })
    }

    fn column_step(&mut self, snapshot: &Snapshot, tree: &Tree, key: Move) {
        let (columns, path) = self.walked(snapshot, tree);
        let places: Vec<usize> = path
            .iter()
            .zip(&columns)
            .filter_map(|(entry, column)| column.iter().position(|listed| listed == entry))
            .collect();
        let lengths: Vec<usize> = columns
            .iter()
            .take(places.len().max(1))
            .map(Vec::len)
            .collect();
        let opens = matches!(path.last(), Some(Entry::Folder(_)))
            && columns
                .get(path.len())
                .is_some_and(|column| !column.is_empty());
        let next = column_moved(&places, &lengths, opens, key);
        self.path = next
            .iter()
            .zip(&columns)
            .filter_map(|(place, column)| column.get(*place).copied())
            .collect();
        self.focus_path();
    }

    /// Focuses the shortcut at the end of the path, if one ends it.
    fn focus_path(&mut self) {
        self.focus = match self.path.last() {
            Some(Entry::Shortcut(id)) => Some(*id),
            Some(Entry::Folder(_)) | None => None,
        };
        self.selected = self.focus.into_iter().collect();
    }

    fn column(&mut self, depth: usize, entry: Entry) {
        self.path.truncate(depth);
        self.path.push(entry);
        self.focus_path();
    }

    /// Opens a shortcut, or the focused one, and counts the visit; a folder opens in the window, as an explorer steps into it.
    fn open(&mut self, id: Option<ItemId>) -> Result<Option<String>> {
        let Some(id) = id.or(self.focus) else {
            return Ok(None);
        };
        let snapshot = self.snapshot()?;
        let tree = snapshot.state.tree();
        if tree.holds(id) {
            // The list's outline opens or closes a folder in place; elsewhere a folder opens as the one the window stands in.
            if self.outline(&snapshot, &tree).is_some() {
                let open = !self.outlined.contains(&id);
                set(&mut self.outlined, id, open);
            } else {
                self.scope = Scope::Collection(id);
                self.forget_selection();
            }
            return Ok(None);
        }
        let found = self.session(|session| {
            session.visit(id);
            Ok(session
                .state()
                .shortcut(id)
                .map(|shortcut| (shortcut.link.value.clone(), shortcut.title.value.clone())))
        })?;
        let (link, title) = found.context("the shortcut is gone")?;
        web::open(&link)?;
        self.want_icon(&link)?;
        Ok(Some(format!("Opened {title}")))
    }

    /// The shortcut that holds `link` in `collection` already, which a new link there would repeat.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn holder(&self, link: &Link, collection: Option<ItemId>) -> Result<Option<ItemId>> {
        let snapshot = self.snapshot()?;
        let tree = snapshot.state.tree();
        Ok(snapshot.state.holding(&tree, collection, link.as_str()))
    }

    /// Selects the shortcut that holds a link already, and says where it sits.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn reveal(&mut self, id: ItemId) -> Result<View> {
        let snapshot = self.snapshot()?;
        let tree = snapshot.state.tree();
        let place = snapshot
            .state
            .path(&tree, snapshot.state.folder_of(&tree, id))
            .join(PATH_SEPARATOR);
        let title = snapshot
            .state
            .shortcut(id)
            .map_or_else(String::new, |shortcut| shortcut.title.value.clone());
        self.focus = Some(id);
        self.selected = BTreeSet::from([id]);
        self.open_to(&snapshot.state, &tree, id);
        self.notice = Some(Notice {
            text: if place.is_empty() {
                format!("Already here: {title}")
            } else {
                format!("Already in {place}: {title}")
            },
            error: false,
        });
        self.view()
    }

    /// Adds a shortcut and focuses it, or selects the one its collection holds already.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read for the view.
    pub fn add(&mut self, addition: Addition) -> Result<View> {
        let added = self.session(|session| {
            let tree = session.state().tree();
            let link = addition.link.as_str();
            // The store may have gained the link while its page was read.
            if let Some(holder) = session.state().holding(&tree, addition.collection, link) {
                return Ok(Err(holder));
            }
            let id = session.fresh_id()?;
            session.edit(Edit::AddShortcut {
                id,
                link: link.to_owned(),
                title: addition.title.clone(),
                folder: addition.collection,
            })?;
            if let Some(icon) = addition.icon.clone() {
                icons::keep(session, &addition.link, icon)?;
            }
            Ok(Ok((id, session.now())))
        });
        self.notice = Some(match added {
            Ok(Ok((id, now))) => {
                self.syncing.edited(now);
                self.focus = Some(id);
                self.selected = BTreeSet::from([id]);
                if let Ok(snapshot) = self.snapshot() {
                    self.open_to(&snapshot.state, &snapshot.state.tree(), id);
                }
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

    fn retitle(&self, id: ItemId, title: &str) -> Result<Option<String>> {
        let title = title.trim().to_owned();
        if title.is_empty() {
            bail!("a shortcut needs a title");
        }
        self.session(|session| session.edit(Edit::Retitle { id, title }))?;
        Ok(None)
    }

    fn save_fields(&self, fields: &Fields) -> Result<Option<String>> {
        let id = item(&fields.id)?;
        let collection = optional(fields.collection.as_deref())?;
        if fields.title.trim().is_empty() {
            bail!("a shortcut needs a title");
        }
        self.session(|session| {
            let state = session.state();
            let shortcut = state
                .shortcut(id)
                .ok_or(StoreError::Edit(EditError::Unknown(Entry::Shortcut(id))))?;
            let tree = state.tree();
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
            if state.folder_of(&tree, id) != collection {
                edits.push(Edit::Move {
                    entry: Entry::Shortcut(id),
                    folder: collection,
                    slot: Slot::End,
                });
            }
            edits.into_iter().try_for_each(|edit| session.edit(edit))
        })?;
        Ok(Some(format!("Saved {}", fields.title)))
    }

    /// Moves shortcuts and folders to the end of a collection, or of the top.
    fn file(&self, ids: &[ItemId], collection: Option<ItemId>) -> Result<Option<String>> {
        let (moved, named) = self.session(|session| {
            let moved = kinds(&session.state().tree(), ids);
            moved.iter().try_for_each(|entry| {
                session.edit(Edit::Move {
                    entry: *entry,
                    folder: collection,
                    slot: Slot::End,
                })
            })?;
            let tree = session.state().tree();
            Ok((
                moved,
                session.state().path(&tree, collection).join(PATH_SEPARATOR),
            ))
        })?;
        // A folder at the top stands in no collection as a shortcut there does, yet reads better as at the top.
        let place = match (
            named.is_empty(),
            moved.iter().any(|entry| matches!(entry, Entry::Folder(_))),
        ) {
            (false, _) => named,
            (true, true) => "the top".to_owned(),
            (true, false) => "no collection".to_owned(),
        };
        Ok(Some(format!("Moved {} to {place}", described(&moved))))
    }

    /// Removes shortcuts and folders, each folder with everything inside it.
    fn remove(&mut self, ids: &[ItemId]) -> Result<Option<String>> {
        let removed = self.session(|session| {
            let removed = kinds(&session.state().tree(), ids);
            removed
                .iter()
                .try_for_each(|entry| session.edit(Edit::Remove { entry: *entry }))?;
            Ok(removed)
        })?;
        for id in ids {
            self.selected.remove(id);
        }
        Ok(Some(format!("Removed {}", described(&removed))))
    }

    fn new_collection(&mut self, name: &str, parent: Option<ItemId>) -> Result<Option<String>> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            bail!("a collection needs a name");
        }
        let id = self.session(|session| {
            let id = session.fresh_id()?;
            session.edit(Edit::AddFolder {
                id,
                name: name.clone(),
                parent,
            })?;
            Ok(id)
        })?;
        // In the columns a folder made where they stand opens in place, as an explorer selects a new folder.
        let opened = parent.and_then(|parent| {
            self.path
                .iter()
                .position(|entry| *entry == Entry::Folder(parent))
                .map(|place| place.saturating_add(1))
        });
        let kept = opened.or_else(|| (self.root_folder() == Root::Folder(parent)).then_some(0));
        match kept {
            Some(kept) if self.layout == Layout::Columns => {
                self.path.truncate(kept);
                self.path.push(Entry::Folder(id));
                self.focus_path();
            }
            Some(_) | None => {
                self.scope = Scope::Collection(id);
                self.forget_selection();
            }
        }
        Ok(Some(format!("Created {name}")))
    }

    fn move_folders(&self, ids: &[ItemId], collection: Option<ItemId>) -> Result<Option<String>> {
        let named = self.session(|session| {
            ids.iter().try_for_each(|id| {
                session.edit(Edit::Move {
                    entry: Entry::Folder(*id),
                    folder: collection,
                    slot: Slot::End,
                })
            })?;
            let tree = session.state().tree();
            Ok(session.state().path(&tree, collection).join(PATH_SEPARATOR))
        })?;
        let place = if named.is_empty() {
            "the top".to_owned()
        } else {
            named
        };
        let moved = if ids.len() == 1 {
            "1 folder".to_owned()
        } else {
            format!("{} folders", ids.len())
        };
        Ok(Some(format!("Moved {moved} to {place}")))
    }

    fn rename_collection(&self, id: ItemId, name: &str) -> Result<Option<String>> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            bail!("a collection needs a name");
        }
        self.session(|session| session.edit(Edit::RenameFolder { id, name }))?;
        Ok(None)
    }

    /// Gives a collection the mark a person typed, each `:name:` written as its emoji, as the core admits it, or clears it.
    fn mark_collection(&self, id: ItemId, typed: &str) -> Result<Option<String>> {
        let expanded = emoji::expand(typed);
        let mark = mark_rules::admit(&expanded).map_err(|refused| anyhow::anyhow!("{refused}"))?;
        self.session(|session| {
            session.edit(Edit::MarkFolder {
                id,
                mark: mark.to_owned(),
            })
        })?;
        Ok(None)
    }

    fn remove_collection(&mut self, id: ItemId) -> Result<Option<String>> {
        self.session(|session| {
            session.edit(Edit::Remove {
                entry: Entry::Folder(id),
            })
        })?;
        if self.scope == Scope::Collection(id) {
            self.scope = Scope::All;
        }
        self.forget_selection();
        Ok(Some("Removed the collection and what it held".to_owned()))
    }

    /// Drops the choices that point at what the state no longer holds.
    fn settle(&mut self, state: &State, tree: &Tree) {
        if let Scope::Collection(id) = self.scope
            && !tree.holds(id)
        {
            self.scope = Scope::All;
        }
        let live = |id: &ItemId| state.shortcut(*id).is_some() || tree.holds(*id);
        self.selected.retain(live);
        self.focus = self.focus.filter(live);
        self.outlined.retain(|id| tree.holds(*id));
        self.unfolded.retain(|id| tree.holds(*id));
        // The sidebar opens the folders over the one the window stands in once, so that folder shows in its tree.
        if let Scope::Collection(id) = self.scope
            && self.revealed != Some(id)
        {
            let lineage = tree.lineage(Some(id));
            let over = lineage.len().saturating_sub(1);
            self.unfolded.extend(lineage.into_iter().take(over));
            self.revealed = Some(id);
        }
    }

    /// The heading of the scope: a folder's own name where the trail above shows the folders over it, and its whole path elsewhere.
    fn title(&self, state: &State, tree: &Tree) -> String {
        match self.scope {
            Scope::All => ALL.to_owned(),
            Scope::Unsorted => UNSORTED.to_owned(),
            Scope::Recent => RECENT.to_owned(),
            Scope::Collection(id) => match (self.browsed(), state.folder(id)) {
                (Root::Folder(_), Some(folder)) => folder.name.value.clone(),
                (Root::Folder(_) | Root::Listing, _) => {
                    state.path(tree, Some(id)).join(PATH_SEPARATOR)
                }
            },
        }
    }

    /// The trail above the listing: the folders the columns open, or the folders from the top down to the one the cards and the list stand in.
    fn trail(&self, state: &State, tree: &Tree) -> Vec<Step> {
        match self.browsed() {
            Root::Folder(folder) => tree
                .lineage(folder)
                .into_iter()
                .enumerate()
                .filter_map(|(depth, id)| {
                    state.folder(id).map(|found| Step {
                        id: text(id),
                        name: found.name.value.clone(),
                        depth,
                    })
                })
                .collect(),
            Root::Listing => trail(state, &self.path),
        }
    }

    /// The headings of the cards and the list, each with how many items follow it, or none while the window gathers nothing.
    fn group_views(&self, snapshot: &Snapshot, tree: &Tree, listing: &[ItemId]) -> Vec<GroupView> {
        if !self.gathers() {
            return Vec::new();
        }
        self.gathered(snapshot, tree, listing)
            .into_iter()
            .map(|(label, ids)| GroupView {
                label,
                count: ids.len(),
            })
            .collect()
    }

    /// The columns of the walk as the window draws them, and the shortcut they end on, which the preview shows; none outside the columns.
    fn walk_view(
        &mut self,
        snapshot: &Snapshot,
        tree: &Tree,
    ) -> (Vec<Vec<ColumnEntry>>, Option<ItemId>) {
        if self.layout != Layout::Columns {
            return (Vec::new(), None);
        }
        let (columns, path) = self.walked(snapshot, tree);
        self.path = path;
        let previewed = match self.path.last() {
            Some(Entry::Shortcut(id)) => Some(*id),
            Some(Entry::Folder(_)) | None => None,
        };
        let shown = columns
            .iter()
            .enumerate()
            .map(|(depth, column)| {
                let mut shown = column_entries((snapshot, tree), column, self.path.get(depth));
                self.headed((snapshot, tree), column, &mut shown);
                shown
            })
            .collect();
        (shown, previewed)
    }

    /// The view of the library as the store holds it now.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn view(&mut self) -> Result<View> {
        let snapshot = self.snapshot()?;
        let state = &snapshot.state;
        let tree = state.tree();
        self.settle(state, &tree);
        let listing = self.listing(&snapshot, &tree);
        let groups = self.group_views(&snapshot, &tree, &listing);
        let counts = library_rules::counts(state, &tree, snapshot.now);
        let item = |id: &ItemId| {
            let context = Context {
                state,
                tree: &tree,
                visits: snapshot.visits.get(id),
            };
            state.shortcut(*id).map(|shortcut| {
                let icon = icon_of(&snapshot.icons, &shortcut.link.value);
                Item::new(context, *id, shortcut, icon)
            })
        };
        let (columns, previewed) = self.walk_view(&snapshot, &tree);
        let preview = previewed.as_ref().and_then(item);
        Ok(View {
            sidebar: Sidebar::new(
                (state, &tree),
                (&self.scope).into(),
                &counts,
                &self.unfolded,
            ),
            title: self.title(state, &tree),
            count: self.shown(&counts, listing.len()),
            layout: self.layout.into(),
            sort: SortName::from(self.settings.sort),
            grouping: GroupName::from(self.settings.grouping),
            search: self.search.clone(),
            folders: self.folder_counts(&snapshot, &tree, &counts),
            outline: self.outline_rows(&snapshot, &tree),
            list_columns: self.list_columns(),
            hidden_columns: self
                .settings
                .hidden
                .iter()
                .copied()
                .map(ColumnName::from)
                .collect(),
            items: listing.iter().filter_map(item).collect(),
            groups,
            columns,
            preview,
            focus: self.focus.map(text),
            selected: self.selected.iter().copied().map(text).collect(),
            notice: self.notice.clone(),
            collections: choices(state, &tree),
            here: self.here().map(text),
            trail: self.trail(state, &tree),
            theme: themes::worn(&self.settings),
            browsers: self.browsers.clone(),
            sync: self.syncing.view(&self.settings.sync, self.has_account()),
        })
    }
}

/// The folders the columns open, from the left, each with the column it sits in.
fn trail(state: &State, path: &[Entry]) -> Vec<Step> {
    path.iter()
        .enumerate()
        .filter_map(|(depth, entry)| match entry {
            Entry::Folder(id) => state.folder(*id).map(|folder| Step {
                id: text(*id),
                name: folder.name.value.clone(),
                depth,
            }),
            Entry::Shortcut(_) => None,
        })
        .collect()
}

/// The entries of one column as the window draws them, with the selected one marked.
fn column_entries(
    (snapshot, tree): (&Snapshot, &Tree),
    column: &[Entry],
    selected: Option<&Entry>,
) -> Vec<ColumnEntry> {
    let state = &snapshot.state;
    column
        .iter()
        .filter_map(|entry| {
            let chosen = selected == Some(entry);
            match entry {
                Entry::Folder(id) => state.folder(*id).map(|folder| ColumnEntry {
                    kind: EntryKind::Folder,
                    id: text(*id),
                    name: folder.name.value.clone(),
                    selected: chosen,
                    opens: !state.entries(tree, Some(*id)).is_empty(),
                    icon: None,
                    site: String::new(),
                    mark: folder.mark.value.clone(),
                    heading: None,
                }),
                Entry::Shortcut(id) => state.shortcut(*id).map(|shortcut| ColumnEntry {
                    kind: EntryKind::Shortcut,
                    id: text(*id),
                    name: shortcut.title.value.clone(),
                    selected: chosen,
                    opens: false,
                    icon: icon_of(&snapshot.icons, &shortcut.link.value),
                    site: url(&shortcut.link.value).name,
                    mark: String::new(),
                    heading: None,
                }),
            }
        })
        .collect()
}

/// The name of the icon a shortcut's host shows on this device, which the window asks [`Library::icons`] to draw.
fn icon_of(icons: &BTreeMap<String, Icon>, link: &str) -> Option<String> {
    match Link::parse(link) {
        Ok(link) => icons.get(link.host()).map(|icon| icon.image.name()),
        // A stored link that no longer parses names no host, so it shows its tile.
        Err(_unparsed) => None,
    }
}

/// How many shortcuts, in words.
fn count(shortcuts: usize) -> String {
    if shortcuts == 1 {
        "1 shortcut".to_owned()
    } else {
        format!("{shortcuts} shortcuts")
    }
}

/// Puts a value in a set or takes it out.
fn set<T: Ord>(values: &mut BTreeSet<T>, value: T, held: bool) {
    if held {
        values.insert(value);
    } else {
        values.remove(&value);
    }
}

/// The identifier of a folder or a shortcut.
const fn entry_id(entry: Entry) -> ItemId {
    match entry {
        Entry::Folder(id) | Entry::Shortcut(id) => id,
    }
}

/// Each identifier as the folder or the shortcut it names, for a selection that may hold both.
fn kinds(tree: &Tree, ids: &[ItemId]) -> Vec<Entry> {
    ids.iter()
        .map(|id| {
            if tree.holds(*id) {
                Entry::Folder(*id)
            } else {
                Entry::Shortcut(*id)
            }
        })
        .collect()
}

/// How many folders and shortcuts, in words.
fn described(entries: &[Entry]) -> String {
    let folders = entries
        .iter()
        .filter(|entry| matches!(entry, Entry::Folder(_)))
        .count();
    let shortcuts = entries.len().saturating_sub(folders);
    let folders_named = if folders == 1 {
        "1 folder".to_owned()
    } else {
        format!("{folders} folders")
    };
    match (folders, shortcuts) {
        (0, _) => count(shortcuts),
        (_, 0) => folders_named,
        _ => format!("{folders_named} and {}", count(shortcuts)),
    }
}

/// The columns a path opens from the first, and the path cut where it leaves them.
///
/// A folder opens its column, which may be empty, and a shortcut ends the path.
fn walk(
    root: Vec<Entry>,
    path: &[Entry],
    open: impl Fn(ItemId) -> Vec<Entry>,
) -> (Vec<Vec<Entry>>, Vec<Entry>) {
    let mut columns = vec![root];
    let mut kept = Vec::new();
    for entry in path {
        if !columns.last().is_some_and(|column| column.contains(entry)) {
            break;
        }
        kept.push(*entry);
        match entry {
            Entry::Folder(id) => columns.push(open(*id)),
            Entry::Shortcut(_) => break,
        }
    }
    (columns, kept)
}

#[cfg(test)]
mod tests;
