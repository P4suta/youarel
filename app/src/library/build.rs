//! The view the window draws, read in one session: the sidebar, a page of the listing the cards or the list show, a page of each column, and the selection counted.

use super::listing::{Laid, Row, Shown};
use super::{
    ALL, LEAD, Library, PAGE, Place, RECENT, Root, TRASH, UNSORTED, entry_id, listable,
    listed_shortcut, selection_shown, trashed_folder,
};
use crate::intent::{ColumnName, GroupName, SortName, text};
use crate::themes;
use crate::view::{
    Choice, CollectionCount, ColumnEntry, ColumnView, Context, EntryKind, Item, ListingView,
    Notice, OpeningView, PATH_SEPARATOR, RowView, SelectionView, Sidebar, Step, View, choice, url,
};
use anyhow::Result;
use std::collections::BTreeSet;
use std::ops::Range;
use youarel_core::library_rules::{Counts, Layout, Scope};
use youarel_core::page_rules::{self, Paging};
use youarel_core::selection_rules::{self, Selection};
use youarel_core::state::{Entry, Folder, ItemId};
use youarel_link::Link;
use youarel_store::folders::Folders;
use youarel_store::settings::{Column, HIDEABLE};
use youarel_store::{Session, StoreError};

/// How a view pages a listing: [`PAGE`] rows, [`LEAD`] of them before a row it brings into sight.
const PAGING: Paging = Paging {
    limit: PAGE,
    lead: LEAD,
};

/// What the notice says when a grouping by site gives way, since it would gather more shortcuts than the store gathers in memory.
const UNGROUPED: &str = "Too many shortcuts here to gather by site, so they show without headings";

/// What the rows of a page read beyond their listing: the session, the counts of the folders when a view read them already, and the selection.
pub(super) struct Reader<'a> {
    pub(super) session: &'a Session,
    pub(super) counts: Option<&'a Counts>,
    pub(super) selection: &'a Selection<ItemId>,
    pub(super) cut: &'a BTreeSet<Entry>,
}

#[derive(Clone, Copy)]
struct Marks {
    selected: bool,
    cut: bool,
}

/// The name of the icon a shortcut's host shows on this device, which the window asks [`Library::icons`] to draw.
fn icon_of(session: &Session, link: &str) -> Result<Option<String>, StoreError> {
    match Link::parse(link) {
        Ok(link) => Ok(session.icon(link.host())?.map(|icon| icon.image.name())),
        // A stored link that no longer parses names no host, so it shows its tile.
        Err(_unparsed) => Ok(None),
    }
}

/// The item the window draws of a shortcut that shows or waits in the trash itself, with the name of its host's icon.
pub(super) fn item(session: &Session, id: ItemId) -> Result<Option<Item>, StoreError> {
    let Some(shortcut) = listed_shortcut(session, id)? else {
        return Ok(None);
    };
    let visits = session.visits_of(id)?;
    let context = Context {
        folders: session.folders(),
        visits: visits.as_ref(),
    };
    let icon = icon_of(session, &shortcut.link.value)?;
    Ok(Some(Item::new(context, id, &shortcut, icon)))
}

/// The card of a folder that shows, with every shortcut inside it, or of one that waits in the trash itself, with every shortcut the trash hides inside it and the path of the folder it came from.
fn folder_count(
    session: &Session,
    counts: &Counts,
    id: ItemId,
) -> Result<Option<(CollectionCount, Option<String>)>, StoreError> {
    let folders = session.folders();
    let card = |folder: &Folder, count| CollectionCount {
        id: text(id),
        name: folder.name.value.clone(),
        mark: folder.mark.value.clone(),
        count,
    };
    if let Some(folder) = folders.folder(id) {
        let count = counts.collections.get(&id).copied().unwrap_or_default();
        return Ok(Some((card(folder, count), None)));
    }
    let Some(folder) = trashed_folder(folders, id) else {
        return Ok(None);
    };
    let origin = folders
        .tree()
        .parent(id)
        .map(|parent| folders.path(Some(parent)).join(PATH_SEPARATOR));
    Ok(Some((card(folder, session.hidden_count(id)?), origin)))
}

/// One row as the window draws it, marked when the selection takes it.
fn row_view(
    session: &Session,
    counts: &Counts,
    row: Row,
    marks: Marks,
) -> Result<RowView, StoreError> {
    Ok(match row.entry {
        Entry::Folder(id) => {
            folder_count(session, counts, id)?.map_or(RowView::Gone, |(folder, origin)| {
                RowView::Folder {
                    folder: Box::new(folder),
                    origin,
                    depth: row.depth,
                    open: row.open,
                    selected: marks.selected,
                    cut: marks.cut,
                }
            })
        }
        Entry::Shortcut(id) => item(session, id)?.map_or(RowView::Gone, |item| RowView::Shortcut {
            item: Box::new(item),
            depth: row.depth,
            selected: marks.selected,
            cut: marks.cut,
        }),
    })
}

