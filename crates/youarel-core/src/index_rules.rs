//! The keys of the indexed store, and the rows each shortcut files under them.
//!
//! Every table keeps its rows in the order of their keys, so a key spells the order a reader lists its rows in.
//! A key ends a text with [`TERMINATOR`], so a text sorts before every longer text that starts with it, as text does.
//! A shortcut sits in its folder while the tree holds that folder and at the top otherwise.
//! A shortcut that shows files a row in every listing's table, one that the trash hides files only the rows that find it again, and one gone for good files none, so its record stays for the merge while every listing passes it over.
//! A shortcut's rows follow from its record, its folder's standing, and its link's site alone, so an edit or a merge changes exactly the rows that [`delta`] names.

use crate::hash_rules;
use crate::library_rules::{added, address};
use crate::match_rules::SEPARATORS;
use crate::position_rules::Key;
use crate::rank_rules::DAY;
use crate::state::{Entry, ItemId, Presence, Shortcut};
use alloc::string::String;
use alloc::vec::Vec;

/// The bytes of a home key: a tag, then a folder's identifier or zeros for the top.
pub const HOME_KEY: usize = 17;
/// The tag of the top's home key.
const TOP: u8 = 0x00;
/// The tag of a folder's home key.
const IN_FOLDER: u8 = 0x01;
/// The byte that ends a text in a key, below every byte that a position, a sort prefix, a site, or a token holds.
pub const TERMINATOR: u8 = 0x00;
/// The kind of a folder in a placement key, which sorts a folder before a shortcut at one position, as entries order.
const FOLDER_KIND: u8 = 0x00;
/// The kind of a shortcut in a placement key.
const SHORTCUT_KIND: u8 = 0x01;
/// How many bytes of a folded title or of a link's address a sort key keeps.
pub const SORT_PREFIX: usize = 32;
/// How many bytes of a folded word a token key keeps.
pub const TOKEN_PREFIX: usize = 12;
/// How many days the record of an item gone for good stays before a compaction may delete it.
pub const TOMBSTONE_HORIZON_DAYS: u64 = 180;
/// How long the record of an item gone for good stays before a compaction may delete it, in milliseconds.
pub const TOMBSTONE_HORIZON: u64 = TOMBSTONE_HORIZON_DAYS * DAY;
/// The bytes of an identifier.
const ID: usize = 16;
/// The bytes of a time in milliseconds.
const TIME: usize = 8;
/// The bytes of a link's hash.
const HASH: usize = 8;

/// The key of a home: the top, or a folder.
pub type HomeKey = [u8; HOME_KEY];

/// The tables of the index rows and of the counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Table {
    /// Each shortcut by its home, its position, and its identifier.
    Placement,
    /// Each shortcut by when it was added.
    Added,
    /// Each shortcut by its home, then when it was added.
    FolderAdded,
    /// Each shortcut by its folded title.
    Title,
    /// Each shortcut by its home, then its folded title.
    FolderTitle,
    /// Each shortcut by its link without the scheme.
    Link,
    /// Each shortcut by its home, then its link without the scheme.
    FolderLink,
    /// Each shortcut by the hash of its link, then its home.
    LinkHash,
    /// Each word of a shortcut's texts.
    Token,
    /// Each shortcut by its link's site, then the latest added first.
    Site,
    /// Each shortcut that declares a folder the tree does not hold, by that folder.
    Orphan,
    /// Each shortcut the trash hides, by its home.
    Hidden,
    /// Each shortcut in the trash, by its folded title.
    Trash,
    /// How many shortcuts sit in each home.
    HomeCount,
    /// How many shortcuts each site holds.
    SiteCount,
    /// How many shortcuts were added on each day since the Unix epoch.
    DayCount,
}

impl Table {
    /// Whether the table counts its keys, so a row adds one under its key and taking it away subtracts one.
    #[must_use]
    pub const fn counts(self) -> bool {
        matches!(self, Self::HomeCount | Self::SiteCount | Self::DayCount)
    }
}

/// One row of a table.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Row {
    pub table: Table,
    pub key: Vec<u8>,
}

/// Where an item stands for the listings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Standing {
    /// It shows where it sits.
    Shown,
    /// It waits in the trash, or sits inside a folder that does.
    Hidden,
    /// It is gone for good, or was never there.
    Gone,
}

/// The standing of an item: gone once it is gone for good, hidden while it waits in the trash or sits in a home the trash hides, and shown otherwise.
#[must_use]
pub const fn standing(presence: Presence, home_hidden: bool) -> Standing {
    match presence {
        Presence::Purged => Standing::Gone,
        Presence::Trashed => Standing::Hidden,
        Presence::Live if home_hidden => Standing::Hidden,
        Presence::Live => Standing::Shown,
    }
}

/// What a shortcut files: its standing, whether it sits in the folder it declares, and whether it files the row of an orphan and the row of the trash.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Filing {
    pub standing: Standing,
    /// Whether the shortcut sits in the folder it declares rather than at the top.
    pub in_folder: bool,
    /// Whether it waits at the top for a folder the tree does not hold.
    pub orphan: bool,
    /// Whether it waits in the trash itself.
    pub trash: bool,
}

/// What a shortcut of a presence files, which declares no folder, as `None`, or one of a standing.
///
/// A shortcut sits in the folder it declares while the tree holds it, shown or hidden, and at the top otherwise, where it waits as an orphan unless it is gone for good.
#[must_use]
pub const fn filing(presence: Presence, declared: Option<Standing>) -> Filing {
    let in_folder = matches!(declared, Some(Standing::Shown | Standing::Hidden));
    let home_hidden = matches!(declared, Some(Standing::Hidden));
    let gone = matches!(presence, Presence::Purged);
    Filing {
        standing: standing(presence, home_hidden),
        in_folder,
        orphan: !gone && matches!(declared, Some(Standing::Gone)),
        trash: matches!(presence, Presence::Trashed),
    }
}

/// What decides a shortcut's rows.
#[derive(Clone, Copy, Debug)]
pub struct Indexed<'a> {
    pub id: ItemId,
    pub shortcut: &'a Shortcut,
    /// The standing of the folder the shortcut declares, which decides whether the shortcut sits there or at the top and whether the trash hides it; a shortcut that declares none passes it over.
    pub folder: Standing,
    /// The registrable domain of the link's host, which the adapter reads with the URL parser.
    pub site: &'a str,
}

/// The rows to take away and the rows to add.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Delta<T> {
    pub removed: Vec<T>,
    pub inserted: Vec<T>,
}

