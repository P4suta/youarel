//! The store's tests: sessions, the database's files, icons, and the record of a publish.

mod differential;

use super::{
    DATABASE_FILE, FORMAT, LOCK_FILE, PUBLISH_LOCK_FILE, PUBLISHED_FILE, Record, Store, StoreError,
    icons,
};
use youarel_core::clock_rules::{DeviceId, Stamp};
use youarel_core::rank_rules::Visits;
use youarel_core::state::{Edit, Entry, ItemId, Slot, State};
use youarel_core::wire_rules::WireError;

const NOW: u64 = 1_700_000_000_000;

fn add(store: &Store, link: &str) -> ItemId {
    store
        .session(NOW, |session| {
            let id = session.fresh_id()?;
            session.edit(Edit::AddShortcut {
                id,
                link: link.to_owned(),
                title: link.to_owned(),
                folder: None,
            })?;
            Ok(id)
        })
        .unwrap()
}

/// The whole state a store holds, read in one session.
fn held(store: &Store) -> State {
    store
        .session(NOW, |session| Ok(session.records()?.into_state()))
        .unwrap()
}

#[test]
fn a_merged_replica_persists_and_the_session_runs_at_its_time() {
    let directory = tempfile::tempdir().unwrap();
    let mine = Store::new(directory.path().join("mine"));
    let theirs = Store::new(directory.path().join("theirs"));
    let shared = add(&theirs, "https://example.net/");
    let other = held(&theirs);
    let (now, changed, again) = mine
        .session(NOW + 7, |session| {
            Ok((
                session.now(),
                session.merge(&other)?,
                session.merge(&other)?,
            ))
        })
        .unwrap();
    assert_eq!((now, changed, again), (NOW + 7, true, false));
    let kept = mine
        .session(NOW + 8, |session| Ok(session.item(shared)?.is_some()))
        .unwrap();
    assert!(kept, "the merged shortcut was written");
    assert_eq!(held(&mine), other);
}

#[test]
fn edits_and_visits_persist_in_the_database_across_sessions() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().join("youarel"));
    let first = add(&store, "https://example.com/");
    let second = add(&store, "https://example.org/");
    store.session(NOW, |session| session.visit(second)).unwrap();
    store
        .session(NOW + 3, |session| session.visit(second))
        .unwrap();
    let (entries, visits, device) = store
        .session(NOW + 1, |session| {
            Ok((
                session.entries(None)?,
                session.visits_of(second)?,
                session.device(),
            ))
        })
        .unwrap();
    assert_eq!(entries, [Entry::Shortcut(first), Entry::Shortcut(second)]);
    assert_eq!(
        visits,
        Some(Visits {
            count: 2,
            last: NOW + 3
        })
    );
    let again = store.session(NOW, |session| Ok(session.device())).unwrap();
    assert_eq!(device, again, "a store keeps its device");
    assert_eq!(
        super::fs::list(store.directory()).unwrap(),
        [DATABASE_FILE, LOCK_FILE],
        "the database holds the state, the device, and the visits alike"
    );
}

#[test]
fn a_new_store_makes_an_empty_database_of_its_own_and_leaves_every_other_file_alone() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().join("youarel"));
    let earlier = store.directory().join("state.v2");
    let staging = store.directory().join(format!("{DATABASE_FILE}.staging"));
    super::fs::create_dir_all(store.directory()).unwrap();
    super::fs::replace(&earlier, b"the records of an earlier build").unwrap();
    super::fs::replace(&staging, b"half a database").unwrap();
    let (state, latest, device) = store
        .session(NOW, |session| {
            Ok((
                session.records()?.into_state(),
                session.latest(),
                session.device(),
            ))
        })
        .unwrap();
    assert_eq!((state, latest), (State::default(), None));
    assert_eq!(
        super::fs::list(store.directory()).unwrap(),
        [DATABASE_FILE, LOCK_FILE, "state.v2"],
        "the making replaced its staging file"
    );
    assert_eq!(
        super::fs::read_bytes_if_present(&earlier).unwrap(),
        Some(b"the records of an earlier build".to_vec())
    );
    let again = store.session(NOW, |session| Ok(session.device())).unwrap();
    assert_eq!(device, again, "a store keeps the device it made");
    let other = Store::new(directory.path().join("other"))
        .session(NOW, |session| Ok(session.device()))
        .unwrap();
    assert_ne!(device, other, "each store makes a device of its own");
}