/// The rows of a listing at a window of places as the window draws them.
///
/// # Errors
///
/// Returns a [`StoreError`] when the store cannot be read.
pub(super) fn row_views(
    reader: &Reader<'_>,
    laid: &Laid,
    window: Range<usize>,
) -> Result<Vec<RowView>, StoreError> {
    let shown = selection_shown(laid);
    let rows = laid.rows(reader.session, window.start, window.len())?;
    // A page without folders needs no counts, which read every folder's.
    let folders = rows.iter().any(|row| matches!(row.entry, Entry::Folder(_)));
    let read = match reader.counts {
        None if folders => Some(reader.session.counts()?),
        Some(_) | None => None,
    };
    let empty = Counts::default();
    let counts = reader.counts.or(read.as_ref()).unwrap_or(&empty);
    let mut views = Vec::with_capacity(rows.len());
    for (place, row) in (window.start..).zip(rows) {
        let selected = reader.selection.contains(shown, place, entry_id(row.entry));
        views.push(row_view(
            reader.session,
            counts,
            row,
            Marks {
                selected,
                cut: reader.cut.contains(&row.entry),
            },
        )?);
    }
    Ok(views)
}

/// An entry of a column whose folder or shortcut went while the column was read.
fn gone(id: ItemId) -> ColumnEntry {
    ColumnEntry {
        kind: EntryKind::Gone,
        id: text(id),
        name: String::new(),
        selected: false,
        cut: false,
        opens: false,
        icon: None,
        site: String::new(),
        mark: String::new(),
    }
}

/// One entry of a column as the window draws it.
fn column_entry(session: &Session, entry: Entry, marks: Marks) -> Result<ColumnEntry, StoreError> {
    let folders = session.folders();
    Ok(match entry {
        Entry::Folder(id) => match folders.folder(id).or_else(|| trashed_folder(folders, id)) {
            Some(folder) => ColumnEntry {
                kind: EntryKind::Folder,
                id: text(id),
                name: folder.name.value.clone(),
                selected: marks.selected,
                cut: marks.cut,
                // A folder in the trash shows nothing inside it, since it opens once it is put back.
                opens: folders.shows(id)
                    && (!folders.children(Some(id)).is_empty()
                        || session.home_count(Some(id))? > 0),
                icon: None,
                site: String::new(),
                mark: folder.mark.value.clone(),
            },
            None => gone(id),
        },
        Entry::Shortcut(id) => match listed_shortcut(session, id)? {
            Some(shortcut) => ColumnEntry {
                kind: EntryKind::Shortcut,
                id: text(id),
                selected: marks.selected,
                cut: marks.cut,
                opens: false,
                icon: icon_of(session, &shortcut.link.value)?,
                site: url(&shortcut.link.value).name,
                name: shortcut.title.value,
                mark: String::new(),
            },
            None => gone(id),
        },
    })
}

/// The entries of a column at a window of places as the window draws them, the one the walk selects marked.
///
/// # Errors
///
/// Returns a [`StoreError`] when the store cannot be read.
pub(super) fn column_entries(
    reader: &Reader<'_>,
    laid: &Laid,
    window: Range<usize>,
) -> Result<Vec<ColumnEntry>, StoreError> {
    let rows = laid.rows(reader.session, window.start, window.len())?;
    let mut entries = Vec::with_capacity(rows.len());
    for (place, row) in (window.start..).zip(rows) {
        let marks = Marks {
            selected: reader
                .selection
                .contains(selection_shown(laid), place, entry_id(row.entry)),
            cut: reader.cut.contains(&row.entry),
        };
        entries.push(column_entry(reader.session, row.entry, marks)?);
    }
    Ok(entries)
}

/// The folders the columns open, from the left, each with the column it sits in.
fn trail(folders: &Folders, path: &[(Entry, usize)]) -> Vec<Step> {
    path.iter()
        .enumerate()
        .filter_map(|(depth, (entry, _))| match entry {
            Entry::Folder(id) => folders.folder(*id).map(|folder| Step {
                id: text(*id),
                name: folder.name.value.clone(),
                depth,
            }),
            Entry::Shortcut(_) => None,
        })
        .collect()
}

