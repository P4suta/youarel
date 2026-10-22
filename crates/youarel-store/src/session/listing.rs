//! A session's listings: the shortcuts of a scope or a folder in an order, a page at a time, the parts a grouping gathers them in, the matches of a search, and what the trash holds.
//!
//! An order that an index keeps reads that index from its start, so a page costs the rows before it and its own.
//! It reads a record only where two rows tie on what their keys keep, or for a time that an identifier does not carry.
//! A grouping by age or by folder reads each group as a part of its own through the indexes, and a grouping by site gathers the shortcuts in memory.
//! A search reads every shortcut of a scope of fewer than [`search_rules::SCAN_LIMIT`] and at most [`CANDIDATE_LIMIT`] through the index of words otherwise.

use super::{Session, decode_shortcut, site};
use crate::StoreError;
use crate::database::{BY_OPENED, SHORTCUTS, VISITS, count, failed, index, open, read_visits};
use redb::{
    AccessGuard, Range, ReadableTable as _, ReadableTableMetadata as _, StorageError, Table,
};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::iter::Peekable;
use std::ops::Bound;
use youarel_core::index_rules::{self, HOME_KEY, Standing, Table as Index};
use youarel_core::library_rules::{
    self, Grouping, Order, Period, Scope, Sort, period, period_bounds, recent_since,
};
use youarel_core::rank_rules::{DAY, Rank, Visits, frecency};
use youarel_core::search_rules::{self, Bounds, CANDIDATE_LIMIT, Strategy};
use youarel_core::state::{Entry, ItemId, Presence, Shortcut};
use youarel_core::typeahead_rules;
use youarel_core::view;

/// The most shortcuts a listing gathers in memory for a grouping that no index keeps, and the most of a run whose kept starts tie that it sorts by their records.
pub const ON_DEMAND_LIMIT: usize = 50_000;
/// What stands between the names of the folders of a path, which a grouping by folder names its headings with.
pub const PATH_SEPARATOR: &str = " / ";

/// The periods in the order their headings show.
const PERIODS: [Period; 3] = [Period::Week, Period::Month, Period::Earlier];
/// The bytes of a time at the start of a key.
const TIME: usize = 8;

/// Which shortcuts a listing holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Span {
    /// The shortcuts that show in a scope.
    Scope(Scope),
    /// The shortcuts that show directly in a folder, or at the top.
    Home(Option<ItemId>),
}

/// Which records a listing keeps: those that show, or those in the trash themselves.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum Kept {
    Shown,
    Trashed,
}

/// What a listing shows: its shortcuts, their order, the search that ranks them, and the grouping that gathers them.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Listing {
    pub span: Span,
    pub sort: Sort,
    /// The typed search, which ranks the matches when it is not empty.
    pub search: String,
    pub grouping: Grouping,
}

impl Listing {
    /// The shortcuts of a span in an order, without a search or a grouping.
    #[must_use]
    pub const fn of(span: Span, sort: Sort) -> Self {
        Self {
            span,
            sort,
            search: String::new(),
            grouping: Grouping::None,
        }
    }
}

/// What part of a listing's shortcuts an indexed part reads.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Within {
    /// Every shortcut of the listing.
    Whole,
    /// The shortcuts added within a period.
    Period(Period),
    /// The shortcuts in no folder, as `None`, or in the folders of one path, as the names from the top joined by [`PATH_SEPARATOR`].
    Folder(Option<String>),
}

/// A part of a listing that the indexes page: its shortcuts, their order, and what part of them it reads.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Part {
    pub span: Span,
    pub sort: Sort,
    pub within: Within,
}

/// The heading a gathered part of a listing falls under.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Group {
    /// The shortcuts added within a period.
    Period(Period),
    /// The shortcuts of a site.
    Site(String),
    /// The shortcuts in no folder, as `None`, or in the folders of one path.
    Folder(Option<String>),
}

/// The parts of a listing in order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Parts {
    /// Parts the indexes page, each with how many shortcuts it holds.
    Indexed(Vec<(Within, usize)>),
    /// Parts gathered in memory, each under its heading or under none, with its shortcuts in order.
    Gathered(Vec<(Option<Group>, Vec<ItemId>)>),
}

/// The parts of a listing, and whether its grouping gave way because it would gather more than [`ON_DEMAND_LIMIT`] shortcuts in memory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Grouped {
    pub parts: Parts,
    pub ungrouped: bool,
}

/// The times a listing keeps its shortcuts from: from the first one on, and before the second when there is one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Times {
    from: u64,
    until: Option<u64>,
}

impl Times {
    /// Every time.
    const EVERY: Self = Self {
        from: 0,
        until: None,
    };

    /// Whether a time lies within these times.
    fn holds(self, time: u64) -> bool {
        time >= self.from && self.until.is_none_or(|until| time < until)
    }
}

/// Which shortcuts a listing reads, as the indexes file them.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Members {
    /// Every shortcut that shows.
    All,
    /// The shortcuts that show in a folder and in every folder inside it.
    Subtree(ItemId),
    /// The shortcuts that show directly in some homes, none of which holds another.
    Homes(Vec<Option<ItemId>>),
}

/// The shortcuts a stream reads: its members, and the times they were added within when it keeps only some.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Filter {
    members: Members,
    times: Option<Times>,
}

/// A table of index rows.
type Keys<'t> = Table<'t, &'static [u8], ()>;
/// A table of values by their keys.
type Values<'t> = Table<'t, &'static [u8], &'static [u8]>;
/// A table of counts by their keys.
type Counted<'t> = Table<'t, &'static [u8], u64>;
/// The rows of an index table within bounds.
type Rows<'a> = Range<'a, &'static [u8], ()>;
/// One row as a range yields it.
type RowResult<'a> = Result<(AccessGuard<'a, &'static [u8]>, AccessGuard<'a, ()>), StorageError>;
/// A shortcut a stream finds, or why it failed.
type Found = Result<ItemId, StoreError>;
/// The shortcuts of a listing in their order, read as they are asked for.
type Stream<'a> = Box<dyn Iterator<Item = Found> + 'a>;
/// A shortcut a stream finds with what it orders by, or why it failed.
type Ordered<T> = Result<(T, ItemId), StoreError>;

