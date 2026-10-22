//! What the library window lists, counts, and moves the selection through.
//!
//! A listing holds the shortcuts that show in a scope and that a search finds, in the order a person chooses among the automatic ones, and the trash holds what waits there.
//! The window holds the scope, the sort, the search, and the selection as values, and reads every listing, count, and move from here.

use crate::index_rules;
use crate::rank_rules::{DAY, Rank, Visits, frecency};
use crate::state::{Entry, ItemId, Presence, Shortcut, State, Tree, standing_in};
use crate::view;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// How long a drag rests over a folder before that folder opens, in milliseconds.
pub const SPRING_DELAY: u64 = 600;

/// The drag and folder that a hover belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Hover {
    pub token: ItemId,
    pub folder: ItemId,
}

/// Whether the same active hover has reached its spring delay.
#[must_use]
pub fn spring_due(active: Option<Hover>, requested: Hover, elapsed: u64) -> bool {
    active == Some(requested) && elapsed >= SPRING_DELAY
}
use core::cmp::Reverse;

/// How many days after it was added a shortcut counts as recently added.
pub const RECENT_DAYS: u64 = 30;
/// The age in milliseconds up to which a shortcut counts as recently added.
pub const RECENT: u64 = RECENT_DAYS * DAY;
/// What ends a link's scheme, which the order by link passes over.
const SCHEME_MARK: &str = "://";

/// Which shortcuts the window shows.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum Scope {
    /// Every shortcut that shows.
    #[default]
    All,
    /// The shortcuts in no collection.
    Unsorted,
    /// The shortcuts added within the last [`RECENT_DAYS`] days.
    Recent,
    /// The shortcuts in a collection or any collection inside it.
    Collection(ItemId),
}

/// The order of a listing without a search, which the window keeps as a setting and never arranges by hand.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum Sort {
    /// The latest added first.
    Newest,
    /// The earliest added first.
    Oldest,
    /// By title, ignoring case, as an explorer sorts by name.
    #[default]
    Title,
    /// By link without its scheme, so the pages of one host sit together, each host's in the order of their paths.
    Link,
    /// The one this device opened last first, and the ones it never opened after them, newest first.
    Opened,
}

/// How the window lays its shortcuts out.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum Layout {
    /// A grid of cards that set each link as type.
    Cards,
    /// A table of rows with sortable columns.
    List,
    /// Columns that walk down the folders, beside a preview of the selected shortcut.
    #[default]
    Columns,
}

/// What the window asks to list.
#[derive(Clone, Copy, Debug)]
pub struct Query<'a> {
    pub scope: &'a Scope,
    /// The typed search, which ranks matches by how well they match when it is not empty.
    pub search: &'a str,
    pub sort: Sort,
}

/// The facts every listing reads: the state, its tree, this device's visits, and the time.
#[derive(Clone, Copy, Debug)]
pub struct Facts<'a> {
    pub state: &'a State,
    pub tree: &'a Tree,
    pub visits: &'a BTreeMap<ItemId, Visits>,
    pub now: u64,
}

/// When a shortcut was added: the time its identifier starts with, or the time of its link's write for an identifier without one.
#[must_use]
pub const fn added(id: ItemId, shortcut: &Shortcut) -> u64 {
    match id.time() {
        Some(time) => time,
        None => shortcut.link.stamp.wall,
    }
}

/// A link without its scheme, which the order by link reads so that the pages of one host sit together.
#[must_use]
pub fn address(link: &str) -> &str {
    link.split_once(SCHEME_MARK).map_or(link, |(_, rest)| rest)
}

/// Whether a shortcut added at `added` counts as recent at `now`; one added after `now`, as a clock that moved back leaves it, counts as recent.
#[must_use]
pub const fn recent(added: u64, now: u64) -> bool {
    now.saturating_sub(added) <= RECENT
}

/// How the cards and the list gather their shortcuts under headings, as a file explorer groups its files.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum Grouping {
    /// One listing without headings.
    #[default]
    None,
    /// By the site of each link, its registrable domain, as the web's rule of the same site draws it.
    Site,
    /// By the collection that holds each shortcut.
    Collection,
    /// By how long ago each shortcut was added.
    Added,
}

/// How many days the period of the past week spans.
pub const WEEK_DAYS: u64 = 7;
/// The age in milliseconds up to which a time falls in the past week.
pub const WEEK: u64 = WEEK_DAYS * DAY;

/// How long ago a shortcut was added, as the headings of the groups by date name it, the latest first.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Period {
    /// Within the past [`WEEK_DAYS`] days.
    Week,
    /// Within the past [`RECENT_DAYS`] days, and before the past week.
    Month,
    Earlier,
}

/// The period a time falls in at `now`, counted back in whole spans of days so that no time zone moves it; a time after `now`, as a clock that moved back leaves it, falls in the past week.
#[must_use]
pub const fn period(time: u64, now: u64) -> Period {
    let age = now.saturating_sub(time);
    if age <= WEEK {
        Period::Week
    } else if age <= RECENT {
        Period::Month
    } else {
        Period::Earlier
    }
}

/// The times a period holds at `now`, as [`period`] draws it: from the first time on, and before the second when there is one.
#[must_use]
pub const fn period_bounds(age: Period, now: u64) -> (u64, Option<u64>) {
    let week = now.saturating_sub(WEEK);
    let month = now.saturating_sub(RECENT);
    match age {
        Period::Week => (week, None),
        Period::Month => (month, Some(week)),
        Period::Earlier => (0, Some(month)),
    }
}

/// The earliest time at which a shortcut added counts as recent at `now`, as [`recent`] draws it.
#[must_use]
pub const fn recent_since(now: u64) -> u64 {
    now.saturating_sub(RECENT)
}

/// A listing gathered under keys, held flat: the items in their new order, and each key with how many items follow under it, in the order of the keys.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Gathered<K, T> {
    pub order: Vec<T>,
    pub groups: Vec<(K, usize)>,
}

/// The listing gathered under keys: the groups in the order of their keys, each with its items in the listing's order.
///
/// It reads an item only through `key`, so a proof over small items stands for shortcuts too.
pub fn gather<T: Copy, K: Ord>(listing: &[T], key: impl Fn(T) -> K) -> Gathered<K, T> {
    let mut order: Vec<T> = Vec::with_capacity(listing.len());
    let mut groups: Vec<(K, usize)> = Vec::with_capacity(listing.len());
    for item in listing {
        let found = key(*item);
        let at = groups.partition_point(|(held, _)| *held < found);
        let before = groups
            .iter()
            .take(at)
            .fold(0_usize, |total, (_, size)| total.saturating_add(*size));
        match groups.get_mut(at) {
            Some((held, size)) if *held == found => {
                order.insert(before.saturating_add(*size), *item);
                *size = size.saturating_add(1);
            }
            Some(_) | None => {
                groups.insert(at, (found, 1));
                order.insert(before, *item);
            }
        }
    }
    Gathered { order, groups }
}