impl Library {
    /// Drops the choices that point at what the window can no longer list: a collection that went or waits in the trash, and what neither shows nor waits in the trash itself.
    pub(super) fn settle(&mut self, session: &Session) -> Result<(), StoreError> {
        let generation = session.generation();
        self.refresh_pins(generation);
        let folders = session.folders();
        if let Place::Scope(Scope::Collection(id)) = self.place
            && !folders.shows(id)
        {
            self.place = Place::Scope(Scope::All);
        }
        if self.settled_generation != Some(generation) {
            let named: Vec<ItemId> = self
                .selection
                .added()
                .iter()
                .chain(self.selection.left())
                .chain(&self.focus)
                .chain(&self.anchor)
                .copied()
                .collect();
            for id in named {
                if !listable(session, self.place, id)? {
                    self.selection.forget(&id);
                    if self.focus == Some(id) {
                        self.focus = None;
                        self.focus_place = None;
                    }
                    if self.anchor == Some(id) {
                        self.anchor = self.focus;
                    }
                }
            }
            self.settled_generation = Some(generation);
        }
        self.outlined.retain(|id| folders.shows(*id));
        self.unfolded.retain(|id| folders.shows(*id));
        // The sidebar opens the folders over the one the window stands in once, so that folder shows in its tree.
        if let Place::Scope(Scope::Collection(id)) = self.place
            && self.revealed != Some(id)
        {
            let lineage = folders.tree().lineage(Some(id));
            let over = lineage.len().saturating_sub(1);
            self.unfolded.extend(lineage.into_iter().take(over));
            self.revealed = Some(id);
        }
        Ok(())
    }

    /// The heading of the place: a folder's own name where the trail above shows the folders over it, and its whole path elsewhere.
    fn title(&self, folders: &Folders) -> String {
        match self.place {
            Place::Scope(Scope::All) => ALL.to_owned(),
            Place::Scope(Scope::Unsorted) => UNSORTED.to_owned(),
            Place::Scope(Scope::Recent) => RECENT.to_owned(),
            Place::Scope(Scope::Collection(id)) => match (self.browsed(), folders.folder(id)) {
                (Root::Folder(_), Some(folder)) => folder.name.value.clone(),
                (Root::Folder(_) | Root::Listing, _) => folders.path(Some(id)).join(PATH_SEPARATOR),
            },
            Place::Trash => TRASH.to_owned(),
        }
    }