/// The key of a home: the top's, or a folder's.
#[must_use]
pub const fn home_key(folder: Option<ItemId>) -> HomeKey {
    let mut key = [TOP; HOME_KEY];
    if let Some(ItemId(id)) = folder
        && let Some((tag, rest)) = key.split_first_mut()
    {
        *tag = IN_FOLDER;
        rest.copy_from_slice(&id);
    }
    key
}

/// The folder a shortcut sits in, as every replica draws it: the one it declares while the tree holds it, and the top otherwise.
#[must_use]
pub const fn home(declared: Option<ItemId>, held: bool) -> Option<ItemId> {
    match declared {
        Some(folder) if held => Some(folder),
        Some(_) | None => None,
    }
}

/// The key of an entry's place among its home's entries: the home, the position, the kind, and the identifier.
#[must_use]
pub fn placement_key(home: Option<ItemId>, position: &Key, entry: Entry) -> Vec<u8> {
    let (kind, ItemId(id)) = match entry {
        Entry::Folder(id) => (FOLDER_KIND, id),
        Entry::Shortcut(id) => (SHORTCUT_KIND, id),
    };
    [
        home_key(home).as_slice(),
        position.as_str().as_bytes(),
        &[TERMINATOR, kind],
        &id,
    ]
    .concat()
}

/// The key of a time and an identifier, which orders by the time and then by the identifier.
#[must_use]
pub const fn added_key(time: u64, ItemId(id): ItemId) -> [u8; TIME + ID] {
    let mut key = [0; TIME + ID];
    let (front, back) = key.split_at_mut(TIME);
    front.copy_from_slice(&time.to_be_bytes());
    back.copy_from_slice(&id);
    key
}

/// The start of a text that a key keeps: at most `N` bytes, and none from its first [`TERMINATOR`] on.
#[must_use]
pub fn prefix<const N: usize>(text: &[u8]) -> &[u8] {
    let kept = text.get(..N).unwrap_or(text);
    kept.split(|byte| *byte == TERMINATOR)
        .next()
        .unwrap_or(kept)
}

/// The key of a text's start and an identifier, which orders by that start and then by the identifier.
#[must_use]
pub fn sort_key(text: &str, ItemId(id): ItemId) -> Vec<u8> {
    let kept = prefix::<SORT_PREFIX>(text.as_bytes());
    let mut key = Vec::with_capacity(kept.len().saturating_add(1).saturating_add(ID));
    key.extend_from_slice(kept);
    key.push(TERMINATOR);
    key.extend_from_slice(&id);
    key
}

/// The start of a sort key that comes before its identifier: the kept text and its terminator.
#[must_use]
pub fn sort_part(key: &[u8]) -> &[u8] {
    key.get(..key.len().saturating_sub(ID)).unwrap_or_default()
}

/// The identifier that ends a key.
#[must_use]
pub fn identified(key: &[u8]) -> Option<ItemId> {
    key.split_last_chunk::<ID>().map(|(_, id)| ItemId(*id))
}

/// The key under which a folded title sorts.
#[must_use]
pub fn title_key(title: &str, id: ItemId) -> Vec<u8> {
    sort_key(&title.to_lowercase(), id)
}

/// The key under which a link sorts: its address without the scheme.
#[must_use]
pub fn link_key(link: &str, id: ItemId) -> Vec<u8> {
    sort_key(address(link), id)
}

/// The start of every key of the shortcuts whose link hashes as `link`'s does, in every home.
#[must_use]
pub fn link_hash_prefix(link: &str) -> [u8; HASH] {
    hash_rules::hash(link.as_bytes()).to_be_bytes()
}

/// The start of every key of the shortcuts in `home` whose link hashes as `link`'s does.
#[must_use]
pub fn link_hash_home(link: &str, home: Option<ItemId>) -> Vec<u8> {
    joined(&link_hash_prefix(link), &home_key(home))
}

/// The words of a text, each folded to ASCII lowercase and cut to at most [`TOKEN_PREFIX`] bytes at a character's end.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|character: char| character.is_whitespace() || SEPARATORS.contains(&character))
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut token = String::new();
            for character in word.chars().map(|character| character.to_ascii_lowercase()) {
                if token.len().saturating_add(character.len_utf8()) > TOKEN_PREFIX {
                    break;
                }
                token.push(character);
            }
            token
        })
}

/// The words of a query in their order, each folded and cut as [`tokens`] files the words of a shortcut.
#[must_use]
pub fn query_words(text: &str) -> Vec<Token> {
    words(text)
        .filter(|word| !word.is_empty())
        .map(Token)
        .collect()
}

/// A word that a search may start a typed word from, folded and cut as [`tokens`] makes it.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Token(String);

