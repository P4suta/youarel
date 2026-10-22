//! The listings the window shows, each laid out in parts that the window reads a page at a time.
//!
//! A part holds folders the session keeps in memory, by their names, shortcuts the store pages through its indexes, shortcuts the store gathered in memory, or the shortcuts in the trash.
//! Every order is automatic: folders by their names, and shortcuts in the order a person chose among the automatic ones.
//! A listing laid out keeps the state of the store it was read in, so the library reads it again once the store changed.

use crate::view::GroupView;
use std::collections::BTreeSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::sync::{Mutex, PoisonError};
use youarel_core::library_rules::{Grouping, Sort};
use youarel_core::page_rules;
use youarel_core::rank_rules::Rank;
use youarel_core::selection_rules;
use youarel_core::state::{Entry, ItemId};
use youarel_core::typeahead_rules;
use youarel_core::view;
use youarel_store::{Group, Listing, Part as Pageable, Parts, Session, Span, StoreError, Within};

/// What the cards, the list, or a column lists.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(super) enum Shown {
    /// A folder's own folders, then the shortcuts directly in it, gathered under a grouping.
    Folder {
        folder: Option<ItemId>,
        sort: Sort,
        grouping: Grouping,
    },
    /// The list's outline of a folder: each folder's folders, the open ones with their own rows beneath them, then its shortcuts.
    Outline {
        folder: Option<ItemId>,
        sort: Sort,
        open: BTreeSet<ItemId>,
    },
    /// The shortcuts of a scope, or the matches of a search, gathered under a grouping.
    Flat { listing: Listing },
    /// A folder's entries as a column shows them: its folders by their names, then its shortcuts in an order, under the headings of a grouping once there are two of them.
    Column {
        folder: Option<ItemId>,
        sort: Sort,
        grouping: Grouping,
    },
    /// What waits in the trash itself: its folders by their names, then its shortcuts by their titles, or those a search finds among them, best first.
    Trash { search: String },
}

impl Shown {
    /// The listing a selection of places holds to, as a hash of what orders its rows.
    #[must_use]
    pub(super) fn fingerprint(&self) -> selection_rules::Listing {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        selection_rules::Listing(hasher.finish())
    }

    /// Whether another listing holds the same rows, perhaps in another order or under other headings.
    #[must_use]
    pub(super) fn same_rows(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Folder { folder, .. }, Self::Folder { folder: other, .. })
            | (Self::Column { folder, .. }, Self::Column { folder: other, .. }) => folder == other,
            (
                Self::Outline { folder, open, .. },
                Self::Outline {
                    folder: other,
                    open: others,
                    ..
                },
            ) => folder == other && open == others,
            (Self::Flat { listing }, Self::Flat { listing: other }) => {
                listing.span == other.span && listing.search == other.search
            }
            (Self::Trash { search }, Self::Trash { search: other }) => search == other,
            (
                Self::Folder { .. }
                | Self::Outline { .. }
                | Self::Flat { .. }
                | Self::Column { .. }
                | Self::Trash { .. },
                _,
            ) => false,
        }
    }
}

/// One row of a listing: an entry, how deep the outline sets it, and whether a folder of the outline shows its rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Row {
    pub(super) entry: Entry,
    pub(super) depth: usize,
    pub(super) open: Option<bool>,
}

impl Row {
    /// A row outside the outline.
    const fn flat(entry: Entry) -> Self {
        Self {
            entry,
            depth: 0,
            open: None,
        }
    }
}

/// One part of a listing.
#[derive(Clone, Debug)]
enum Part {
    /// Folders in their order.
    Folders(Vec<ItemId>),
    /// One folder of the outline, which shows its rows or not.
    Opened {
        id: ItemId,
        depth: usize,
        open: bool,
    },
    /// Shortcuts the store pages through its indexes.
    Paged {
        part: Pageable,
        depth: usize,
        count: usize,
    },
    /// Shortcuts the store gathered in memory.
    Gathered(Vec<ItemId>),
    /// The shortcuts that wait in the trash themselves, by their titles, which the store pages.
    Trash { count: usize },
}

impl Part {
    /// How many rows the part holds.
    const fn size(&self) -> usize {
        match self {
            Self::Folders(ids) | Self::Gathered(ids) => ids.len(),
            Self::Opened { .. } => 1,
            Self::Paged { count, .. } | Self::Trash { count } => *count,
        }
    }
}

/// The parts of a listing in order, each under its heading or none.
type Headed = Vec<(Part, Option<String>)>;