    /// The trail above the listing: the folders the columns open, or the folders from the top down to the one the cards and the list stand in.
    fn trail(&self, folders: &Folders) -> Vec<Step> {
        match self.browsed() {
            Root::Folder(folder) => folders
                .tree()
                .lineage(folder)
                .into_iter()
                .enumerate()
                .filter_map(|(depth, id)| {
                    folders.folder(id).map(|found| Step {
                        id: text(id),
                        name: found.name.value.clone(),
                        depth,
                    })
                })
                .collect(),
            Root::Listing => trail(folders, &self.path),
        }
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

    /// The count beside the heading: every shortcut inside the folder the window stands in, as the sidebar counts it, or the length of the listing shown.
    fn count(&self, counts: &Counts, listed: usize) -> usize {
        let root = if self.layout == Layout::Columns {
            self.root_folder()
        } else {
            self.browsed()
        };
        match root {
            Root::Folder(None) => counts.all,
            Root::Folder(Some(id)) => counts.collections.get(&id).copied().unwrap_or_default(),
            Root::Listing => listed,
        }
    }

    /// The selection counted over a listing: every row it takes there, and the folders among them and among the rows it adds from elsewhere.
    fn selection_view(&self, session: &Session, laid: Option<&Laid>) -> SelectionView {
        let shown = laid.map_or_else(selection_rules::Shown::default, selection_shown);
        let rows = laid.map_or_else(Vec::new, Laid::folder_rows);
        let listed: BTreeSet<ItemId> = rows.iter().map(|(_, id)| *id).collect();
        let taken = rows
            .iter()
            .filter(|(place, id)| self.selection.contains(shown, *place, *id))
            .count();
        let elsewhere = session
            .folders()
            .records()
            .keys()
            .filter(|id| {
                session.folders().holds(**id)
                    && self.selection.added().binary_search(id).is_ok()
                    && !listed.contains(id)
            })
            .count();
        SelectionView {
            count: self.selection.count(shown),
            folders: taken.saturating_add(elsewhere),
        }
    }

    /// Says that a grouping gave way, unless the notice says something else already.
    fn note_ungrouped(&mut self, laid: &Laid) {
        if laid.ungrouped && self.notice.is_none() {
            self.notice = Some(Notice {
                text: UNGROUPED.to_owned(),
                error: false,
            });
        }
    }

    /// The page of the listing the cards or the list show: around the row the view brings into sight, or where the page showed before.
    fn listing_view(
        &mut self,
        session: &Session,
        counts: &Counts,
    ) -> Result<(ListingView, SelectionView), StoreError> {
        let laid = self.main_laid(session)?;
        self.note_ungrouped(&laid);
        let total = laid.total();
        let focus = self.focus_place(session, &laid)?;
        let reveal = focus.filter(|_| self.revealing);
        let first = page_rules::start(total, self.viewport, reveal, PAGING);
        if reveal.is_some() {
            self.viewport = first;
        }
        let reader = Reader {
            session,
            counts: Some(counts),
            selection: &self.selection,
            cut: &self.cut,
        };
        let rows = row_views(&reader, &laid, page_rules::window(total, first, PAGE))?;
        let selection = self.selection_view(session, Some(&laid));
        let listing = ListingView {
            identity: laid.identity(),
            anchor: None,
            key: laid.key(),
            total,
            first,
            rows,
            focus,
            reveal,
            fresh: core::mem::take(&mut self.fresh),
            folders: laid.folders(),
            groups: laid.groups(),
            outline: matches!(laid.shown, Shown::Outline { .. }),
        };
        Ok((listing, selection))
    }

    /// The columns of the walk as the window draws them, each a page around its selected entry or where the page showed it before, and the shortcut they end on, which the preview shows; none outside the columns.
    fn columns_view(
        &mut self,
        session: &Session,
    ) -> Result<(Vec<ColumnView>, Option<Item>), StoreError> {
        if self.layout != Layout::Columns {
            return Ok((Vec::new(), None));
        }
        let (columns, places) = self.walk(session)?;
        let mut shown = Vec::with_capacity(columns.len());
        for (depth, column) in columns.iter().enumerate() {
            self.note_ungrouped(column);
            let selected = places.get(depth).copied();
            let total = column.total();
            let sight = self.column_sight(depth, column);
            let first =
                page_rules::start(total, sight, selected.filter(|_| self.revealing), PAGING);
            let entries = column_entries(
                &Reader {
                    session,
                    counts: None,
                    selection: &self.selection,
                    cut: &self.cut,
                },
                column,
                page_rules::window(total, first, PAGE),
            )?;
            shown.push(ColumnView {
                identity: column.identity(),
                anchor: None,
                key: column.key(),
                total,
                first,
                entries,
                selected,
                groups: column.groups(),
            });
        }
        let preview = match self.path.last() {
            Some((Entry::Shortcut(id), _)) => item(session, *id)?,
            Some((Entry::Folder(_), _)) | None => None,
        };
        Ok((shown, preview))
    }

    /// The view of the library as one session of the store reads it.
    pub(super) fn build(&mut self, session: &Session) -> Result<View> {
        self.settle(session)?;
        let counts = session.counts()?;
        let (listing, selection) = if self.layout == Layout::Columns {
            let laid = self.selection_laid(session)?;
            (
                ListingView::default(),
                self.selection_view(session, Some(&laid)),
            )
        } else {
            self.listing_view(session, &counts)?
        };
        let (columns, preview) = self.columns_view(session)?;
        self.revealing = false;
        let listed = if self.layout == Layout::Columns {
            columns.first().map_or(0, |column| column.total)
        } else {
            listing.total
        };
        let folders = session.folders();
        Ok(View {
            cut: self.cut.len(),
            undo: self.history.last().map(|step| step.action.to_owned()),
            redo: self.history.next().map(|step| step.action.to_owned()),
            back: self.places.last().is_some(),
            forward: self.places.next().is_some(),
            sidebar: Sidebar::new(folders, (&self.place).into(), &counts, &self.unfolded),
            title: self.title(folders),
            count: self.count(&counts, listed),
            layout: self.layout.into(),
            sort: SortName::from(self.settings.sort),
            grouping: GroupName::from(self.settings.grouping),
            search: self.search.clone(),
            listing,
            list_columns: self.list_columns(),
            hidden_columns: self
                .settings
                .hidden
                .iter()
                .copied()
                .map(ColumnName::from)
                .collect(),
            columns,
            preview,
            focus: self.focus.map(text),
            selection,
            notice: self.notice.clone(),
            here: self.here().map(|id| Choice {
                id: text(id),
                path: folders.path(Some(id)).join(PATH_SEPARATOR),
            }),
            trail: self.trail(folders),
            theme: themes::worn(&self.settings),
            browsers: self.browsers.clone(),
            sync: self.syncing.view(&self.settings.sync, self.has_account()),
            opening: OpeningView {
                layout: self.settings.layout.into(),
                start: self
                    .settings
                    .start
                    .filter(|id| folders.shows(*id))
                    .map(|id| choice(folders, id)),
            },
        })
    }
}