impl Token {
    /// The word as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The words a search may start a typed word from: those of the title, the alias, the link's address, and the note, each once.
#[must_use]
pub fn tokens(shortcut: &Shortcut) -> Vec<Token> {
    let mut found: Vec<Token> = [
        shortcut.title.value.as_str(),
        shortcut.alias.value.as_str(),
        address(&shortcut.link.value),
        shortcut.note.value.as_str(),
    ]
    .into_iter()
    .flat_map(words)
    .filter(|word| !word.is_empty())
    .map(Token)
    .collect();
    found.sort_unstable();
    found.dedup();
    found
}

/// A text, its terminator, and more bytes.
fn ended(text: &[u8], more: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(text.len().saturating_add(1).saturating_add(more.len()));
    key.extend_from_slice(text);
    key.push(TERMINATOR);
    key.extend_from_slice(more);
    key
}

/// Two parts of a key, one after the other.
fn joined(front: &[u8], back: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(front.len().saturating_add(back.len()));
    key.extend_from_slice(front);
    key.extend_from_slice(back);
    key
}

/// The site's part of a key, which keeps the site whole up to its first terminator, since a group by site names it whole.
#[must_use]
pub fn site_part(site: &str) -> &[u8] {
    let bytes = site.as_bytes();
    bytes
        .split(|byte| *byte == TERMINATOR)
        .next()
        .unwrap_or(bytes)
}

/// The rows of a shortcut that shows: one in each table of its home, its time, its texts, and its site.
fn shown_rows(id: ItemId, shortcut: &Shortcut, home: Option<ItemId>, site: &str) -> Vec<Row> {
    let home_key = home_key(home);
    let time = added(id, shortcut);
    let by_time = added_key(time, id);
    let title = title_key(&shortcut.title.value, id);
    let link = link_key(&shortcut.link.value, id);
    let row = |table, key| Row { table, key };
    let mut rows = alloc::vec![
        row(
            Table::Placement,
            placement_key(home, &shortcut.position.value, Entry::Shortcut(id)),
        ),
        row(Table::Added, by_time.to_vec()),
        row(Table::FolderAdded, joined(&home_key, &by_time)),
        row(Table::FolderTitle, joined(&home_key, &title)),
        row(Table::Title, title),
        row(Table::FolderLink, joined(&home_key, &link)),
        row(Table::Link, link),
        row(
            Table::LinkHash,
            joined(&link_hash_home(&shortcut.link.value, home), &id.0),
        ),
        row(Table::Site, ended(site_part(site), &added_key(!time, id)),),
        row(Table::HomeCount, home_key.to_vec()),
        row(Table::SiteCount, site_part(site).to_vec()),
        row(Table::DayCount, day_of(time).to_be_bytes().to_vec()),
    ];
    rows.extend(
        tokens(shortcut)
            .iter()
            .map(|token| row(Table::Token, ended(token.as_str().as_bytes(), &id.0))),
    );
    rows
}

/// The rows a shortcut files, as [`filing`] decides them: none once gone for good, one in each table of its home, its time, its texts, and its site while it shows, and its row by its home while the trash hides it, with its row of the trash while it waits there itself; and a row of the folder it declares while it waits at the top in that folder's stead.
#[must_use]
pub fn rows(indexed: &Indexed<'_>) -> Vec<Row> {
    let Indexed {
        id,
        shortcut,
        folder,
        site,
    } = *indexed;
    let declared = shortcut.folder.value;
    let filing = filing(shortcut.presence.value, declared.map(|_| folder));
    let home = home(declared, filing.in_folder);
    let row = |table, key| Row { table, key };
    let mut rows = match filing.standing {
        Standing::Gone => return Vec::new(),
        Standing::Shown => shown_rows(id, shortcut, home, site),
        Standing::Hidden => alloc::vec![row(Table::Hidden, joined(&home_key(home), &id.0))],
    };
    if filing.trash {
        rows.push(row(Table::Trash, title_key(&shortcut.title.value, id)));
    }
    if let (Some(ItemId(awaited)), true) = (declared, filing.orphan) {
        rows.push(row(Table::Orphan, joined(&awaited, &id.0)));
    }
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// One row that a delta takes away or adds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Change<'a, T> {
    Removed(&'a T),
    Inserted(&'a T),
}

/// The walk over two ascending lists of rows that yields, in ascending order, each row that only one of them holds.
#[derive(Clone, Debug)]
pub struct Changes<'a, T> {
    old: &'a [T],
    new: &'a [T],
}

impl<'a, T: Ord> Iterator for Changes<'a, T> {
    type Item = Change<'a, T>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match (self.old.split_first(), self.new.split_first()) {
                (None, None) => return None,
                (Some((first, rest)), None) => {
                    self.old = rest;
                    return Some(Change::Removed(first));
                }
                (None, Some((first, rest))) => {
                    self.new = rest;
                    return Some(Change::Inserted(first));
                }
                (Some((left, left_rest)), Some((right, right_rest))) => match left.cmp(right) {
                    core::cmp::Ordering::Less => {
                        self.old = left_rest;
                        return Some(Change::Removed(left));
                    }
                    core::cmp::Ordering::Greater => {
                        self.new = right_rest;
                        return Some(Change::Inserted(right));
                    }
                    core::cmp::Ordering::Equal => {
                        self.old = left_rest;
                        self.new = right_rest;
                    }
                },
            }
        }
    }
}

/// The rows that turn the ascending rows `before` into the ascending rows `after`, one at a time.
#[must_use]
pub const fn changes<'a, T>(before: &'a [T], after: &'a [T]) -> Changes<'a, T> {
    Changes {
        old: before,
        new: after,
    }
}

/// What turns the ascending rows `before` into the ascending rows `after`: every row of `before` that `after` lacks, and every row of `after` that `before` lacks, each in ascending order.
#[must_use]
pub fn delta<T: Ord + Clone>(before: &[T], after: &[T]) -> Delta<T> {
    let mut found = Delta {
        removed: Vec::new(),
        inserted: Vec::new(),
    };
    for change in changes(before, after) {
        match change {
            Change::Removed(row) => found.removed.push(row.clone()),
            Change::Inserted(row) => found.inserted.push(row.clone()),
        }
    }
    found
}

/// The day whose count per day counts a shortcut added at a time.
#[must_use]
pub const fn day_of(time: u64) -> u64 {
    match time.checked_div(DAY) {
        Some(day) => day,
        None => 0,
    }
}

/// The whole days within times, which the counts per day count at once.
///
/// The times run from `from` on and before `until`, or on without an end, and their whole days from the first day that starts at `from` or later to before the day `until` falls in.
/// Times that hold no whole day, or whose first whole day would start past every time, hold none, and a reader counts their rows one by one.
#[must_use]
pub const fn whole_days(from: u64, until: Option<u64>) -> Option<(u64, Option<u64>)> {
    let first = from.div_ceil(DAY);
    if first.checked_mul(DAY).is_none() {
        return None;
    }
    match until {
        None => Some((first, None)),
        Some(until) => {
            let end = day_of(until);
            if end > first {
                Some((first, Some(end)))
            } else {
                None
            }
        }
    }
}

/// Whether the record of an item gone for good, gone at `purged`, has stayed past the horizon at `now`, so a compaction may delete it; a removal after `now`, as a clock that moved back leaves it, stays.
///
/// An item in the trash keeps its record whole, since a put back returns it.
#[must_use]
pub const fn purgeable(purged: u64, now: u64) -> bool {
    match now.checked_sub(purged) {
        Some(age) => age >= TOMBSTONE_HORIZON,
        None => false,
    }
}

/// The properties Kani proves over small keys: a key's order is the order of its parts, a kept start never reverses the order of texts, a shortcut sits at the top exactly when its folder is gone, a shortcut files as its presence and its folder decide, and a delta turns the old rows into the new ones.
#[cfg(any(test, kani))]
mod proofs {
    #[cfg(test)]
    use super::Delta;
    #[cfg(kani)]
    use super::{Change, Key, changes, home, home_key, placement_key, prefix, purgeable};
    use super::{Filing, Standing, filing, whole_days};
    use crate::rank_rules::DAY;
    use crate::state::Presence;
    #[cfg(kani)]
    use crate::state::{Entry, ItemId};
    #[cfg(kani)]
    use alloc::vec::Vec;

    /// Every presence, which the tests sweep.
    #[cfg(test)]
    pub(super) const PRESENCES: [Presence; 3] =
        [Presence::Live, Presence::Trashed, Presence::Purged];
    /// Every standing of a folder a shortcut declares, and none for a shortcut that declares none, which the tests sweep.
    #[cfg(test)]
    pub(super) const DECLARED: [Option<Standing>; 4] = [
        None,
        Some(Standing::Shown),
        Some(Standing::Hidden),
        Some(Standing::Gone),
    ];