/// A listing laid out in parts, each under its heading or none, as the store held it in one state.
#[derive(Debug)]
pub(super) struct Laid {
    pub(super) shown: Shown,
    /// How many sessions had changed the store when it was laid out.
    pub(super) generation: u64,
    parts: Headed,
    /// Whether the grouping asked for gave way, since it would gather more shortcuts than the store gathers in memory.
    pub(super) ungrouped: bool,
    /// The rows read last with the place of the first, which answer a read within them without the store, since an index pages by reading the rows before a place.
    recent: Mutex<Option<(usize, Vec<Row>)>>,
}

/// The parts of a listing's shortcuts as the store answers them, each under its heading when the listing shows headings.
fn shortcut_parts(
    listing: &Listing,
    parts: Parts,
    heading: &dyn Fn(&Group) -> String,
    headed: bool,
) -> Headed {
    match parts {
        Parts::Indexed(parts) => parts
            .into_iter()
            .map(|(within, count)| {
                let label = match &within {
                    Within::Whole => None,
                    Within::Period(age) => Some(heading(&Group::Period(*age))),
                    Within::Folder(path) => Some(heading(&Group::Folder(path.clone()))),
                };
                let part = Pageable {
                    span: listing.span,
                    sort: listing.sort,
                    within,
                };
                (
                    Part::Paged {
                        part,
                        depth: 0,
                        count,
                    },
                    label.filter(|_| headed),
                )
            })
            .collect(),
        Parts::Gathered(parts) => parts
            .into_iter()
            .map(|(group, ids)| {
                let label = group.map(|group| heading(&group)).filter(|_| headed);
                (Part::Gathered(ids), label)
            })
            .collect(),
    }
}

/// What an outline shows of every folder: its shortcuts in an order, and its rows beneath it while it is open.
#[derive(Clone, Copy, Debug)]
struct Opening<'a> {
    sort: Sort,
    open: &'a BTreeSet<ItemId>,
}

/// The parts of the outline of a folder at a depth, its open folders' rows beneath them.
fn outline(
    session: &Session,
    folder: Option<ItemId>,
    depth: usize,
    opening: Opening<'_>,
) -> Result<Headed, StoreError> {
    let mut parts = Vec::new();
    for child in session.folders().named(folder) {
        let shows = opening.open.contains(child);
        parts.push((
            Part::Opened {
                id: *child,
                depth,
                open: shows,
            },
            None,
        ));
        if shows {
            parts.extend(outline(
                session,
                Some(*child),
                depth.saturating_add(1),
                opening,
            )?);
        }
    }
    let part = Pageable {
        span: Span::Home(folder),
        sort: opening.sort,
        within: Within::Whole,
    };
    parts.push((
        Part::Paged {
            part,
            depth,
            count: session.home_count(folder)?,
        },
        None,
    ));
    Ok(parts)
}

/// How many headings a column must hold before it shows them, since one heading over all tells nothing.
const HEADINGS_WORTH_SHOWING: usize = 2;

/// The parts of a folder and whether its grouping gave way: its folders by their names, then the shortcuts directly in it in an order, under the headings of a grouping, which a column shows only once there are two of them.
fn folder_parts(
    session: &Session,
    (folder, sort, grouping): (Option<ItemId>, Sort, Grouping),
    heading: &dyn Fn(&Group) -> String,
    column: bool,
) -> Result<(Headed, bool), StoreError> {
    let listing = Listing {
        grouping,
        ..Listing::of(Span::Home(folder), sort)
    };
    let grouped = session.parts(&listing)?;
    let headings = match &grouped.parts {
        Parts::Indexed(parts) => parts.len(),
        Parts::Gathered(parts) => parts.len(),
    };
    let headed = grouping != Grouping::None && (!column || headings >= HEADINGS_WORTH_SHOWING);
    let mut parts = vec![(
        Part::Folders(session.folders().named(folder).to_vec()),
        None,
    )];
    parts.extend(shortcut_parts(&listing, grouped.parts, heading, headed));
    Ok((parts, grouped.ungrouped))
}

/// The items of an order that a search finds, best first, each tie in the order they came in.
fn ranked(found: impl Iterator<Item = (usize, Option<u32>)>, order: &[ItemId]) -> Vec<ItemId> {
    let mut ranks: Vec<Rank<usize>> = found
        .filter_map(|(place, score)| score.map(|score| Rank::new(score, 0, place)))
        .collect();
    ranks.sort();
    ranks
        .into_iter()
        .filter_map(|rank| order.get(rank.into_id()).copied())
        .collect()
}