#[test]
fn a_failed_session_writes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_path_buf());
    let refused = store.session(NOW, |session| {
        let id = session.fresh_id()?;
        session.edit(Edit::AddShortcut {
            id,
            link: "https://example.com/".to_owned(),
            title: String::new(),
            folder: None,
        })?;
        session.visit(id)?;
        session.edit(Edit::Retitle {
            id: ItemId([9; 16]),
            title: String::new(),
        })
    });
    let error = refused.unwrap_err();
    assert!(matches!(error, StoreError::Edit(_)));
    assert_eq!(error.to_string(), "no such shortcut or folder");
    assert_eq!(held(&store), State::default());
    let latest = store.session(NOW, |session| Ok(session.latest())).unwrap();
    assert_eq!(latest, None, "no stamp of the refused session stays");
}

#[test]
fn later_edits_order_after_earlier_ones_even_when_the_clock_moves_back() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_path_buf());
    let id = add(&store, "https://example.com/");
    store
        .session(NOW - 10_000, |session| {
            session.edit(Edit::Retitle {
                id,
                title: "renamed".to_owned(),
            })
        })
        .unwrap();
    let (title, latest) = store
        .session(NOW, |session| {
            Ok((
                session.item(id)?.map(|shortcut| shortcut.title.value),
                session.latest(),
            ))
        })
        .unwrap();
    assert_eq!(title.as_deref(), Some("renamed"));
    assert_eq!(
        latest.map(|stamp| (stamp.wall, stamp.counter)),
        Some((NOW, 1))
    );
}

#[test]
fn the_window_s_settings_default_save_read_back_and_name_a_damaged_file() {
    use super::settings::{SETTINGS_FILE, Settings};
    use youarel_core::library_rules::{Layout, Sort};
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().join("youarel"));
    assert_eq!(store.settings().unwrap(), Settings::default());
    let chosen = Settings {
        layout: Layout::List,
        sort: Sort::Title,
        ..Settings::default()
    };
    store.save_settings(&chosen).unwrap();
    assert_eq!(store.settings().unwrap(), chosen);
    super::fs::replace(&store.directory().join(SETTINGS_FILE), b"{}").unwrap();
    let error = store.settings().unwrap_err();
    assert!(error.to_string().contains(SETTINGS_FILE), "{error}");
    assert!(std::error::Error::source(&error).is_some());
}

/// Keeps an icon for a host in one session at `now`, its image and its identifier spelled by one byte.
fn keep(store: &Store, now: u64, host: &str, image: u8) {
    store
        .session(now, |session| {
            session.keep_icon(host, icons::IconId([image; 32]), &picture(image))
        })
        .unwrap();
}

fn picture(image: u8) -> Vec<u8> {
    vec![image; 8]
}

/// The image each host shows, and when the device fetched it.
fn shown(store: &Store) -> Vec<(String, u8, u64)> {
    store
        .session(NOW, |session| {
            Ok(session
                .icons()?
                .iter()
                .map(|(host, icon)| (host.clone(), icon.image.0[0], icon.fetched))
                .collect())
        })
        .unwrap()
}