    /// What the filing promises: gone exactly for a shortcut gone for good, shown exactly for a live one whose folder the trash does not hide, in its folder exactly while the tree holds that folder, an orphan exactly while it waits for a folder the tree does not hold and is not gone, and in the trash exactly while it waits there itself.
    pub(super) fn filing_holds(presence: Presence, declared: Option<Standing>) -> bool {
        let Filing {
            standing,
            in_folder,
            orphan,
            trash,
        } = filing(presence, declared);
        let gone = presence == Presence::Purged;
        let held = matches!(declared, Some(Standing::Shown | Standing::Hidden));
        let shows = presence == Presence::Live && declared != Some(Standing::Hidden);
        (standing == Standing::Gone) == gone
            && (standing == Standing::Shown) == shows
            && in_folder == held
            && orphan == (!gone && declared == Some(Standing::Gone))
            && trash == (presence == Presence::Trashed)
    }

    /// The pairs of positions the harnesses place two entries at, which hold every way two positions order, since keys and their parts both order the reversed pair the other way: a position and a longer one that it starts, one position, and positions apart at their first symbol and at a later one.
    #[cfg(kani)]
    const STARTING: (&str, &str) = ("V", "VV");
    #[cfg(kani)]
    const SAME: (&str, &str) = ("V", "V");
    #[cfg(kani)]
    const APART_FIRST: (&str, &str) = ("1", "z");
    #[cfg(kani)]
    const APART_LATER: (&str, &str) = ("VV", "Vz");
    /// The most rows of each side of a delta that the harness draws.
    pub(super) const ROWS: usize = 3;
    /// The days from the epoch within which the harness of the whole days draws its times.
    #[cfg(kani)]
    const DAYS: u64 = 3;
    /// The longest text the harness of the kept start draws, and how many bytes it keeps.
    #[cfg(kani)]
    const TEXT: usize = 3;
    #[cfg(kani)]
    const KEPT: usize = 2;

    /// The rows below eight that a list holds, as the bits of a mask.
    fn mask(rows: &[u8]) -> u8 {
        rows.iter().fold(0, |mask, row| {
            mask | 1_u8.checked_shl(u32::from(*row)).unwrap_or_default()
        })
    }

    /// Whether rows ascend strictly.
    pub(super) fn ascending(rows: &[u8]) -> bool {
        rows.windows(2).all(|pair| pair.first() < pair.last())
    }

    /// The rows below eight that one side of a walk took, and whether it took them in strictly ascending order.
    #[cfg(kani)]
    struct Taken {
        mask: u8,
        last: Option<u8>,
        ascending: bool,
    }

    #[cfg(kani)]
    impl Default for Taken {
        fn default() -> Self {
            Self {
                mask: 0,
                last: None,
                ascending: true,
            }
        }
    }

    #[cfg(kani)]
    impl Taken {
        /// Takes one more row.
        fn take(&mut self, row: u8) {
            self.ascending &= self.last.is_none_or(|last| last < row);
            self.last = Some(row);
            self.mask |= mask(&[row]);
        }
    }

    /// What the whole days of times promise: the first starts at `from` or later and the day before it starts earlier, the last ends at the time `until` falls in a day after, so the rows before and after them fill the rest; and times without them hold no day whole.
    pub(super) fn days_spec(from: u64, until: Option<u64>) -> bool {
        let earliest = from.div_ceil(DAY);
        let fits = |start: u64| {
            start
                .checked_add(DAY)
                .is_some_and(|stop| until.is_none_or(|until| stop <= until))
        };
        match whole_days(from, until) {
            Some((first, end)) => {
                let starts = first.checked_mul(DAY);
                let tight = first
                    .checked_sub(1)
                    .is_none_or(|before| before.checked_mul(DAY).is_some_and(|start| start < from));
                let ended = match (end, until) {
                    (None, None) => true,
                    (Some(end), Some(until)) => {
                        end > first
                            && end.checked_mul(DAY).is_some_and(|start| {
                                until.checked_sub(start).is_some_and(|rest| rest < DAY)
                            })
                    }
                    (Some(_), None) | (None, Some(_)) => false,
                };
                starts.is_some_and(|start| start >= from) && tight && ended
            }
            None => earliest.checked_mul(DAY).is_none_or(|start| !fits(start)),
        }
    }

    /// What a delta promises for strictly ascending rows below eight: it takes away exactly the old rows the new lack and adds exactly the new rows the old lack, each in ascending order.
    #[cfg(test)]
    pub(super) fn delta_holds(before: &[u8], after: &[u8], changes: &Delta<u8>) -> bool {
        ascending(&changes.removed)
            && ascending(&changes.inserted)
            && mask(&changes.removed) == mask(before) & !mask(after)
            && mask(&changes.inserted) == mask(after) & !mask(before)
    }

    /// An identifier whose first and last bytes are drawn, which tells a key that keeps an identifier's bytes in order from one that drops or reverses them.
    #[cfg(kani)]
    fn any_id() -> ItemId {
        let mut id = [0; super::ID];
        id[0] = kani::any();
        id[super::ID - 1] = kani::any();
        ItemId(id)
    }

    /// An entry of either kind with a drawn identifier.
    #[cfg(kani)]
    fn any_entry() -> Entry {
        let id = any_id();
        if kani::any() {
            Entry::Folder(id)
        } else {
            Entry::Shortcut(id)
        }
    }

    /// The top, or a folder of a drawn identifier.
    #[cfg(kani)]
    fn any_home() -> Option<ItemId> {
        if kani::any() { Some(any_id()) } else { None }
    }

    /// Whether the placement keys of two entries in two homes at the positions `first` and `second` order as their homes, then their positions, then their entries.
    #[cfg(kani)]
    fn orders_as_parts(
        (first, second): (&str, &str),
        homes: (Option<ItemId>, Option<ItemId>),
        entries: (Entry, Entry),
    ) -> bool {
        let (first_position, second_position) =
            (Key::parse(first).unwrap(), Key::parse(second).unwrap());
        let keys = placement_key(homes.0, &first_position, entries.0).cmp(&placement_key(
            homes.1,
            &second_position,
            entries.1,
        ));
        let parts = (home_key(homes.0), &first_position, entries.0).cmp(&(
            home_key(homes.1),
            &second_position,
            entries.1,
        ));
        keys == parts
    }