/// The parts of the trash: its folders by their names, then its shortcuts by their titles, which the store pages, or those a search finds among them, best first.
fn trash_parts(session: &Session, search: &str) -> Result<Headed, StoreError> {
    if search.is_empty() {
        return Ok(vec![
            (Part::Folders(session.folders().trashed()), None),
            (
                Part::Trash {
                    count: session.trashed_count()?,
                },
                None,
            ),
        ]);
    }
    let (folders, shortcuts) = trash_found(session, search)?;
    Ok(vec![
        (Part::Folders(folders), None),
        (Part::Gathered(shortcuts), None),
    ])
}

/// The folders and the shortcuts in the trash itself that a search finds, each best first and each tie by its name.
fn trash_found(session: &Session, search: &str) -> Result<(Vec<ItemId>, Vec<ItemId>), StoreError> {
    let folders = session.folders();
    let trashed = folders.trashed();
    let folder_scores = trashed.iter().enumerate().map(|(place, id)| {
        let score = folders.records().get(id).and_then(|folder| {
            view::score_folder(search, folder, &folders.path(folders.tree().parent(*id)))
        });
        (place, score)
    });
    let found_folders = ranked(folder_scores, &trashed);
    let shortcuts = session.trash_page(0, usize::MAX)?;
    let mut scores = Vec::with_capacity(shortcuts.len());
    for (place, id) in shortcuts.iter().enumerate() {
        let score = session.trashed(*id)?.and_then(|shortcut| {
            let path = folders.path(folders.home(&shortcut));
            view::score_shortcut(search, &shortcut, &path)
        });
        scores.push((place, score));
    }
    Ok((found_folders, ranked(scores.into_iter(), &shortcuts)))
}

/// Lays a listing out as the store holds it now, each heading named by `heading`.
///
/// # Errors
///
/// Returns a [`StoreError`] when the store cannot be read.
pub(super) fn lay(
    session: &Session,
    shown: Shown,
    heading: &dyn Fn(&Group) -> String,
) -> Result<Laid, StoreError> {
    let mut ungrouped = false;
    let parts = match &shown {
        Shown::Folder {
            folder,
            sort,
            grouping,
        } => {
            let (parts, gave_way) =
                folder_parts(session, (*folder, *sort, *grouping), heading, false)?;
            ungrouped = gave_way;
            parts
        }
        Shown::Outline { folder, sort, open } => {
            outline(session, *folder, 0, Opening { sort: *sort, open })?
        }
        Shown::Flat { listing } => {
            let grouped = session.parts(listing)?;
            ungrouped = grouped.ungrouped;
            let headed = listing.grouping != Grouping::None;
            shortcut_parts(listing, grouped.parts, heading, headed)
        }
        Shown::Column {
            folder,
            sort,
            grouping,
        } => {
            let (parts, gave_way) =
                folder_parts(session, (*folder, *sort, *grouping), heading, true)?;
            ungrouped = gave_way;
            parts
        }
        Shown::Trash { search } => trash_parts(session, search)?,
    };
    Ok(Laid {
        shown,
        generation: session.generation(),
        parts,
        ungrouped,
        recent: Mutex::new(None),
    })
}