#[test]
fn an_icon_kept_for_a_host_shows_in_the_next_session_and_its_image_reads_back() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().join("youarel"));
    assert_eq!(store.images(&[icons::IconId([1; 32])]).unwrap(), [None]);
    assert!(
        !crate::fs::exists(&directory.path().join("youarel").join(DATABASE_FILE)).unwrap(),
        "a read of images makes no database"
    );
    assert_eq!(shown(&store), []);
    keep(&store, NOW + 5, "example.com", 1);
    assert_eq!(shown(&store), [("example.com".to_owned(), 1, NOW + 5)]);
    let icon = store
        .session(NOW, |session| {
            Ok((session.icon("example.com")?, session.icon("example.org")?))
        })
        .unwrap();
    assert_eq!(
        icon,
        (
            Some(icons::Icon {
                image: icons::IconId([1; 32]),
                fetched: NOW + 5
            }),
            None
        )
    );
    assert_eq!(
        store.image(icons::IconId([1; 32])).unwrap(),
        Some(picture(1))
    );
    assert_eq!(store.image(icons::IconId([2; 32])).unwrap(), None);
    assert_eq!(
        store
            .images(&[icons::IconId([2; 32]), icons::IconId([1; 32])])
            .unwrap(),
        [None, Some(picture(1))]
    );
}

#[test]
fn a_new_icon_for_a_host_takes_its_place_and_an_image_no_host_shows_goes() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_path_buf());
    let kept = |image: u8| store.image(icons::IconId([image; 32])).unwrap().is_some();
    keep(&store, NOW, "a.example", 1);
    keep(&store, NOW, "b.example", 1);
    keep(&store, NOW, "a.example", 2);
    assert!(
        kept(1) && kept(2),
        "another host still shows the first image"
    );
    keep(&store, NOW, "b.example", 3);
    assert!(
        !kept(1) && kept(2) && kept(3),
        "no host shows the first image"
    );
    keep(&store, NOW, "b.example", 3);
    assert!(kept(3), "an image kept again for its own host stays");
    store
        .session(NOW, |session| {
            session.keep_icon("c.example", icons::IconId([4; 32]), &picture(4))?;
            session.keep_icon("c.example", icons::IconId([5; 32]), &picture(5))
        })
        .unwrap();
    assert!(
        !kept(4) && kept(5),
        "an image replaced in its session never lands"
    );
    assert_eq!(
        shown(&store)
            .into_iter()
            .map(|(host, image, _)| (host, image))
            .collect::<Vec<_>>(),
        [
            ("a.example".to_owned(), 2),
            ("b.example".to_owned(), 3),
            ("c.example".to_owned(), 5)
        ]
    );
}

#[test]
fn an_image_beyond_the_limit_is_refused_and_a_failed_session_keeps_no_icon() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_path_buf());
    let longest = usize::try_from(icons::LONGEST_IMAGE).unwrap();
    let refused = store
        .session(NOW, |session| {
            session.keep_icon("a.example", icons::IconId([1; 32]), &vec![0; longest + 1])
        })
        .unwrap_err();
    assert!(matches!(refused, StoreError::Image(length) if length == longest + 1));
    assert!(refused.to_string().contains("passes"), "{refused}");
    assert!(std::error::Error::source(&refused).is_none());
    let failed = store.session(NOW, |session| {
        session.keep_icon("a.example", icons::IconId([2; 32]), &vec![0; longest])?;
        Err::<(), _>(StoreError::Clock)
    });
    assert!(failed.is_err());
    assert_eq!(shown(&store), []);
    assert_eq!(store.image(icons::IconId([2; 32])).unwrap(), None);
    store
        .session(NOW, |session| {
            session.keep_icon("a.example", icons::IconId([2; 32]), &vec![0; longest])
        })
        .unwrap();
    assert_eq!(
        store
            .image(icons::IconId([2; 32]))
            .unwrap()
            .map(|image| image.len()),
        Some(longest)
    );
}

