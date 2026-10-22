//! The listing's own cases, which the differential test meets rarely or never: long runs of tied kept starts, a grouping that gives way, the search through the index of words, a visit at time zero, the edges of the periods, folders of one path, the count of sessions, and the windows of a folder's entries.

use super::{Filter, Kept, Members, Search, Span, TextOrder, Times, Within, candidates, window};
use crate::{Group, Grouped, Listing, Part, Parts, Session, Store};
use youarel_core::index_rules;
use youarel_core::library_rules::{Grouping, Period, RECENT, Scope, Sort, WEEK};
use youarel_core::search_rules::{self, Strategy};
use youarel_core::state::{Edit, Entry, ItemId, Slot};

/// The time the tests run at.
const NOW: u64 = 1_700_000_000_000;
/// A title whose first thirty-two bytes every title of the tied runs shares.
const LONG: &str = "a title long enough to pass the kept start of a key";

/// A store for one test.
fn store() -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_path_buf());
    (directory, store)
}

/// An identifier made at a time, told apart by a byte.
fn made(time: u64, byte: u8) -> ItemId {
    ItemId::v7(time, [byte; 10])
}

/// Adds a shortcut of a link and a title.
fn add(session: &mut Session, id: ItemId, (link, title): (&str, &str), folder: Option<ItemId>) {
    session
        .edit(Edit::AddShortcut {
            id,
            link: link.to_owned(),
            title: title.to_owned(),
            folder,
        })
        .unwrap();
}

/// Adds a folder.
fn folder(session: &mut Session, id: ItemId, name: &str, parent: Option<ItemId>) {
    session
        .edit(Edit::AddFolder {
            id,
            name: name.to_owned(),
            parent,
        })
        .unwrap();
}

/// Every shortcut of a stream.
fn all(stream: super::Stream<'_>) -> Vec<ItemId> {
    window(stream, 0, usize::MAX).unwrap()
}

fn assert_first_of(session: &Session, part: &Part, listed: &[ItemId]) {
    let candidates = std::collections::BTreeSet::from([made(NOW - 4, 4), made(NOW - 6, 6)]);
    assert_eq!(
        session.first_of(part, &candidates).unwrap(),
        listed.iter().position(|id| candidates.contains(id))
    );
}

fn assert_trashed_prefix(session: &mut Session, child: ItemId) {
    let trashed = made(NOW - 5, 5);
    session
        .edit(Edit::Trash {
            entry: Entry::Shortcut(trashed),
        })
        .unwrap();
    assert_eq!(session.trash_starting("al").unwrap(), Some(trashed));
    assert_eq!(
        session
            .trash_first_of(&std::collections::BTreeSet::from([trashed]))
            .unwrap(),
        Some(0)
    );
    let part = Part {
        span: Span::Home(Some(child)),
        sort: Sort::Title,
        within: Within::Whole,
    };
    assert_eq!(session.starting(&part, "al").unwrap(), None);
}

#[test]
fn a_title_prefix_finds_the_first_match_of_each_scope_and_order() {
    let (_directory, store) = store();
    store
        .session(NOW, |session| {
            let parent = made(NOW, 1);
            let child = made(NOW, 2);
            folder(session, parent, "Parent", None);
            folder(session, child, "Child", Some(parent));
            for (number, title, home) in [
                (3, "Alpha", None),
                (4, "Alpine", Some(parent)),
                (5, "ALPHA", Some(child)),
                (6, "Beta", Some(child)),
                (7, "Äther", Some(child)),
                (8, LONG, None),
            ] {
                let id = made(NOW - u64::from(number), number);
                add(
                    session,
                    id,
                    (&format!("https://{number}.example/"), title),
                    home,
                );
                session.visit(id)?;
            }
            for span in [
                Span::Scope(Scope::All),
                Span::Scope(Scope::Recent),
                Span::Scope(Scope::Collection(parent)),
                Span::Home(Some(child)),
                Span::Home(None),
            ] {
                for sort in [
                    Sort::Title,
                    Sort::Link,
                    Sort::Newest,
                    Sort::Oldest,
                    Sort::Opened,
                ] {
                    let part = Part {
                        span,
                        sort,
                        within: Within::Whole,
                    };
                    let listed = session.page(&part, 0, usize::MAX)?;
                    assert_first_of(session, &part, &listed);
                    for prefix in ["a", "al", "alpha", "ä", LONG, "missing"] {
                        let expected = listed.iter().copied().find(|id| {
                            session.item(*id).unwrap().is_some_and(|shortcut| {
                                shortcut.title.value.to_lowercase().starts_with(prefix)
                            })
                        });
                        assert_eq!(
                            session.starting(&part, prefix)?,
                            expected,
                            "{span:?} {sort:?} {prefix}"
                        );
                    }
                }
            }
            assert_trashed_prefix(session, child);
            Ok(())
        })
        .unwrap();
}