/// The listing gathered under keys, each group with its own items, for a caller that walks the groups one by one.
pub fn grouped<T: Copy, K: Ord>(listing: &[T], key: impl Fn(T) -> K) -> Vec<(K, Vec<T>)> {
    let Gathered { order, groups } = gather(listing, key);
    let mut rest = order.as_slice();
    groups
        .into_iter()
        .map(|(held, size)| {
            let (members, after) = rest.split_at(size.min(rest.len()));
            rest = after;
            (held, members.to_vec())
        })
        .collect()
}

/// Whether a shortcut that shows lies in a scope.
fn holds(facts: Facts<'_>, scope: &Scope, id: ItemId, shortcut: &Shortcut) -> bool {
    let home = facts.state.folder_of(facts.tree, id);
    match scope {
        Scope::All => true,
        Scope::Unsorted => home.is_none(),
        Scope::Recent => recent(added(id, shortcut), facts.now),
        Scope::Collection(folder) => home.is_some_and(|home| facts.tree.within(home, *folder)),
    }
}

/// What a listing without a search sorts a shortcut by.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Order {
    Newest(Reverse<u64>, ItemId),
    Oldest(u64, ItemId),
    /// The title without its case, the title itself, and the identifier.
    Title(String, String, ItemId),
    Link(String, ItemId),
    /// When this device opened it last, or zero for never, the latest first, then the latest added first.
    Opened(Reverse<u64>, Reverse<u64>, ItemId),
}

/// What a listing without a search sorts a shortcut by, which this device opened last at `opened` or at zero for never.
#[must_use]
pub fn order(sort: Sort, id: ItemId, shortcut: &Shortcut, opened: u64) -> Order {
    let time = added(id, shortcut);
    match sort {
        Sort::Newest => Order::Newest(Reverse(time), id),
        Sort::Oldest => Order::Oldest(time, id),
        Sort::Title => {
            let title = &shortcut.title.value;
            Order::Title(title.to_lowercase(), String::clone(title), id)
        }
        Sort::Link => Order::Link(String::from(address(&shortcut.link.value)), id),
        Sort::Opened => Order::Opened(Reverse(opened), Reverse(time), id),
    }
}

/// The shortcuts of a listing without a search, in the order of `sort`.
fn sorted(facts: Facts<'_>, mut found: Vec<(ItemId, &Shortcut)>, query: Query<'_>) -> Vec<ItemId> {
    let opened = |id: &ItemId| facts.visits.get(id).map_or(0, |visits| visits.last);
    found.sort_by_cached_key(|(id, shortcut)| order(query.sort, *id, shortcut, opened(id)));
    found.into_iter().map(|(id, _)| id).collect()
}

/// Every shortcut of a state that shows in its tree.
fn shown<'a>(state: &'a State, tree: &'a Tree) -> impl Iterator<Item = (ItemId, &'a Shortcut)> {
    state
        .shortcuts
        .iter()
        .filter(|(_, shortcut)| standing_in(tree, shortcut) == index_rules::Standing::Shown)
        .map(|(id, shortcut)| (*id, shortcut))
}

/// The shortcuts that show in the query's scope, ranked by the search when it is not empty, and in the query's order otherwise.
#[must_use]
pub fn listing(facts: Facts<'_>, query: Query<'_>) -> Vec<ItemId> {
    let found: Vec<(ItemId, &Shortcut)> = shown(facts.state, facts.tree)
        .filter(|(id, shortcut)| holds(facts, query.scope, *id, shortcut))
        .collect();
    if query.search.is_empty() {
        return sorted(facts, found, query);
    }
    let mut ranked: Vec<Rank<ItemId>> = found
        .into_iter()
        .filter_map(|(id, _)| {
            let score = view::shortcut_score(facts.state, facts.tree, query.search, id)?;
            let visited = facts.visits.get(&id).copied().unwrap_or_default();
            Some(Rank::new(score, frecency(visited, facts.now), id))
        })
        .collect();
    ranked.sort();
    ranked.into_iter().map(Rank::into_id).collect()
}

/// How many shortcuts that show each scope holds, and how many items wait in the trash.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Counts {
    pub all: usize,
    pub unsorted: usize,
    pub recent: usize,
    /// The shortcuts in each collection that shows and the collections inside it.
    pub collections: BTreeMap<ItemId, usize>,
    /// The folders and the shortcuts in the trash themselves, without what a folder there hides.
    pub trash: usize,
}

const fn count(counts: &mut usize) {
    *counts = counts.saturating_add(1);
}

/// The counts of every scope and of the trash.
#[must_use]
pub fn counts(state: &State, tree: &Tree, now: u64) -> Counts {
    let mut counts = Counts::default();
    let facts = Facts {
        state,
        tree,
        visits: &BTreeMap::new(),
        now,
    };
    for (id, shortcut) in shown(state, tree) {
        count(&mut counts.all);
        if holds(facts, &Scope::Unsorted, id, shortcut) {
            count(&mut counts.unsorted);
        }
        if holds(facts, &Scope::Recent, id, shortcut) {
            count(&mut counts.recent);
        }
        for folder in tree.lineage(state.folder_of(tree, id)) {
            count(counts.collections.entry(folder).or_default());
        }
    }
    counts.trash = trash(state).len();
    counts
}

/// What a name sorts by: the name without its case, the name itself, and the identifier.
#[must_use]
pub fn by_name(name: &str, id: ItemId) -> (String, String, ItemId) {
    (name.to_lowercase(), String::from(name), id)
}

/// What the trash lists: every folder and every shortcut in the trash itself, the folders first, each by its name.
///
/// A folder in the trash hides what it holds, which comes back with it, and a shortcut put in the trash on its own before its folder lists on its own.
#[must_use]
pub fn trash(state: &State) -> Vec<Entry> {
    let mut folders: Vec<(String, String, ItemId)> = state
        .folders
        .iter()
        .filter(|(_, folder)| folder.presence.value == Presence::Trashed)
        .map(|(id, folder)| by_name(&folder.name.value, *id))
        .collect();
    folders.sort_unstable();
    let mut shortcuts: Vec<(String, String, ItemId)> = state
        .shortcuts
        .iter()
        .filter(|(_, shortcut)| shortcut.presence.value == Presence::Trashed)
        .map(|(id, shortcut)| by_name(&shortcut.title.value, *id))
        .collect();
    shortcuts.sort_unstable();
    folders
        .into_iter()
        .map(|(_, _, id)| Entry::Folder(id))
        .chain(shortcuts.into_iter().map(|(_, _, id)| Entry::Shortcut(id)))
        .collect()
}

/// A key that moves the selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Move {
    Up,
    Down,
    Left,
    Right,
    First,
    Last,
}

/// Where a key moves the selection among `count` items that a grid shows `across` to a row, or a list shows one to a row.
///
/// Up and down move a row, and down onto a shorter last row lands on its last item; left and right move one item; nothing selects nothing when there is no item.
#[must_use]
pub fn moved(selected: Option<usize>, count: usize, across: usize, key: Move) -> Option<usize> {
    let last = count.checked_sub(1)?;
    let across = across.max(1);
    let Some(current) = selected.map(|selected| selected.min(last)) else {
        return Some(if key == Move::Last { last } else { 0 });
    };
    let row = |place: usize| place.checked_div(across).unwrap_or_default();
    Some(match key {
        Move::Up => current.checked_sub(across).unwrap_or(current),
        Move::Down => match current.checked_add(across) {
            Some(below) if below <= last => below,
            Some(_) | None if row(current) < row(last) => last,
            Some(_) | None => current,
        },
        Move::Left => current.saturating_sub(1),
        Move::Right => current.saturating_add(1).min(last),
        Move::First => 0,
        Move::Last => last,
    })
}