#[test]
fn the_record_of_a_publish_reads_back_for_its_target_alone() {
    use youarel_core::sync_rules::Carried;
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().join("youarel"));
    assert_eq!(store.published("/shared").unwrap(), None, "no publish yet");
    let carried = Carried {
        latest: Some(Stamp {
            wall: NOW,
            counter: 2,
            device: DeviceId([7; 16]),
        }),
        operations: 4,
    };
    store.record_published("/shared", carried).unwrap();
    assert_eq!(store.published("/shared").unwrap(), Some(carried));
    assert_eq!(
        store.published("/shared/").unwrap(),
        None,
        "a record names its target exactly"
    );
    let empty = Carried {
        latest: None,
        operations: 0,
    };
    store
        .record_published("https://relay.example", empty)
        .unwrap();
    assert_eq!(
        store.published("https://relay.example").unwrap(),
        Some(empty)
    );
    assert_eq!(
        store.published("/shared").unwrap(),
        None,
        "the record of the publish before is gone"
    );
    // Windows refuses to read a file another handle holds locked, so the test looks for the lock's name alone.
    let held = store.publishing().unwrap();
    let names = super::fs::list(store.directory()).unwrap();
    assert!(
        names.iter().any(|name| name == PUBLISH_LOCK_FILE),
        "the hold names its file: {names:?}"
    );
    drop(held);
    drop(store.publishing().unwrap());
}

#[test]
fn a_damaged_record_of_a_publish_names_its_file() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_path_buf());
    let path = store.directory().join(PUBLISHED_FILE);
    super::fs::replace(&path, b"{}").unwrap();
    let unshaped = store.published("/shared").unwrap_err();
    assert!(matches!(unshaped, StoreError::Local { .. }));
    assert!(unshaped.to_string().contains(PUBLISHED_FILE), "{unshaped}");
    let misnamed =
        br#"{"target":"/shared","latest":{"wall":1,"counter":0,"device":"zz"},"operations":1}"#;
    super::fs::replace(&path, misnamed).unwrap();
    let undecoded = store.published("/shared").unwrap_err();
    assert!(matches!(undecoded, StoreError::Decode { .. }));
    assert!(
        undecoded.to_string().contains(PUBLISHED_FILE),
        "{undecoded}"
    );
}

/// A state of two folders, one inside the other, and three shortcuts, one of them removed, as another device made it.
fn made_elsewhere() -> State {
    let mut state = State::default();
    let stamp = |wall| Stamp {
        wall,
        counter: 0,
        device: DeviceId([5; 16]),
    };
    let edits = [
        Edit::AddFolder {
            id: ItemId([1; 16]),
            name: "Work".to_owned(),
            parent: None,
        },
        Edit::AddFolder {
            id: ItemId([2; 16]),
            name: "Infra".to_owned(),
            parent: Some(ItemId([1; 16])),
        },
        Edit::AddShortcut {
            id: ItemId([3; 16]),
            link: "https://a.example/".to_owned(),
            title: "A".to_owned(),
            folder: Some(ItemId([2; 16])),
        },
        Edit::AddShortcut {
            id: ItemId([4; 16]),
            link: "https://b.example/".to_owned(),
            title: "B".to_owned(),
            folder: None,
        },
        Edit::AddShortcut {
            id: ItemId([6; 16]),
            link: "https://c.example/".to_owned(),
            title: "C".to_owned(),
            folder: None,
        },
        Edit::Remove {
            entry: Entry::Shortcut(ItemId([6; 16])),
        },
    ];
    for (wall, edit) in (1..).zip(edits) {
        state.apply(edit, stamp(wall)).unwrap();
    }
    state
}

