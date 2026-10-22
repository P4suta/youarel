//! The store against the reference model: random edits, merges, trash, put backs, removals for good, and visits run through both, and every listing, count, stamp, and record must agree, while every index table holds exactly the rows its records file.
//!
//! The merges bring another device's edits and records that no edit makes, such as a folder that comes back after its removal, a shortcut in a folder the store has never seen, and folders that declare each other, so every path of the index's upkeep runs.

use crate::codec::hex;
use crate::session::site;
use crate::{
    Group, Listing, PATH_SEPARATOR, Part, Parts, Record, Session, Span, Store, StoreError, Within,
};
use std::collections::BTreeMap;
use youarel_core::clock_rules::{self, DeviceId, Stamp};
use youarel_core::index_rules::{self, Standing};
use youarel_core::library_rules::{self, Facts, Grouping, Query, Scope, Sort};
use youarel_core::merge_rules::Register;
use youarel_core::position_rules::Key;
use youarel_core::rank_rules::Visits;
use youarel_core::record_rules::Body;
use youarel_core::state::{Edit, Entry, Folder, ItemId, Presence, Shortcut, Slot, State};
use youarel_core::ui_rules::{Row, Ui};
use youarel_core::view;

/// The time the runs start at.
const START: u64 = 1_700_000_000_000;
/// A day in milliseconds.
const DAY: u64 = 86_400_000;
/// The other device, whose edits arrive through merges.
const OTHER: DeviceId = DeviceId([0xbb; 16]);
/// A device that no edit names, whose stamps only records of no edit carry.
const STRANGER: DeviceId = DeviceId([0xcc; 16]);
/// A folder that no edit adds, which only a merged record brings.
const UNSEEN: ItemId = ItemId([0xee; 16]);

const LINKS: [&str; 5] = [
    "https://a.example/",
    "https://a.example/x",
    "http://b.example/page",
    "https://a-host-whose-name-passes-the-kept-start.example/one",
    "https://a-host-whose-name-passes-the-kept-start.example/two",
];
const TITLES: [&str; 8] = [
    "Alpha",
    "alpha",
    "ALPHA beta",
    "École",
    "zeta",
    "a title long enough to pass the kept start, one",
    "a title long enough to pass the kept start, two",
    "nul\u{0}inside",
];
const TEXTS: [&str; 4] = ["", "gh", "read later", "Zed"];
const NAMES: [&str; 3] = ["Work", "work", "Docs"];
const SEARCHES: [&str; 6] = ["", "a", "al", "ex", "work", "zz"];
const SORTS: [Sort; 5] = [
    Sort::Newest,
    Sort::Oldest,
    Sort::Title,
    Sort::Link,
    Sort::Opened,
];
const POSITIONS: [&str; 5] = ["1", "V", "VV", "a1", "z"];
/// Every presence a merged record may write.
const PRESENCES: [Presence; 3] = [Presence::Live, Presence::Trashed, Presence::Purged];

/// A generator of numbers that repeats from its seed.
struct Draw(u64);