/// The place of the item in `column` of the row of a group that starts `offset` items into it, or the last of a shorter row.
fn in_row(start: usize, size: usize, offset: usize, column: usize) -> usize {
    let length = size.saturating_sub(offset);
    start
        .saturating_add(offset)
        .saturating_add(column.min(length.saturating_sub(1)))
}

/// Where a key moves the selection in a grid whose groups each begin a row under their heading: `sizes` holds how many items each group holds in turn.
///
/// Up and down move to the row above or below across the headings, to the same column or the last item of a shorter row; the other keys move as [`moved`] does over all the items.
#[must_use]
pub fn grouped_moved(
    selected: Option<usize>,
    sizes: &[usize],
    across: usize,
    key: Move,
) -> Option<usize> {
    let count = sizes
        .iter()
        .fold(0_usize, |total, size| total.saturating_add(*size));
    let last = count.checked_sub(1)?;
    let (up, current) = match (key, selected) {
        (Move::Up, Some(selected)) => (true, selected.min(last)),
        (Move::Down, Some(selected)) => (false, selected.min(last)),
        (Move::Up | Move::Down, None)
        | (Move::Left | Move::Right | Move::First | Move::Last, Some(_) | None) => {
            return moved(selected, count, across, key);
        }
    };
    let across = across.max(1);
    // The group the selection sits in, the nonempty groups on either side, and where each starts.
    let mut begin = 0_usize;
    let mut own = None;
    let mut before = None;
    let mut after = None;
    for size in sizes {
        let end = begin.saturating_add(*size);
        if own.is_none() && begin <= current && current < end {
            own = Some((begin, *size));
        } else if *size > 0 && own.is_none() {
            before = Some((begin, *size));
        } else if *size > 0 && after.is_none() && own.is_some() {
            after = Some((begin, *size));
        }
        begin = end;
    }
    let (start, size) = own?;
    let offset = current.saturating_sub(start);
    let column = offset.checked_rem(across).unwrap_or_default();
    let row_start = offset.saturating_sub(column);
    Some(if up {
        if row_start > 0 {
            in_row(start, size, row_start.saturating_sub(across), column)
        } else {
            before.map_or(current, |(above, length)| {
                let last_row = length
                    .saturating_sub(1)
                    .checked_div(across)
                    .unwrap_or_default()
                    .saturating_mul(across);
                in_row(above, length, last_row, column)
            })
        }
    } else {
        let next_row = row_start.saturating_add(across);
        if next_row < size {
            in_row(start, size, next_row, column)
        } else {
            after.map_or(current, |(below, length)| in_row(below, length, 0, column))
        }
    })
}

/// Where a key moves the selection in the columns of a folder walk.
///
/// `path` holds the selected place in each column from the left, `lengths` the entries of each, and `opens` whether the last selected entry is a folder that holds entries.
/// Up and down move within the last column, right opens the selected folder into a new column, and left closes the last column while one remains.
#[must_use]
pub fn column_moved(path: &[usize], lengths: &[usize], opens: bool, key: Move) -> Vec<usize> {
    let kept = kept_columns(path, lengths);
    let Some(depth) = kept.checked_sub(1) else {
        return lengths
            .first()
            .filter(|length| **length > 0)
            .map(|_| alloc::vec![0])
            .unwrap_or_default();
    };
    let mut next: Vec<usize> = path
        .iter()
        .zip(lengths)
        .take(kept)
        .map(|(place, length)| (*place).min(length.saturating_sub(1)))
        .collect();
    let Some(current) = next.pop() else {
        return next;
    };
    let standing = Standing {
        current,
        last: lengths
            .get(depth)
            .map_or(0, |length| length.saturating_sub(1)),
        first: depth == 0,
        opens: opens && kept == path.len(),
    };
    let step = column_step(standing, key);
    next.extend(step.place);
    if step.opens {
        next.push(0);
    }
    next
}

/// How many places of a walk's path stay: the leading ones whose columns hold entries.
#[must_use]
pub fn kept_columns(path: &[usize], lengths: &[usize]) -> usize {
    path.iter()
        .zip(lengths)
        .take_while(|(_, length)| **length > 0)
        .count()
}

/// Where the selection stands in the deepest column of a walk, before a key moves it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Standing {
    /// Its place in the column, at most the column's last.
    pub current: usize,
    /// The last place of the column.
    pub last: usize,
    /// Whether the column is the first, which never closes.
    pub first: bool,
    /// Whether right opens a new column: the selected entry is a folder that holds entries, and the path ends at it.
    pub opens: bool,
}

/// What a key does to the deepest column: the place it selects there, or none when the column closes, and whether a new column opens after it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ColumnStep {
    pub place: Option<usize>,
    pub opens: bool,
}

/// What a key does to the deepest column of a walk: up and down move one place within it, first and last go to its ends, right keeps the place and opens a new column when it may, and left closes the column unless it is the first.
#[must_use]
pub fn column_step(standing: Standing, key: Move) -> ColumnStep {
    let Standing {
        current,
        last,
        first,
        opens,
    } = standing;
    let place = match key {
        Move::Up => Some(current.saturating_sub(1)),
        Move::Down => Some(current.saturating_add(1).min(last)),
        Move::First => Some(0),
        Move::Last => Some(last),
        Move::Right => Some(current),
        Move::Left => {
            if first {
                Some(current)
            } else {
                None
            }
        }
    };
    ColumnStep {
        place,
        opens: opens && matches!(key, Move::Right),
    }
}