#[test]
fn a_database_of_another_format_or_a_damaged_record_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_path_buf());
    let id = add(&store, "https://example.com/");
    let path = store.directory().join(DATABASE_FILE);
    {
        let database = super::fs::open_database(&path, super::CACHE_BYTES).unwrap();
        let transaction = database.begin_write().unwrap();
        super::database::open(&transaction, super::database::META)
            .unwrap()
            .insert(super::database::FORMAT_KEY, 9_u16.to_be_bytes().as_slice())
            .unwrap();
        transaction.commit().unwrap();
    }
    let other = store.session(NOW, |_| Ok(())).unwrap_err();
    assert!(matches!(other, StoreError::Format(Some(9))));
    assert_eq!(
        other.to_string(),
        format!("the store's database has format 9, but this youarel reads format {FORMAT}")
    );
    {
        let database = super::fs::open_database(&path, super::CACHE_BYTES).unwrap();
        let transaction = database.begin_write().unwrap();
        super::database::set_format(&transaction).unwrap();
        super::database::open(&transaction, super::database::SHORTCUTS)
            .unwrap()
            .insert(id.0.as_slice(), [0xff].as_slice())
            .unwrap();
        transaction.commit().unwrap();
    }
    let damaged = store.session(NOW, |session| session.item(id)).unwrap_err();
    assert!(matches!(damaged, StoreError::Damaged(WireError::Tag(0xff))));
    assert!(damaged.to_string().contains("would not have written"));
    assert!(std::error::Error::source(&damaged).is_some());
}

#[test]
fn each_failure_of_the_database_says_what_failed() {
    let missing = StoreError::Missing("its device");
    assert_eq!(missing.to_string(), "the store's database lacks its device");
    assert!(std::error::Error::source(&missing).is_none());
    let unnamed = StoreError::Format(None);
    assert_eq!(unnamed.to_string(), "the store's database names no format");
    assert!(std::error::Error::source(&unnamed).is_none());
    let failed = StoreError::Database(redb::Error::DatabaseAlreadyOpen);
    assert_eq!(failed.to_string(), "the store's database failed");
    assert!(std::error::Error::source(&failed).is_some());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(DATABASE_FILE);
    let held = super::fs::open_database(&path, super::CACHE_BYTES).unwrap();
    let again = super::fs::open_database(&path, super::CACHE_BYTES).unwrap_err();
    assert!(again.to_string().contains(DATABASE_FILE), "{again}");
    drop(held);
}

#[test]
fn the_previous_database_format_is_refused_without_upgrading_it() {
    const PREVIOUS_FORMAT: u16 = 2;
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_owned());
    let path = directory.path().join(DATABASE_FILE);
    {
        let database = super::fs::open_database(&path, super::CACHE_BYTES).unwrap();
        let transaction = database.begin_write().unwrap();
        super::database::open(&transaction, super::database::META)
            .unwrap()
            .insert(
                super::database::FORMAT_KEY,
                PREVIOUS_FORMAT.to_be_bytes().as_slice(),
            )
            .unwrap();
        transaction.commit().unwrap();
    }
    let refused = store.session(NOW, |_session| Ok(())).unwrap_err();
    assert!(matches!(refused, StoreError::Format(Some(PREVIOUS_FORMAT))));
    let database = super::fs::open_database(&path, super::CACHE_BYTES).unwrap();
    let transaction = database.begin_write().unwrap();
    assert_eq!(
        super::database::format(&transaction).unwrap(),
        Some(PREVIOUS_FORMAT)
    );
}

#[test]
fn a_merged_record_changes_the_store_once() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_path_buf());
    let state = made_elsewhere();
    let (folder, shortcut) = (
        state.folders.first_key_value().unwrap(),
        state.shortcuts.first_key_value().unwrap(),
    );
    let changes = store
        .session(NOW, |session| {
            Ok([
                session.merge_record(Record::Folder(*folder.0, folder.1.clone()))?,
                session.merge_record(Record::Folder(*folder.0, folder.1.clone()))?,
                session.merge_record(Record::Shortcut(*shortcut.0, shortcut.1.clone()))?,
                session.merge_record(Record::Shortcut(*shortcut.0, shortcut.1.clone()))?,
            ])
        })
        .unwrap();
    assert_eq!(changes, [true, false, true, false]);
    let folders_alone = State {
        devices: std::collections::BTreeMap::new(),
        folders: state.folders.clone(),
        shortcuts: std::collections::BTreeMap::new(),
    };
    let elsewhere = tempfile::tempdir().unwrap();
    let merged = Store::new(elsewhere.path().to_path_buf())
        .session(NOW, |session| {
            Ok([
                session.merge(&folders_alone)?,
                session.merge(&folders_alone)?,
            ])
        })
        .unwrap();
    assert_eq!(
        merged,
        [true, false],
        "a state of folders alone changes the store once"
    );
}