impl Draw {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut mixed = self.0;
        mixed = (mixed ^ mixed.wrapping_shr(30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ mixed.wrapping_shr(27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        mixed ^ mixed.wrapping_shr(31)
    }

    fn below(&mut self, count: usize) -> usize {
        let drawn = self.next().checked_rem(u64::try_from(count).unwrap());
        usize::try_from(drawn.unwrap()).unwrap()
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

/// The shortcuts the runs name: four that carry the time they were made, two of them made at one moment, and four that carry none.
fn shortcut_ids() -> [ItemId; 8] {
    [
        ItemId::v7(START - 40 * DAY, [1; 10]),
        ItemId::v7(START - 2 * DAY, [2; 10]),
        ItemId::v7(START, [3; 10]),
        ItemId::v7(START, [4; 10]),
        ItemId([5; 16]),
        ItemId([6; 16]),
        ItemId([7; 16]),
        ItemId([8; 16]),
    ]
}

/// The folders the runs name, and the one that only a merge brings.
const FOLDERS: [ItemId; 5] = [
    ItemId([0x11; 16]),
    ItemId([0x12; 16]),
    ItemId([0x13; 16]),
    ItemId([0x14; 16]),
    UNSEEN,
];

/// What the reference model holds: the store's state, the other device's, this device's visits, the time, and what the runs met.
struct Model {
    state: State,
    other: State,
    visits: BTreeMap<ItemId, Visits>,
    now: u64,
    seen: Seen,
}

/// How often the runs met each case that the upkeep of the index treats apart, which shows the runs reach them.
#[derive(Clone, Copy, Debug, Default)]
struct Seen {
    applied: usize,
    refused: usize,
    emptied: usize,
    kept: usize,
    returned: usize,
    orphaned: usize,
    cycled: usize,
    tied_titles: usize,
    tied_times: usize,
    hidden: usize,
    shown_again: usize,
}

impl Seen {
    fn add(&mut self, other: Self) {
        let pairs = [
            (&mut self.applied, other.applied),
            (&mut self.refused, other.refused),
            (&mut self.emptied, other.emptied),
            (&mut self.kept, other.kept),
            (&mut self.returned, other.returned),
            (&mut self.orphaned, other.orphaned),
            (&mut self.cycled, other.cycled),
            (&mut self.tied_titles, other.tied_titles),
            (&mut self.tied_times, other.tied_times),
            (&mut self.hidden, other.hidden),
            (&mut self.shown_again, other.shown_again),
        ];
        for (held, more) in pairs {
            *held = held.saturating_add(more);
        }
    }

    fn all(self) -> bool {
        [
            self.applied,
            self.refused,
            self.emptied,
            self.kept,
            self.returned,
            self.orphaned,
            self.cycled,
            self.tied_titles,
            self.tied_times,
            self.hidden,
            self.shown_again,
        ]
        .iter()
        .all(|count| *count > 0)
    }
}

/// Whether a merge or an edit brought a folder back to life, or in for the first time, while a shortcut not gone for good waited for it.
fn returned(before: &State, after: &State) -> bool {
    FOLDERS.iter().any(|folder| {
        before.folder(*folder).is_none()
            && after.folder(*folder).is_some()
            && after.shortcuts.values().any(|shortcut| {
                shortcut.presence.value != Presence::Purged
                    && shortcut.folder.value == Some(*folder)
            })
    })
}

/// Whether a merge or an edit showed again a live shortcut that a folder in the trash hid.
fn shown_again(before: &State, after: &State) -> bool {
    let (was, now) = (before.tree(), after.tree());
    after.shortcuts.iter().any(|(id, shortcut)| {
        shortcut.presence.value == Presence::Live
            && before.standing(&was, *id) == Standing::Hidden
            && after.standing(&now, *id) == Standing::Shown
    })
}

/// How many shortcuts not gone for good a state holds.
fn kept(state: &State) -> usize {
    state
        .shortcuts
        .values()
        .filter(|shortcut| shortcut.presence.value != Presence::Purged)
        .count()
}

/// Merges one record through the store and the reference model, which must agree on whether it changed anything.
fn merge(session: &mut Session, model: &mut Model, record: Record) {
    let before = model.state.clone();
    model.state = model.state.merge(&alone(&record));
    let changed = model.state != before;
    assert_eq!(session.merge_record(record).unwrap(), changed);
    if !changed {
        model.seen.kept = model.seen.kept.saturating_add(1);
    }
    if returned(&before, &model.state) {
        model.seen.returned = model.seen.returned.saturating_add(1);
    }
    if shown_again(&before, &model.state) {
        model.seen.shown_again = model.seen.shown_again.saturating_add(1);
    }
}

fn maybe_folder(draw: &mut Draw) -> Option<ItemId> {
    if draw.chance(35) {
        None
    } else {
        Some(draw.pick(&FOLDERS))
    }
}

fn any_entry(draw: &mut Draw) -> Entry {
    if draw.chance(30) {
        Entry::Folder(draw.pick(&FOLDERS))
    } else {
        Entry::Shortcut(draw.pick(&shortcut_ids()))
    }
}

fn any_slot(draw: &mut Draw) -> Slot {
    match draw.below(3) {
        0 => Slot::End,
        1 => Slot::Before(any_entry(draw)),
        _ => Slot::After(any_entry(draw)),
    }
}

/// An identifier the state lacks when there is one, mostly, for an addition that lands, and any of them otherwise.
fn fresh(draw: &mut Draw, ids: &[ItemId], held: impl Fn(ItemId) -> bool) -> ItemId {
    let absent: Vec<ItemId> = ids.iter().copied().filter(|id| !held(*id)).collect();
    if absent.is_empty() || draw.chance(20) {
        draw.pick(ids)
    } else {
        draw.pick(&absent)
    }
}

/// An identifier the state holds in a presence `wanted` takes when there is one, mostly, for an edit that lands, and any of them otherwise.
fn standing(draw: &mut Draw, ids: &[ItemId], wanted: impl Fn(ItemId) -> bool) -> ItemId {
    let present: Vec<ItemId> = ids.iter().copied().filter(|id| wanted(*id)).collect();
    if present.is_empty() || draw.chance(20) {
        draw.pick(ids)
    } else {
        draw.pick(&present)
    }
}

/// An entry the state holds in the trash when there is one, mostly, for a put back that lands, and any entry otherwise.
fn in_trash(draw: &mut Draw, state: &State) -> Entry {
    let trashed = library_rules::trash(state);
    if trashed.is_empty() || draw.chance(20) {
        any_entry(draw)
    } else {
        draw.pick(&trashed)
    }
}

/// An edit of the trash: a live entry put in it, an entry put back, or an entry removed for good.
fn trash_edit(draw: &mut Draw, state: &State, id: ItemId, folder: ItemId) -> Edit {
    match draw.below(4) {
        0 => Edit::Trash {
            entry: Entry::Shortcut(id),
        },
        1 => Edit::Trash {
            entry: Entry::Folder(folder),
        },
        2 => Edit::PutBack {
            entry: in_trash(draw, state),
        },
        _ => Edit::Remove {
            entry: if draw.chance(50) {
                in_trash(draw, state)
            } else {
                any_entry(draw)
            },
        },
    }
}

/// An edit of any kind, of an entry that may exist, be in the trash, be gone for good, or be unknown, with an edit that lands drawn more often.
fn any_edit(draw: &mut Draw, state: &State) -> Edit {
    let shortcuts = shortcut_ids();
    let id = standing(draw, &shortcuts, |held| state.shortcut(held).is_some());
    let folder = standing(draw, &FOLDERS, |held| state.folder(held).is_some());
    match draw.below(16) {
        0..=2 => Edit::AddShortcut {
            id: fresh(draw, &shortcuts, |held| state.shortcuts.contains_key(&held)),
            link: draw.pick(&LINKS).to_owned(),
            title: draw.pick(&TITLES).to_owned(),
            folder: maybe_folder(draw),
        },
        3 | 4 => Edit::AddFolder {
            id: fresh(draw, &FOLDERS, |held| state.folders.contains_key(&held)),
            name: draw.pick(&NAMES).to_owned(),
            parent: maybe_folder(draw),
        },
        5 => Edit::Retitle {
            id,
            title: draw.pick(&TITLES).to_owned(),
        },
        6 => Edit::Realias {
            id,
            alias: draw.pick(&TEXTS).to_owned(),
        },
        7 => Edit::SetNote {
            id,
            note: draw.pick(&TEXTS).to_owned(),
        },
        8 => Edit::RenameFolder {
            id: folder,
            name: draw.pick(&NAMES).to_owned(),
        },
        9 => Edit::MarkFolder {
            id: folder,
            mark: draw.pick(&TEXTS).to_owned(),
        },
        10 => Edit::Move {
            entry: Entry::Shortcut(id),
            folder: maybe_folder(draw),
            slot: any_slot(draw),
        },
        11 => Edit::Move {
            entry: Entry::Folder(folder),
            folder: maybe_folder(draw),
            slot: any_slot(draw),
        },
        12 => Edit::Move {
            entry: any_entry(draw),
            folder: maybe_folder(draw),
            slot: any_slot(draw),
        },
        _ => trash_edit(draw, state, id, folder),
    }
}

/// A stamp from a pool that holds the stamp no device wrote, stamps before and after the latest, and stamps of three devices.
fn any_stamp(draw: &mut Draw, model: &Model) -> Stamp {
    let latest = model.state.latest().map_or(START, |stamp| stamp.wall);
    let wall = draw.pick(&[
        0,
        START - DAY,
        latest,
        latest.saturating_add(1),
        START + DAY,
    ]);
    if wall == 0 {
        return Stamp::origin(DeviceId([0; 16]));
    }
    Stamp {
        wall,
        counter: u32::try_from(draw.below(2)).unwrap(),
        device: draw.pick(&[OTHER, STRANGER, DeviceId([0xdd; 16])]),
    }
}

fn register<T>(value: T, stamp: Stamp) -> Register<T> {
    Register { value, stamp }
}

fn position(draw: &mut Draw) -> Key {
    Key::parse(draw.pick(&POSITIONS)).unwrap()
}

/// A folder's record that no edit makes: the store's own with one register written at a drawn stamp, or a new one.
fn odd_folder(draw: &mut Draw, model: &Model) -> Record {
    let id = draw.pick(&FOLDERS);
    let stamp = any_stamp(draw, model);
    let mut folder = model
        .state
        .folders
        .get(&id)
        .cloned()
        .unwrap_or_else(|| Folder {
            name: register(draw.pick(&NAMES).to_owned(), stamp),
            mark: register(String::new(), stamp),
            parent: register(None, stamp),
            position: register(position(draw), stamp),
            presence: register(Presence::Live, stamp),
        });
    match draw.below(4) {
        0 => folder.presence = register(draw.pick(&PRESENCES), stamp),
        1 => folder.parent = register(maybe_folder(draw), stamp),
        2 => folder.position = register(position(draw), stamp),
        _ => folder.name = register(draw.pick(&NAMES).to_owned(), stamp),
    }
    Record::Folder(id, folder)
}

/// A shortcut's record that no edit makes: the store's own with one register written at a drawn stamp, or a new one.
fn odd_shortcut(draw: &mut Draw, model: &Model) -> Record {
    let id = draw.pick(&shortcut_ids());
    let stamp = any_stamp(draw, model);
    let mut shortcut = model
        .state
        .shortcuts
        .get(&id)
        .cloned()
        .unwrap_or_else(|| Shortcut {
            link: register(draw.pick(&LINKS).to_owned(), stamp),
            title: register(draw.pick(&TITLES).to_owned(), stamp),
            alias: register(String::new(), stamp),
            folder: register(maybe_folder(draw), stamp),
            position: register(position(draw), stamp),
            presence: register(Presence::Live, stamp),
            note: register(String::new(), stamp),
        });
    match draw.below(5) {
        0 => shortcut.presence = register(draw.pick(&PRESENCES), stamp),
        1 => shortcut.folder = register(maybe_folder(draw), stamp),
        2 => shortcut.position = register(position(draw), stamp),
        3 => shortcut.title = register(draw.pick(&TITLES).to_owned(), stamp),
        _ => shortcut.link = register(draw.pick(&LINKS).to_owned(), stamp),
    }
    Record::Shortcut(id, shortcut)
}

/// The state that holds one record alone, which the reference model merges.
fn alone(record: &Record) -> State {
    let mut state = State::default();
    match record {
        Record::Device(id, name) => {
            state.devices.insert(*id, name.clone());
        }
        Record::Folder(id, folder) => {
            state.folders.insert(*id, folder.clone());
        }
        Record::Shortcut(id, shortcut) => {
            state.shortcuts.insert(*id, shortcut.clone());
        }
    }
    state
}

/// An edit of this device through the store and through the reference model, which must agree on its refusal or on its stamp.
fn local_edit(session: &mut Session, model: &mut Model, edit: Edit) {
    let removes_folder = matches!(
        edit,
        Edit::Remove {
            entry: Entry::Folder(_)
        }
    );
    match session.edit(edit.clone()) {
        Ok(()) => {
            let stamp = session.latest().unwrap();
            let before = model.state.clone();
            model.state.apply(edit, stamp).unwrap();
            model.seen.applied = model.seen.applied.saturating_add(1);
            if removes_folder && kept(&model.state) < kept(&before) {
                model.seen.emptied = model.seen.emptied.saturating_add(1);
            }
            if shown_again(&before, &model.state) {
                model.seen.shown_again = model.seen.shown_again.saturating_add(1);
            }
        }
        Err(StoreError::Edit(refusal)) => {
            let mut copy = model.state.clone();
            let stamp = Stamp::origin(session.device());
            assert_eq!(copy.apply(edit, stamp), Err(refusal));
            model.seen.refused = model.seen.refused.saturating_add(1);
        }
        Err(other) => panic!("{other}"),
    }
}

/// An edit of the other device, whose records then arrive one by one in a drawn order.
fn remote_edit(session: &mut Session, model: &mut Model, draw: &mut Draw) {
    if draw.chance(50) {
        model.other = model.other.merge(&model.state);
    }
    let edit = any_edit(draw, &model.other);
    let origin = Stamp::origin(OTHER);
    let seen = model
        .other
        .latest()
        .map_or(origin, |latest| clock_rules::observe(origin, latest));
    let wall = model.now.saturating_sub(draw.pick(&[0, 1_000, DAY]));
    let stamp = clock_rules::tick(seen, wall).unwrap();
    if model.other.apply(edit, stamp).is_err() {
        return;
    }
    let mut records: Vec<Record> = model
        .other
        .folders
        .iter()
        .map(|(id, folder)| Record::Folder(*id, folder.clone()))
        .chain(
            model
                .other
                .shortcuts
                .iter()
                .map(|(id, shortcut)| Record::Shortcut(*id, shortcut.clone())),
        )
        .collect();
    for at in (1..records.len()).rev() {
        let other = draw.below(at.saturating_add(1));
        records.swap(at, other);
    }
    for record in records {
        merge(session, model, record);
    }
}

/// One drawn step through the store and the reference model.
fn step(session: &mut Session, model: &mut Model, draw: &mut Draw) {
    match draw.below(20) {
        0..=8 => {
            let edit = any_edit(draw, &model.state);
            local_edit(session, model, edit);
        }
        9 => local_edit(
            session,
            model,
            Edit::Remove {
                entry: Entry::Folder(draw.pick(&FOLDERS)),
            },
        ),
        10 => {
            let folder = draw.pick(&FOLDERS);
            let entry = Entry::Folder(folder);
            let edit = if model.state.folder(folder).is_some() {
                Edit::Trash { entry }
            } else {
                Edit::PutBack { entry }
            };
            local_edit(session, model, edit);
        }
        11..=13 => remote_edit(session, model, draw),
        14..=16 => {
            let record = if draw.chance(50) {
                odd_folder(draw, model)
            } else {
                odd_shortcut(draw, model)
            };
            merge(session, model, record);
        }
        _ => {
            let id = draw.pick(&shortcut_ids());
            let visits = model.visits.entry(id).or_default();
            visits.count = visits.count.saturating_add(1);
            visits.last = model.now;
            session.visit(id).unwrap();
        }
    }
}

/// The scopes the comparison lists: the fixed ones, and a collection of every folder.
fn scopes() -> Vec<Scope> {
    [Scope::All, Scope::Unsorted, Scope::Recent]
        .into_iter()
        .chain(FOLDERS.map(Scope::Collection))
        .collect()
}

/// The spans the comparison lists: every scope, and the top and every folder as a home.
fn spans() -> Vec<Span> {
    scopes()
        .into_iter()
        .map(Span::Scope)
        .chain(
            std::iter::once(None)
                .chain(FOLDERS.map(Some))
                .map(Span::Home),
        )
        .collect()
}

/// Every grouping a listing takes.
const GROUPINGS: [Grouping; 4] = [
    Grouping::None,
    Grouping::Site,
    Grouping::Collection,
    Grouping::Added,
];

/// A listing gathered under its headings in turn, each with its shortcuts in order.
type Gathered = Vec<(Option<Group>, Vec<ItemId>)>;

/// The listing the reference model draws: the scope's listing, of those sitting in the home for a span of a home, gathered under the grouping's headings in their order.
fn reference(model: &Model, facts: Facts<'_>, listing: &Listing) -> Gathered {
    let tree = facts.tree;
    let scope = match listing.span {
        Span::Scope(scope) => scope,
        Span::Home(home) => home.map_or(Scope::All, Scope::Collection),
    };
    let mut listed = library_rules::listing(
        facts,
        Query {
            scope: &scope,
            search: &listing.search,
            sort: listing.sort,
        },
    );
    if let Span::Home(home) = listing.span {
        listed.retain(|id| model.state.folder_of(tree, *id) == home);
    }
    let group = |id: ItemId| {
        let shortcut = model.state.shown(tree, id)?;
        match listing.grouping {
            Grouping::None => None,
            Grouping::Added => Some(Group::Period(library_rules::period(
                library_rules::added(id, shortcut),
                model.now,
            ))),
            Grouping::Site => Some(Group::Site(site(&shortcut.link.value))),
            Grouping::Collection => {
                Some(Group::Folder(model.state.folder_of(tree, id).map(
                    |folder| model.state.path(tree, Some(folder)).join(PATH_SEPARATOR),
                )))
            }
        }
    };
    if listing.grouping == Grouping::None {
        return vec![(None, listed)];
    }
    library_rules::grouped(&listed, group)
}

/// The listing the store answers, read part by part, with each indexed part's count, a window of it, and the place of one of its rows checked on the way.
fn answered(session: &Session, listing: &Listing, draw: &mut Draw) -> Gathered {
    let grouped = session.parts(listing).unwrap();
    assert!(!grouped.ungrouped, "{listing:?}");
    match grouped.parts {
        Parts::Gathered(parts) => parts,
        Parts::Indexed(parts) => parts
            .into_iter()
            .map(|(within, count)| {
                let part = Part {
                    span: listing.span,
                    sort: listing.sort,
                    within: within.clone(),
                };
                let ids = session.page(&part, 0, usize::MAX).unwrap();
                assert_eq!(ids.len(), count, "{listing:?} {within:?}");
                let (first, limit) = (draw.below(4), draw.below(4));
                let start = first.min(ids.len());
                let end = start.saturating_add(limit).min(ids.len());
                assert_eq!(
                    session.page(&part, first, limit).unwrap(),
                    ids[start..end],
                    "{listing:?} {within:?} {first} {limit}"
                );
                if let Some(id) = ids.get(start) {
                    assert_eq!(session.place_of(&part, *id).unwrap(), Some(start));
                }
                let heading = match within {
                    Within::Whole => None,
                    Within::Period(age) => Some(Group::Period(age)),
                    Within::Folder(path) => Some(Group::Folder(path)),
                };
                (heading, ids)
            })
            .collect(),
    }
}

/// Requires every listing the store answers, in every part, to be the reference model's.
fn compare_listings(session: &Session, model: &Model, draw: &mut Draw) {
    let tree = model.state.tree();
    let facts = Facts {
        state: &model.state,
        tree: &tree,
        visits: &model.visits,
        now: model.now,
    };
    for span in spans() {
        for grouping in GROUPINGS {
            for search in SEARCHES {
                // A search ranks its matches whatever the order, so one order stands for all.
                let sorts: &[Sort] = if search.is_empty() {
                    &SORTS
                } else {
                    &SORTS[..1]
                };
                for sort in sorts {
                    let listing = Listing {
                        span,
                        sort: *sort,
                        search: search.to_owned(),
                        grouping,
                    };
                    assert_eq!(
                        answered(session, &listing, draw),
                        reference(model, facts, &listing),
                        "{listing:?}"
                    );
                }
            }
        }
    }
}

/// Requires the folders, the entries, the holders of links, and the counts that the store answers to be the reference model's.
fn compare_folders(session: &Session, model: &Model) {
    let tree = model.state.tree();
    assert_eq!(session.folders().records(), &model.state.folders);
    assert_eq!(session.folders().tree(), &tree);
    let homes: Vec<Option<ItemId>> = std::iter::once(None).chain(FOLDERS.map(Some)).collect();
    for home in &homes {
        let entries = model.state.entries(&tree, *home);
        assert_eq!(session.entries(*home).unwrap(), entries, "{home:?}");
        let mut named: Vec<(String, String, ItemId)> = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Folder(id) => model
                    .state
                    .folders
                    .get(id)
                    .map(|folder| library_rules::by_name(&folder.name.value, *id)),
                Entry::Shortcut(_) => None,
            })
            .collect();
        named.sort();
        let by_name: Vec<ItemId> = named.into_iter().map(|(_, _, id)| id).collect();
        assert_eq!(session.folders().named(*home), by_name, "{home:?}");
        for link in LINKS {
            assert_eq!(
                session.holder(*home, link).unwrap(),
                model.state.holding(&tree, *home, link),
                "{home:?} {link}"
            );
        }
    }
    assert_eq!(
        session.counts().unwrap(),
        library_rules::counts(&model.state, &tree, model.now)
    );
}

/// Requires the trash that the store lists, its folders by name and its shortcuts page by page and place by place, to be the reference model's, and each shortcut in it to read as the model holds it.
fn compare_trash(session: &Session, model: &Model, draw: &mut Draw) {
    let wanted = library_rules::trash(&model.state);
    let listed: Vec<Entry> = session
        .folders()
        .trashed()
        .into_iter()
        .map(Entry::Folder)
        .chain(
            session
                .trash_page(0, usize::MAX)
                .unwrap()
                .into_iter()
                .map(Entry::Shortcut),
        )
        .collect();
    assert_eq!(listed, wanted);
    assert_eq!(session.trash_count().unwrap(), wanted.len());
    let shortcuts: Vec<ItemId> = wanted
        .iter()
        .filter_map(|entry| match entry {
            Entry::Shortcut(id) => Some(*id),
            Entry::Folder(_) => None,
        })
        .collect();
    assert_eq!(session.trashed_count().unwrap(), shortcuts.len());
    let tree = model.state.tree();
    for folder in FOLDERS {
        let hidden = model
            .state
            .shortcuts
            .keys()
            .filter(|id| {
                model.state.standing(&tree, **id) == Standing::Hidden
                    && model
                        .state
                        .folder_of(&tree, **id)
                        .is_some_and(|home| tree.within(home, folder))
            })
            .count();
        assert_eq!(session.hidden_count(folder).unwrap(), hidden, "{folder:?}");
    }
    let (first, limit) = (draw.below(4), draw.below(4));
    let start = first.min(shortcuts.len());
    let end = start.saturating_add(limit).min(shortcuts.len());
    assert_eq!(
        session.trash_page(first, limit).unwrap(),
        shortcuts[start..end],
        "{first} {limit}"
    );
    for id in shortcut_ids() {
        assert_eq!(
            session.trash_place(id).unwrap(),
            shortcuts.iter().position(|held| *held == id)
        );
    }
    for id in shortcut_ids() {
        let trashed = model
            .state
            .shortcuts
            .get(&id)
            .filter(|shortcut| shortcut.presence.value == Presence::Trashed);
        assert_eq!(session.trashed(id).unwrap().as_ref(), trashed);
    }
}

/// The shortcuts of the reference model that show and that `keep` takes, in the order of their identifiers.
fn live_where(model: &Model, keep: &dyn Fn(ItemId, &Shortcut) -> bool) -> Vec<(ItemId, Shortcut)> {
    let tree = model.state.tree();
    model
        .state
        .shortcuts
        .iter()
        .filter(|(id, shortcut)| model.state.shown(&tree, **id).is_some() && keep(**id, shortcut))
        .map(|(id, shortcut)| (*id, shortcut.clone()))
        .collect()
}

/// Requires the shortcuts that each name names to be the reference model's: by alias, by title, by link, and by the start of an identifier's spelling.
fn compare_names(session: &Session, model: &Model) {
    for name in TITLES.iter().chain(&TEXTS).chain(&LINKS).chain(&NAMES) {
        assert_eq!(
            session.aliased(name).unwrap(),
            live_where(model, &|_, shortcut| !name.is_empty()
                && shortcut.alias.value == *name),
            "{name:?}"
        );
        assert_eq!(
            session.titled(name).unwrap(),
            live_where(model, &|_, shortcut| shortcut.title.value == *name),
            "{name:?}"
        );
        assert_eq!(
            session.holders(name).unwrap(),
            live_where(model, &|_, shortcut| shortcut.link.value == *name),
            "{name:?}"
        );
    }
    for id in shortcut_ids() {
        let spelled = hex(&id.0);
        for length in [0, 1, 6, 7, 31, 32] {
            let start = spelled.get(..length).unwrap();
            assert_eq!(
                session.spelled(start).unwrap(),
                live_where(model, &|other, _| hex(&other.0).starts_with(start)),
                "{start}"
            );
        }
    }
}

/// Requires every other answer of the store to be the reference model's, and every index table to hold exactly the rows its records file.
fn compare_answers(session: &Session, model: &Model) {
    compare_folders(session, model);
    compare_names(session, model);
    let tree = model.state.tree();
    for link in LINKS {
        let held = model.state.shortcuts.iter().any(|(id, shortcut)| {
            model.state.shown(&tree, *id).is_some() && shortcut.link.value == link
        });
        assert_eq!(session.holds_link(link).unwrap(), held, "{link}");
    }
    for id in shortcut_ids() {
        assert_eq!(
            session.item(id).unwrap(),
            model.state.shown(&tree, id).cloned()
        );
        assert_eq!(
            session.visits_of(id).unwrap(),
            model.visits.get(&id).copied()
        );
    }
    assert_eq!(
        session.shortcuts().unwrap(),
        live_where(model, &|_, _| true)
    );
    assert_eq!(session.latest(), model.state.latest());
    for device in [session.device(), OTHER, STRANGER] {
        assert_eq!(
            session.latest_of(device).unwrap(),
            model.state.latest_of(device)
        );
    }
    assert_eq!(session.records().unwrap(), Body::of(&model.state));
    for search in SEARCHES {
        let ui = Ui {
            query: search.to_owned(),
            ..Ui::default()
        };
        let wanted: Vec<Entry> = view::rows(&model.state, &ui, &model.visits, model.now)
            .into_iter()
            .filter_map(|row| match row {
                Row::Entry(entry) => Some(entry),
                Row::Parent(_) | Row::Add(_) => None,
            })
            .collect();
        assert_eq!(session.find(search).unwrap(), wanted, "{search:?}");
    }
    session.audit();
}

/// Counts the cases that the state now holds: a shortcut in a gone folder's stead, a folder whose parent a cycle breaks, a live shortcut a folder in the trash hides, two titles of one kept start, and two shortcuts added at one moment.
fn meet(model: &mut Model) {
    let state = &model.state;
    let tree = state.tree();
    let live: Vec<(&ItemId, &Shortcut)> = state
        .shortcuts
        .iter()
        .filter(|(_, shortcut)| shortcut.presence.value != Presence::Purged)
        .collect();
    let orphaned = live.iter().any(|(_, shortcut)| {
        shortcut
            .folder
            .value
            .is_some_and(|folder| !tree.holds(folder))
    });
    let cycled = state.folders.iter().any(|(id, folder)| {
        tree.holds(*id)
            && folder
                .parent
                .value
                .is_some_and(|parent| tree.holds(parent) && tree.parent(*id) != Some(parent))
    });
    let hidden = live.iter().any(|(id, shortcut)| {
        shortcut.presence.value == Presence::Live && state.standing(&tree, **id) == Standing::Hidden
    });
    let kept = |shortcut: &Shortcut| {
        index_rules::sort_part(&index_rules::title_key(
            &shortcut.title.value,
            ItemId::default(),
        ))
        .to_vec()
    };
    let mut pairs = live
        .iter()
        .flat_map(|left| live.iter().map(move |right| (left, right)));
    let tied_titles = pairs.clone().any(|((left, one), (right, other))| {
        left < right && kept(one) == kept(other) && one.title.value != other.title.value
    });
    let tied_times = pairs.any(|((left, one), (right, other))| {
        left < right && library_rules::added(**left, one) == library_rules::added(**right, other)
    });
    for (met, count) in [
        (orphaned, &mut model.seen.orphaned),
        (cycled, &mut model.seen.cycled),
        (hidden, &mut model.seen.hidden),
        (tied_titles, &mut model.seen.tied_titles),
        (tied_times, &mut model.seen.tied_times),
    ] {
        if met {
            *count = count.saturating_add(1);
        }
    }
}

/// Runs the steps of one seed through a new store, compares after each session, and answers what the run met.
fn run(seed: u64, sessions: usize, steps: usize) -> Seen {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_path_buf());
    let mut draw = Draw(seed);
    let mut model = Model {
        state: State::default(),
        other: State::default(),
        visits: BTreeMap::new(),
        now: START,
        seen: Seen::default(),
    };
    for _ in 0..sessions {
        let advance = u64::try_from(draw.below(3)).unwrap().saturating_mul(1_000);
        model.now = if draw.chance(10) {
            model.now.saturating_sub(5_000)
        } else {
            model.now.saturating_add(advance)
        };
        store
            .session(model.now, |session| {
                for _ in 0..steps {
                    step(session, &mut model, &mut draw);
                    meet(&mut model);
                }
                compare_listings(session, &model, &mut draw);
                compare_answers(session, &model);
                compare_trash(session, &model, &mut draw);
                Ok(())
            })
            .unwrap();
    }
    model.seen
}

#[test]
fn the_store_agrees_with_the_reference_model_over_random_edits_merges_trash_and_removals() {
    let mut seen = Seen::default();
    for seed in 0..8 {
        seen.add(run(seed, 12, 9));
    }
    assert!(seen.all(), "every case the upkeep treats apart: {seen:?}");
}

#[test]
fn a_long_run_of_one_session_agrees_with_the_reference_model() {
    let seen = run(42, 3, 60);
    assert!(seen.applied > 0 && seen.refused > 0, "{seen:?}");
}