/// An order of texts, how long a run of tied kept starts it sorts by the records, and which records it keeps.
#[derive(Clone, Copy, Debug)]
struct TextOrder {
    sort: Sort,
    bound: usize,
    kept: Kept,
}

/// A search: what was typed, and how it finds the shortcuts it ranks.
#[derive(Clone, Debug)]
struct Search<'q> {
    query: &'q str,
    strategy: Strategy<index_rules::Token>,
}

/// A shortcut a search found: its score, its frecency on this device, and its record.
struct Scored {
    score: u32,
    frecency: u32,
    id: ItemId,
    shortcut: Shortcut,
}

/// The tables a listing reads, each opened once for one call.
struct Tables<'t> {
    added: Keys<'t>,
    home_added: Keys<'t>,
    title: Keys<'t>,
    home_title: Keys<'t>,
    link: Keys<'t>,
    home_link: Keys<'t>,
    tokens: Keys<'t>,
    trash: Keys<'t>,
    opened: Keys<'t>,
    homes: Counted<'t>,
    days: Counted<'t>,
    shortcuts: Values<'t>,
    visits: Values<'t>,
}

/// The rows of an index table within bounds.
fn within<'a>(table: &'a Keys<'_>, bounds: &Bounds) -> Result<Rows<'a>, StoreError> {
    let end = bounds
        .end
        .as_deref()
        .map_or(Bound::Unbounded, Bound::Excluded);
    table
        .range::<&[u8]>((Bound::Included(bounds.start.as_slice()), end))
        .map_err(failed)
}

/// The bounds of the rows of a home's index of times, or of the whole index for an empty prefix, within times.
fn added_bounds(prefix: &[u8], times: Times) -> Bounds {
    let start = [prefix, &times.from.to_be_bytes()].concat();
    let end = times.until.map_or_else(
        || search_rules::starting(prefix).end,
        |until| Some([prefix, &until.to_be_bytes()].concat()),
    );
    Bounds { start, end }
}

/// The identifier that ends a key.
fn identified(key: &[u8]) -> Result<ItemId, StoreError> {
    index_rules::identified(key).ok_or(StoreError::Missing("an identifier"))
}

/// The time and the identifier of a key of an index of times.
fn added_parts(key: &[u8]) -> Result<(u64, ItemId), StoreError> {
    let time = key
        .first_chunk::<TIME>()
        .map(|bytes| u64::from_be_bytes(*bytes))
        .ok_or(StoreError::Missing("a time"))?;
    Ok((time, identified(key)?))
}

/// A key after the prefix of its home, when its index files it under one.
fn after_home(key: &[u8], prefixed: bool) -> &[u8] {
    if prefixed {
        key.get(HOME_KEY..).unwrap_or_default()
    } else {
        key
    }
}

/// A count as the session answers it.
fn size(count: u64) -> Result<usize, StoreError> {
    usize::try_from(count).map_err(|_huge| StoreError::Missing("a count that fits"))
}

/// How often and how lately this device opened a shortcut.
fn visited(tables: &Tables<'_>, id: ItemId) -> Result<Visits, StoreError> {
    tables
        .visits
        .get(id.0.as_slice())
        .map_err(failed)?
        .map_or(Ok(Visits::default()), |held| read_visits(held.value()))
}

/// How many live shortcuts sit directly in a home.
fn sitting(tables: &Tables<'_>, home: Option<ItemId>) -> Result<usize, StoreError> {
    let key = index_rules::home_key(home);
    tables
        .homes
        .get(key.as_slice())
        .map_err(failed)?
        .map_or(Ok(0), |held| size(held.value()))
}

/// The shortcuts with a word equal to a typed word, then those with a word that starts with it, each once, at most `limit` of them.
fn candidates(tables: &Tables<'_>, word: &[u8], limit: usize) -> Result<Vec<ItemId>, StoreError> {
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    for bounds in [search_rules::equal(word), search_rules::starting(word)] {
        for row in within(&tables.tokens, &bounds)? {
            if found.len() >= limit {
                return Ok(found);
            }
            let (key, _) = row.map_err(failed)?;
            let id = identified(key.value())?;
            if seen.insert(id) {
                found.push(id);
            }
        }
    }
    Ok(found)
}

/// Counts the rows of an index table within bounds.
fn counted(table: &Keys<'_>, bounds: &Bounds) -> Result<usize, StoreError> {
    let mut rows = 0_usize;
    for row in within(table, bounds)? {
        row.map_err(failed)?;
        rows = rows.saturating_add(1);
    }
    Ok(rows)
}

/// The time a day starts at.
fn day_start(day: u64) -> Result<u64, StoreError> {
    day.checked_mul(DAY)
        .ok_or(StoreError::Missing("a day that starts at a time"))
}

/// How many shortcuts were added within times: the whole days among them by the counts per day, and the rest of the days at either end by their rows, so a count costs a row per day and the rows of two days at most.
///
/// # Errors
///
/// Returns a [`StoreError`] when the store cannot be read.
fn counted_within(added: &Keys<'_>, days: &Counted<'_>, times: Times) -> Result<usize, StoreError> {
    let Some((first, end)) = index_rules::whole_days(times.from, times.until) else {
        return counted(added, &added_bounds(&[], times));
    };
    let before = Times {
        from: times.from,
        until: Some(day_start(first)?),
    };
    let mut total = counted(added, &added_bounds(&[], before))?;
    if let (Some(end), Some(until)) = (end, times.until) {
        let after = Times {
            from: day_start(end)?,
            until: Some(until),
        };
        total = total.saturating_add(counted(added, &added_bounds(&[], after))?);
    }
    let start = first.to_be_bytes();
    let stop = end.map(u64::to_be_bytes);
    let finish = stop
        .as_ref()
        .map_or(Bound::Unbounded, |stop| Bound::Excluded(stop.as_slice()));
    for entry in days
        .range::<&[u8]>((Bound::Included(start.as_slice()), finish))
        .map_err(failed)?
    {
        let (_, held) = entry.map_err(failed)?;
        total = total.saturating_add(size(held.value())?);
    }
    Ok(total)
}