#[test]
fn a_folder_moved_after_another_takes_the_place_between_it_and_the_next() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_path_buf());
    let (order, wanted) = store
        .session(NOW, |session| {
            let mut made = Vec::new();
            for name in ["A", "B", "C"] {
                let id = session.fresh_id()?;
                session.edit(Edit::AddFolder {
                    id,
                    name: name.to_owned(),
                    parent: None,
                })?;
                made.push(id);
            }
            let (first, second, third) = (made[0], made[1], made[2]);
            session.edit(Edit::Move {
                entry: Entry::Folder(third),
                folder: None,
                slot: Slot::After(Entry::Folder(first)),
            })?;
            Ok((
                session.folders().children(None).to_vec(),
                vec![first, third, second],
            ))
        })
        .unwrap();
    assert_eq!(
        order, wanted,
        "the folder after the anchor bounds the place"
    );
}

/// Adds a live shortcut with an identifier of one repeated byte, its link, its title, its alias, and its folder.
fn named_shortcut(
    session: &mut super::Session,
    byte: u8,
    (link, title, alias): (&str, &str, &str),
    folder: Option<ItemId>,
) -> Result<ItemId, StoreError> {
    let id = ItemId([byte; 16]);
    session.edit(Edit::AddShortcut {
        id,
        link: link.to_owned(),
        title: title.to_owned(),
        folder,
    })?;
    session.edit(Edit::Realias {
        id,
        alias: alias.to_owned(),
    })?;
    Ok(id)
}

#[test]
fn a_name_finds_the_live_shortcuts_it_names_through_the_indexes() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path().to_path_buf());
    let link = "https://example.com/";
    let (dash, docs, held, gone, folder) = store
        .session(NOW, |session| {
            let folder = ItemId([0x0f; 16]);
            session.edit(Edit::AddFolder {
                id: folder,
                name: "Work".to_owned(),
                parent: None,
            })?;
            let dash = named_shortcut(session, 0x1a, (link, "Docs", "-"), None)?;
            let docs = named_shortcut(
                session,
                0x2b,
                ("https://docs.example/", "docs", "docs"),
                None,
            )?;
            let held = named_shortcut(session, 0x3c, (link, "Docs", "gh"), Some(folder))?;
            let gone = named_shortcut(session, 0x1b, ("https://gone.example/", "-", "-"), None)?;
            session.edit(Edit::Remove {
                entry: Entry::Shortcut(gone),
            })?;
            Ok((dash, docs, held, gone, folder))
        })
        .unwrap();
    store
        .session(NOW, |session| {
            let ids = |named: crate::Named| named.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
            assert_eq!(
                ids(session.aliased("-")?),
                [dash],
                "an alias of separators alone"
            );
            assert_eq!(
                ids(session.aliased("docs")?),
                [docs],
                "a word that a title also holds"
            );
            assert_eq!(ids(session.aliased("")?), Vec::<ItemId>::new());
            assert_eq!(ids(session.titled("Docs")?), [dash, held]);
            assert_eq!(
                ids(session.titled("docs")?),
                [docs],
                "a title matches exactly"
            );
            assert_eq!(ids(session.holders(link)?), [dash, held], "in every folder");
            assert_eq!(ids(session.spelled("1a1a1a1")?), [dash], "an odd digit");
            assert_eq!(
                ids(session.spelled("1")?),
                [dash],
                "a removed shortcut goes unnamed"
            );
            assert_eq!(
                ids(session.spelled("1A")?),
                Vec::<ItemId>::new(),
                "a capital"
            );
            assert_eq!(ids(session.spelled("")?), [dash, docs, held]);
            assert_eq!(session.item(gone)?, None);
            assert!(session.folders().holds(folder));
            Ok(())
        })
        .unwrap();
}