    /// Every two entries in every two homes, at one position and at a position and a longer one that it starts: placement keys order as their homes, then their positions, then their entries.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(40)]
    fn a_placement_key_orders_as_its_home_its_position_and_its_entry() {
        let homes = (any_home(), any_home());
        let entries = (any_entry(), any_entry());
        kani::assert(
            orders_as_parts(STARTING, homes, entries),
            "a placement key at a position before a longer one that starts with it orders as its parts",
        );
        kani::assert(
            orders_as_parts(SAME, homes, entries),
            "placement keys at one position order as their parts",
        );
        kani::cover!(
            homes.0 == homes.1 && entries.0 != entries.1,
            "two entries in one home"
        );
    }

    /// Every two entries in every two homes, at positions apart at their first symbol and at a later one: placement keys order as their homes, then their positions, then their entries.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(40)]
    fn placement_keys_at_positions_apart_order_as_their_parts() {
        let homes = (any_home(), any_home());
        let entries = (any_entry(), any_entry());
        kani::assert(
            orders_as_parts(APART_FIRST, homes, entries),
            "placement keys at positions apart at their first symbol order as their parts",
        );
        kani::assert(
            orders_as_parts(APART_LATER, homes, entries),
            "placement keys at positions apart at a later symbol order as their parts",
        );
        kani::cover!(homes.0 != homes.1, "two homes");
    }

    /// Every pair of texts of up to three bytes, a terminator among them: the start a key keeps never orders two texts against their order, and a key orders as its kept start and then its identifier.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(8)]
    fn a_kept_start_never_reverses_the_order_of_texts() {
        let first: [u8; TEXT] = kani::any();
        let second: [u8; TEXT] = kani::any();
        let (first_length, second_length): (usize, usize) = (kani::any(), kani::any());
        kani::assume(first_length <= TEXT && second_length <= TEXT);
        let (left, right) = (&first[..first_length], &second[..second_length]);
        let (left_kept, right_kept) = (prefix::<KEPT>(left), prefix::<KEPT>(right));
        kani::assert(
            left_kept.len() <= KEPT && !left_kept.contains(&super::TERMINATOR),
            "a kept start holds no terminator and fits",
        );
        kani::assert(
            left_kept >= right_kept || left < right,
            "a kept start never reverses the order of texts",
        );
        let (left_id, right_id): (u8, u8) = (kani::any(), kani::any());
        let key = |kept: &[u8], id: u8| -> Vec<u8> {
            let mut key = kept.to_vec();
            key.push(super::TERMINATOR);
            key.push(id);
            key
        };
        kani::assert(
            key(left_kept, left_id).cmp(&key(right_kept, right_id))
                == (left_kept, left_id).cmp(&(right_kept, right_id)),
            "a key orders as its kept start and then its identifier",
        );
        kani::cover!(
            left_kept == right_kept && left != right,
            "two texts keep one start"
        );
    }

    /// Every presence, and every folder a shortcut declares in every standing or none: the shortcut files as its presence and that standing decide.
    #[cfg(kani)]
    #[kani::proof]
    fn a_shortcut_files_as_its_presence_and_its_folder_s_standing_decide() {
        let presence = match kani::any::<u8>() {
            0 => Presence::Live,
            1 => Presence::Trashed,
            _ => Presence::Purged,
        };
        let declared = match kani::any::<u8>() {
            0 => None,
            1 => Some(Standing::Shown),
            2 => Some(Standing::Hidden),
            _ => Some(Standing::Gone),
        };
        kani::assert(
            filing_holds(presence, declared),
            "a shortcut files as its presence and its folder's standing decide",
        );
        let filed = filing(presence, declared);
        kani::cover!(
            presence == Presence::Live && filed.standing == Standing::Hidden,
            "a live shortcut the trash hides in its folder"
        );
        kani::cover!(
            filed.orphan && filed.trash,
            "a shortcut in the trash that waits for its folder"
        );
    }

    /// Every declared folder, whether the tree holds it, every removal time, and every clock reading: a shortcut sits at the top exactly when it declares no folder or the tree does not hold its folder, the top's key sorts before every folder's, and the record of an item gone for good may go exactly once it stayed past the horizon.
    #[cfg(kani)]
    #[kani::proof]
    fn a_shortcut_sits_at_the_top_exactly_when_its_folder_is_gone() {
        let declared = any_home();
        let held: bool = kani::any();
        let sits = home(declared, held);
        kani::assert(
            sits.is_none() == (declared.is_none() || !held),
            "the top exactly when the folder is gone",
        );
        kani::assert(
            sits.is_none() || sits == declared,
            "a shortcut sits only in the folder it declares",
        );
        kani::assert(
            declared.is_none() || home_key(None) < home_key(declared),
            "the top sorts before every folder",
        );
        let purged: u64 = kani::any();
        let now: u64 = kani::any();
        kani::assert(
            purgeable(purged, now) == (now >= purged && now - purged >= super::TOMBSTONE_HORIZON),
            "the record of an item gone for good goes once it stayed past the horizon",
        );
        kani::cover!(
            declared.is_some() && !held,
            "a shortcut whose folder is gone"
        );
    }

    /// Every start and every end or none within the first days from the epoch or the last days before the end of time: the whole days lie within the times and the rows before and after them fill the rest.
    #[cfg(kani)]
    #[kani::proof]
    fn the_whole_days_of_times_lie_within_them() {
        let span = DAYS * DAY;
        let base = if kani::any() { u64::MAX - span } else { 0 };
        let (from, until): (u64, Option<u64>) = (kani::any(), kani::any());
        kani::assume(from <= span && until.is_none_or(|until| until <= span));
        let (from, until) = (base + from, until.map(|until| base + until));
        kani::assert(
            days_spec(from, until),
            "the whole days lie within the times and the rest fills them",
        );
        kani::cover!(
            matches!(whole_days(from, until), Some((_, Some(_)))) && from % DAY != 0,
            "times that start and end within days, with whole days between"
        );
        kani::cover!(
            until.is_some() && whole_days(from, until).is_none() && from < until.unwrap_or(0),
            "times within two days, which hold no day whole"
        );
    }

    /// Every two strictly ascending lists of up to three rows below four: the delta takes away exactly the rows that went and adds exactly the rows that came.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(8)]
    fn a_delta_removes_every_old_row_and_inserts_every_new_one() {
        let before: [u8; ROWS] = kani::any();
        let after: [u8; ROWS] = kani::any();
        let (before_length, after_length): (usize, usize) = (kani::any(), kani::any());
        kani::assume(before_length <= ROWS && after_length <= ROWS);
        let (old, new) = (&before[..before_length], &after[..after_length]);
        kani::assume(ascending(old) && ascending(new));
        kani::assume(old.iter().chain(new).all(|row| *row < 4));
        let (mut removed, mut inserted) = (Taken::default(), Taken::default());
        for change in changes(old, new) {
            match change {
                Change::Removed(row) => removed.take(*row),
                Change::Inserted(row) => inserted.take(*row),
            }
        }
        kani::assert(
            removed.ascending && removed.mask == mask(old) & !mask(new),
            "a delta removes every old row the new lack, in ascending order",
        );
        kani::assert(
            inserted.ascending && inserted.mask == mask(new) & !mask(old),
            "a delta inserts every new row the old lack, in ascending order",
        );
        kani::cover!(removed.mask != 0 && inserted.mask != 0, "rows go and come");
    }