/// The items of a stream from a place on, at most `limit` of them, with the first failure of any row read on the way.
fn window<T>(
    stream: impl Iterator<Item = Result<T, StoreError>>,
    first: usize,
    limit: usize,
) -> Result<Vec<T>, StoreError> {
    let mut page = Vec::new();
    for (place, found) in stream.enumerate() {
        if page.len() >= limit {
            break;
        }
        let id = found?;
        if place >= first {
            page.push(id);
        }
    }
    Ok(page)
}

/// The rows of one index of times, the earliest first, each with its time.
struct Added<'a> {
    rows: Rows<'a>,
    prefixed: bool,
}

impl Added<'_> {
    /// A row read into its time and its identifier.
    fn read(&self, row: Option<RowResult<'_>>) -> Option<Ordered<u64>> {
        Some(
            row?.map_err(failed)
                .and_then(|(key, _)| added_parts(after_home(key.value(), self.prefixed))),
        )
    }
}

impl Iterator for Added<'_> {
    type Item = Ordered<u64>;

    fn next(&mut self) -> Option<Self::Item> {
        let row = self.rows.next();
        self.read(row)
    }
}

impl DoubleEndedIterator for Added<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let row = self.rows.next_back();
        self.read(row)
    }
}

/// The rows of one index of texts in their order, each with its key after the home.
struct Keyed<'a> {
    rows: Rows<'a>,
    prefixed: bool,
}

impl Iterator for Keyed<'_> {
    type Item = Ordered<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        let row = self.rows.next()?;
        Some(row.map_err(failed).and_then(|(key, _)| {
            let kept = after_home(key.value(), self.prefixed);
            Ok((kept.to_vec(), identified(kept)?))
        }))
    }
}

/// The rows of several sources in one order, each source in that order already.
struct Merge<T, I> {
    heads: BinaryHeap<Reverse<(T, ItemId, usize)>>,
    sources: Vec<I>,
}

impl<T: Ord, I: Iterator<Item = Ordered<T>>> Merge<T, I> {
    fn new(mut sources: Vec<I>) -> Result<Self, StoreError> {
        let mut heads = BinaryHeap::with_capacity(sources.len());
        for (at, source) in sources.iter_mut().enumerate() {
            if let Some(first) = source.next() {
                let (order, id) = first?;
                heads.push(Reverse((order, id, at)));
            }
        }
        Ok(Self { heads, sources })
    }
}

impl<T: Ord, I: Iterator<Item = Ordered<T>>> Iterator for Merge<T, I> {
    type Item = Ordered<T>;

    fn next(&mut self) -> Option<Self::Item> {
        let Reverse((order, id, at)) = self.heads.pop()?;
        if let Some(next) = self.sources.get_mut(at).and_then(Iterator::next) {
            match next {
                Ok((after, following)) => self.heads.push(Reverse((after, following, at))),
                Err(error) => return Some(Err(error)),
            }
        }
        Some(Ok((order, id)))
    }
}

/// The rows of one index of times the latest first, each run of one time in the order of its identifiers.
struct Newest<I: Iterator> {
    rows: Peekable<I>,
    /// The rest of a run of one time, its first identifier last.
    run: Vec<ItemId>,
    time: u64,
}

impl<I: Iterator<Item = Ordered<u64>>> Iterator for Newest<I> {
    type Item = Ordered<Reverse<u64>>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(id) = self.run.pop() {
            return Some(Ok((Reverse(self.time), id)));
        }
        let (time, id) = match self.rows.next()? {
            Ok(row) => row,
            Err(error) => return Some(Err(error)),
        };
        self.time = time;
        self.run.push(id);
        while let Some(Ok((next, _))) = self.rows.peek() {
            if *next != time {
                break;
            }
            if let Some(Ok((_, tied))) = self.rows.next() {
                self.run.push(tied);
            }
        }
        self.run.pop().map(|first| Ok((Reverse(time), first)))
    }
}

/// The rows of an order of texts, each run that ties on the kept start sorted by the records' own order while it reaches no further than a bound.
struct Runs<'a, 't, I: Iterator> {
    session: &'a Session,
    tables: &'a Tables<'t>,
    order: TextOrder,
    rows: Peekable<I>,
    pending: VecDeque<ItemId>,
}

impl<I: Iterator<Item = Ordered<Vec<u8>>>> Runs<'_, '_, I> {
    /// A run sorted by the records' own order, when it reaches no further than the bound.
    fn sorted(&self, mut run: Vec<ItemId>) -> Result<Vec<ItemId>, StoreError> {
        if run.len() <= self.order.bound {
            let mut failure = None;
            run.sort_by_cached_key(|id| {
                match self.session.order_of(self.tables, *id, self.order) {
                    Ok(order) => order,
                    Err(error) => {
                        failure.get_or_insert(error);
                        None
                    }
                }
            });
            if let Some(error) = failure {
                return Err(error);
            }
        }
        Ok(run)
    }
}