/// The properties Kani proves over every short grid and walk, every identifier's time, and every short grouping.
#[cfg(any(test, kani))]
mod proofs {
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(17)]
    fn a_hover_springs_only_for_its_drag_and_folder_after_its_delay() {
        let active: Option<([u8; 16], [u8; 16])> = kani::any();
        let active = active.map(|(token, folder)| super::Hover {
            token: ItemId(token),
            folder: ItemId(folder),
        });
        let requested = super::Hover {
            token: ItemId(kani::any()),
            folder: ItemId(kani::any()),
        };
        let elapsed: u64 = kani::any();
        let due = super::spring_due(active, requested, elapsed);
        if due {
            kani::assert(
                active == Some(requested),
                "only the same active drag and folder spring",
            );
            kani::assert(
                elapsed >= super::SPRING_DELAY,
                "a hover waits for its delay",
            );
        } else {
            kani::assert(
                active != Some(requested) || elapsed < super::SPRING_DELAY,
                "every eligible hover springs",
            );
        }
        kani::cover!(
            due && elapsed == super::SPRING_DELAY,
            "the delay boundary springs"
        );
        kani::cover!(active.is_none(), "a cancelled hover stays closed");
        kani::cover!(
            active.is_some_and(|hover| hover.token != requested.token),
            "a stale drag stays closed"
        );
        kani::cover!(
            active.is_some_and(|hover| hover.folder != requested.folder),
            "another folder stays closed"
        );
    }

    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(17)]
    fn rejects_a_spring_before_its_delay() {
        let hover = super::Hover {
            token: ItemId([1; 16]),
            folder: ItemId([2; 16]),
        };
        kani::assert(
            super::spring_due(Some(hover), hover, super::SPRING_DELAY.saturating_sub(1)),
            "NEGATIVE_LIBRARY_CONTROL",
        );
    }
    #[cfg(test)]
    use super::column_moved;
    use super::{
        ColumnStep, Gathered, Move, Period, RECENT, Standing, WEEK, column_step, gather,
        grouped_moved, kept_columns, moved, period, period_bounds, recent, recent_since,
    };
    #[cfg(kani)]
    use crate::state::ItemId;
    #[cfg(test)]
    use alloc::vec::Vec;

    /// What a grouping promises over a listing of the places `0, 1, …` in turn, the place `p` keyed by `keys[p]`: the groups in the strict order of their keys, each item under its own key, each group in the listing's order, and every item of the listing once.
    ///
    /// `gather` reads its items only through the key, so places stand for shortcuts.
    pub(super) fn grouping_spec(listing: &[u8], keys: &[u8]) -> bool {
        let key = |place: u8| keys.get(usize::from(place)).copied().unwrap_or(u8::MAX);
        let Gathered { order, groups } = gather(listing, key);
        let ordered = groups.windows(2).all(|pair| match pair {
            [(left, _), (right, _)] => left < right,
            _ => true,
        });
        let mut start = 0_usize;
        let mut kept = true;
        for (held, size) in &groups {
            let end = start.saturating_add(*size);
            let members = order.get(start..end).unwrap_or_default();
            let keyed = members.len() == *size
                && *size > 0
                && members.iter().all(|place| key(*place) == *held);
            let in_order = members.windows(2).all(|pair| match pair {
                [earlier, later] => earlier < later,
                _ => true,
            });
            kept = kept && keyed && in_order;
            start = end;
        }
        let whole = start == listing.len()
            && order.len() == listing.len()
            && listing.iter().all(|place| order.contains(place));
        ordered && kept && whole
    }

    /// What a move in a grouped grid promises: one group moves exactly as the plain grid does, the keys other than up and down move as over one listing, up and down land in the next row toward the key or stay at the edge, and a move lands on an item exactly when there is one.
    pub(super) fn grouped_spec(
        selected: Option<usize>,
        sizes: &[usize],
        across: usize,
        key: Move,
    ) -> bool {
        let count = sizes
            .iter()
            .fold(0_usize, |total, size| total.saturating_add(*size));
        let found = grouped_moved(selected, sizes, across, key);
        let plain = moved(selected, count, across, key);
        // Groups without items add no row, so a grid of one nonempty group moves as the plain one.
        let single = sizes.iter().filter(|size| **size > 0).count() > 1 || found == plain;
        let vertical = matches!(key, Move::Up | Move::Down);
        let linear = (vertical && selected.is_some()) || found == plain;
        let lands = found.is_none() == (count == 0) && found.is_none_or(|place| place < count);
        let stepped = match (vertical, selected.zip(count.checked_sub(1))) {
            (true, Some((selected, last))) => {
                let current = selected.min(last);
                let (row, column) = position(sizes, across, current);
                let (bottom, _) = position(sizes, across, last);
                let target = if key == Move::Up {
                    row.checked_sub(1)
                } else if row == bottom {
                    None
                } else {
                    row.checked_add(1)
                };
                let expected = target.map_or(Some(current), |target| {
                    // The widest column of the target row, then the item at the wanted column of it.
                    let mut widest = None;
                    for place in 0..count {
                        let (at, across_at) = position(sizes, across, place);
                        if at == target {
                            widest = Some(across_at);
                        }
                    }
                    let wanted = widest.map(|widest| column.min(widest));
                    (0..count).find(|place| {
                        let (at, across_at) = position(sizes, across, *place);
                        at == target && Some(across_at) == wanted
                    })
                });
                found == expected
            }
            (true | false, _) => true,
        };
        single && linear && lands && stepped
    }

    /// The row and the column of an item of a grid whose groups each begin a row, counted one by one as a reference.
    fn position(sizes: &[usize], across: usize, wanted: usize) -> (usize, usize) {
        let across = across.max(1);
        let mut row = 0_usize;
        let mut seen = 0_usize;
        for size in sizes {
            for at in 0..*size {
                let column = at.checked_rem(across).unwrap_or_default();
                if at > 0 && column == 0 {
                    row = row.saturating_add(1);
                }
                if seen == wanted {
                    return (row, column);
                }
                seen = seen.saturating_add(1);
            }
            if *size > 0 {
                row = row.saturating_add(1);
            }
        }
        (row, 0)
    }

    /// What the periods promise: the past week up to its span, the past month after it up to its span, everything older after that, and a later time never in an older period.
    pub(super) fn period_spec(time: u64, later: u64, now: u64) -> bool {
        let age = now.saturating_sub(time);
        let named = match period(time, now) {
            Period::Week => age <= WEEK,
            Period::Month => WEEK < age && age <= RECENT,
            Period::Earlier => RECENT < age,
        };
        let monotone = later < time || period(later, now) <= period(time, now);
        named && monotone
    }

    /// What a period's bounds promise: they hold a time exactly when it falls in that period, and the recent times begin where the past month does.
    pub(super) fn bounds_spec(time: u64, now: u64, age: Period) -> bool {
        let (from, until) = period_bounds(age, now);
        let held = from <= time && until.is_none_or(|until| time < until);
        held == (period(time, now) == age) && recent(time, now) == (time >= recent_since(now))
    }

    /// The most items, columns, and places the harnesses draw.
    pub(super) const BOUND: usize = 4;
    /// Every key the moves read.
    pub(super) const KEYS: [Move; 6] = [
        Move::Up,
        Move::Down,
        Move::Left,
        Move::Right,
        Move::First,
        Move::Last,
    ];

    /// What a grid move promises: no selection without items, a selection on an item otherwise, and a move of at most a row.
    pub(super) fn grid_spec(
        selected: Option<usize>,
        count: usize,
        across: usize,
        key: Move,
    ) -> bool {
        let found = moved(selected, count, across, key);
        match (found, count.checked_sub(1)) {
            (None, None) => true,
            (Some(place), Some(last)) => {
                let from = selected.map_or(0, |selected| selected.min(last));
                let step = place.abs_diff(from);
                let bounded = match key {
                    Move::Up | Move::Down => step <= across.max(1) || place == last,
                    Move::Left | Move::Right => step <= 1,
                    Move::First | Move::Last => true,
                };
                place <= last && (selected.is_none() || bounded)
            }
            (Some(_), None) | (None, Some(_)) => false,
        }
    }

    /// What a step in a column promises, for a selection within its column: a place within the column, up and down at most one away, first and last at its ends, right where it stands, left where it stands in the first column and none elsewhere, and a new column only for right when one may open.
    pub(super) fn column_step_spec(standing: Standing, key: Move) -> bool {
        let ColumnStep { place, opens } = column_step(standing, key);
        let Standing {
            current,
            last,
            first,
            opens: may_open,
        } = standing;
        let within = place.is_none_or(|place| place <= last);
        let moved_right = match key {
            Move::Up | Move::Down => place.is_some_and(|place| place.abs_diff(current) <= 1),
            Move::First => place == Some(0),
            Move::Last => place == Some(last),
            Move::Right => place == Some(current),
            Move::Left => place == first.then_some(current),
        };
        let opened = opens == (may_open && key == Move::Right);
        within && moved_right && opened
    }

    /// What keeping a walk promises: no more places than the path and the columns hold, every kept place's column holding entries, and the walk cut only at the end or at an empty column.
    pub(super) fn kept_spec(path: &[usize], lengths: &[usize]) -> bool {
        let kept = kept_columns(path, lengths);
        let bound = path.len().min(lengths.len());
        let held = lengths
            .get(..kept)
            .is_some_and(|kept_lengths| kept_lengths.iter().all(|length| *length > 0));
        let cut = kept == bound || lengths.get(kept) == Some(&0);
        kept <= bound && held && cut
    }

    /// The column move, when it keeps its promises: every place of an open column within it, at most one column more and only when right opens a folder, and one column kept while the first has entries.
    ///
    /// The tests draw it over a small domain, and the harnesses prove the step and the keeping it composes.
    #[cfg(test)]
    pub(super) fn column_checked(
        path: &[usize],
        lengths: &[usize],
        opens: bool,
        key: Move,
    ) -> Option<Vec<usize>> {
        let next = column_moved(path, lengths, opens, key);
        let within = next
            .iter()
            .zip(lengths.iter().take(path.len().max(1)))
            .all(|(place, length)| place < length);
        let grown = next.len() <= path.len().max(1).saturating_add(1);
        let opened = next.len() <= path.len().max(1) || (opens && key == Move::Right);
        let kept = lengths.first().is_none_or(|first| *first == 0) || !next.is_empty();
        (within && grown && opened && kept).then_some(next)
    }

    #[cfg(kani)]
    fn any_key() -> Move {
        let choice: usize = kani::any();
        kani::assume(choice < KEYS.len());
        KEYS.get(choice).copied().unwrap_or(Move::Up)
    }

    /// Every grid of up to four items and four to a row, from every selection or none.
    #[cfg(kani)]
    #[kani::proof]
    fn a_grid_move_lands_on_an_item() {
        let count: usize = kani::any();
        let across: usize = kani::any();
        let selected: Option<usize> = kani::any();
        kani::assume(count <= BOUND && across <= BOUND);
        kani::assume(selected.is_none_or(|place| place <= BOUND));
        let key = any_key();
        kani::assert(
            grid_spec(selected, count, across, key),
            "a move lands on an item",
        );
        let found = moved(selected, count, across, key);
        kani::cover!(
            key == Move::Down
                && selected.is_some_and(|place| found == Some(place + across))
                && across > 1,
            "down a row"
        );
        kani::cover!(
            key == Move::Down
                && selected.is_some_and(|place| place + across >= count && found != Some(place)),
            "down onto a shorter last row"
        );
        kani::cover!(found.is_none(), "nothing to select");
    }

    /// Every standing in a column under every key.
    #[cfg(kani)]
    #[kani::proof]
    fn a_column_step_keeps_the_selection_in_its_column_and_opens_or_closes_only_as_asked() {
        let standing = Standing {
            current: kani::any(),
            last: kani::any(),
            first: kani::any(),
            opens: kani::any(),
        };
        kani::assume(standing.current <= standing.last);
        let key = any_key();
        kani::assert(
            column_step_spec(standing, key),
            "a step keeps the selection in its column",
        );
        let step = column_step(standing, key);
        kani::cover!(step.place.is_none(), "a column closes");
        kani::cover!(step.opens, "a folder opens");
        kani::cover!(
            step.place.is_some_and(|place| place != standing.current),
            "the selection moves within a column"
        );
    }

    /// Every path and every list of column lengths, each of up to three.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(4)]
    fn a_walk_keeps_the_leading_places_whose_columns_hold_entries() {
        let path: [usize; 3] = kani::any();
        let lengths: [usize; 3] = kani::any();
        let depth: usize = kani::any();
        let columns: usize = kani::any();
        kani::assume(depth <= 3 && columns <= 3);
        let (Some(path), Some(lengths)) = (path.get(..depth), lengths.get(..columns)) else {
            return;
        };
        kani::assert(
            kept_spec(path, lengths),
            "a walk keeps the leading places whose columns hold entries",
        );
        let kept = kept_columns(path, lengths);
        kani::cover!(
            kept > 0 && kept < path.len(),
            "an empty column cuts the walk"
        );
    }

    /// Every time a version 7 identifier holds, and every clock reading.
    #[cfg(kani)]
    #[kani::proof]
    fn an_identifier_gives_back_its_time_and_recency_follows_the_age() {
        let now: u64 = kani::any();
        let random: [u8; 10] = kani::any();
        kani::assume(now < 1 << 48);
        kani::assert(
            ItemId::v7(now, random).time() == Some(now),
            "the identifier gives back its time",
        );
        let added: u64 = kani::any();
        let clock: u64 = kani::any();
        let fresh = recent(added, clock);
        kani::assert(
            fresh == (clock <= added || clock - added <= RECENT),
            "recency follows the age",
        );
        kani::cover!(fresh && clock > added, "a recent addition");
        kani::cover!(!fresh, "an old addition");
    }

    /// Every grid of up to two groups of up to three items each and up to three to a row, from every selection or none, under every key.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(8)]
    fn a_grouped_grid_moves_a_row_across_the_headings() {
        // An empty group stands for a missing one, since it adds no row, so the two groups stay concrete.
        let sizes: [usize; 2] = kani::any();
        let across: usize = kani::any();
        let selected: Option<usize> = kani::any();
        kani::assume(across <= 3);
        kani::assume(sizes.iter().all(|size| *size <= 3));
        kani::assume(selected.is_none_or(|place| place <= 6));
        let key = any_key();
        kani::assert(
            grouped_spec(selected, &sizes, across, key),
            "a move lands a row away across the headings",
        );
        let found = grouped_moved(selected, &sizes, across, key);
        kani::cover!(
            key == Move::Down
                && selected.is_some_and(|place| place < sizes[0])
                && found.is_some_and(|place| place >= sizes[0]),
            "down into the next group"
        );
        kani::cover!(
            key == Move::Up && found != selected && found.is_some(),
            "up a row"
        );
    }

    /// Every period of every pair of times at every clock reading.
    #[cfg(kani)]
    #[kani::proof]
    fn a_time_falls_in_the_period_of_its_age_and_a_later_one_never_in_an_older() {
        let time: u64 = kani::any();
        let later: u64 = kani::any();
        let now: u64 = kani::any();
        kani::assert(period_spec(time, later, now), "the period follows the age");
        kani::cover!(period(time, now) == Period::Month, "the past month");
        kani::cover!(period(time, now) == Period::Earlier, "earlier");
    }

    /// Every time, every clock reading, and every period: a period's bounds hold exactly the times that fall in it.
    #[cfg(kani)]
    #[kani::proof]
    fn a_period_s_bounds_hold_exactly_its_times() {
        let time: u64 = kani::any();
        let now: u64 = kani::any();
        let age = match kani::any::<u8>() {
            0 => Period::Week,
            1 => Period::Month,
            _ => Period::Earlier,
        };
        kani::assert(
            bounds_spec(time, now, age),
            "a period's bounds hold exactly its times",
        );
        kani::cover!(
            age == Period::Month && period(time, now) == Period::Month,
            "a time of the past month"
        );
        kani::cover!(
            age == Period::Earlier && period(time, now) == Period::Earlier,
            "an earlier time"
        );
    }

    /// Every listing of up to three items under every key up to three each.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(4)]
    fn a_grouping_keeps_every_shortcut_once_under_its_key_in_order() {
        // The listing's length stays concrete, which keeps the solver's memory small; the tests draw the shorter ones.
        let keys: [u8; 3] = kani::any();
        kani::assume(keys.iter().all(|key| *key < 3));
        let listing = [0, 1, 2];
        kani::assert(
            grouping_spec(&listing, &keys),
            "a grouping keeps every shortcut once under its key",
        );
        let gathered = gather(&listing, |place: u8| keys.get(usize::from(place)).copied());
        kani::cover!(
            gathered.groups.iter().any(|(_, size)| *size == 2),
            "two shortcuts share a group"
        );
        kani::cover!(gathered.groups.len() == 3, "three groups");
    }

    /// Down from the last item must stay on it; this probe claims it moves past the end and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_move_past_the_last_item() {
        kani::assert(
            moved(Some(2), 3, 1, Move::Down) == Some(3),
            "NEGATIVE_LIBRARY_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_hover_waits_and_refuses_stale_drags_and_folders() {
        let hover = super::Hover {
            token: ItemId([1; 16]),
            folder: ItemId([2; 16]),
        };
        for elapsed in [
            0,
            super::SPRING_DELAY - 1,
            super::SPRING_DELAY,
            super::SPRING_DELAY + 1,
            u64::MAX,
        ] {
            assert_eq!(
                super::spring_due(Some(hover), hover, elapsed),
                elapsed >= super::SPRING_DELAY
            );
            assert!(!super::spring_due(None, hover, elapsed));
            assert!(!super::spring_due(
                Some(hover),
                super::Hover {
                    token: ItemId([3; 16]),
                    ..hover
                },
                elapsed
            ));
            assert!(!super::spring_due(
                Some(hover),
                super::Hover {
                    folder: ItemId([3; 16]),
                    ..hover
                },
                elapsed
            ));
        }
    }
    use super::proofs::{
        BOUND, KEYS, bounds_spec, column_checked, column_step_spec, grid_spec, grouped_spec,
        grouping_spec, kept_spec, period_spec,
    };
    use super::{
        Counts, Facts, Move, Period, Query, RECENT, RECENT_DAYS, Scope, Sort, Standing, WEEK,
        WEEK_DAYS, added, by_name, column_moved, counts, grouped, grouped_moved, listing, moved,
        period, period_bounds, recent, recent_since, trash,
    };
    use crate::clock_rules::{DeviceId, Stamp};
    use crate::rank_rules::{DAY, Visits};
    use crate::state::{Edit, Entry, ItemId, State};
    use alloc::collections::BTreeMap;
    use alloc::string::String;
    use alloc::vec::Vec;

    const NOW: u64 = 1_000 * DAY;
    const DEVICE: DeviceId = DeviceId([5; 16]);

    fn stamp(wall: u64) -> Stamp {
        Stamp {
            wall,
            counter: 0,
            device: DEVICE,
        }
    }

    /// The time `days` days before now.
    fn ago(days: u64) -> u64 {
        NOW.checked_sub(days.checked_mul(DAY).unwrap()).unwrap()
    }

    /// An identifier made `days` days before now.
    fn made(days: u64, byte: u8) -> ItemId {
        ItemId::v7(ago(days), [byte; 10])
    }

    struct Library {
        state: State,
        work: ItemId,
        specs: ItemId,
        whatwg: ItemId,
        w3c: ItemId,
        weather: ItemId,
        news: ItemId,
    }

    /// Work holds Specs, which holds the WHATWG standard; the W3C note sits in Work; the weather and the news sit at the top.
    fn library() -> Library {
        let mut state = State::default();
        let work = made(50, 1);
        let specs = made(50, 2);
        let whatwg = made(40, 3);
        let w3c = made(10, 4);
        let weather = made(2, 5);
        let news = ItemId([6; 16]);
        let shortcut = |id, link: &str, title: &str, folder| Edit::AddShortcut {
            id,
            link: String::from(link),
            title: String::from(title),
            folder,
        };
        let edits = [
            Edit::AddFolder {
                id: work,
                name: String::from("Work"),
                parent: None,
            },
            Edit::AddFolder {
                id: specs,
                name: String::from("Specs"),
                parent: Some(work),
            },
            shortcut(
                whatwg,
                "https://url.spec.whatwg.org/",
                "URL Standard",
                Some(specs),
            ),
            shortcut(w3c, "https://www.w3.org/TR/jlreq/", "jlreq", Some(work)),
            shortcut(weather, "https://tenki.jp/", "weather", None),
            shortcut(news, "https://news.ycombinator.com/", "Hacker News", None),
            Edit::SetNote {
                id: w3c,
                note: String::from("the spec of Japanese layout"),
            },
            Edit::SetNote {
                id: news,
                note: String::from("read later"),
            },
        ];
        for (wall, edit) in (1..).zip(edits) {
            state.apply(edit, stamp(wall)).unwrap();
        }
        Library {
            state,
            work,
            specs,
            whatwg,
            w3c,
            weather,
            news,
        }
    }

    fn list(library: &Library, scope: &Scope, search: &str, sort: Sort) -> Vec<ItemId> {
        let tree = library.state.tree();
        let mut visits = BTreeMap::new();
        visits.insert(
            library.news,
            Visits {
                count: 3,
                last: ago(1),
            },
        );
        visits.insert(
            library.w3c,
            Visits {
                count: 1,
                last: ago(2),
            },
        );
        let facts = Facts {
            state: &library.state,
            tree: &tree,
            visits: &visits,
            now: NOW,
        };
        listing(
            facts,
            Query {
                scope,
                search,
                sort,
            },
        )
    }

    #[test]
    fn each_scope_holds_its_shortcuts() {
        let library = library();
        let Library {
            work,
            specs,
            whatwg,
            w3c,
            weather,
            news,
            ..
        } = library;
        let newest = |scope: &Scope| list(&library, scope, "", Sort::Newest);
        assert_eq!(newest(&Scope::All), [weather, w3c, whatwg, news]);
        assert_eq!(newest(&Scope::Unsorted), [weather, news]);
        assert_eq!(newest(&Scope::Recent), [weather, w3c]);
        assert_eq!(newest(&Scope::Collection(work)), [w3c, whatwg]);
        assert_eq!(newest(&Scope::Collection(specs)), [whatwg]);
    }

    #[test]
    fn each_sort_orders_the_listing() {
        let library = library();
        let Library {
            whatwg,
            w3c,
            weather,
            news,
            ..
        } = library;
        let all = |sort| list(&library, &Scope::All, "", sort);
        assert_eq!(all(Sort::Oldest), [news, whatwg, w3c, weather]);
        assert_eq!(all(Sort::Title), [news, w3c, whatwg, weather]);
        assert_eq!(
            all(Sort::Link),
            [news, weather, whatwg, w3c],
            "the links by host, the scheme passed over"
        );
        assert_eq!(all(Sort::Opened), [news, w3c, weather, whatwg]);
        assert_eq!(Sort::default(), Sort::Title, "a listing sorts by name");
    }

    #[test]
    fn a_search_ranks_the_matches_of_the_scope() {
        let library = library();
        assert_eq!(
            list(&library, &Scope::All, "spec", Sort::Newest),
            [library.w3c, library.whatwg],
            "equal matches rank by this device's visits"
        );
        assert_eq!(
            list(&library, &Scope::All, "whatwg", Sort::Newest),
            [library.whatwg]
        );
        assert_eq!(list(&library, &Scope::Unsorted, "spec", Sort::Newest), []);
        assert_eq!(
            list(&library, &Scope::All, "later", Sort::Oldest),
            [library.news]
        );
    }

    #[test]
    fn a_removed_shortcut_leaves_every_listing_and_count() {
        let mut library = library();
        library
            .state
            .apply(
                Edit::Remove {
                    entry: Entry::Shortcut(library.news),
                },
                stamp(20),
            )
            .unwrap();
        assert_eq!(list(&library, &Scope::All, "", Sort::Newest).len(), 3);
        let tree = library.state.tree();
        let left = counts(&library.state, &tree, NOW);
        assert_eq!((left.all, left.unsorted), (3, 1));
    }

    #[test]
    fn the_counts_cover_every_scope_and_each_collection_holds_what_lies_inside() {
        let library = library();
        let tree = library.state.tree();
        let mut collections_count = BTreeMap::new();
        collections_count.insert(library.work, 2);
        collections_count.insert(library.specs, 1);
        assert_eq!(
            counts(&library.state, &tree, NOW),
            Counts {
                all: 4,
                unsorted: 2,
                recent: 2,
                collections: collections_count,
                trash: 0,
            }
        );
    }

    #[test]
    fn the_trash_lists_its_folders_then_its_shortcuts_by_name_and_hides_what_a_folder_holds() {
        let mut library = library();
        let (work, whatwg, weather, news) =
            (library.work, library.whatwg, library.weather, library.news);
        let trashed = [
            Entry::Shortcut(whatwg),
            Entry::Folder(work),
            Entry::Shortcut(weather),
            Entry::Shortcut(news),
        ];
        for (wall, entry) in (20..).zip(trashed) {
            library
                .state
                .apply(Edit::Trash { entry }, stamp(wall))
                .unwrap();
        }
        assert_eq!(
            trash(&library.state),
            [
                Entry::Folder(work),
                Entry::Shortcut(news),
                Entry::Shortcut(whatwg),
                Entry::Shortcut(weather),
            ],
            "Hacker News, URL Standard, weather"
        );
        let tree = library.state.tree();
        let left = counts(&library.state, &tree, NOW);
        assert_eq!(
            (left.all, left.unsorted, left.trash, left.collections.len()),
            (0, 0, 4, 0),
            "the trash hides the note inside Work as well"
        );
        assert_eq!(list(&library, &Scope::All, "", Sort::Newest), []);
        assert_eq!(list(&library, &Scope::All, "spec", Sort::Newest), []);
        library
            .state
            .apply(
                Edit::PutBack {
                    entry: Entry::Folder(work),
                },
                stamp(24),
            )
            .unwrap();
        assert_eq!(
            list(&library, &Scope::Collection(work), "", Sort::Newest),
            [library.w3c],
            "the shortcut put in the trash on its own stays there"
        );
        assert_eq!(by_name("École", news).0, "école");
    }

    #[test]
    fn an_identifier_without_a_time_dates_from_its_link() {
        let library = library();
        let shortcut = library.state.shortcut(library.news).unwrap();
        assert_eq!(added(library.news, shortcut), shortcut.link.stamp.wall);
        let whatwg = library.state.shortcut(library.whatwg).unwrap();
        assert_eq!(added(library.whatwg, whatwg), ago(40));
        assert!(recent(ago(RECENT_DAYS), NOW));
        assert!(!recent(ago(RECENT_DAYS).checked_sub(1).unwrap(), NOW));
        assert!(recent(NOW.checked_add(1).unwrap(), NOW));
        assert_eq!(RECENT, ago(0).checked_sub(ago(RECENT_DAYS)).unwrap());
    }

    #[test]
    fn a_grid_moves_by_rows_and_items() {
        assert_eq!(moved(None, 0, 3, Move::Down), None);
        assert_eq!(moved(Some(4), 0, 3, Move::Down), None);
        assert_eq!(moved(None, 5, 3, Move::Down), Some(0));
        assert_eq!(moved(None, 5, 3, Move::Last), Some(4));
        assert_eq!(moved(Some(1), 5, 3, Move::Down), Some(4));
        assert_eq!(
            moved(Some(2), 5, 3, Move::Down),
            Some(4),
            "onto the shorter last row"
        );
        assert_eq!(moved(Some(4), 5, 3, Move::Down), Some(4));
        assert_eq!(moved(Some(4), 5, 3, Move::Up), Some(1));
        assert_eq!(moved(Some(1), 5, 3, Move::Up), Some(1));
        assert_eq!(moved(Some(3), 5, 3, Move::Left), Some(2));
        assert_eq!(moved(Some(0), 5, 3, Move::Left), Some(0));
        assert_eq!(moved(Some(4), 5, 3, Move::Right), Some(4));
        assert_eq!(
            moved(Some(9), 5, 0, Move::Up),
            Some(3),
            "a stale selection clamps first"
        );
        assert_eq!(moved(Some(2), 5, 1, Move::First), Some(0));
        assert_eq!(moved(Some(2), 5, 1, Move::Last), Some(4));
    }

    #[test]
    fn the_columns_open_and_close_folders() {
        assert_eq!(
            column_moved(&[], &[], false, Move::Down),
            Vec::<usize>::new()
        );
        assert_eq!(column_moved(&[], &[3], false, Move::Down), [0]);
        assert_eq!(
            column_moved(&[0], &[0], false, Move::Down),
            Vec::<usize>::new()
        );
        assert_eq!(column_moved(&[0], &[3], false, Move::Down), [1]);
        assert_eq!(column_moved(&[2], &[3], false, Move::Down), [2]);
        assert_eq!(column_moved(&[1], &[3], true, Move::Right), [1, 0]);
        assert_eq!(column_moved(&[1], &[3], false, Move::Right), [1]);
        assert_eq!(column_moved(&[1, 0], &[3, 2], false, Move::Left), [1]);
        assert_eq!(column_moved(&[1], &[3], false, Move::Left), [1]);
        assert_eq!(column_moved(&[1, 1], &[3, 2], false, Move::Up), [1, 0]);
        assert_eq!(column_moved(&[1, 0], &[3, 2], false, Move::Last), [1, 1]);
        assert_eq!(column_moved(&[1, 1], &[3, 2], false, Move::First), [1, 0]);
        assert_eq!(
            column_moved(&[5, 1], &[3, 0], false, Move::Up),
            [1],
            "an emptied column closes"
        );
        assert_eq!(
            column_moved(&[1, 0], &[3], true, Move::Right),
            [1],
            "a column beyond the facts closes"
        );
    }

    #[test]
    fn the_specification_agrees_over_a_drawn_domain() {
        for count in 0..=BOUND {
            for across in 0..=BOUND {
                for selected in (0..=BOUND).map(Some).chain([None]) {
                    for key in KEYS {
                        assert!(
                            grid_spec(selected, count, across, key),
                            "{selected:?} {count} {across} {key:?}"
                        );
                    }
                }
            }
        }
        let small = [0, 1, 3];
        for first in small {
            for second in small {
                let lengths = [first, second];
                for depth in 0..=2 {
                    for place in small {
                        let path = alloc::vec![place; depth];
                        for opens in [false, true] {
                            for key in KEYS {
                                assert!(column_checked(&path, &lengths, opens, key).is_some());
                                assert!(column_checked(&path, &lengths[..1], opens, key).is_some());
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_specifications_of_grouping_agree_over_a_drawn_domain() {
        let all = [0, 1, 2];
        for count in 0..=3 {
            for code in 0..27_u8 {
                let keys = [code % 3, code / 3 % 3, code / 9];
                assert!(grouping_spec(&all[..count], &keys), "{count} {keys:?}");
            }
        }
        let times = [
            0,
            WEEK - 1,
            WEEK,
            WEEK + 1,
            RECENT,
            RECENT + 1,
            NOW,
            u64::MAX,
        ];
        for time in times {
            for later in times {
                for now in times {
                    assert!(period_spec(time, later, now), "{time} {later} {now}");
                }
            }
        }
    }

    #[test]
    fn a_grouped_grid_moves_a_row_across_the_headings() {
        // Two groups of five and two, three to a row: rows 0 1 2 | 3 4 | 5 6.
        let sizes = [5, 2];
        assert_eq!(grouped_moved(Some(4), &sizes, 3, Move::Down), Some(6));
        assert_eq!(grouped_moved(Some(2), &sizes, 3, Move::Down), Some(4));
        assert_eq!(grouped_moved(Some(6), &sizes, 3, Move::Up), Some(4));
        assert_eq!(grouped_moved(Some(5), &sizes, 3, Move::Up), Some(3));
        assert_eq!(grouped_moved(Some(6), &sizes, 3, Move::Down), Some(6));
        assert_eq!(grouped_moved(Some(1), &sizes, 3, Move::Up), Some(1));
        assert_eq!(grouped_moved(Some(4), &sizes, 3, Move::Right), Some(5));
        assert_eq!(grouped_moved(None, &sizes, 3, Move::Down), Some(0));
        assert_eq!(grouped_moved(Some(0), &[0, 0], 3, Move::Down), None);
        assert_eq!(
            grouped_moved(Some(2), &[0, 4], 2, Move::Up),
            Some(0),
            "an empty group adds no row"
        );
    }

    #[test]
    fn the_column_step_and_keeping_specifications_agree_over_a_drawn_domain() {
        for last in 0..=3 {
            for current in 0..=last {
                for first in [false, true] {
                    for opens in [false, true] {
                        let standing = Standing {
                            current,
                            last,
                            first,
                            opens,
                        };
                        for key in KEYS {
                            assert!(column_step_spec(standing, key), "{standing:?} {key:?}");
                        }
                    }
                }
            }
        }
        let small = [0, 1, 3];
        for depth in 0..=3 {
            for columns in 0..=3 {
                for first in small {
                    for second in small {
                        let lengths = [first, second, 1];
                        let path = [2, 0, 1];
                        assert!(kept_spec(&path[..depth], &lengths[..columns]));
                    }
                }
            }
        }
    }

    #[test]
    fn the_grouped_grid_specification_agrees_over_a_drawn_domain() {
        for first in 0..=3 {
            for second in 0..=3 {
                for groups in 0..=2 {
                    let sizes = [first, second];
                    for across in 0..=3 {
                        for selected in (0..=6).map(Some).chain([None]) {
                            for key in KEYS {
                                assert!(
                                    grouped_spec(selected, &sizes[..groups], across, key),
                                    "{selected:?} {sizes:?} {groups} {across} {key:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_grouping_gathers_the_listing_under_ordered_keys() {
        let ids = [0, 1, 2, 3].map(|place: u8| ItemId([place; 16]));
        let sites = ["mozilla.org", "github.com", "mozilla.org", "example.com"];
        let groups = grouped(&ids, |id| sites[usize::from(id.0[0])]);
        assert_eq!(
            groups,
            [
                ("example.com", alloc::vec![ids[3]]),
                ("github.com", alloc::vec![ids[1]]),
                ("mozilla.org", alloc::vec![ids[0], ids[2]]),
            ]
        );
        assert_eq!(grouped(&[], |id: ItemId| id), []);
    }

    #[test]
    fn a_period_s_bounds_hold_exactly_its_times() {
        assert_eq!(period_bounds(Period::Week, NOW), (NOW - WEEK, None));
        assert_eq!(
            period_bounds(Period::Month, NOW),
            (NOW - RECENT, Some(NOW - WEEK))
        );
        assert_eq!(period_bounds(Period::Earlier, NOW), (0, Some(NOW - RECENT)));
        assert_eq!(period_bounds(Period::Month, WEEK - 1), (0, Some(0)));
        assert_eq!(recent_since(NOW), NOW - RECENT);
        assert_eq!(recent_since(RECENT - 1), 0);
        let nows = [
            0,
            1,
            WEEK - 1,
            WEEK,
            WEEK + 1,
            RECENT,
            RECENT + 1,
            NOW,
            u64::MAX,
        ];
        for now in nows {
            let mut times = alloc::vec![0, 1, now, u64::MAX];
            for span in [WEEK, RECENT] {
                for edge in [
                    now.saturating_sub(span),
                    now.saturating_sub(span).saturating_sub(1),
                ] {
                    times.push(edge);
                }
            }
            times.push(now.saturating_add(1));
            for time in times {
                for age in [Period::Week, Period::Month, Period::Earlier] {
                    assert!(bounds_spec(time, now, age), "{time} {now} {age:?}");
                }
            }
        }
    }

    #[test]
    fn a_time_falls_in_the_period_of_its_age() {
        assert_eq!(period(NOW, NOW), Period::Week);
        assert_eq!(period(NOW - WEEK, NOW), Period::Week);
        assert_eq!(period(NOW - WEEK - 1, NOW), Period::Month);
        assert_eq!(period(NOW - RECENT, NOW), Period::Month);
        assert_eq!(period(NOW - RECENT - 1, NOW), Period::Earlier);
        assert_eq!(
            period(NOW + 1, NOW),
            Period::Week,
            "a clock that moved back"
        );
        assert_eq!(WEEK, WEEK_DAYS * DAY);
    }
}