fn starting_in(
    session: &Session,
    entries: impl Iterator<Item = Entry>,
    letters: &str,
) -> Result<Option<Entry>, StoreError> {
    for entry in entries {
        let name = match entry {
            Entry::Folder(id) => session
                .folders()
                .records()
                .get(&id)
                .map(|folder| folder.name.value.as_str()),
            Entry::Shortcut(id) => {
                let shortcut = super::listed_shortcut(session, id)?;
                if shortcut
                    .is_some_and(|shortcut| typeahead_rules::starts(&shortcut.title.value, letters))
                {
                    return Ok(Some(entry));
                }
                continue;
            }
        };
        if name.is_some_and(|name| typeahead_rules::starts(name, letters)) {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

impl Laid {
    pub(super) fn first_of(
        &self,
        session: &Session,
        entries: &[Entry],
    ) -> Result<Option<usize>, StoreError> {
        let folders: BTreeSet<ItemId> = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Folder(id) => Some(*id),
                Entry::Shortcut(_) => None,
            })
            .collect();
        let shortcuts: BTreeSet<ItemId> = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Shortcut(id) => Some(*id),
                Entry::Folder(_) => None,
            })
            .collect();
        let mut start = 0_usize;
        for (part, _) in &self.parts {
            let found = match part {
                Part::Folders(ids) => ids.iter().position(|id| folders.contains(id)),
                Part::Opened { id, .. } => folders.contains(id).then_some(0),
                Part::Gathered(ids) => ids.iter().position(|id| shortcuts.contains(id)),
                Part::Paged { part, .. } => session.first_of(part, &shortcuts)?,
                Part::Trash { .. } => session.trash_first_of(&shortcuts)?,
            };
            if let Some(found) = found {
                return Ok(Some(start.saturating_add(found)));
            }
            start = start.saturating_add(part.size());
        }
        Ok(None)
    }
    /// The first displayed entry whose folded name starts with the letters.
    pub(super) fn starting(
        &self,
        session: &Session,
        letters: &str,
    ) -> Result<Option<Entry>, StoreError> {
        for (part, _) in &self.parts {
            let found = match part {
                Part::Folders(ids) => {
                    starting_in(session, ids.iter().copied().map(Entry::Folder), letters)?
                }
                Part::Opened { id, .. } => {
                    starting_in(session, core::iter::once(Entry::Folder(*id)), letters)?
                }
                Part::Gathered(ids) => {
                    starting_in(session, ids.iter().copied().map(Entry::Shortcut), letters)?
                }
                Part::Paged { part, .. } => session.starting(part, letters)?.map(Entry::Shortcut),
                Part::Trash { .. } => session.trash_starting(letters)?.map(Entry::Shortcut),
            };
            if found.is_some() {
                return Ok(found);
            }
        }
        Ok(None)
    }

    /// How many rows each part holds, in turn.
    #[must_use]
    pub(super) fn sizes(&self) -> Vec<usize> {
        self.parts.iter().map(|(part, _)| part.size()).collect()
    }

    /// How many rows the listing holds.
    #[must_use]
    pub(super) fn total(&self) -> usize {
        self.parts
            .iter()
            .fold(0_usize, |sum, (part, _)| sum.saturating_add(part.size()))
    }

    /// What tells the listing apart in the state of the store it was read in, which a page the window asks for later answers with.
    #[must_use]
    pub(super) fn key(&self) -> String {
        format!("{}-{}", self.identity(), self.generation)
    }

    pub(super) fn identity(&self) -> String {
        format!("{:016x}", self.shown.fingerprint().0)
    }

    /// How many of the first rows are folders.
    #[must_use]
    pub(super) fn folders(&self) -> usize {
        match self.parts.first() {
            Some((Part::Folders(ids), _)) => ids.len(),
            Some(_) | None => 0,
        }
    }

    /// The headings over the rows, each with the place of its first row and how many rows follow under it.
    #[must_use]
    pub(super) fn groups(&self) -> Vec<GroupView> {
        let mut place = 0_usize;
        let mut groups = Vec::new();
        for (part, label) in &self.parts {
            if let Some(label) = label {
                groups.push(GroupView {
                    label: label.clone(),
                    place,
                    count: part.size(),
                });
            }
            place = place.saturating_add(part.size());
        }
        groups
    }

    /// Every folder row with its place.
    #[must_use]
    pub(super) fn folder_rows(&self) -> Vec<(usize, ItemId)> {
        let mut place = 0_usize;
        let mut rows = Vec::new();
        for (part, _) in &self.parts {
            match part {
                Part::Folders(ids) => {
                    rows.extend(
                        ids.iter()
                            .enumerate()
                            .map(|(at, id)| (place.saturating_add(at), *id)),
                    );
                }
                Part::Opened { id, .. } => rows.push((place, *id)),
                Part::Paged { .. } | Part::Gathered(_) | Part::Trash { .. } => {}
            }
            place = place.saturating_add(part.size());
        }
        rows
    }

    /// The rows from a place on, at most `limit` of them.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub(super) fn rows(
        &self,
        session: &Session,
        first: usize,
        limit: usize,
    ) -> Result<Vec<Row>, StoreError> {
        if let Some(held) = self.recalled(first, limit) {
            return Ok(held);
        }
        let mut rows = Vec::new();
        let Some((mut at, mut offset)) = page_rules::locate(&self.sizes(), first) else {
            return Ok(rows);
        };
        while rows.len() < limit {
            let Some((part, _)) = self.parts.get(at) else {
                break;
            };
            rows.extend(part_rows(
                session,
                part,
                offset,
                limit.saturating_sub(rows.len()),
            )?);
            at = at.saturating_add(1);
            offset = 0;
        }
        // A read of one row, such as a check of a place, keeps the page read before it.
        if rows.len() > 1 {
            *self.recent.lock().unwrap_or_else(PoisonError::into_inner) =
                Some((first, rows.clone()));
        }
        Ok(rows)
    }

    /// The rows of a span that the rows read last hold whole, or hold up to the listing's end.
    fn recalled(&self, first: usize, limit: usize) -> Option<Vec<Row>> {
        let total = self.total();
        let held = |(start, rows): &(usize, Vec<Row>)| {
            let offset = first.checked_sub(*start)?;
            let end = offset.saturating_add(limit);
            let to_the_end = start.saturating_add(rows.len()) >= total;
            if end > rows.len() && !to_the_end {
                return None;
            }
            rows.get(offset..end.min(rows.len())).map(<[Row]>::to_vec)
        };
        self.recent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(held)
    }

    /// The row at a place, or none past the listing's end.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub(super) fn row(&self, session: &Session, place: usize) -> Result<Option<Row>, StoreError> {
        Ok(self.rows(session, place, 1)?.into_iter().next())
    }

    /// The place of an entry, read part by part, or none when the listing does not hold it.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub(super) fn place(
        &self,
        session: &Session,
        entry: Entry,
    ) -> Result<Option<usize>, StoreError> {
        let mut start = 0_usize;
        for (part, _) in &self.parts {
            let found = match (part, entry) {
                (Part::Folders(ids), Entry::Folder(id)) => ids.iter().position(|held| *held == id),
                (Part::Opened { id: held, .. }, Entry::Folder(id)) => (*held == id).then_some(0),
                (Part::Paged { part, .. }, Entry::Shortcut(id)) => session.place_of(part, id)?,
                (Part::Gathered(ids), Entry::Shortcut(id)) => {
                    ids.iter().position(|held| *held == id)
                }
                (Part::Trash { .. }, Entry::Shortcut(id)) => session.trash_place(id)?,
                (
                    Part::Folders(_)
                    | Part::Opened { .. }
                    | Part::Paged { .. }
                    | Part::Gathered(_)
                    | Part::Trash { .. },
                    Entry::Folder(_) | Entry::Shortcut(_),
                ) => None,
            };
            if let Some(offset) = found {
                return Ok(Some(start.saturating_add(offset)));
            }
            start = start.saturating_add(part.size());
        }
        Ok(None)
    }

    /// Hands each row in a window of places to `each` with its place, reading each part the window reaches once.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub(super) fn each(
        &self,
        session: &Session,
        window: Range<usize>,
        mut each: impl FnMut(usize, Entry),
    ) -> Result<(), StoreError> {
        let mut start = 0_usize;
        for (part, _) in &self.parts {
            let end = start.saturating_add(part.size());
            // The places within the part that the window reaches.
            let from = window.start.max(start).saturating_sub(start);
            let to = window.end.min(end).saturating_sub(start);
            if from < to {
                let rows = part_rows(session, part, from, to.saturating_sub(from))?;
                for (offset, row) in (from..).zip(rows) {
                    each(start.saturating_add(offset), row.entry);
                }
            }
            start = end;
        }
        Ok(())
    }
}

/// The rows of one part from a place within it on, at most `limit` of them.
fn part_rows(
    session: &Session,
    part: &Part,
    offset: usize,
    limit: usize,
) -> Result<Vec<Row>, StoreError> {
    Ok(match part {
        Part::Folders(ids) => ids
            .iter()
            .skip(offset)
            .take(limit)
            .map(|id| Row::flat(Entry::Folder(*id)))
            .collect(),
        Part::Opened { id, depth, open } => (offset == 0 && limit > 0)
            .then_some(Row {
                entry: Entry::Folder(*id),
                depth: *depth,
                open: Some(*open),
            })
            .into_iter()
            .collect(),
        Part::Paged { part, depth, .. } => session
            .page(part, offset, limit)?
            .into_iter()
            .map(|id| Row {
                entry: Entry::Shortcut(id),
                depth: *depth,
                open: None,
            })
            .collect(),
        Part::Gathered(ids) => ids
            .iter()
            .skip(offset)
            .take(limit)
            .map(|id| Row::flat(Entry::Shortcut(*id)))
            .collect(),
        Part::Trash { .. } => session
            .trash_page(offset, limit)?
            .into_iter()
            .map(|id| Row::flat(Entry::Shortcut(id)))
            .collect(),
    })
}