impl<I: Iterator<Item = Ordered<Vec<u8>>>> Iterator for Runs<'_, '_, I> {
    type Item = Found;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(id) = self.pending.pop_front() {
            return Some(Ok(id));
        }
        let (key, id) = match self.rows.next()? {
            Ok(row) => row,
            Err(error) => return Some(Err(error)),
        };
        let start = index_rules::sort_part(&key).to_vec();
        let mut run = vec![id];
        while let Some(Ok((next, _))) = self.rows.peek() {
            if index_rules::sort_part(next) != start.as_slice() {
                break;
            }
            if let Some(Ok((_, tied))) = self.rows.next() {
                run.push(tied);
            }
        }
        match self.sorted(run) {
            Ok(sorted) => self.pending = sorted.into(),
            Err(error) => return Some(Err(error)),
        }
        self.pending.pop_front().map(Ok)
    }
}

impl Session {
    /// The tables a listing reads.
    fn tables(&self) -> Result<Tables<'_>, StoreError> {
        Ok(Tables {
            added: open(&self.transaction, index(Index::Added))?,
            home_added: open(&self.transaction, index(Index::FolderAdded))?,
            title: open(&self.transaction, index(Index::Title))?,
            home_title: open(&self.transaction, index(Index::FolderTitle))?,
            link: open(&self.transaction, index(Index::Link))?,
            home_link: open(&self.transaction, index(Index::FolderLink))?,
            tokens: open(&self.transaction, index(Index::Token))?,
            trash: open(&self.transaction, index(Index::Trash))?,
            opened: open(&self.transaction, BY_OPENED)?,
            homes: open(&self.transaction, count(Index::HomeCount))?,
            days: open(&self.transaction, count(Index::DayCount))?,
            shortcuts: open(&self.transaction, SHORTCUTS)?,
            visits: open(&self.transaction, VISITS)?,
        })
    }

    /// A shortcut that a listing keeps, as the tables of a listing read it: one that shows, or one in the trash itself.
    fn keeping(
        &self,
        tables: &Tables<'_>,
        id: ItemId,
        kept: Kept,
    ) -> Result<Option<Shortcut>, StoreError> {
        let Some(held) = tables.shortcuts.get(id.0.as_slice()).map_err(failed)? else {
            return Ok(None);
        };
        let shortcut = decode_shortcut(held.value(), &self.table)?;
        let keeps = match kept {
            Kept::Shown => self.folders.standing(&shortcut) == Standing::Shown,
            Kept::Trashed => shortcut.presence.value == Presence::Trashed,
        };
        Ok(keeps.then_some(shortcut))
    }

    /// A shortcut that shows, as the tables of a listing read it.
    fn live(&self, tables: &Tables<'_>, id: ItemId) -> Result<Option<Shortcut>, StoreError> {
        self.keeping(tables, id, Kept::Shown)
    }

    /// What the reference model sorts a shortcut by in an order of texts, or none for a shortcut the order does not keep.
    fn order_of(
        &self,
        tables: &Tables<'_>,
        id: ItemId,
        order: TextOrder,
    ) -> Result<Option<Order>, StoreError> {
        Ok(self
            .keeping(tables, id, order.kept)?
            .map(|shortcut| library_rules::order(order.sort, id, &shortcut, 0)))
    }

    /// When a shortcut was added: the time its identifier carries, or the time its record gives, or none for a shortcut that is gone.
    fn added_at(&self, tables: &Tables<'_>, id: ItemId) -> Result<Option<u64>, StoreError> {
        match id.time() {
            Some(time) => Ok(Some(time)),
            None => Ok(self
                .live(tables, id)?
                .map(|shortcut| library_rules::added(id, &shortcut))),
        }
    }

    /// Whether a shortcut was added within the times a filter keeps, which every one is when the filter keeps every time.
    fn timely(
        &self,
        tables: &Tables<'_>,
        times: Option<Times>,
        id: ItemId,
    ) -> Result<bool, StoreError> {
        match times {
            None => Ok(true),
            Some(times) => Ok(self
                .added_at(tables, id)?
                .is_some_and(|time| times.holds(time))),
        }
    }

    /// Whether a shortcut that shows lies among a filter's members and within its times.
    fn admits(&self, filter: &Filter, id: ItemId, shortcut: &Shortcut) -> bool {
        let home = self.folders.home(shortcut);
        let member = match &filter.members {
            Members::All => true,
            Members::Subtree(folder) => {
                home.is_some_and(|home| self.folders.tree().within(home, *folder))
            }
            Members::Homes(homes) => homes.contains(&home),
        };
        member
            && filter
                .times
                .is_none_or(|times| times.holds(library_rules::added(id, shortcut)))
    }

    /// The homes that members sit in: the top and every folder that shows, a folder that shows and every folder that shows inside it, or the homes named.
    fn homes_of(&self, members: &Members) -> Vec<Option<ItemId>> {
        let tree = self.folders.tree();
        match members {
            Members::All => core::iter::once(None)
                .chain(
                    tree.folders()
                        .map(|(id, _)| id)
                        .filter(|id| tree.shows(*id))
                        .map(Some),
                )
                .collect(),
            Members::Subtree(folder) => tree
                .folders()
                .map(|(id, _)| id)
                .filter(|id| tree.shows(*id) && tree.within(*id, *folder))
                .map(Some)
                .collect(),
            Members::Homes(homes) => homes.clone(),
        }
    }

    /// The path of a folder that a grouping by folder names its heading with.
    fn path_named(&self, folder: ItemId) -> String {
        self.folders.path(Some(folder)).join(PATH_SEPARATOR)
    }

    /// The members of a span.
    fn members(span: Span) -> Members {
        match span {
            Span::Scope(Scope::All | Scope::Recent) => Members::All,
            Span::Scope(Scope::Unsorted) => Members::Homes(vec![None]),
            Span::Scope(Scope::Collection(folder)) => Members::Subtree(folder),
            Span::Home(home) => Members::Homes(vec![home]),
        }
    }

    /// The filter of a part of a listing: its span's members and the times it keeps, narrowed to a period or to the homes of a folder's path.
    fn filter(&self, span: Span, within_part: &Within) -> Filter {
        let members = Self::members(span);
        let since = match span {
            Span::Scope(Scope::Recent) => Some(recent_since(self.now)),
            Span::Scope(Scope::All | Scope::Unsorted | Scope::Collection(_)) | Span::Home(_) => {
                None
            }
        };
        let (members, age) = match within_part {
            Within::Whole => (members, None),
            Within::Period(age) => (members, Some(*age)),
            Within::Folder(path) => {
                let homes = self
                    .homes_of(&members)
                    .into_iter()
                    .filter(|home| home.map(|folder| self.path_named(folder)) == *path)
                    .collect();
                (Members::Homes(homes), None)
            }
        };
        let times = match (since, age) {
            (None, None) => None,
            (Some(from), None) => Some(Times { from, until: None }),
            (None, Some(age)) => {
                let (from, until) = period_bounds(age, self.now);
                Some(Times { from, until })
            }
            (Some(since), Some(age)) => {
                let (from, until) = period_bounds(age, self.now);
                Some(Times {
                    from: from.max(since),
                    until,
                })
            }
        };
        Filter { members, times }
    }

    /// How many shortcuts a filter keeps.
    fn count(&self, tables: &Tables<'_>, filter: &Filter) -> Result<usize, StoreError> {
        match (&filter.members, filter.times) {
            (Members::All, None) => size(tables.added.len().map_err(failed)?),
            (Members::All, Some(times)) => counted_within(&tables.added, &tables.days, times),
            (members, None) => self
                .homes_of(members)
                .into_iter()
                .try_fold(0_usize, |sum, home| {
                    Ok(sum.saturating_add(sitting(tables, home)?))
                }),
            (members, Some(times)) => {
                self.homes_of(members)
                    .into_iter()
                    .try_fold(0_usize, |sum, home| {
                        let bounds = added_bounds(&index_rules::home_key(home), times);
                        Ok(sum.saturating_add(counted(&tables.home_added, &bounds)?))
                    })
            }
        }
    }

    /// The sources of a filter in an index of times: the whole index for every shortcut, and the rows of each home otherwise.
    fn added_sources<'a>(
        &self,
        tables: &'a Tables<'_>,
        filter: &Filter,
    ) -> Result<Vec<Added<'a>>, StoreError> {
        let times = filter.times.unwrap_or(Times::EVERY);
        match &filter.members {
            Members::All => Ok(vec![Added {
                rows: within(&tables.added, &added_bounds(&[], times))?,
                prefixed: false,
            }]),
            members @ (Members::Subtree(_) | Members::Homes(_)) => self
                .homes_of(members)
                .into_iter()
                .map(|home| {
                    let bounds = added_bounds(&index_rules::home_key(home), times);
                    Ok(Added {
                        rows: within(&tables.home_added, &bounds)?,
                        prefixed: true,
                    })
                })
                .collect(),
        }
    }

    /// The shortcuts a filter keeps, the earliest added first.
    fn oldest<'a>(
        &self,
        tables: &'a Tables<'_>,
        filter: &Filter,
    ) -> Result<Stream<'a>, StoreError> {
        let merged = Merge::new(self.added_sources(tables, filter)?)?;
        Ok(Box::new(merged.map(|found| found.map(|(_, id)| id))))
    }

    /// The shortcuts a filter keeps, the latest added first and those added at one time in the order of their identifiers.
    fn newest<'a>(
        &self,
        tables: &'a Tables<'_>,
        filter: &Filter,
    ) -> Result<Stream<'a>, StoreError> {
        let sources = self
            .added_sources(tables, filter)?
            .into_iter()
            .map(|source| Newest {
                rows: source.rev().peekable(),
                run: Vec::new(),
                time: 0,
            })
            .collect();
        let merged = Merge::new(sources)?;
        Ok(Box::new(merged.map(|found| found.map(|(_, id)| id))))
    }

    /// The shortcuts a filter keeps in an order of texts, each run that ties on the kept start sorted by the records while it reaches no further than a bound.
    fn texts<'a>(
        &'a self,
        tables: &'a Tables<'_>,
        filter: &Filter,
        order: TextOrder,
    ) -> Result<Stream<'a>, StoreError> {
        let (whole, homed) = match order.sort {
            Sort::Link => (&tables.link, &tables.home_link),
            Sort::Title | Sort::Newest | Sort::Oldest | Sort::Opened => {
                (&tables.title, &tables.home_title)
            }
        };
        let sources = match &filter.members {
            Members::All => vec![Keyed {
                rows: within(whole, &search_rules::starting(&[]))?,
                prefixed: false,
            }],
            members @ (Members::Subtree(_) | Members::Homes(_)) => self
                .homes_of(members)
                .into_iter()
                .map(|home| {
                    let bounds = search_rules::starting(&index_rules::home_key(home));
                    Ok(Keyed {
                        rows: within(homed, &bounds)?,
                        prefixed: true,
                    })
                })
                .collect::<Result<Vec<_>, StoreError>>()?,
        };
        let times = filter.times;
        let kept = Merge::new(sources)?.filter_map(move |found| match found {
            Ok((key, id)) => match self.timely(tables, times, id) {
                Ok(true) => Some(Ok((key, id))),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            },
            Err(error) => Some(Err(error)),
        });
        Ok(Box::new(Runs {
            session: self,
            tables,
            order,
            rows: kept.peekable(),
            pending: VecDeque::new(),
        }))
    }

    /// The shortcuts a filter keeps, those this device opened the latest first, then the others the latest added first.
    fn opened<'a>(
        &'a self,
        tables: &'a Tables<'_>,
        filter: &Filter,
    ) -> Result<Stream<'a>, StoreError> {
        let mut visited = Vec::new();
        let mut seen = BTreeSet::new();
        for row in tables.opened.iter().map_err(failed)? {
            let (key, _) = row.map_err(failed)?;
            let (inverted, id) = added_parts(key.value())?;
            let last = !inverted;
            // A visit at time zero ranks as none, as the reference model reads a shortcut never opened.
            if last == 0 {
                continue;
            }
            seen.insert(id);
            if let Some(shortcut) = self.live(tables, id)?
                && self.admits(filter, id, &shortcut)
            {
                visited.push((
                    Reverse(last),
                    Reverse(library_rules::added(id, &shortcut)),
                    id,
                ));
            }
        }
        visited.sort_unstable();
        let rest = self
            .newest(tables, filter)?
            .filter(move |found| !matches!(found, Ok(id) if seen.contains(id)));
        Ok(Box::new(
            visited.into_iter().map(|(_, _, id)| Ok(id)).chain(rest),
        ))
    }

    /// The shortcuts a filter keeps in an order.
    fn ordered<'a>(
        &'a self,
        tables: &'a Tables<'_>,
        filter: &Filter,
        sort: Sort,
    ) -> Result<Stream<'a>, StoreError> {
        match sort {
            Sort::Oldest => self.oldest(tables, filter),
            Sort::Newest => self.newest(tables, filter),
            Sort::Title | Sort::Link => self.texts(
                tables,
                filter,
                TextOrder {
                    sort,
                    bound: ON_DEMAND_LIMIT,
                    kept: Kept::Shown,
                },
            ),
            Sort::Opened => self.opened(tables, filter),
        }
    }

    /// The parts of a listing: the matches of a search gathered under the grouping, and otherwise the parts the indexes page, or the sites gathered while they stay within a bound.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn parts(&self, listing: &Listing) -> Result<Grouped, StoreError> {
        self.parts_within(listing, ON_DEMAND_LIMIT)
    }

    /// The parts of a listing, a grouping by site gathered while the listing holds at most `bound` shortcuts.
    fn parts_within(&self, listing: &Listing, bound: usize) -> Result<Grouped, StoreError> {
        let tables = self.tables()?;
        let filter = self.filter(listing.span, &Within::Whole);
        if !listing.search.is_empty() {
            let total = self.count(&tables, &filter)?;
            let search = Search {
                query: &listing.search,
                strategy: search_rules::strategy(total, &listing.search),
            };
            let found = self.matches(&tables, &filter, search)?;
            return Ok(Grouped {
                parts: Parts::Gathered(self.gathered(found, listing.grouping)),
                ungrouped: false,
            });
        }
        let parts = match listing.grouping {
            Grouping::None => vec![(Within::Whole, self.count(&tables, &filter)?)],
            Grouping::Added => {
                let mut parts = Vec::new();
                for age in PERIODS {
                    let part = Within::Period(age);
                    let held = self.count(&tables, &self.filter(listing.span, &part))?;
                    if held > 0 {
                        parts.push((part, held));
                    }
                }
                parts
            }
            Grouping::Collection => {
                let mut paths: BTreeMap<Option<String>, Vec<Option<ItemId>>> = BTreeMap::new();
                for home in self.homes_of(&filter.members) {
                    let path = home.map(|folder| self.path_named(folder));
                    paths.entry(path).or_default().push(home);
                }
                let mut parts = Vec::new();
                for (path, homes) in paths {
                    let narrowed = Filter {
                        members: Members::Homes(homes),
                        times: filter.times,
                    };
                    let held = self.count(&tables, &narrowed)?;
                    if held > 0 {
                        parts.push((Within::Folder(path), held));
                    }
                }
                parts
            }
            Grouping::Site => {
                let total = self.count(&tables, &filter)?;
                if total > bound {
                    return Ok(Grouped {
                        parts: Parts::Indexed(vec![(Within::Whole, total)]),
                        ungrouped: true,
                    });
                }
                let mut found = Vec::with_capacity(total);
                for id in self.ordered(&tables, &filter, listing.sort)? {
                    let id = id?;
                    if let Some(shortcut) = self.live(&tables, id)? {
                        found.push((id, shortcut));
                    }
                }
                return Ok(Grouped {
                    parts: Parts::Gathered(self.gathered(found, Grouping::Site)),
                    ungrouped: false,
                });
            }
        };
        Ok(Grouped {
            parts: Parts::Indexed(parts),
            ungrouped: false,
        })
    }

    /// Shortcuts in an order gathered under a grouping: one part under no heading without one, and otherwise a part under each heading, in the order of the headings, each in the order given.
    fn gathered(
        &self,
        found: Vec<(ItemId, Shortcut)>,
        grouping: Grouping,
    ) -> Vec<(Option<Group>, Vec<ItemId>)> {
        let key = |id: ItemId, shortcut: &Shortcut| match grouping {
            Grouping::None => None,
            Grouping::Added => Some(Group::Period(period(
                library_rules::added(id, shortcut),
                self.now,
            ))),
            Grouping::Site => Some(Group::Site(site(&shortcut.link.value))),
            Grouping::Collection => Some(Group::Folder(
                self.folders
                    .home(shortcut)
                    .map(|folder| self.path_named(folder)),
            )),
        };
        let mut parts: BTreeMap<Option<Group>, Vec<ItemId>> = BTreeMap::new();
        if grouping == Grouping::None {
            parts.insert(None, Vec::new());
        }
        for (id, shortcut) in found {
            parts.entry(key(id, &shortcut)).or_default().push(id);
        }
        parts.into_iter().collect()
    }

    /// The shortcuts a search finds among those a filter keeps, each with its score, its frecency on this device, and its record, in no order.
    fn scored(
        &self,
        tables: &Tables<'_>,
        filter: &Filter,
        search: Search<'_>,
    ) -> Result<Vec<Scored>, StoreError> {
        let Search { query, strategy } = search;
        let candidates = match strategy {
            Strategy::Scan => self
                .oldest(tables, filter)?
                .collect::<Result<Vec<_>, _>>()?,
            Strategy::Index(word) => candidates(tables, word.as_str().as_bytes(), CANDIDATE_LIMIT)?,
            Strategy::Nothing => Vec::new(),
        };
        let mut scored = Vec::new();
        for id in candidates {
            let Some(shortcut) = self.live(tables, id)? else {
                continue;
            };
            if !self.admits(filter, id, &shortcut) {
                continue;
            }
            let path = self.folders.path(self.folders.home(&shortcut));
            if let Some(score) = view::score_shortcut(query, &shortcut, &path) {
                let visits = visited(tables, id)?;
                scored.push(Scored {
                    score,
                    frecency: frecency(visits, self.now),
                    id,
                    shortcut,
                });
            }
        }
        Ok(scored)
    }

    /// The shortcuts a search finds among those a filter keeps, the best first, each with its record.
    fn matches(
        &self,
        tables: &Tables<'_>,
        filter: &Filter,
        search: Search<'_>,
    ) -> Result<Vec<(ItemId, Shortcut)>, StoreError> {
        let mut ranked: Vec<(Rank<ItemId>, Shortcut)> = self
            .scored(tables, filter, search)?
            .into_iter()
            .map(|found| {
                (
                    Rank::new(found.score, found.frecency, found.id),
                    found.shortcut,
                )
            })
            .collect();
        ranked.sort_by_key(|(rank, _)| *rank);
        Ok(ranked
            .into_iter()
            .map(|(rank, shortcut)| (rank.into_id(), shortcut))
            .collect())
    }

    /// The filter of an indexed part of a listing.
    fn part_filter(&self, part: &Part) -> Filter {
        self.filter(part.span, &part.within)
    }

    /// A page of an indexed part of a listing: its shortcuts from a place on, at most `limit` of them.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn page(&self, part: &Part, first: usize, limit: usize) -> Result<Vec<ItemId>, StoreError> {
        let tables = self.tables()?;
        let filter = self.part_filter(part);
        window(self.ordered(&tables, &filter, part.sort)?, first, limit)
    }

    /// The place of a shortcut in an indexed part of a listing, or none when the part does not hold it.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn place_of(&self, part: &Part, id: ItemId) -> Result<Option<usize>, StoreError> {
        self.find_place(part, |found| found == id)
    }

    /// The first place occupied by any of these shortcuts, or none when none belong to the part.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the part cannot be read.
    pub fn first_of(
        &self,
        part: &Part,
        ids: &BTreeSet<ItemId>,
    ) -> Result<Option<usize>, StoreError> {
        if ids.is_empty() {
            return Ok(None);
        }
        self.find_place(part, |found| ids.contains(&found))
    }

    fn find_place(
        &self,
        part: &Part,
        matches: impl Fn(ItemId) -> bool,
    ) -> Result<Option<usize>, StoreError> {
        let tables = self.tables()?;
        let filter = self.part_filter(part);
        for (place, found) in self.ordered(&tables, &filter, part.sort)?.enumerate() {
            if matches(found?) {
                return Ok(Some(place));
            }
        }
        Ok(None)
    }

    /// The first shortcut of a part whose folded title starts with the letters, found through the title index.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the index, records, or visits cannot be read.
    pub fn starting(&self, part: &Part, letters: &str) -> Result<Option<ItemId>, StoreError> {
        let tables = self.tables()?;
        let filter = self.part_filter(part);
        let prefix = typeahead_rules::seek(letters);
        let sources = match &filter.members {
            Members::All => vec![Keyed {
                rows: within(&tables.title, &search_rules::starting(&prefix))?,
                prefixed: false,
            }],
            members @ (Members::Subtree(_) | Members::Homes(_)) => self
                .homes_of(members)
                .into_iter()
                .map(|home| {
                    let start = [index_rules::home_key(home).as_slice(), &prefix].concat();
                    Ok(Keyed {
                        rows: within(&tables.home_title, &search_rules::starting(&start))?,
                        prefixed: true,
                    })
                })
                .collect::<Result<Vec<_>, StoreError>>()?,
        };
        let mut first: Option<(Order, ItemId)> = None;
        for row in Merge::new(sources)? {
            let (_, id) = row?;
            if let Some(shortcut) = self.live(&tables, id)?
                && self.admits(&filter, id, &shortcut)
                && typeahead_rules::starts(&shortcut.title.value, letters)
            {
                let opened = if part.sort == Sort::Opened {
                    visited(&tables, id)?.last
                } else {
                    0
                };
                let order = library_rules::order(part.sort, id, &shortcut, opened);
                if first.as_ref().is_none_or(|(before, _)| order < *before) {
                    first = Some((order, id));
                }
            }
        }
        Ok(first.map(|(_, id)| id))
    }

    /// The first title in the trash that starts with the folded letters, found through its title index.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the index or records cannot be read.
    pub fn trash_starting(&self, letters: &str) -> Result<Option<ItemId>, StoreError> {
        let tables = self.tables()?;
        let bounds = search_rules::starting(&typeahead_rules::seek(letters));
        let mut first: Option<(Order, ItemId)> = None;
        for row in within(&tables.trash, &bounds)? {
            let (key, _) = row.map_err(failed)?;
            let id = identified(key.value())?;
            if let Some(shortcut) = self.keeping(&tables, id, Kept::Trashed)?
                && typeahead_rules::starts(&shortcut.title.value, letters)
            {
                let order = library_rules::order(Sort::Title, id, &shortcut, 0);
                if first.as_ref().is_none_or(|(before, _)| order < *before) {
                    first = Some((order, id));
                }
            }
        }
        Ok(first.map(|(_, id)| id))
    }

    /// How many live shortcuts sit directly in a folder, or at the top.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn home_count(&self, home: Option<ItemId>) -> Result<usize, StoreError> {
        let tables = self.tables()?;
        sitting(&tables, home)
    }

    /// How many live shortcuts were added at a time or later, read through the counts per day.
    pub(crate) fn added_since(&self, since: u64) -> Result<usize, StoreError> {
        let added = open(&self.transaction, index(Index::Added))?;
        let days = open(&self.transaction, count(Index::DayCount))?;
        counted_within(
            &added,
            &days,
            Times {
                from: since,
                until: None,
            },
        )
    }

    /// The live folders and shortcuts that a search finds, best first, as the terminal launcher ranks them, or the entries of the top for an empty search.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn find(&self, query: &str) -> Result<Vec<Entry>, StoreError> {
        if query.is_empty() {
            return self.entries(None);
        }
        let tables = self.tables()?;
        let filter = self.filter(Span::Scope(Scope::All), &Within::Whole);
        let search = Search {
            query,
            strategy: search_rules::strategy(self.count(&tables, &filter)?, query),
        };
        self.found(&tables, &filter, search)
    }

    /// The folders and the shortcuts that show and that a search finds, best first.
    fn found(
        &self,
        tables: &Tables<'_>,
        filter: &Filter,
        search: Search<'_>,
    ) -> Result<Vec<Entry>, StoreError> {
        let query = search.query;
        let mut ranks: Vec<Rank<Entry>> = self
            .scored(tables, filter, search)?
            .into_iter()
            .map(|found| Rank::new(found.score, found.frecency, Entry::Shortcut(found.id)))
            .collect();
        for (id, folder) in self.folders.records() {
            if !self.folders.shows(*id) {
                continue;
            }
            let path = self.folders.path(Some(*id));
            if let Some(score) = view::score_folder(query, folder, &path) {
                ranks.push(Rank::new(score, 0, Entry::Folder(*id)));
            }
        }
        ranks.sort();
        Ok(ranks.into_iter().map(Rank::into_id).collect())
    }

    /// A page of the live entries of a folder, or of the top, in their order: its folders and its shortcuts each in its place.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn entries_page(
        &self,
        folder: Option<ItemId>,
        first: usize,
        limit: usize,
    ) -> Result<Vec<Entry>, StoreError> {
        let placement = open(&self.transaction, index(Index::Placement))?;
        let folders: Vec<Ordered<Vec<u8>>> = self
            .folders
            .children(folder)
            .iter()
            .filter_map(|id| {
                self.folders.folder(*id).map(|child| {
                    let key = index_rules::placement_key(
                        folder,
                        &child.position.value,
                        Entry::Folder(*id),
                    );
                    Ok((key, *id))
                })
            })
            .collect();
        let bounds = search_rules::starting(&index_rules::home_key(folder));
        let shortcuts = Keyed {
            rows: within(&placement, &bounds)?,
            prefixed: false,
        };
        let sources: Vec<Box<dyn Iterator<Item = Ordered<Vec<u8>>> + '_>> =
            vec![Box::new(folders.into_iter()), Box::new(shortcuts)];
        let mut page = Vec::new();
        for (place, found) in Merge::new(sources)?.enumerate() {
            if page.len() >= limit {
                break;
            }
            let (key, id) = found?;
            if place >= first {
                page.push(placed_entry(&key, id)?);
            }
        }
        Ok(page)
    }

    /// The shortcuts in the trash themselves in their order, by their titles, as the index of the trash keeps them; the folders in the trash, which [`Folders::trashed`](crate::folders::Folders::trashed) names by their names, list before them.
    fn trash<'a, 't: 'a>(
        &'a self,
        tables: &'a Tables<'t>,
    ) -> Result<impl Iterator<Item = Found> + 'a, StoreError> {
        Ok(Runs {
            session: self,
            tables,
            order: TextOrder {
                sort: Sort::Title,
                bound: ON_DEMAND_LIMIT,
                kept: Kept::Trashed,
            },
            rows: Keyed {
                rows: within(&tables.trash, &search_rules::starting(&[]))?,
                prefixed: false,
            }
            .peekable(),
            pending: VecDeque::new(),
        })
    }

    /// A page of the shortcuts in the trash from a place on, at most `limit` of them.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn trash_page(&self, first: usize, limit: usize) -> Result<Vec<ItemId>, StoreError> {
        let tables = self.tables()?;
        window(self.trash(&tables)?, first, limit)
    }

    /// The place of a shortcut among the shortcuts in the trash, or none when the trash does not hold it.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn trash_place(&self, id: ItemId) -> Result<Option<usize>, StoreError> {
        self.trash_find_place(|found| found == id)
    }

    /// The first place in the trash occupied by any of these shortcuts.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the trash cannot be read.
    pub fn trash_first_of(&self, ids: &BTreeSet<ItemId>) -> Result<Option<usize>, StoreError> {
        if ids.is_empty() {
            return Ok(None);
        }
        self.trash_find_place(|found| ids.contains(&found))
    }

    fn trash_find_place(
        &self,
        matches: impl Fn(ItemId) -> bool,
    ) -> Result<Option<usize>, StoreError> {
        let tables = self.tables()?;
        for (place, found) in self.trash(&tables)?.enumerate() {
            if matches(found?) {
                return Ok(Some(place));
            }
        }
        Ok(None)
    }
}

/// The bytes of a kind and an identifier at the end of a placement key.
const KIND_AND_ID: usize = 17;
/// The kind byte of a folder in a placement key.
const FOLDER_KIND: u8 = 0;

/// The entry a placement key places, which its kind byte names.
fn placed_entry(key: &[u8], id: ItemId) -> Result<Entry, StoreError> {
    let kind = key
        .len()
        .checked_sub(KIND_AND_ID)
        .and_then(|at| key.get(at))
        .ok_or(StoreError::Missing("a kind"))?;
    Ok(if *kind == FOLDER_KIND {
        Entry::Folder(id)
    } else {
        Entry::Shortcut(id)
    })
}

#[cfg(test)]
mod tests;