    /// A position that starts a longer one must place its entry first; this probe claims the longer one comes first and must fail.
    #[cfg(kani)]
    #[kani::proof]
    #[kani::unwind(40)]
    fn rejects_an_index_key_out_of_order() {
        let entry = Entry::Shortcut(ItemId([0; 16]));
        let shorter = Key::parse("V").unwrap();
        let longer = Key::parse("VV").unwrap();
        kani::assert(
            placement_key(None, &longer, entry) < placement_key(None, &shorter, entry),
            "NEGATIVE_INDEX_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::{DECLARED, PRESENCES, ROWS, days_spec, delta_holds, filing_holds};
    use super::{
        Delta, HOME_KEY, Indexed, Key, Row, SORT_PREFIX, Standing, TERMINATOR, TOKEN_PREFIX,
        TOMBSTONE_HORIZON, Table, added_key, day_of, delta, filing, home, home_key, identified,
        joined, link_hash_home, link_hash_prefix, link_key, placement_key, prefix, purgeable,
        query_words, rows, site_part, sort_key, sort_part, standing, title_key, tokens, whole_days,
    };
    use crate::state::Presence;

    #[test]
    fn a_shortcut_files_as_its_presence_and_its_folder_s_standing_decide() {
        for presence in PRESENCES {
            for declared in DECLARED {
                assert!(
                    filing_holds(presence, declared),
                    "{presence:?} {declared:?}"
                );
            }
        }
        assert_eq!(standing(Presence::Live, false), Standing::Shown);
        assert_eq!(standing(Presence::Live, true), Standing::Hidden);
        assert_eq!(standing(Presence::Trashed, false), Standing::Hidden);
        assert_eq!(standing(Presence::Purged, true), Standing::Gone);
        let waiting = filing(Presence::Trashed, Some(Standing::Gone));
        assert!(waiting.orphan && waiting.trash && !waiting.in_folder);
    }

    #[test]
    fn a_shortcut_the_trash_hides_files_its_home_and_one_in_the_trash_its_title() {
        let state = filed(Some(FOLDER));
        let mut shortcut = state.shortcut(ID).unwrap().clone();
        let filed_by = |record: &super::Shortcut, folder| {
            rows(&Indexed {
                id: ID,
                shortcut: record,
                folder,
                site: "example.com",
            })
        };
        let hidden = filed_by(&shortcut, Standing::Hidden);
        assert_eq!(
            hidden,
            [Row {
                table: Table::Hidden,
                key: joined(&home_key(Some(FOLDER)), &ID.0),
            }],
            "a live shortcut in a folder the trash hides"
        );
        shortcut.presence.value = Presence::Trashed;
        let trashed = filed_by(&shortcut, Standing::Shown);
        assert_eq!(
            trashed,
            [
                Row {
                    table: Table::Hidden,
                    key: joined(&home_key(Some(FOLDER)), &ID.0),
                },
                Row {
                    table: Table::Trash,
                    key: title_key("Title", ID),
                },
            ]
        );
        let waiting = filed_by(&shortcut, Standing::Gone);
        assert_eq!(
            waiting
                .iter()
                .map(|row| (row.table, row.key.clone()))
                .collect::<Vec<_>>(),
            [
                (Table::Orphan, [FOLDER.0, ID.0].concat()),
                (Table::Hidden, joined(&home_key(None), &ID.0)),
                (Table::Trash, title_key("Title", ID)),
            ],
            "a shortcut in the trash whose folder went waits at the top"
        );
        shortcut.presence.value = Presence::Purged;
        assert_eq!(filed_by(&shortcut, Standing::Gone), []);
    }

    #[test]
    fn the_counts_per_day_take_the_whole_days_of_times() {
        assert_eq!(day_of(DAY - 1), 0);
        assert_eq!(day_of(3 * DAY), 3);
        assert_eq!(whole_days(0, None), Some((0, None)));
        assert_eq!(whole_days(1, None), Some((1, None)));
        assert_eq!(whole_days(DAY, Some(3 * DAY)), Some((1, Some(3))));
        assert_eq!(whole_days(DAY + 1, Some(3 * DAY + 5)), Some((2, Some(3))));
        assert_eq!(whole_days(DAY + 1, Some(3 * DAY - 1)), None, "no day whole");
        assert_eq!(whole_days(DAY, Some(2 * DAY)), Some((1, Some(2))));
        assert_eq!(whole_days(DAY, Some(2 * DAY - 1)), None);
        assert_eq!(
            whole_days(u64::MAX, None),
            None,
            "no day starts past every time"
        );
        let edges = [
            0,
            1,
            DAY - 1,
            DAY,
            DAY + 1,
            2 * DAY,
            2 * DAY + 1,
            3 * DAY - 1,
            u64::MAX - DAY,
            u64::MAX,
        ];
        for from in edges {
            for until in edges.map(Some).into_iter().chain([None]) {
                assert!(days_spec(from, until), "{from} {until:?}");
            }
        }
    }

    #[test]
    fn a_query_s_words_keep_their_order_and_their_repeats() {
        let words: Vec<String> = query_words("Serde serde.JSON  extraordinarily ")
            .iter()
            .map(|word| String::from(word.as_str()))
            .collect();
        assert_eq!(words, ["serde", "serde", "json", "extraordinar"]);
        assert_eq!(query_words(" / -"), []);
    }
    use crate::clock_rules::{DeviceId, Stamp};
    use crate::rank_rules::DAY;
    use crate::state::{Edit, Entry, ItemId, State};
    use alloc::string::String;
    use alloc::vec::Vec;

    const ID: ItemId = ItemId([7; 16]);

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    #[test]
    fn a_home_key_tags_the_top_and_a_folder() {
        assert_eq!(home_key(None), [0; HOME_KEY]);
        let mut folder = [1; HOME_KEY];
        folder[1..].copy_from_slice(&[9; 16]);
        assert_eq!(home_key(Some(ItemId([9; 16]))), folder);
        assert_eq!(home(Some(ID), true), Some(ID));
        assert_eq!(home(Some(ID), false), None);
        assert_eq!(home(None, true), None);
    }

    #[test]
    fn a_placement_key_spells_its_home_position_kind_and_identifier() {
        let made = placement_key(Some(ItemId([2; 16])), &key("Vz"), Entry::Shortcut(ID));
        let mut expected = home_key(Some(ItemId([2; 16]))).to_vec();
        expected.extend_from_slice(b"Vz");
        expected.extend_from_slice(&[TERMINATOR, 1]);
        expected.extend_from_slice(&ID.0);
        assert_eq!(made, expected);
        let folder = placement_key(Some(ItemId([2; 16])), &key("Vz"), Entry::Folder(ID));
        assert!(folder < made, "a folder before a shortcut at one position");
        assert!(
            placement_key(None, &key("V"), Entry::Shortcut(ItemId([9; 16])))
                < placement_key(None, &key("VV"), Entry::Folder(ItemId([0; 16]))),
            "a position before a longer one that starts with it"
        );
        assert_eq!(identified(&made), Some(ID));
        assert_eq!(identified(&[1, 2]), None);
    }

    #[test]
    fn an_added_key_orders_by_time_then_identifier() {
        let early = added_key(5, ItemId([9; 16]));
        let late = added_key(6, ItemId([0; 16]));
        assert!(early < late);
        assert_eq!(&early[..8], &5_u64.to_be_bytes());
        assert_eq!(&early[8..], &[9; 16]);
        assert!(added_key(5, ItemId([1; 16])) < early);
    }

    #[test]
    fn a_sort_key_keeps_the_start_of_its_text_up_to_a_terminator() {
        assert_eq!(prefix::<2>(b"abc"), b"ab");
        assert_eq!(prefix::<4>(b"a\0bc"), b"a");
        assert_eq!(prefix::<4>(b""), b"");
        let long = "x".repeat(SORT_PREFIX + 5);
        let made = sort_key(&long, ID);
        assert_eq!(made.len(), SORT_PREFIX + 1 + 16);
        assert_eq!(
            sort_part(&made),
            [&long.as_bytes()[..SORT_PREFIX], &[TERMINATOR]].concat()
        );
        assert_eq!(identified(&made), Some(ID));
        assert_eq!(sort_part(&[1, 2]), b"");
        assert_eq!(title_key("ÉCOLE", ID), sort_key("école", ID));
        assert_eq!(
            link_key("https://example.com/a", ID),
            sort_key("example.com/a", ID)
        );
        assert_eq!(link_key("mailto:a", ID), sort_key("mailto:a", ID));
        assert!(sort_key("ab", ID) < sort_key("abc", ItemId([0; 16])));
    }

    #[test]
    fn the_kept_start_never_reverses_the_order_of_texts_over_a_drawn_domain() {
        let pieces: [&[u8]; 5] = [b"", b"a", b"b", b"\0", b"ab"];
        let mut texts: Vec<Vec<u8>> = Vec::new();
        for first in pieces {
            for second in pieces {
                texts.push([first, second].concat());
            }
        }
        for left in &texts {
            for right in &texts {
                let (left_kept, right_kept) = (prefix::<2>(left), prefix::<2>(right));
                assert!(
                    left_kept >= right_kept || left < right,
                    "{left:?} {right:?}"
                );
            }
        }
    }

    #[test]
    fn a_link_hash_prefix_names_the_link_and_then_its_home() {
        let any = link_hash_prefix("https://a.example/");
        assert_eq!(
            any,
            crate::hash_rules::hash(b"https://a.example/").to_be_bytes()
        );
        let top = link_hash_home("https://a.example/", None);
        assert_eq!(top, [any.to_vec(), home_key(None).to_vec()].concat());
        assert_ne!(any, link_hash_prefix("https://b.example/"));
    }

    #[test]
    fn tokens_are_the_folded_words_of_every_text_each_once() {
        let mut state = State::default();
        state
            .apply(
                Edit::AddShortcut {
                    id: ID,
                    link: String::from("https://Docs.RS/serde_json"),
                    title: String::from("Serde JSON  docs"),
                    folder: None,
                },
                Stamp {
                    wall: 1,
                    counter: 0,
                    device: DeviceId([1; 16]),
                },
            )
            .unwrap();
        let mut shortcut = state.shortcut(ID).unwrap().clone();
        shortcut.alias.value = String::from("sj");
        shortcut.note.value = String::from("an extraordinarily-long word ÉÉÉÉÉÉÉ");
        let found = tokens(&shortcut);
        let words: Vec<&str> = found.iter().map(super::Token::as_str).collect();
        assert_eq!(
            words,
            [
                "an",
                "docs",
                "extraordinar",
                "json",
                "long",
                "rs",
                "serde",
                "sj",
                "word",
                "ÉÉÉÉÉÉ"
            ]
        );
        assert!(
            tokens(&shortcut)
                .iter()
                .all(|token| token.as_str().len() <= TOKEN_PREFIX)
        );
    }

    #[test]
    fn a_site_part_ends_at_its_first_terminator() {
        assert_eq!(site_part("example.com"), b"example.com");
        assert_eq!(site_part("a\0b"), b"a");
    }

    /// A shortcut in a folder, as the reference model adds it.
    fn filed(folder: Option<ItemId>) -> State {
        let mut state = State::default();
        let stamp = |wall| Stamp {
            wall,
            counter: 0,
            device: DeviceId([1; 16]),
        };
        if let Some(id) = folder {
            state
                .apply(
                    Edit::AddFolder {
                        id,
                        name: String::from("Work"),
                        parent: None,
                    },
                    stamp(1),
                )
                .unwrap();
        }
        state
            .apply(
                Edit::AddShortcut {
                    id: ID,
                    link: String::from("https://www.example.com/a"),
                    title: String::from("Title"),
                    folder,
                },
                stamp(5 * DAY + 3),
            )
            .unwrap();
        state
    }

    fn tables(rows: &[Row]) -> Vec<Table> {
        rows.iter().map(|row| row.table).collect()
    }

    /// The folder the filed shortcut sits in.
    const FOLDER: ItemId = ItemId([3; 16]);

    /// The rows of a shortcut filed in [`FOLDER`] while it shows.
    fn filed_rows() -> Vec<Row> {
        let state = filed(Some(FOLDER));
        rows(&Indexed {
            id: ID,
            shortcut: state.shortcut(ID).unwrap(),
            folder: Standing::Shown,
            site: "example.com",
        })
    }

    #[test]
    fn a_shown_shortcut_files_one_row_in_each_table_and_one_gone_for_good_none() {
        let state = filed(Some(FOLDER));
        let shortcut = state.shortcut(ID).unwrap();
        let words: Vec<String> = tokens(shortcut)
            .iter()
            .map(|token| String::from(token.as_str()))
            .collect();
        assert_eq!(words, ["a", "com", "example", "title", "www"]);
        let mut expected = alloc::vec![
            Table::Placement,
            Table::Added,
            Table::FolderAdded,
            Table::Title,
            Table::FolderTitle,
            Table::Link,
            Table::FolderLink,
            Table::LinkHash,
        ];
        expected.extend(words.iter().map(|_| Table::Token));
        expected.extend([
            Table::Site,
            Table::HomeCount,
            Table::SiteCount,
            Table::DayCount,
        ]);
        let filed_rows = filed_rows();
        assert_eq!(tables(&filed_rows), expected);
        let tokened: Vec<Vec<u8>> = filed_rows
            .iter()
            .filter(|row| row.table == Table::Token)
            .map(|row| row.key.clone())
            .collect();
        assert_eq!(
            tokened,
            words
                .iter()
                .map(|word| [word.as_bytes(), &[TERMINATOR], &ID.0].concat())
                .collect::<Vec<_>>()
        );
        let mut purged = shortcut.clone();
        purged.presence.value = Presence::Purged;
        let gone = Indexed {
            id: ID,
            shortcut: &purged,
            folder: Standing::Shown,
            site: "example.com",
        };
        assert_eq!(rows(&gone), []);
    }

    #[test]
    fn each_row_spells_its_key_from_the_shortcut() {
        let filed_rows = filed_rows();
        let home = home_key(Some(FOLDER)).to_vec();
        let find = |table: Table| {
            filed_rows
                .iter()
                .find(|row| row.table == table)
                .map(|row| row.key.clone())
                .unwrap()
        };
        assert_eq!(find(Table::HomeCount), home);
        assert_eq!(find(Table::SiteCount), b"example.com");
        assert_eq!(find(Table::DayCount), 5_u64.to_be_bytes());
        assert_eq!(
            find(Table::Added),
            added_key(5 * DAY + 3, ID).to_vec(),
            "an identifier without a time dates from its link"
        );
        assert_eq!(
            find(Table::FolderAdded),
            [home.clone(), added_key(5 * DAY + 3, ID).to_vec()].concat()
        );
        assert_eq!(find(Table::Title), title_key("Title", ID));
        assert_eq!(
            find(Table::FolderTitle),
            [home.clone(), title_key("Title", ID)].concat()
        );
        assert_eq!(find(Table::Link), link_key("https://www.example.com/a", ID));
        assert_eq!(
            find(Table::FolderLink),
            [home, link_key("https://www.example.com/a", ID)].concat()
        );
        assert_eq!(
            find(Table::LinkHash),
            [
                link_hash_home("https://www.example.com/a", Some(FOLDER)),
                ID.0.to_vec()
            ]
            .concat()
        );
        assert_eq!(
            find(Table::Site),
            [
                b"example.com".to_vec(),
                alloc::vec![TERMINATOR],
                added_key(!(5 * DAY + 3), ID).to_vec()
            ]
            .concat()
        );
    }

    #[test]
    fn a_shortcut_whose_folder_is_gone_files_at_the_top_and_under_its_folder_as_an_orphan() {
        let folder = FOLDER;
        let state = filed(Some(folder));
        let shortcut = state.shortcut(ID).unwrap();
        let orphan = rows(&Indexed {
            id: ID,
            shortcut,
            folder: Standing::Gone,
            site: "example.com",
        });
        let placement = orphan
            .iter()
            .find(|row| row.table == Table::Placement)
            .unwrap();
        assert_eq!(&placement.key[..HOME_KEY], &home_key(None));
        let orphaned: Vec<&Row> = orphan
            .iter()
            .filter(|row| row.table == Table::Orphan)
            .collect();
        assert_eq!(orphaned.len(), 1);
        assert_eq!(orphaned[0].key, [folder.0, ID.0].concat());
        let top = filed(None);
        let at_top = rows(&Indexed {
            id: ID,
            shortcut: top.shortcut(ID).unwrap(),
            folder: Standing::Gone,
            site: "example.com",
        });
        assert!(
            at_top.iter().all(|row| row.table != Table::Orphan),
            "a shortcut that declares the top orphans nothing"
        );
    }

    #[test]
    fn a_retitle_changes_only_the_rows_of_the_title() {
        let state = filed(None);
        let shortcut = state.shortcut(ID).unwrap();
        let indexed = Indexed {
            id: ID,
            shortcut,
            folder: Standing::Shown,
            site: "example.com",
        };
        let mut retitled = shortcut.clone();
        retitled.title.value = String::from("Other");
        let changed = delta(
            &rows(&indexed),
            &rows(&Indexed {
                shortcut: &retitled,
                ..indexed
            }),
        );
        assert_eq!(
            tables(&changed.removed),
            [Table::Title, Table::FolderTitle, Table::Token]
        );
        assert_eq!(
            tables(&changed.inserted),
            [Table::Title, Table::FolderTitle, Table::Token]
        );
        assert_eq!(delta::<u8>(&[], &[]), Delta::default());
    }

    #[test]
    fn the_counting_tables_are_the_three_counts() {
        let counted: Vec<Table> = [
            Table::Placement,
            Table::Added,
            Table::FolderAdded,
            Table::Title,
            Table::FolderTitle,
            Table::Link,
            Table::FolderLink,
            Table::LinkHash,
            Table::Token,
            Table::Site,
            Table::Orphan,
            Table::Hidden,
            Table::Trash,
            Table::HomeCount,
            Table::SiteCount,
            Table::DayCount,
        ]
        .into_iter()
        .filter(|table| table.counts())
        .collect();
        assert_eq!(
            counted,
            [Table::HomeCount, Table::SiteCount, Table::DayCount]
        );
    }

    #[test]
    fn the_delta_specification_agrees_over_every_short_ascending_list() {
        let mut lists: Vec<Vec<u8>> = alloc::vec![Vec::new()];
        for row in 0..4 {
            let longer: Vec<Vec<u8>> = lists
                .iter()
                .filter(|list| list.len() < ROWS)
                .map(|list| [list.as_slice(), &[row]].concat())
                .collect();
            lists.extend(longer);
        }
        assert_eq!(
            lists.len(),
            15,
            "every ascending list of up to three of four rows"
        );
        for before in &lists {
            for after in &lists {
                let changes = delta(before, after);
                assert!(delta_holds(before, after, &changes), "{before:?} {after:?}");
            }
        }
    }

    #[test]
    fn a_record_gone_for_good_goes_once_it_stayed_past_the_horizon() {
        assert!(!purgeable(10, 10 + TOMBSTONE_HORIZON - 1));
        assert!(purgeable(10, 10 + TOMBSTONE_HORIZON));
        assert!(!purgeable(10, 9), "a removal after the clock stays");
        assert_eq!(TOMBSTONE_HORIZON, 180 * DAY);
    }
}