#[test]
fn a_run_of_tied_kept_starts_sorts_by_its_records_as_far_as_its_bound() {
    let (_directory, store) = store();
    let early = ItemId([1; 16]);
    let late = ItemId([9; 16]);
    let middle = ItemId([5; 16]);
    store
        .session(NOW, |session| {
            add(
                session,
                late,
                ("https://a.example/", &format!("{LONG} one")),
                None,
            );
            add(
                session,
                early,
                ("https://b.example/", &format!("{LONG} two")),
                None,
            );
            add(session, middle, ("https://c.example/", "zeta"), None);
            let tables = session.tables()?;
            let filter = Filter {
                members: Members::All,
                times: None,
            };
            let order = |bound| TextOrder {
                sort: Sort::Title,
                bound,
                kept: Kept::Shown,
            };
            assert_eq!(
                all(session.texts(&tables, &filter, order(2))?),
                [late, early, middle],
                "a run within the bound sorts by its whole titles"
            );
            assert_eq!(
                all(session.texts(&tables, &filter, order(1))?),
                [early, late, middle],
                "a run past the bound keeps the order of its keys"
            );
            drop(tables);
            for id in [late, early] {
                session.edit(Edit::Trash {
                    entry: Entry::Shortcut(id),
                })?;
            }
            assert_eq!(
                session.trash_page(0, usize::MAX)?,
                [late, early],
                "the trash sorts a run of tied kept starts by its whole titles"
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn a_grouping_by_site_gives_way_past_its_bound() {
    let (_directory, store) = store();
    store
        .session(NOW, |session| {
            add(session, made(NOW, 1), ("https://a.example/", "a"), None);
            add(session, made(NOW, 2), ("https://b.example/", "b"), None);
            let listing = Listing {
                grouping: Grouping::Site,
                ..Listing::of(Span::Scope(Scope::All), Sort::Oldest)
            };
            assert_eq!(
                session.parts_within(&listing, 1)?,
                Grouped {
                    parts: Parts::Indexed(vec![(Within::Whole, 2)]),
                    ungrouped: true,
                }
            );
            let gathered = session.parts_within(&listing, 2)?;
            assert!(!gathered.ungrouped);
            assert_eq!(
                gathered.parts,
                Parts::Gathered(vec![
                    (
                        Some(Group::Site("a.example".to_owned())),
                        vec![made(NOW, 1)]
                    ),
                    (
                        Some(Group::Site("b.example".to_owned())),
                        vec![made(NOW, 2)]
                    ),
                ])
            );
            Ok(())
        })
        .unwrap();
}

/// The shortcuts of the search tests: two words that start with `rus`, one that holds it later, one whose alias it is, and the folder that holds one of them.
struct Searched {
    rust: ItemId,
    rusty: ItemId,
    trust: ItemId,
    aliased: ItemId,
    work: ItemId,
}

/// The bytes that tell the identifiers of the search tests apart.
const SEARCHED: [u8; 5] = [1, 2, 3, 4, 5];
/// The links and the titles of the search tests' shortcuts.
const RUST: (&str, &str) = ("https://doc.rust-lang.org/", "Rust docs");
const RUSTY: (&str, &str) = ("https://example.com/nail", "Rusty nail");
const TRUST: (&str, &str) = ("https://trust.example/", "trust");
const ALIASED: (&str, &str) = ("https://d.example/", "Something");

/// A store of the search tests' shortcuts.
fn searched(store: &Store) -> Searched {
    let [rust, rusty, trust, aliased, work] = SEARCHED.map(|byte| made(NOW, byte));
    let found = Searched {
        rust,
        rusty,
        trust,
        aliased,
        work,
    };
    store
        .session(NOW, |session| {
            folder(session, found.work, "Work", None);
            add(session, found.rust, RUST, None);
            add(session, found.rusty, RUSTY, Some(found.work));
            add(session, found.trust, TRUST, None);
            add(session, found.aliased, ALIASED, None);
            session.edit(Edit::Realias {
                id: found.aliased,
                alias: "rus".to_owned(),
            })
        })
        .unwrap();
    found
}

/// A search for `rus` through the index of words.
fn indexed() -> Search<'static> {
    Search {
        query: "rus",
        strategy: Strategy::Index(search_rules::longest("rus").unwrap()),
    }
}

/// Every shortcut a filter keeps.
const EVERY: Filter = Filter {
    members: Members::All,
    times: None,
};

#[test]
fn a_search_through_the_index_reads_the_words_that_start_with_the_longest_word_typed() {
    let (_directory, store) = store();
    let ids = searched(&store);
    store
        .session(NOW, |session| {
            let tables = session.tables()?;
            let found: Vec<ItemId> = session
                .matches(&tables, &EVERY, indexed())?
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            assert_eq!(found.first(), Some(&ids.aliased), "the exact alias first");
            assert!(found.contains(&ids.rust) && found.contains(&ids.rusty));
            assert!(
                !found.contains(&ids.trust),
                "a word must start with what was typed"
            );
            let scan = Search {
                query: "rus",
                strategy: Strategy::Scan,
            };
            assert!(
                session
                    .matches(&tables, &EVERY, scan)?
                    .iter()
                    .any(|(id, _)| *id == ids.trust),
                "a scan matches characters in order"
            );
            let nothing = Search {
                query: "rus",
                strategy: Strategy::Nothing,
            };
            assert_eq!(session.matches(&tables, &EVERY, nothing)?, []);
            Ok(())
        })
        .unwrap();
}

#[test]
fn an_indexed_search_keeps_to_its_scope_its_limit_and_the_folders() {
    let (_directory, store) = store();
    let ids = searched(&store);
    store
        .session(NOW, |session| {
            let tables = session.tables()?;
            let inside = Filter {
                members: Members::Homes(vec![Some(ids.work)]),
                times: None,
            };
            let filed: Vec<ItemId> = session
                .matches(&tables, &inside, indexed())?
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            assert_eq!(filed, [ids.rusty], "the candidates keep to the scope");
            let first = candidates(&tables, b"rus", 1)?;
            assert_eq!(first, [ids.aliased], "the equal word first");
            assert_eq!(candidates(&tables, b"rus", 0)?, []);
            let words = candidates(&tables, b"rus", usize::MAX)?;
            assert_eq!(words.len(), 3, "each shortcut once");
            let entries = session.found(&tables, &EVERY, indexed())?;
            assert_eq!(entries.first(), Some(&Entry::Shortcut(ids.aliased)));
            let folders = Search {
                query: "work",
                strategy: Strategy::Index(search_rules::longest("work").unwrap()),
            };
            let named = session.found(&tables, &EVERY, folders)?;
            assert_eq!(named.first(), Some(&Entry::Folder(ids.work)));
            Ok(())
        })
        .unwrap();
}

#[test]
fn a_visit_at_time_zero_ranks_as_none() {
    let (_directory, store) = store();
    let older = made(NOW - 1, 1);
    let newer = made(NOW, 2);
    store
        .session(NOW, |session| {
            add(session, older, ("https://a.example/", "a"), None);
            add(session, newer, ("https://b.example/", "b"), None);
            Ok(())
        })
        .unwrap();
    store.session(0, |session| session.visit(older)).unwrap();
    let opened = store
        .session(NOW, |session| {
            session.page(
                &Part {
                    span: Span::Scope(Scope::All),
                    sort: Sort::Opened,
                    within: Within::Whole,
                },
                0,
                usize::MAX,
            )
        })
        .unwrap();
    assert_eq!(opened, [newer, older]);
    store.session(NOW, |session| session.visit(older)).unwrap();
    let again = store
        .session(NOW, |session| {
            session.page(
                &Part {
                    span: Span::Scope(Scope::All),
                    sort: Sort::Opened,
                    within: Within::Whole,
                },
                0,
                usize::MAX,
            )
        })
        .unwrap();
    assert_eq!(again, [older, newer], "a visit since ranks first");
}

#[test]
fn the_periods_and_the_recent_scope_hold_their_edges() {
    let (_directory, store) = store();
    let ahead = made(NOW + 1, 1);
    let week = made(NOW - WEEK, 2);
    let past_week = made(NOW - WEEK - 1, 3);
    let month = made(NOW - RECENT, 4);
    let past_month = made(NOW - RECENT - 1, 5);
    let parts = store
        .session(NOW, |session| {
            for (id, link) in [
                (ahead, "https://a.example/"),
                (week, "https://b.example/"),
                (past_week, "https://c.example/"),
                (month, "https://d.example/"),
                (past_month, "https://e.example/"),
            ] {
                add(session, id, (link, link), None);
            }
            let listing = Listing {
                grouping: Grouping::Added,
                ..Listing::of(Span::Scope(Scope::All), Sort::Oldest)
            };
            let Parts::Indexed(parts) = session.parts(&listing)?.parts else {
                panic!("periods are indexed");
            };
            let mut read = Vec::new();
            for (within, count) in parts {
                let part = Part {
                    span: Span::Scope(Scope::All),
                    sort: Sort::Oldest,
                    within: within.clone(),
                };
                read.push((within, count, session.page(&part, 0, usize::MAX)?));
            }
            let recent = session.page(
                &Part {
                    span: Span::Scope(Scope::Recent),
                    sort: Sort::Oldest,
                    within: Within::Whole,
                },
                0,
                usize::MAX,
            )?;
            Ok((read, recent))
        })
        .unwrap();
    assert_eq!(
        parts.0,
        [
            (Within::Period(Period::Week), 2, vec![week, ahead]),
            (Within::Period(Period::Month), 2, vec![month, past_week]),
            (Within::Period(Period::Earlier), 1, vec![past_month]),
        ]
    );
    assert_eq!(parts.1, [month, past_week, week, ahead]);
}

#[test]
fn folders_of_one_path_group_together_in_the_order_of_their_shortcuts() {
    let (_directory, store) = store();
    let (first, second) = (made(NOW, 1), made(NOW, 2));
    let (in_second, in_first, at_top) = (made(NOW, 3), made(NOW, 4), made(NOW, 5));
    let read = store
        .session(NOW, |session| {
            folder(session, second, "Work", None);
            folder(session, first, "Work", None);
            session.edit(Edit::Move {
                entry: Entry::Folder(first),
                folder: None,
                slot: Slot::Before(Entry::Folder(second)),
            })?;
            add(
                session,
                in_second,
                ("https://a.example/", "a"),
                Some(second),
            );
            add(session, in_first, ("https://b.example/", "b"), Some(first));
            add(session, at_top, ("https://c.example/", "c"), None);
            let listing = Listing {
                grouping: Grouping::Collection,
                ..Listing::of(Span::Scope(Scope::Recent), Sort::Title)
            };
            let Parts::Indexed(parts) = session.parts(&listing)?.parts else {
                panic!("folders are indexed");
            };
            let mut read = Vec::new();
            for (within, count) in parts {
                let part = Part {
                    span: listing.span,
                    sort: Sort::Title,
                    within: within.clone(),
                };
                read.push((within, count, session.page(&part, 0, usize::MAX)?));
            }
            Ok(read)
        })
        .unwrap();
    assert_eq!(
        read,
        [
            (Within::Folder(None), 1, vec![at_top]),
            (
                Within::Folder(Some("Work".to_owned())),
                2,
                vec![in_second, in_first]
            ),
        ],
        "a in the second Work sorts before b in the first"
    );
}

#[test]
fn a_session_that_changes_the_store_counts_once_more() {
    let (_directory, store) = store();
    let before = store
        .session(NOW, |session| Ok(session.generation()))
        .unwrap();
    let during = store
        .session(NOW, |session| {
            add(session, made(NOW, 1), ("https://a.example/", "a"), None);
            Ok(session.generation())
        })
        .unwrap();
    let after = store
        .session(NOW, |session| Ok(session.generation()))
        .unwrap();
    assert_eq!((before, during, after), (0, 1, 1));
    let read = store
        .session(NOW, |session| Ok(session.generation()))
        .unwrap();
    assert_eq!(read, 1, "a session that reads leaves the count");
}

#[test]
fn a_folder_s_entries_page_in_their_order() {
    let (_directory, store) = store();
    let work = made(NOW, 1);
    let ids = [made(NOW, 2), made(NOW, 3), made(NOW, 4)];
    store
        .session(NOW, |session| {
            add(session, ids[0], ("https://a.example/", "a"), None);
            folder(session, work, "Work", None);
            add(session, ids[1], ("https://b.example/", "b"), None);
            add(session, ids[2], ("https://c.example/", "c"), Some(work));
            let top = session.entries(None)?;
            assert_eq!(
                top,
                [
                    Entry::Shortcut(ids[0]),
                    Entry::Folder(work),
                    Entry::Shortcut(ids[1])
                ]
            );
            assert_eq!(session.entries_page(None, 1, 1)?, [Entry::Folder(work)]);
            assert_eq!(session.entries_page(None, 2, 5)?, [Entry::Shortcut(ids[1])]);
            assert_eq!(session.entries_page(None, 3, 5)?, []);
            assert_eq!(session.entries_page(None, 0, 0)?, []);
            assert_eq!(session.home_count(None)?, 2);
            assert_eq!(session.home_count(Some(work))?, 1);
            let part = Part {
                span: Span::Home(None),
                sort: Sort::Title,
                within: Within::Whole,
            };
            assert_eq!(session.place_of(&part, ids[1])?, Some(1));
            assert_eq!(session.place_of(&part, ids[2])?, None);
            Ok(())
        })
        .unwrap();
}

#[test]
fn times_hold_from_their_first_to_before_their_last() {
    let times = Times {
        from: 3,
        until: Some(5),
    };
    assert!(!times.holds(2) && times.holds(3) && times.holds(4) && !times.holds(5));
    assert!(Times::EVERY.holds(0) && Times::EVERY.holds(u64::MAX));
    let key = index_rules::home_key(None);
    assert_eq!(
        super::added_bounds(&key, times).start,
        [&key[..], &3_u64.to_be_bytes()].concat()
    );
    assert_eq!(
        super::added_bounds(&[], Times::EVERY).end,
        None,
        "every time of the whole index reaches its end"
    );
}
