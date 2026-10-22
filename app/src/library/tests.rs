use super::{Addition, Begun, Library, MOST_CHOICES, PAGE};
use crate::intent::{
    ColumnName, EntryName, Extend, Fields, GroupName, Intent, KeyName, LayoutName, ScopeName,
    SortName, TargetName,
};
use crate::syncing::SyncState;
use crate::view::{CollectionCount, ColumnEntry, EntryKind, Item, RowView, View};
use std::path::PathBuf;
use youarel_core::folder_rules::Seen;
use youarel_core::state::{Edit, ItemId};
use youarel_core::sync_rules::{BACKOFF, INTERVAL, QUIET, SECOND, Sync};
use youarel_link::Link;
use youarel_store::Store;
use youarel_store::icons::IconId;

const OWN_CHAIN: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OWN_HEAD: &str = "1111111111111111111111111111111111111111111111111111111111111111/head";
const OTHER_HEAD: &str = "2222222222222222222222222222222222222222222222222222222222222222/head";

fn library() -> (tempfile::TempDir, Library) {
    let home = tempfile::tempdir().unwrap();
    let library = Library::new(Store::new(home.path().to_path_buf()));
    (home, library)
}

/// The shortcuts of the page the cards or the list show.
fn items(view: &View) -> Vec<&Item> {
    view.listing
        .rows
        .iter()
        .filter_map(|row| match row {
            RowView::Shortcut { item, .. } => Some(item.as_ref()),
            RowView::Folder { .. } | RowView::Gone => None,
        })
        .collect()
}

/// The folders of the page the cards or the list show.
fn folder_rows(view: &View) -> Vec<&CollectionCount> {
    view.listing
        .rows
        .iter()
        .filter_map(|row| match row {
            RowView::Folder { folder, .. } => Some(folder.as_ref()),
            RowView::Shortcut { .. } | RowView::Gone => None,
        })
        .collect()
}

/// The entries of the page of the column at `depth`.
fn entries(view: &View, depth: usize) -> Vec<&ColumnEntry> {
    view.columns
        .get(depth)
        .map_or_else(Vec::new, |column| column.entries.iter().collect())
}

fn addition(link: &str, title: &str, collection: Option<ItemId>) -> Addition {
    Addition {
        link: Link::parse(link).unwrap(),
        title: title.to_owned(),
        collection,
        icon: None,
        missing: None,
    }
}

fn add(library: &mut Library, link: &str, title: &str) -> View {
    library.add(addition(link, title, None)).unwrap()
}

fn act(library: &mut Library, intent: Intent) -> View {
    library.act(intent).unwrap()
}

fn move_focus(library: &mut Library, key: KeyName, across: usize) -> View {
    act(
        library,
        Intent::Step {
            key,
            across,
            extend: false,
        },
    )
}

/// The titles of the shortcuts the page shows: those of the cards or the list, or those of the first column.
fn titles(view: &View) -> Vec<&str> {
    if view.layout == LayoutName::Columns {
        return entries(view, 0)
            .into_iter()
            .filter(|entry| entry.kind == EntryKind::Shortcut)
            .map(|entry| entry.name.as_str())
            .collect();
    }
    items(view)
        .into_iter()
        .map(|item| item.title.as_str())
        .collect()
}

/// The identifier of a shortcut the page shows, in the cards, the list, or any column.
fn id_of(view: &View, title: &str) -> String {
    let listed = items(view)
        .into_iter()
        .find(|item| item.title == title)
        .map(|item| item.id.clone());
    listed
        .or_else(|| {
            view.columns
                .iter()
                .flat_map(|column| &column.entries)
                .find(|entry| entry.kind == EntryKind::Shortcut && entry.name == title)
                .map(|entry| entry.id.clone())
        })
        .unwrap()
}

#[test]
fn an_added_shortcut_shows_focused_in_every_layout() {
    let (_home, mut library) = library();
    let empty = library.view().unwrap();
    assert_eq!((empty.count, empty.title.as_str()), (0, "All"));
    let view = add(&mut library, "https://url.spec.whatwg.org/", "URL Standard");
    assert_eq!(titles(&view), ["URL Standard"]);
    assert_eq!(
        view.notice.as_ref().map(|notice| notice.text.as_str()),
        Some("Added URL Standard")
    );
    assert_eq!(
        view.focus.as_deref(),
        Some(id_of(&view, "URL Standard").as_str())
    );
    let item = view.preview.as_ref().unwrap();
    assert_eq!(
        (item.url.name.as_str(), item.url.suffix.as_str()),
        ("whatwg", ".org")
    );
    assert_eq!(item.collection, None);
    for layout in [LayoutName::List, LayoutName::Columns, LayoutName::Cards] {
        let shown = act(&mut library, Intent::Layout { layout });
        assert_eq!(shown.layout, layout);
        assert_eq!(shown.count, 1);
    }
    let missing = library
        .add(Addition {
            missing: Some("no icon".to_owned()),
            ..addition("https://example.com/", "Example", None)
        })
        .unwrap();
    assert_eq!(
        missing.notice.map(|notice| notice.text),
        Some("Added Example (no icon)".to_owned())
    );
}

#[test]
fn a_collection_holds_a_link_once_and_another_collection_may_hold_it_too() {
    let (_home, mut library) = library();
    let link = Link::parse("https://a.example/").unwrap();
    add(&mut library, link.as_str(), "A");
    let held = library.holder(&link, None).unwrap().unwrap();
    let again = library
        .add(addition(link.as_str(), "A again", None))
        .unwrap();
    assert_eq!(
        (again.count, again.notice.map(|notice| notice.text)),
        (1, Some("Already here: A".to_owned()))
    );
    assert_eq!(again.focus, Some(crate::intent::text(held)));
    let work = collection(&mut library, "Work", None);
    let work_id = item_of(&work);
    assert_eq!(library.holder(&link, Some(work_id)).unwrap(), None);
    library
        .add(addition(link.as_str(), "A", Some(work_id)))
        .unwrap();
    let revealed = library
        .reveal(library.holder(&link, Some(work_id)).unwrap().unwrap())
        .unwrap();
    assert_eq!(
        revealed.notice.map(|notice| notice.text),
        Some("Already in Work: A".to_owned())
    );
    assert_eq!(
        revealed
            .trail
            .iter()
            .map(|step| step.name.as_str())
            .collect::<Vec<_>>(),
        ["Work"],
        "the columns open down to the shortcut the collection holds"
    );
    assert!(revealed.preview.is_some_and(|item| item.title == "A"));
}

fn item_of(text: &str) -> ItemId {
    crate::intent::item(text).unwrap()
}

#[test]
fn a_title_typed_in_place_replaces_the_old_one_unless_blank() {
    let (_home, mut library) = library();
    let view = add(&mut library, "https://a.example/", "A");
    let id = id_of(&view, "A");
    let renamed = act(
        &mut library,
        Intent::Retitle {
            id: id.clone(),
            title: " Alpha ".to_owned(),
        },
    );
    assert_eq!(titles(&renamed), ["Alpha"]);
    let refused = act(
        &mut library,
        Intent::Retitle {
            id,
            title: " ".to_owned(),
        },
    );
    assert_eq!(titles(&refused), ["Alpha"]);
    assert!(refused.notice.is_some_and(|notice| notice.error));
}

/// The links of the grouping tests: two pages of one site under two subdomains, one of another site, and one filed in Work.
const MDN_PAGE: &str = "https://developer.mozilla.org/a";
const GITHUB_PAGE: &str = "https://github.com/b";
const MOZILLA_PAGE: &str = "https://www.mozilla.org/c";
const EXAMPLE_PAGE: &str = "https://example.com/d";

/// Three links at the top, two of one site, and one more in the collection Work, ordered by title.
fn sites() -> (tempfile::TempDir, Library) {
    let (home, mut library) = library();
    add(&mut library, MDN_PAGE, "A");
    add(&mut library, GITHUB_PAGE, "B");
    add(&mut library, MOZILLA_PAGE, "C");
    let work = collection(&mut library, "Work", None);
    library
        .add(addition(EXAMPLE_PAGE, "D", Some(item_of(&work))))
        .unwrap();
    act(
        &mut library,
        Intent::Sort {
            sort: SortName::Title,
        },
    );
    (home, library)
}

fn labels(view: &View) -> Vec<(String, usize)> {
    view.listing
        .groups
        .iter()
        .map(|group| (group.label.clone(), group.count))
        .collect()
}

/// The cards over the scope of the shortcuts added lately, which lists every shortcut flat across the collections.
fn flat_cards(library: &mut Library) {
    act(
        library,
        Intent::Layout {
            layout: LayoutName::Cards,
        },
    );
    act(
        library,
        Intent::Scope {
            scope: ScopeName::Recent,
        },
    );
}

#[test]
fn every_layout_gathers_under_sites_collections_or_ages() {
    let (_home, mut library) = sites();
    flat_cards(&mut library);
    let by_site = act(
        &mut library,
        Intent::Group {
            grouping: GroupName::Site,
        },
    );
    assert_eq!(
        labels(&by_site),
        [
            ("example.com".to_owned(), 1),
            ("github.com".to_owned(), 1),
            ("mozilla.org".to_owned(), 2)
        ]
    );
    assert_eq!(
        titles(&by_site),
        ["D", "B", "A", "C"],
        "the keys follow the headings"
    );
    let by_collection = act(
        &mut library,
        Intent::Group {
            grouping: GroupName::Collection,
        },
    );
    assert_eq!(
        labels(&by_collection),
        [("No collection".to_owned(), 3), ("Work".to_owned(), 1)]
    );
    let by_age = act(
        &mut library,
        Intent::Group {
            grouping: GroupName::Added,
        },
    );
    assert_eq!(labels(&by_age), [("Past 7 days".to_owned(), 4)]);
    let top = act(
        &mut library,
        Intent::Scope {
            scope: ScopeName::All,
        },
    );
    assert_eq!(
        (labels(&top), titles(&top)),
        (vec![("Past 7 days".to_owned(), 3)], vec!["A", "B", "C"]),
        "the cards at the top gather the shortcuts there, and show Work as a folder"
    );
    assert_eq!(
        top.listing
            .groups
            .iter()
            .map(|group| group.place)
            .collect::<Vec<_>>(),
        [1],
        "a heading stands at the place of its first row, after the folder"
    );
    assert_eq!((top.listing.folders, top.listing.total), (1, 4));
}

#[test]
fn down_in_gathered_cards_goes_to_the_row_under_the_next_heading() {
    let (_home, mut library) = sites();
    flat_cards(&mut library);
    act(
        &mut library,
        Intent::Group {
            grouping: GroupName::Site,
        },
    );
    let focused = |view: &View| {
        items(view)
            .into_iter()
            .find(|item| view.focus.as_ref() == Some(&item.id))
            .map(|item| item.title.clone())
    };
    let step = |shown: &mut Library, key: KeyName| {
        act(
            shown,
            Intent::Step {
                key,
                across: 3,
                extend: false,
            },
        )
    };
    let first = step(&mut library, KeyName::First);
    assert_eq!(focused(&first), Some("D".to_owned()));
    let down = step(&mut library, KeyName::Down);
    assert_eq!(
        focused(&down),
        Some("B".to_owned()),
        "each site begins a row of its own"
    );
    let further = step(&mut library, KeyName::Down);
    assert_eq!(focused(&further), Some("A".to_owned()));
    let up = step(&mut library, KeyName::Up);
    assert_eq!(focused(&up), Some("B".to_owned()));
}

fn folder_names(view: &View) -> Vec<(&str, usize)> {
    folder_rows(view)
        .into_iter()
        .map(|folder| (folder.name.as_str(), folder.count))
        .collect()
}

fn trail_names(view: &View) -> Vec<&str> {
    view.trail.iter().map(|step| step.name.as_str()).collect()
}

#[test]
fn the_cards_and_the_list_stand_in_one_folder_and_open_the_folders_inside_it() {
    for layout in [LayoutName::Cards, LayoutName::List] {
        let (_home, mut library) = sites();
        let top = act(&mut library, Intent::Layout { layout });
        assert_eq!(
            (top.title.as_str(), top.count, trail_names(&top)),
            ("All", 4, vec![]),
            "the top counts every shortcut inside it"
        );
        assert_eq!(folder_names(&top), [("Work", 1)]);
        assert_eq!(titles(&top), ["A", "B", "C"]);
        let first = act(
            &mut library,
            Intent::Step {
                key: KeyName::First,
                across: 3,
                extend: false,
            },
        );
        let work = folder_rows(&first).first().map(|folder| folder.id.clone());
        assert_eq!(first.focus, work, "the keys reach the folders first");
        assert_eq!(first.listing.focus, Some(0));
        let below = act(
            &mut library,
            Intent::Step {
                key: KeyName::Down,
                across: 3,
                extend: false,
            },
        );
        assert_eq!(
            below.focus.as_deref(),
            Some(id_of(&below, "A").as_str()),
            "the shortcuts begin a row under the folders"
        );
        // The cards step into a folder they open; the list opens it in place, and the sidebar or the trail steps into it.
        let inside = if layout == LayoutName::Cards {
            act(&mut library, Intent::Open { id: work.clone() })
        } else {
            let id = work.clone().unwrap();
            act(
                &mut library,
                Intent::Scope {
                    scope: ScopeName::Collection { id },
                },
            )
        };
        assert_eq!(
            (inside.title.as_str(), inside.count, trail_names(&inside)),
            ("Work", 1, vec!["Work"])
        );
        assert_eq!(
            (titles(&inside), folder_rows(&inside).len()),
            (vec!["D"], 0)
        );
        collection(&mut library, "Specs", work.clone());
        let deeper = library.view().unwrap();
        assert_eq!(trail_names(&deeper), ["Work", "Specs"]);
        act(
            &mut library,
            Intent::Scope {
                scope: ScopeName::All,
            },
        );
        let searched = act(
            &mut library,
            Intent::Search {
                text: "D".to_owned(),
            },
        );
        assert!(
            folder_rows(&searched).is_empty() && titles(&searched).contains(&"D"),
            "a search lists what it finds inside the folders too"
        );
    }
}

#[test]
fn the_list_keeps_the_columns_a_person_shows_and_drops_the_collection_inside_a_folder() {
    let (_home, mut library) = sites();
    let top = act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    assert_eq!(
        top.list_columns,
        [ColumnName::Link, ColumnName::Added],
        "every row inside one folder sits in the same collection"
    );
    let recent = act(
        &mut library,
        Intent::Scope {
            scope: ScopeName::Recent,
        },
    );
    assert_eq!(
        recent.list_columns,
        [ColumnName::Link, ColumnName::Collection, ColumnName::Added]
    );
    let hidden = act(
        &mut library,
        Intent::ShowColumn {
            column: ColumnName::Added,
            shown: false,
        },
    );
    assert_eq!(
        (hidden.list_columns, hidden.hidden_columns),
        (
            vec![ColumnName::Link, ColumnName::Collection],
            vec![ColumnName::Added]
        )
    );
    let reopened = Library::new(library.store()).view().unwrap();
    assert_eq!(
        reopened.hidden_columns,
        [ColumnName::Added],
        "the choice outlives the window"
    );
    let shown = act(
        &mut library,
        Intent::ShowColumn {
            column: ColumnName::Added,
            shown: true,
        },
    );
    assert_eq!(shown.hidden_columns, []);
}

/// Each row of the list's outline as how deep it sits and whether it shows the rows beneath it.
fn outline_rows(view: &View) -> Vec<(usize, bool)> {
    view.listing
        .rows
        .iter()
        .filter_map(|row| match row {
            RowView::Folder { depth, open, .. } => Some((*depth, *open == Some(true))),
            RowView::Shortcut { depth, .. } => Some((*depth, false)),
            RowView::Gone => None,
        })
        .collect()
}

#[test]
fn the_list_opens_a_folder_in_place_and_the_arrows_open_and_close_it() {
    let (_home, mut library) = sites();
    let top = act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    let work = folder_rows(&top)
        .first()
        .map(|folder| folder.id.clone())
        .unwrap();
    let rows = outline_rows;
    assert!(top.listing.outline);
    assert_eq!(rows(&top), [(0, false); 4], "Work and the three shortcuts");
    let open = act(
        &mut library,
        Intent::Disclose {
            id: work.clone(),
            open: true,
            deep: false,
        },
    );
    assert_eq!(
        rows(&open),
        [(0, true), (1, false), (0, false), (0, false), (0, false)]
    );
    assert_eq!(
        titles(&open),
        ["D", "A", "B", "C"],
        "the shortcut inside Work shows under it"
    );
    act(
        &mut library,
        Intent::Select {
            id: id_of(&open, "D"),
            extend: Extend::Replace,
            place: Some(1),
        },
    );
    let left = |shown: &mut Library, key: KeyName| move_focus(shown, key, 1);
    let up = left(&mut library, KeyName::Left);
    assert_eq!(
        (up.focus.as_deref(), up.listing.focus),
        (Some(work.as_str()), Some(0)),
        "the left arrow steps up to the folder"
    );
    let closed = left(&mut library, KeyName::Left);
    assert_eq!(closed.listing.total, 4, "and then closes it");
    let reopened = left(&mut library, KeyName::Right);
    assert_eq!(reopened.listing.total, 5, "the right arrow opens it");
    assert!(
        !reopened.listing.fresh,
        "a folder opened in place keeps the place in sight"
    );
    let opened = act(&mut library, Intent::Open { id: Some(work) });
    assert_eq!(
        (opened.listing.total, opened.title.as_str()),
        (4, "All"),
        "opening a folder of the list closes it in place, and the list stays where it stands"
    );
    let again = act(&mut library, Intent::Open { id: None });
    assert_eq!(
        again.listing.total, 5,
        "the return key opens the focused folder again"
    );
    let gathered = act(
        &mut library,
        Intent::Group {
            grouping: GroupName::Site,
        },
    );
    assert!(
        !gathered.listing.outline && gathered.listing.fresh,
        "headings take the outline's place"
    );
}

#[test]
fn the_sidebar_opens_its_tree_to_the_folder_shown_and_keeps_a_closed_folder_closed() {
    let (_home, mut library) = library();
    let work = collection(&mut library, "Work", None);
    let specs = collection(&mut library, "Specs", Some(work.clone()));
    collection(&mut library, "Life", None);
    let tree = |view: &View| {
        view.sidebar
            .collections
            .iter()
            .map(|folder| (folder.name.clone(), folder.depth, folder.opens, folder.open))
            .collect::<Vec<_>>()
    };
    let shown = act(
        &mut library,
        Intent::Scope {
            scope: ScopeName::Collection { id: specs },
        },
    );
    assert_eq!(
        tree(&shown),
        [
            ("Life".to_owned(), 0, false, false),
            ("Work".to_owned(), 0, true, true),
            ("Specs".to_owned(), 1, false, false)
        ],
        "the tree sets each folder's folders by name"
    );
    let closed = act(
        &mut library,
        Intent::DiscloseSidebar {
            id: work,
            open: false,
        },
    );
    assert_eq!(
        tree(&closed),
        [
            ("Life".to_owned(), 0, false, false),
            ("Work".to_owned(), 0, true, false)
        ],
        "a folder closed stays closed while the window stands inside it"
    );
}

#[test]
fn a_collection_shows_the_mark_a_person_gives_it_in_every_place() {
    let (_home, mut library) = library();
    let work = collection(&mut library, "Work", None);
    let marked = act(
        &mut library,
        Intent::MarkCollection {
            id: work.clone(),
            mark: " 📚 ".to_owned(),
        },
    );
    assert_eq!(
        marked
            .sidebar
            .collections
            .first()
            .map(|folder| folder.mark.as_str()),
        Some("📚")
    );
    let column = entries(&marked, 0);
    assert_eq!(column.first().map(|entry| entry.mark.as_str()), Some("📚"));
    let refused = act(
        &mut library,
        Intent::MarkCollection {
            id: work,
            mark: "a b".to_owned(),
        },
    );
    assert!(refused.notice.is_some_and(|notice| notice.error));
    act(
        &mut library,
        Intent::Scope {
            scope: ScopeName::All,
        },
    );
    let cards = act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::Cards,
        },
    );
    assert_eq!(
        folder_rows(&cards)
            .first()
            .map(|folder| folder.mark.as_str()),
        Some("📚"),
        "a refused mark leaves the old one"
    );
    let id = folder_rows(&cards)
        .first()
        .map(|folder| folder.id.clone())
        .unwrap();
    let coded = act(
        &mut library,
        Intent::MarkCollection {
            id,
            mark: ":rocket:".to_owned(),
        },
    );
    assert_eq!(
        folder_rows(&coded)
            .first()
            .map(|folder| folder.mark.as_str()),
        Some("🚀"),
        "a shortcode writes its emoji"
    );
}

#[test]
fn select_all_takes_every_folder_and_shortcut_the_list_shows() {
    let (_home, mut library) = sites();
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    let all = act(&mut library, Intent::SelectAll);
    assert_eq!(
        (
            all.selection.count,
            all.selection.folders,
            all.focus.is_some()
        ),
        (4, 1, true),
        "Work and the three shortcuts at the top, with the first focused"
    );
    assert!(all.listing.rows.iter().all(|row| match row {
        RowView::Folder { selected, .. } | RowView::Shortcut { selected, .. } => *selected,
        RowView::Gone => false,
    }));
    let toggled = act(
        &mut library,
        Intent::Select {
            id: id_of(&all, "B"),
            extend: Extend::Toggle,
            place: Some(2),
        },
    );
    assert_eq!(
        toggled.selection.count, 3,
        "a click with the command key leaves one row out"
    );
    let reordered = act(
        &mut library,
        Intent::Sort {
            sort: SortName::Newest,
        },
    );
    assert_eq!(
        reordered.selection.count, 3,
        "another order keeps every row it took"
    );
    let trashed = act(
        &mut library,
        Intent::Trash {
            target: TargetName::Selection,
        },
    );
    assert_eq!(
        (titles(&trashed), folder_rows(&trashed).len()),
        (vec!["B"], 0),
        "the trash takes what the selection names and nothing it left out"
    );
    assert_eq!(
        (trashed.selection.count, trashed.focus.as_deref()),
        (1, Some(id_of(&trashed, "B").as_str())),
        "the row after the ones that left takes the selection"
    );
}

#[test]
fn a_selection_of_folders_and_shortcuts_moves_and_goes_away_together() {
    let (_home, mut library) = sites();
    let top = act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    let work = folder_rows(&top)
        .first()
        .map(|folder| folder.id.clone())
        .unwrap();
    let archive = collection(&mut library, "Archive", None);
    let moved = act(
        &mut library,
        Intent::File {
            target: TargetName::Ids {
                ids: vec![work.clone(), id_of(&top, "A")],
            },
            collection: Some(archive.clone()),
        },
    );
    assert_eq!(
        moved.notice.map(|notice| notice.text),
        Some("Moved 1 folder and 1 shortcut to Archive".to_owned())
    );
    let inside = act(&mut library, Intent::Open { id: Some(archive) });
    assert_eq!(
        (folder_names(&inside), titles(&inside)),
        (vec![("Work", 1)], vec!["A"])
    );
    let trashed = act(
        &mut library,
        Intent::Trash {
            target: TargetName::Ids {
                ids: vec![work, id_of(&inside, "A")],
            },
        },
    );
    assert_eq!((trashed.count, folder_rows(&trashed).len()), (0, 0));
    assert_eq!(
        trashed.notice.map(|notice| notice.text),
        Some("Moved 1 folder and 1 shortcut to the Trash".to_owned())
    );
    assert_eq!(
        (trashed.selection.count, trashed.focus),
        (0, None),
        "an empty listing leaves nothing to select"
    );
}

#[test]
fn a_column_shows_its_folders_first_and_its_shortcuts_under_their_headings() {
    let (_home, mut library) = sites();
    let columns = act(
        &mut library,
        Intent::Group {
            grouping: GroupName::Site,
        },
    );
    let first = entries(&columns, 0)
        .into_iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    let headings = |view: &View| {
        view.columns.first().map_or_else(Vec::new, |column| {
            column
                .groups
                .iter()
                .map(|group| (group.label.clone(), group.place, group.count))
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(
        (first, headings(&columns)),
        (
            vec!["Work", "B", "A", "C"],
            vec![
                ("github.com".to_owned(), 1, 1),
                ("mozilla.org".to_owned(), 2, 2)
            ]
        ),
        "a column shows its folders first and its shortcuts under their sites"
    );
    let ungrouped = act(
        &mut library,
        Intent::Group {
            grouping: GroupName::None,
        },
    );
    assert_eq!(headings(&ungrouped), []);
}

#[test]
fn a_folder_moves_into_another_but_never_into_itself() {
    let (_home, mut library) = library();
    let work = collection(&mut library, "Work", None);
    let specs = collection(&mut library, "Specs", None);
    let moved = act(
        &mut library,
        Intent::File {
            target: TargetName::Ids {
                ids: vec![specs.clone()],
            },
            collection: Some(work.clone()),
        },
    );
    assert_eq!(
        moved.notice.map(|notice| notice.text),
        Some("Moved 1 folder to Work".to_owned())
    );
    assert_eq!(paths(&library), ["Work", "Work / Specs"]);
    let refused = act(
        &mut library,
        Intent::File {
            target: TargetName::Ids {
                ids: vec![work.clone()],
            },
            collection: Some(specs),
        },
    );
    assert!(refused.notice.is_some_and(|notice| notice.error));
    let top = act(
        &mut library,
        Intent::File {
            target: TargetName::Ids { ids: vec![work] },
            collection: None,
        },
    );
    assert_eq!(
        top.notice.map(|notice| notice.text),
        Some("Moved 1 folder to the top".to_owned())
    );
}

/// The path of every collection, in the order of the tree.
fn paths(library: &Library) -> Vec<String> {
    library
        .choices("", 0, MOST_CHOICES)
        .unwrap()
        .into_iter()
        .map(|choice| choice.path)
        .collect()
}

/// A collection named `name` inside `parent`, by its identifier.
fn collection(library: &mut Library, name: &str, parent: Option<String>) -> String {
    let before = library.choices("", 0, MOST_CHOICES).unwrap();
    act(
        library,
        Intent::NewCollection {
            name: name.to_owned(),
            parent,
        },
    );
    library
        .choices("", 0, MOST_CHOICES)
        .unwrap()
        .into_iter()
        .find(|choice| !before.contains(choice))
        .map(|choice| choice.id)
        .unwrap()
}

#[test]
fn a_new_collection_needs_a_name_and_opens_where_it_is_made() {
    let (_home, mut library) = library();
    let work = collection(&mut library, " Work ", None);
    let shown = library.view().unwrap();
    assert_eq!(
        (
            shown.title.as_str(),
            shown.here.as_ref().map(|here| here.id.as_str())
        ),
        ("All", Some(work.as_str())),
        "the columns open a new collection in place"
    );
    assert_eq!(
        shown
            .sidebar
            .collections
            .first()
            .map(|count| count.id.as_str()),
        Some(work.as_str())
    );
    let specs = collection(&mut library, "Specs", Some(work));
    let nested = library.view().unwrap();
    assert_eq!(
        nested
            .trail
            .iter()
            .map(|step| (step.name.as_str(), step.depth))
            .collect::<Vec<_>>(),
        [("Work", 0), ("Specs", 1)]
    );
    assert_eq!(
        nested.here.map(|here| (here.id, here.path)),
        Some((specs, "Work / Specs".to_owned()))
    );
    assert_eq!(paths(&library), ["Work", "Work / Specs"]);
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::Cards,
        },
    );
    collection(&mut library, "Life", None);
    let scoped = library.view().unwrap();
    assert_eq!(
        (scoped.title.as_str(), scoped.trail.len()),
        ("Life", 1),
        "the cards open a new collection as the folder they stand in"
    );
    let refused = act(
        &mut library,
        Intent::NewCollection {
            name: "  ".to_owned(),
            parent: None,
        },
    );
    assert!(refused.notice.is_some_and(|notice| notice.error));
}

#[test]
fn collections_hold_what_is_filed_and_go_away_with_it() {
    let (_home, mut library) = library();
    let view = add(&mut library, "https://a.example/", "A");
    let a = id_of(&view, "A");
    let work = collection(&mut library, "Work", None);
    let specs = collection(&mut library, "Specs", Some(work.clone()));
    let filed = act(
        &mut library,
        Intent::File {
            target: TargetName::Ids { ids: vec![a] },
            collection: Some(specs),
        },
    );
    assert_eq!(
        filed.notice.map(|notice| notice.text),
        Some("Moved 1 shortcut to Work / Specs".to_owned())
    );
    let work_view = act(
        &mut library,
        Intent::Scope {
            scope: ScopeName::Collection { id: work.clone() },
        },
    );
    assert_eq!(work_view.count, 1, "a collection holds what lies inside it");
    assert_eq!(
        work_view
            .sidebar
            .collections
            .first()
            .map(|count| count.count),
        Some(1)
    );
    let inside = act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::Cards,
        },
    );
    assert_eq!(
        (folder_names(&inside), titles(&inside)),
        (vec![("Specs", 1)], vec![]),
        "the cards stand in Work, whose shortcut sits in Specs"
    );
    let flat = act(
        &mut library,
        Intent::Scope {
            scope: ScopeName::Recent,
        },
    );
    assert_eq!(
        items(&flat)
            .first()
            .and_then(|item| item.collection.as_deref()),
        Some("Work / Specs")
    );
    act(
        &mut library,
        Intent::Scope {
            scope: ScopeName::Collection { id: work.clone() },
        },
    );
    act(
        &mut library,
        Intent::RenameCollection {
            id: work.clone(),
            name: "Job".to_owned(),
        },
    );
    let trashed = act(
        &mut library,
        Intent::Trash {
            target: TargetName::Ids { ids: vec![work] },
        },
    );
    assert_eq!(
        (trashed.title.as_str(), trashed.count, trashed.sidebar.trash),
        ("All", 0, 1),
        "a collection in the trash takes what it holds out of sight, and the window leaves it"
    );
}

#[test]
fn the_edit_sheet_saves_every_field_at_once() {
    let (_home, mut library) = library();
    let view = add(&mut library, "https://a.example/", "A");
    let id = id_of(&view, "A");
    let work = collection(&mut library, "Work", None);
    act(
        &mut library,
        Intent::Scope {
            scope: ScopeName::Recent,
        },
    );
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::Cards,
        },
    );
    let fields = Fields {
        id,
        title: "Alpha".to_owned(),
        alias: "al".to_owned(),
        note: "read first".to_owned(),
        collection: Some(work),
    };
    let saved = act(
        &mut library,
        Intent::Edit {
            fields: fields.clone(),
        },
    );
    let alpha = *items(&saved).first().unwrap();
    assert_eq!(
        (
            alpha.title.as_str(),
            alpha.alias.as_str(),
            alpha.note.as_str(),
            alpha.collection.as_deref()
        ),
        ("Alpha", "al", "read first", Some("Work"))
    );
    let unfiled = Fields {
        collection: None,
        ..fields.clone()
    };
    let kept = act(&mut library, Intent::Edit { fields: unfiled });
    assert_eq!(items(&kept).first().unwrap().collection, None);
    let untitled = Fields {
        title: " ".to_owned(),
        ..fields
    };
    let refused = act(&mut library, Intent::Edit { fields: untitled });
    assert_eq!(
        refused.notice.map(|notice| (notice.text, notice.error)),
        Some(("a shortcut needs a title".to_owned(), true))
    );
}

#[test]
fn keys_move_the_focus_through_the_cards_and_clicks_extend_the_selection() {
    let (_home, mut library) = library();
    for name in ["a", "b", "c", "d", "e"] {
        add(&mut library, &format!("https://{name}.example/"), name);
    }
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::Cards,
        },
    );
    let sorted = act(
        &mut library,
        Intent::Sort {
            sort: SortName::Title,
        },
    );
    assert_eq!(titles(&sorted), ["a", "b", "c", "d", "e"]);
    let first = act(
        &mut library,
        Intent::Step {
            key: KeyName::First,
            across: 3,
            extend: false,
        },
    );
    assert_eq!(first.focus, Some(id_of(&first, "a")));
    let down = act(
        &mut library,
        Intent::Step {
            key: KeyName::Down,
            across: 3,
            extend: false,
        },
    );
    assert_eq!(down.focus, Some(id_of(&down, "d")));
    assert_eq!(down.listing.focus, Some(3));
    let ranged = act(
        &mut library,
        Intent::Select {
            id: id_of(&down, "b"),
            extend: Extend::Range,
            place: Some(1),
        },
    );
    assert_eq!(ranged.selection.count, 3);
    let toggled = act(
        &mut library,
        Intent::Select {
            id: id_of(&down, "c"),
            extend: Extend::Toggle,
            place: Some(7),
        },
    );
    assert_eq!(
        toggled.selection.count, 2,
        "a place the page names wrongly gives way to the place of the row"
    );
    let trashed = act(
        &mut library,
        Intent::Trash {
            target: TargetName::Selection,
        },
    );
    assert_eq!(titles(&trashed), ["a", "c", "e"]);
    assert_eq!(
        (trashed.selection.count, trashed.focus.as_deref()),
        (1, Some(id_of(&trashed, "c").as_str())),
        "the row that came to the first place the selection left takes it"
    );
    let searched = act(
        &mut library,
        Intent::Search {
            text: "c.ex".to_owned(),
        },
    );
    assert_eq!(titles(&searched), ["c"]);
}

#[test]
fn the_columns_walk_down_the_folders_into_a_preview() {
    let (_home, mut library) = library();
    let view = add(&mut library, "https://a.example/", "A");
    let work = collection(&mut library, "Work", None);
    act(
        &mut library,
        Intent::File {
            target: TargetName::Ids {
                ids: vec![id_of(&view, "A")],
            },
            collection: Some(work),
        },
    );
    act(
        &mut library,
        Intent::Scope {
            scope: ScopeName::All,
        },
    );
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::Columns,
        },
    );
    let step = |shown: &mut Library, key| {
        act(
            shown,
            Intent::Step {
                key,
                across: 1,
                extend: false,
            },
        )
    };
    let first = step(&mut library, KeyName::Down);
    assert_eq!(
        first.columns.len(),
        2,
        "the selected folder opens its column"
    );
    assert!(
        entries(&first, 0)
            .first()
            .is_some_and(|entry| entry.selected && entry.opens)
    );
    assert_eq!(
        first.columns.first().and_then(|column| column.selected),
        Some(0)
    );
    let inside = step(&mut library, KeyName::Right);
    assert_eq!(
        inside.preview.as_ref().map(|item| item.title.as_str()),
        Some("A")
    );
    let back = step(&mut library, KeyName::Left);
    assert_eq!(back.preview, None);
    let clicked = act(
        &mut library,
        Intent::Column {
            depth: 1,
            entry: EntryName::Shortcut {
                id: inside.focus.unwrap(),
            },
            place: Some(0),
            extend: Extend::Replace,
        },
    );
    assert_eq!(clicked.preview.map(|item| item.title), Some("A".to_owned()));
    let unsorted = act(
        &mut library,
        Intent::Scope {
            scope: ScopeName::Unsorted,
        },
    );
    assert_eq!(
        unsorted.columns.len(),
        1,
        "a scope without folders lists its shortcuts in one column"
    );
}

#[test]
fn the_order_persists_the_layout_waits_for_the_settings_and_a_bad_intent_becomes_a_notice() {
    let home = tempfile::tempdir().unwrap();
    let mut library = Library::new(Store::new(home.path().to_path_buf()));
    assert_eq!(
        library.view().unwrap().layout,
        LayoutName::Columns,
        "the window opens with the columns at first"
    );
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    act(
        &mut library,
        Intent::Sort {
            sort: SortName::Oldest,
        },
    );
    let again = Library::new(Store::new(home.path().to_path_buf()))
        .view()
        .unwrap();
    assert_eq!(
        (again.layout, again.sort),
        (LayoutName::Columns, SortName::Oldest),
        "the window keeps no layout it showed, only the order a person chose"
    );
    let refused = act(
        &mut library,
        Intent::Select {
            id: "zz".to_owned(),
            extend: Extend::Replace,
            place: None,
        },
    );
    assert!(
        refused
            .notice
            .is_some_and(|notice| notice.error && notice.text.contains("zz"))
    );
    let shown = act(&mut library, Intent::Show);
    assert_eq!(shown.notice, None);
    let nothing = act(&mut library, Intent::Open { id: None });
    assert_eq!(nothing.notice, None, "opening with no focus does nothing");
    youarel_store::fs::replace(
        &home.path().join(youarel_store::settings::SETTINGS_FILE),
        b"{}",
    )
    .unwrap();
    let damaged = Library::new(Store::new(home.path().to_path_buf()))
        .view()
        .unwrap();
    assert!(damaged.notice.is_some_and(|notice| notice.error));
}

#[test]
fn the_window_syncs_only_when_chosen_and_keeps_the_time_of_its_last_success() {
    const NOW: u64 = 1_700_000_000_000;
    let (home, mut library) = library();
    let shared = home.path().join("shared");
    let target = shared.to_str().unwrap().to_owned();
    let refused = act(
        &mut library,
        Intent::SyncSettings {
            enabled: true,
            target: " ".to_owned(),
        },
    );
    assert!(
        refused.notice.is_some_and(|notice| notice.error),
        "a sync needs a target"
    );
    act(
        &mut library,
        Intent::SyncSettings {
            enabled: true,
            target: target.clone(),
        },
    );
    assert_eq!(library.view().unwrap().sync.state, SyncState::NoAccount);
    assert_eq!(library.begin_sync(NOW), None, "no account, no sync");
    let account = home.path().join(youarel_cli::sync::ACCOUNT_DIRECTORY);
    youarel_store::fs::create_dir_all(&account).unwrap();
    youarel_store::fs::replace(&account.join(youarel_sync::account::GENESIS), b"named").unwrap();
    assert_eq!(library.begin_sync(NOW), Some((Sync::Merge, target.clone())));
    assert_eq!(library.view().unwrap().sync.state, SyncState::Syncing);
    library.end_sync(NOW, Err(anyhow::anyhow!("unreachable")));
    let failed = library.view().unwrap().sync;
    assert_eq!((failed.state, failed.last), (SyncState::Failed, None));
    act(&mut library, Intent::SyncNow);
    assert_eq!(
        library.begin_sync(NOW + 1),
        Some((Sync::Merge, target.clone())),
        "a request skips the backoff"
    );
    library.end_sync(NOW + 1, Ok(false));
    let again = Library::new(Store::new(home.path().to_path_buf()))
        .view()
        .unwrap()
        .sync;
    assert_eq!(
        (again.enabled, again.target, again.state, again.last),
        (true, target, SyncState::Synced, Some(NOW + 1)),
        "the choice and the last success persist"
    );
    act(
        &mut library,
        Intent::SyncSettings {
            enabled: false,
            target: String::new(),
        },
    );
    assert_eq!(library.view().unwrap().sync.state, SyncState::Off);
    assert_eq!(library.begin_sync(NOW + 2), None, "off, no sync");
}

/// A library that syncs through `target` for a device that belongs to an account, and whose first sync, a merge that found nothing to publish, began at `now` and succeeded.
fn synced_library(target: &str, now: u64) -> (tempfile::TempDir, Library) {
    let (home, mut library) = library();
    let account = home.path().join(youarel_cli::sync::ACCOUNT_DIRECTORY);
    youarel_store::fs::create_dir_all(&account).unwrap();
    youarel_store::fs::replace(&account.join(youarel_sync::account::GENESIS), b"named").unwrap();
    act(
        &mut library,
        Intent::SyncSettings {
            enabled: true,
            target: target.to_owned(),
        },
    );
    assert_eq!(
        library.begin_sync(now),
        Some((Sync::Merge, target.to_owned()))
    );
    library.end_sync(now, Ok(false));
    (home, library)
}

/// How often the window asks whether a sync is due.
const TICK: u64 = 5 * SECOND;

/// The syncs that begin at the window's ticks from `from` through `until`, each ending as `found` says for it, with the times they began.
///
/// A merge ends with whether the store holds something new to publish, and a publish carries all of it.
fn ticks(
    library: &mut Library,
    (from, until): (u64, u64),
    mut found: impl FnMut(u64) -> bool,
) -> Vec<(u64, Sync)> {
    let mut began = Vec::new();
    let mut now = from;
    while now <= until {
        if let Some((sync, _)) = library.begin_sync(now) {
            began.push((now, sync));
            let pending = sync == Sync::Merge && found(now);
            library.end_sync(now, Ok(pending));
        }
        let Some(next) = now.checked_add(TICK) else {
            break;
        };
        now = next;
    }
    began
}

/// The syncs among `began` that published.
fn publishes(began: &[(u64, Sync)]) -> Vec<u64> {
    began
        .iter()
        .filter(|(_, sync)| *sync == Sync::Publish)
        .map(|(now, _)| *now)
        .collect()
}

/// Twenty minutes, over which an idle window stays quiet.
const IDLE: u64 = 20 * 60 * SECOND;

#[test]
fn an_idle_window_only_reads_and_an_edit_publishes_once() {
    // The window stamps an edit with the clock, so the ticks count from the clock too.
    let now = youarel_cli::clock::now().unwrap();
    let (_home, mut library) = synced_library("/shared", now);
    let idle = ticks(&mut library, (now + TICK, now + IDLE), |_| false);
    assert_eq!(publishes(&idle), Vec::<u64>::new(), "{idle:?}");
    assert_eq!(
        idle.iter().map(|(at, _)| *at).collect::<Vec<_>>(),
        (1..=4)
            .map(|step| now + step * INTERVAL)
            .collect::<Vec<_>>(),
        "a merge every interval"
    );
    add(&mut library, "https://edited.example/", "Edited");
    let added = now + IDLE + TICK;
    let after = ticks(&mut library, (added, added + IDLE), |_| false);
    assert_eq!(publishes(&after), [added], "{after:?}");
    let edited = id_of(&library.view().unwrap(), "Edited");
    act(
        &mut library,
        Intent::Retitle {
            id: edited,
            title: "Retitled".to_owned(),
        },
    );
    let retitled = added + IDLE + TICK;
    let again = ticks(&mut library, (retitled, retitled + IDLE), |_| false);
    assert_eq!(
        publishes(&again),
        [retitled],
        "an intent's edit publishes once too"
    );
    let refused = act(
        &mut library,
        Intent::Retitle {
            id: "00".repeat(16),
            title: "Nothing".to_owned(),
        },
    );
    assert!(refused.notice.is_some_and(|notice| notice.error));
    let after_refusal = retitled + IDLE + TICK;
    let quiet = ticks(&mut library, (after_refusal, after_refusal + IDLE), |_| {
        false
    });
    assert_eq!(
        publishes(&quiet),
        Vec::<u64>::new(),
        "a refused edit wrote nothing to publish"
    );
}

#[test]
fn a_merge_that_brings_another_device_s_change_publishes_nothing() {
    const NOW: u64 = 1_700_000_000_000;
    let (_home, mut library) = synced_library("/shared", NOW);
    library.learned_own(OWN_CHAIN.to_owned());
    assert!(library.noticed(Seen::Touched(OTHER_HEAD), NOW + 1));
    let began = ticks(&mut library, (NOW + TICK, NOW + INTERVAL), |_| false);
    assert_eq!(began, [(NOW + TICK, Sync::Merge)]);
}

#[test]
fn a_missing_bundle_of_its_own_publishes_once() {
    const NOW: u64 = 1_700_000_000_000;
    let (_home, mut library) = synced_library("/shared", NOW);
    let mut missing = true;
    let began = ticks(&mut library, (NOW + TICK, NOW + 3 * INTERVAL), |_| {
        core::mem::take(&mut missing)
    });
    assert_eq!(
        began,
        [
            (NOW + INTERVAL, Sync::Merge),
            (NOW + INTERVAL + TICK, Sync::Publish),
            (NOW + INTERVAL + TICK + INTERVAL, Sync::Merge),
        ]
    );
}

fn record_publish(store: &Store) {
    let now = youarel_cli::clock::now().unwrap();
    store
        .session(now, |session| {
            let Some(pending) = session
                .prepare_publication(youarel_store::PublishRequest::new("/shared", 0), |_body| {
                    [0; 32]
                })?
            else {
                return Err(youarel_store::StoreError::Missing("the test's publication"));
            };
            for sequence in pending.sequences() {
                session.seal_publication_part(&pending, sequence, b"sealed")?;
            }
            session.commit_publication(&pending)
        })
        .unwrap();
}

#[test]
fn a_restart_publishes_once_what_an_earlier_run_left_unpublished() {
    let now = youarel_cli::clock::now().unwrap();
    let (home, mut library) = synced_library("/shared", now);
    add(&mut library, "https://unpublished.example/", "Unpublished");
    drop(library);
    let store = Store::new(home.path().to_path_buf());
    let mut restarted = Library::new(store.clone());
    let began = ticks(&mut restarted, (now + TICK, now + IDLE), |_| false);
    assert_eq!(publishes(&began), [now + TICK], "{began:?}");
    record_publish(&store);
    let mut published = Library::new(store.clone());
    let quiet = ticks(&mut published, (now + TICK, now + IDLE), |_| false);
    assert_eq!(
        quiet.first(),
        Some(&(now + TICK, Sync::Merge)),
        "a restart publishes nothing published already"
    );
    assert_eq!(publishes(&quiet), Vec::<u64>::new());
    add(&mut published, "https://later.example/", "Later");
    drop(published);
    let mut later = Library::new(store);
    let resumed = ticks(&mut later, (now + IDLE, now + IDLE), |_| false);
    assert_eq!(resumed, [(now + IDLE, Sync::Publish)], "a later write");
}

#[test]
fn another_device_s_bundle_brings_a_merge_once_the_folder_rests_and_never_the_device_s_own() {
    const NOW: u64 = 1_700_000_000_000;
    let (_home, mut library) = synced_library("/shared", NOW);
    library.learned_own(OWN_CHAIN.to_owned());
    assert!(!library.noticed(Seen::Touched(OWN_HEAD), NOW + 1));
    assert!(!library.noticed(Seen::Read, NOW + 1));
    assert_eq!(library.begin_sync(NOW + 1 + QUIET), None, "its own write");
    assert!(library.noticed(Seen::Touched(OTHER_HEAD), NOW + 1));
    assert_eq!(
        library.begin_sync(NOW + QUIET),
        None,
        "the folder rests first"
    );
    assert_eq!(
        library.begin_sync(NOW + 1 + QUIET),
        Some((Sync::Merge, "/shared".to_owned()))
    );
    library.end_sync(NOW + 1 + QUIET, Err(anyhow::anyhow!("unreachable")));
    assert!(library.rewatch(), "a failed sync starts the watch again");
    assert!(library.noticed(Seen::Touched(OTHER_HEAD), NOW + 2 + QUIET));
    assert_eq!(
        library.begin_sync(NOW + 2 + 2 * QUIET),
        None,
        "no merge during a backoff"
    );
    assert_eq!(
        library.begin_sync(NOW + 1 + QUIET + BACKOFF),
        Some((Sync::Merge, "/shared".to_owned())),
        "a failed merge tries again as a merge"
    );
}

#[test]
fn the_window_watches_a_shared_folder_and_no_relay() {
    const NOW: u64 = 1_700_000_000_000;
    let (_home, mut library) = synced_library("/shared", NOW);
    assert_eq!(library.watch_wanted(), Some(PathBuf::from("/shared")));
    library.watching(true);
    assert!(library.view().unwrap().sync.watched);
    library.watching(false);
    assert!(!library.view().unwrap().sync.watched);
    act(
        &mut library,
        Intent::SyncSettings {
            enabled: true,
            target: "https://relay.example".to_owned(),
        },
    );
    assert_eq!(library.watch_wanted(), None, "a relay has no folder");
    act(
        &mut library,
        Intent::SyncSettings {
            enabled: false,
            target: "/shared".to_owned(),
        },
    );
    assert_eq!(library.watch_wanted(), None, "no sync, no watch");
}

#[test]
fn themes_are_chosen_by_name_and_a_name_that_is_none_is_refused() {
    let (_home, mut library) = library();
    let first = library.view().unwrap().theme.unwrap();
    assert_eq!(
        (
            first.follow,
            first.light.id.as_str(),
            first.dark.id.as_str()
        ),
        (true, "github-light", "github-dark")
    );
    let chosen = act(
        &mut library,
        Intent::Theme {
            follow: false,
            light: "solarized-light".to_owned(),
            dark: "dracula".to_owned(),
        },
    )
    .theme
    .unwrap();
    assert_eq!(
        (
            chosen.follow,
            chosen.light.name.as_str(),
            chosen.dark.name.as_str()
        ),
        (false, "Solarized Light", "Dracula")
    );
    let unknown = act(
        &mut library,
        Intent::Theme {
            follow: true,
            light: "nowhere".to_owned(),
            dark: "dracula".to_owned(),
        },
    );
    assert!(
        unknown
            .notice
            .is_some_and(|notice| notice.error && notice.text.contains("nowhere"))
    );
}

#[test]
fn an_imported_theme_is_worn_kept_and_removed() {
    let home = tempfile::tempdir().unwrap();
    let mut library = Library::new(Store::new(home.path().to_path_buf()));
    let imported = act(
        &mut library,
        Intent::ImportTheme {
            text: r##"{ "name": "Midnight", "type": "dark", "colors": { "editor.background": "#101010" } }"##.to_owned(),
        },
    );
    assert_eq!(
        imported.notice.map(|notice| notice.text),
        Some("Imported Midnight".to_owned())
    );
    assert_eq!(library.themes().len(), 17);
    act(
        &mut library,
        Intent::Theme {
            follow: true,
            light: "solarized-light".to_owned(),
            dark: "imported-midnight".to_owned(),
        },
    );
    let again = Library::new(Store::new(home.path().to_path_buf()))
        .view()
        .unwrap();
    assert_eq!(
        again.theme.map(|kept| kept.dark.name),
        Some("Midnight".to_owned()),
        "the choice persists"
    );
    let removed = act(
        &mut library,
        Intent::RemoveTheme {
            id: "imported-midnight".to_owned(),
        },
    );
    assert_eq!(
        removed.theme.map(|left| left.dark.id),
        Some("github-dark".to_owned())
    );
    assert_eq!(library.themes().len(), 16);
    let broken = act(
        &mut library,
        Intent::ImportTheme {
            text: "not json".to_owned(),
        },
    );
    assert!(broken.notice.is_some_and(|notice| notice.error));
}

/// More shortcuts than two pages of the window hold.
const MANY: usize = 450;
/// The link the many shortcuts share before their numbers.
const MANY_LINK: &str = "https://many.example/";

/// A library of `count` shortcuts at the top, written in one session, each titled by its number with zeros before it, so their titles sort as their numbers do.
fn many(count: usize) -> (tempfile::TempDir, Library) {
    let home = tempfile::tempdir().unwrap();
    let store = Store::new(home.path().to_path_buf());
    store
        .session(youarel_cli::clock::now().unwrap(), |session| {
            for number in 0..count {
                let id = session.fresh_id()?;
                session.edit(Edit::AddShortcut {
                    id,
                    link: format!("{MANY_LINK}{number}"),
                    title: format!("{number:04}"),
                    folder: None,
                })?;
            }
            Ok(())
        })
        .unwrap();
    (home, Library::new(store))
}

#[test]
fn a_listing_shows_a_page_around_the_place_in_sight_and_pages_on_request() {
    let (_home, mut library) = many(MANY);
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    act(
        &mut library,
        Intent::Sort {
            sort: SortName::Newest,
        },
    );
    library.page(300, 100).unwrap();
    let top = act(
        &mut library,
        Intent::Sort {
            sort: SortName::Title,
        },
    );
    assert_eq!(
        (top.listing.total, top.listing.first, top.listing.rows.len()),
        (MANY, 0, PAGE)
    );
    assert!(top.listing.fresh, "another order starts from the top");
    assert_eq!(titles(&top).first(), Some(&"0000"));
    let page = library.page(300, 100).unwrap();
    assert_eq!(
        (page.key == top.listing.key, page.first, page.rows.len()),
        (true, 300, 100)
    );
    let kept = library.view().unwrap();
    assert_eq!(
        (kept.listing.first, kept.listing.fresh),
        (MANY - PAGE, false),
        "the next view answers around the place in sight, as full as the listing allows"
    );
    let last = act(
        &mut library,
        Intent::Step {
            key: KeyName::Last,
            across: 1,
            extend: false,
        },
    );
    assert_eq!(
        (last.listing.focus, last.listing.reveal),
        (Some(MANY - 1), Some(MANY - 1))
    );
    assert_eq!(titles(&last).last(), Some(&"0449"));
    let first = act(
        &mut library,
        Intent::Step {
            key: KeyName::First,
            across: 1,
            extend: false,
        },
    );
    assert_eq!(
        (first.listing.first, first.listing.reveal),
        (0, Some(0)),
        "a row far from the page in sight brings its own page"
    );
    let quiet = library.view().unwrap();
    assert_eq!(
        (quiet.listing.reveal, quiet.listing.focus),
        (None, Some(0)),
        "a view that answers no step brings nothing into sight"
    );
    assert_eq!(library.page(MANY + 5, 10).unwrap().rows, []);
}

#[test]
fn select_all_names_no_row_and_a_move_to_the_trash_runs_in_batches() {
    let (_home, mut library) = many(MANY);
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::Cards,
        },
    );
    let all = act(&mut library, Intent::SelectAll);
    assert_eq!((all.selection.count, all.selection.folders), (MANY, 0));
    let begun = library
        .begin(Intent::Trash {
            target: TargetName::Selection,
        })
        .unwrap();
    let Begun::Working(job) = begun else {
        panic!("a move to the trash works in batches");
    };
    let mut job = (*job).in_batches_of(PAGE);
    let first = library.advance(&mut job).unwrap();
    assert_eq!(
        (first.done, first.total, first.text.as_str()),
        (PAGE, MANY, "Moving to the Trash 200 of 450")
    );
    let between = library.page(0, 10).unwrap();
    assert_eq!(
        between.total,
        MANY - PAGE,
        "the window reads pages between the batches"
    );
    let mut rest = Vec::new();
    while let Some(progress) = library.advance(&mut job) {
        rest.push(progress.done);
    }
    assert_eq!(rest, [2 * PAGE, MANY]);
    let done = library.finish(&mut job).unwrap();
    assert_eq!(
        done.notice.map(|notice| notice.text),
        Some("Moved 450 shortcuts to the Trash".to_owned())
    );
    assert_eq!(
        (done.count, done.selection.count, done.sidebar.trash),
        (0, 0, MANY)
    );
    let undone = act(&mut library, Intent::Undo);
    assert_eq!((undone.count, undone.sidebar.trash), (MANY, 0));
    assert_eq!(undone.redo.as_deref(), Some("Move to Trash"));
    assert_eq!(undone.undo, None, "all batches form one history step");
}

#[test]
fn a_range_moves_together_and_a_folder_never_moves_into_itself() {
    let (_home, mut library) = sites();
    let archive = collection(&mut library, "Archive", None);
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    let top = act(
        &mut library,
        Intent::Scope {
            scope: ScopeName::All,
        },
    );
    let folders: Vec<String> = folder_rows(&top)
        .into_iter()
        .map(|folder| folder.id.clone())
        .collect();
    assert_eq!(
        folders.first(),
        Some(&archive),
        "Archive, then Work, by name"
    );
    let select = |held: &mut Library, place: usize, extend: Extend| {
        let id = if place < folders.len() {
            folders[place].clone()
        } else {
            id_of(&top, titles(&top)[place - folders.len()])
        };
        act(
            held,
            Intent::Select {
                id,
                extend,
                place: Some(place),
            },
        )
    };
    select(&mut library, 0, Extend::Replace);
    let range = select(&mut library, 4, Extend::Range);
    assert_eq!((range.selection.count, range.selection.folders), (5, 2));
    let file = |held: &mut Library| {
        act(
            held,
            Intent::File {
                target: TargetName::Selection,
                collection: Some(archive.clone()),
            },
        )
    };
    let refused = file(&mut library);
    assert!(
        refused.notice.is_some_and(|notice| notice.error),
        "Archive cannot move into itself"
    );
    assert_eq!(
        library.view().unwrap().listing.total,
        5,
        "a refused move moves nothing"
    );
    select(&mut library, 0, Extend::Toggle);
    let moved = file(&mut library);
    assert_eq!(
        moved.notice.map(|notice| notice.text),
        Some("Moved 1 folder and 3 shortcuts to Archive".to_owned())
    );
    assert_eq!(
        (moved.listing.total, moved.selection.count),
        (1, 0),
        "Archive alone stays at the top, and the range lets go"
    );
}

#[test]
fn a_column_pages_its_entries_and_steps_past_its_first_page() {
    let (_home, mut library) = many(MANY);
    let view = library.view().unwrap();
    let top = view.columns.first().unwrap();
    assert_eq!((top.total, top.first, top.entries.len()), (MANY, 0, PAGE));
    let later = library.column_page(0, 400, 100).unwrap();
    assert_eq!(
        (later.first, later.entries.len(), later.key == top.key),
        (400, MANY - 400, true)
    );
    assert!(library.column_page(3, 0, 10).is_err(), "no such column");
    let step = |held: &mut Library, key: KeyName| {
        act(
            held,
            Intent::Step {
                key,
                across: 1,
                extend: false,
            },
        )
    };
    let entered = step(&mut library, KeyName::Down);
    assert_eq!(
        entered.columns.first().and_then(|first| first.selected),
        Some(0),
        "the first key selects the first entry"
    );
    let last = step(&mut library, KeyName::Last);
    let stepped = last.columns.first().unwrap();
    assert_eq!(
        (stepped.selected, stepped.first),
        (Some(MANY - 1), MANY - PAGE)
    );
    assert!(
        stepped.entries.last().is_some_and(|entry| entry.selected),
        "the page holds the entry the step selected"
    );
    assert_eq!(last.preview.map(|item| item.title), Some("0449".to_owned()));
}

#[test]
fn the_picker_finds_collections_by_name_a_page_at_a_time() {
    let (_home, mut library) = library();
    let work = collection(&mut library, "Work", None);
    collection(&mut library, "Specs", Some(work));
    collection(&mut library, "Life", None);
    assert_eq!(paths(&library), ["Life", "Work", "Work / Specs"]);
    let found = library.choices("spe", 0, MOST_CHOICES).unwrap();
    assert_eq!(
        found.first().map(|choice| choice.path.as_str()),
        Some("Work / Specs")
    );
    let second = library.choices("", 1, 1).unwrap();
    assert_eq!(
        second
            .iter()
            .map(|choice| choice.path.as_str())
            .collect::<Vec<_>>(),
        ["Work"]
    );
    assert_eq!(library.choices("", 0, usize::MAX).unwrap().len(), 3);
    assert_eq!(library.choices("nothing like it", 0, 5).unwrap(), []);
}

#[test]
fn an_import_reports_its_counts() {
    let (_home, mut library) = library();
    let imported = act(
        &mut library,
        Intent::Import {
            bytes: b"<DL><DT><A HREF=\"https://a.example/\">A</A><DT><A HREF=\"javascript:x\">x</A></DL>".to_vec(),
        },
    );
    assert_eq!(
        imported.notice.map(|notice| notice.text),
        Some(
            "imported 1 shortcuts and 0 folders; 0 were already here, and 1 were no web links"
                .to_owned()
        )
    );
}

/// The launcher's own icon, which stands for any site's.
const PNG: &[u8] = include_bytes!("../../icons/32x32.png");

#[test]
fn an_added_link_s_icon_shows_on_every_shortcut_of_its_host() {
    let (_home, mut library) = library();
    library
        .add(Addition {
            icon: Some((IconId([7; 32]), PNG.to_vec())),
            ..addition("https://a.example/one", "One", None)
        })
        .unwrap();
    add(&mut library, "https://a.example/two", "Two");
    let columns = add(&mut library, "https://b.example/", "Other");
    let mut tiles: Vec<(&str, Option<String>)> = entries(&columns, 0)
        .into_iter()
        .map(|entry| (entry.name.as_str(), entry.icon.clone()))
        .collect();
    tiles.sort();
    let view = act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    let mut names: Vec<(&str, Option<String>)> = items(&view)
        .into_iter()
        .map(|item| (item.title.as_str(), item.icon.clone()))
        .collect();
    names.sort();
    let name = IconId([7; 32]).name();
    assert_eq!(
        names,
        [
            ("One", Some(name.clone())),
            ("Other", None),
            ("Two", Some(name))
        ],
        "one icon for the host, and a tile for a host without one"
    );
    assert_eq!(tiles, names, "the columns show the same icons");
}

#[test]
fn an_opened_link_asks_its_site_for_an_icon_only_while_its_host_shows_none() {
    let (_home, mut library) = library();
    add(&mut library, "https://a.example/", "A");
    library.want_icon("https://a.example/page").unwrap();
    let wanted = library.icons_wanted();
    assert_eq!(wanted, [Link::parse("https://a.example/page").unwrap()]);
    assert_eq!(library.icons_wanted(), [], "each link is handed over once");
    library
        .keep_icon(&wanted[0], (IconId([7; 32]), PNG.to_vec()))
        .unwrap();
    library.want_icon("https://a.example/other").unwrap();
    assert_eq!(
        library.icons_wanted(),
        [],
        "a host that shows an icon asks its site nothing"
    );
    let view = act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::Cards,
        },
    );
    assert_eq!(
        items(&view)
            .into_iter()
            .filter(|item| item.icon.is_some())
            .count(),
        1
    );
    assert!(library.want_icon("no link").is_err());
}

#[test]
fn icons_draw_again_by_name_and_an_image_that_does_not_draw_shows_none() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::new(home.path().to_path_buf());
    store
        .session(1, |session| {
            for (link, host, image, name) in [
                ("https://drawn.example/", "drawn.example", PNG, 1),
                (
                    "https://broken.example/",
                    "broken.example",
                    &b"not an image"[..],
                    2,
                ),
            ] {
                let id = session.fresh_id()?;
                session.edit(Edit::AddShortcut {
                    id,
                    link: link.to_owned(),
                    title: link.to_owned(),
                    folder: None,
                })?;
                session.keep_icon(host, IconId([name; 32]), image)?;
            }
            Ok(())
        })
        .unwrap();
    let mut library = Library::new(store);
    let view = act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    let mut names: Vec<String> = items(&view)
        .into_iter()
        .filter_map(|item| item.icon.clone())
        .collect();
    assert_eq!(names.len(), 2);
    let drawn = library.icons(&names).unwrap();
    assert_eq!(drawn.len(), 1, "the broken image draws nothing");
    assert!(
        drawn
            .values()
            .all(|address| address.starts_with("data:image/png;base64,"))
    );
    assert_eq!(
        library.icons(&names).unwrap(),
        drawn,
        "a drawn icon stays drawn"
    );
    names.push(IconId([3; 32]).name());
    assert_eq!(
        library.icons(&names).unwrap(),
        drawn,
        "an image the device does not keep draws nothing"
    );
    assert!(library.icons(&["zz".to_owned()]).is_err());
}

#[test]
fn the_window_opens_where_and_as_the_settings_say_and_nowhere_it_was() {
    let (home, mut library) = library();
    let work = collection(&mut library, "Work", None);
    act(
        &mut library,
        Intent::Opening {
            layout: LayoutName::Cards,
            start: Some(work.clone()),
        },
    );
    act(
        &mut library,
        Intent::Scope {
            scope: ScopeName::Recent,
        },
    );
    let reopen = || {
        Library::new(Store::new(home.path().to_path_buf()))
            .view()
            .unwrap()
    };
    let opened = reopen();
    assert_eq!(
        (
            opened.layout,
            opened.title.as_str(),
            opened.opening.layout,
            opened.opening.start.map(|start| start.path)
        ),
        (
            LayoutName::Cards,
            "Work",
            LayoutName::Cards,
            Some("Work".to_owned())
        ),
        "the window opens at Work as cards, wherever it stood"
    );
    trash(&mut library, &[&work]);
    let fallen = reopen();
    assert_eq!(
        (fallen.title.as_str(), fallen.opening.start),
        ("All", None),
        "a place in the trash gives way to every shortcut"
    );
    act(
        &mut library,
        Intent::Opening {
            layout: LayoutName::List,
            start: None,
        },
    );
    assert_eq!(
        (reopen().layout, reopen().title),
        (LayoutName::List, "All".to_owned())
    );
}

#[test]
fn every_listing_orders_itself_with_folders_by_name_first() {
    let (_home, mut library) = library();
    for name in ["b", "A", "c"] {
        collection(&mut library, name, None);
    }
    for (link, title) in [
        ("https://b.example/", "z"),
        ("https://a.example/", "Y"),
        ("https://c.example/", "x"),
    ] {
        add(&mut library, link, title);
    }
    let names = |view: &View| -> (Vec<String>, Vec<String>) {
        (
            folder_rows(view)
                .into_iter()
                .map(|folder| folder.name.clone())
                .collect(),
            titles(view).into_iter().map(str::to_owned).collect(),
        )
    };
    for layout in [LayoutName::Cards, LayoutName::List] {
        act(&mut library, Intent::Layout { layout });
        let top = show(&mut library, ScopeName::All);
        assert_eq!(
            names(&top),
            (
                vec!["A".to_owned(), "b".to_owned(), "c".to_owned()],
                vec!["x".to_owned(), "Y".to_owned(), "z".to_owned()]
            ),
            "{layout:?} sorts by name without the case"
        );
    }
    let columns = act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::Columns,
        },
    );
    assert_eq!(
        entries(&columns, 0)
            .into_iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["A", "b", "c", "x", "Y", "z"]
    );
    let by_link = act(
        &mut library,
        Intent::Sort {
            sort: SortName::Link,
        },
    );
    assert_eq!(
        entries(&by_link, 0)
            .into_iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["A", "b", "c", "Y", "z", "x"],
        "the shortcuts take the order chosen, and the folders keep their names'"
    );
}

/// Shows a place the window names.
fn show(library: &mut Library, scope: ScopeName) -> View {
    act(library, Intent::Scope { scope })
}

/// The identifier of the shortcut that holds a link in a collection, or at the top.
fn holder_of(library: &Library, link: &str, collection: Option<&str>) -> String {
    let id = library
        .holder(&Link::parse(link).unwrap(), collection.map(item_of))
        .unwrap()
        .unwrap();
    crate::intent::text(id)
}

/// The links of the trash tests: A in Work, B in Docs inside Work, and C at the top.
const LINK_A: &str = "https://a.example/";
const LINK_B: &str = "https://b.example/";
const LINK_C: &str = "https://c.example/";

/// Work holds A and Docs, Docs holds B, and C sits at the top, shown in the list; answers Work and Docs.
fn filed() -> (tempfile::TempDir, Library, String, String) {
    let (home, mut library) = library();
    let work = collection(&mut library, "Work", None);
    let docs = collection(&mut library, "Docs", Some(work.clone()));
    for (link, title, folder) in [
        (LINK_A, "A", Some(&work)),
        (LINK_B, "B", Some(&docs)),
        (LINK_C, "C", None),
    ] {
        library
            .add(addition(link, title, folder.map(|id| item_of(id))))
            .unwrap();
    }
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    (home, library, work, docs)
}

/// Moves shortcuts and folders to the trash by their identifiers.
fn trash(library: &mut Library, ids: &[&String]) -> View {
    act(
        library,
        Intent::Trash {
            target: TargetName::Ids {
                ids: ids.iter().map(|id| (*id).clone()).collect(),
            },
        },
    )
}

#[test]
fn the_trash_lists_what_went_there_with_the_folder_it_came_from() {
    let (_home, mut library, work, docs) = filed();
    let a = holder_of(&library, LINK_A, Some(&work));
    trash(&mut library, &[&a, &docs]);
    let all = show(&mut library, ScopeName::All);
    assert_eq!(
        (all.count, all.sidebar.trash),
        (1, 2),
        "C alone shows, and the trash holds A and Docs"
    );
    let bin = show(&mut library, ScopeName::Trash);
    assert_eq!((bin.title.as_str(), bin.listing.total), ("Trash", 2));
    assert!(
        matches!(
            bin.listing.rows.first(),
            Some(RowView::Folder { folder, origin, .. })
                if folder.name == "Docs" && folder.count == 1 && origin.as_deref() == Some("Work")
        ),
        "Docs came from Work and hides B"
    );
    assert_eq!(
        items(&bin)
            .first()
            .and_then(|item| item.collection.as_deref()),
        Some("Work")
    );
    let refused = act(
        &mut library,
        Intent::Open {
            id: Some(a.clone()),
        },
    );
    assert_eq!(
        refused.notice.map(|notice| (notice.text, notice.error)),
        Some(("A is in the Trash; put it back to open it".to_owned(), true))
    );
    let found = act(
        &mut library,
        Intent::Search {
            text: "doc".to_owned(),
        },
    );
    assert_eq!(
        (folder_rows(&found).len(), titles(&found)),
        (1, Vec::<&str>::new()),
        "a search in the trash finds what waits there"
    );
    let none = act(
        &mut library,
        Intent::Search {
            text: "zzz".to_owned(),
        },
    );
    assert_eq!(none.listing.total, 0);
    act(
        &mut library,
        Intent::Search {
            text: String::new(),
        },
    );
    let renamed = act(
        &mut library,
        Intent::Retitle {
            id: a,
            title: "Alpha".to_owned(),
        },
    );
    assert!(
        renamed.notice.is_some_and(|notice| notice.error),
        "what waits in the trash takes no edit"
    );
    assert!(renamed.here.is_none(), "nothing new goes into the trash");
}

#[test]
fn putting_back_shows_a_folder_and_everything_it_hid_again() {
    let (_home, mut library, work, docs) = filed();
    trash(&mut library, &[&docs]);
    let hidden = show(&mut library, ScopeName::Collection { id: work.clone() });
    assert_eq!(hidden.count, 1, "Work holds A alone once Docs went");
    show(&mut library, ScopeName::Trash);
    let picked = act(
        &mut library,
        Intent::Select {
            id: docs,
            extend: Extend::Replace,
            place: Some(0),
        },
    );
    assert_eq!(picked.selection.count, 1);
    let back = act(
        &mut library,
        Intent::PutBack {
            target: TargetName::Selection,
        },
    );
    assert_eq!((back.listing.total, back.sidebar.trash), (0, 0));
    assert_eq!(
        back.notice.map(|notice| notice.text),
        Some("Put back 1 folder".to_owned())
    );
    let shown = show(&mut library, ScopeName::Collection { id: work });
    assert_eq!(
        (shown.count, folder_names(&shown)),
        (2, vec![("Docs", 1)]),
        "Docs and B come back where they were"
    );
}

#[test]
fn emptying_the_trash_deletes_what_waits_there_and_spares_what_came_back() {
    let (_home, mut library, work, docs) = filed();
    let a = holder_of(&library, LINK_A, Some(&work));
    let c = holder_of(&library, LINK_C, None);
    trash(&mut library, &[&docs, &a, &c]);
    let Begun::Working(job) = library.begin(Intent::EmptyTrash).unwrap() else {
        panic!("emptying the trash works in batches");
    };
    act(
        &mut library,
        Intent::PutBack {
            target: TargetName::Ids { ids: vec![c] },
        },
    );
    let mut job = *job;
    while library.advance(&mut job).is_some() {}
    let emptied = library.finish(&mut job).unwrap();
    assert_eq!(
        (
            emptied.notice.map(|notice| notice.text),
            emptied.sidebar.trash
        ),
        (Some("Emptied the Trash".to_owned()), 0)
    );
    let all = show(&mut library, ScopeName::All);
    assert_eq!(
        titles(&all),
        ["C"],
        "C came back before its batch ran, and A and B went for good"
    );
    assert_eq!(paths(&library), ["Work"]);
    let gone = Link::parse(LINK_B).unwrap();
    assert!(
        library
            .session(|session| session.holds_link(gone.as_str()))
            .is_ok_and(|held| !held)
    );
}

#[test]
fn deleting_immediately_takes_an_item_for_good_from_anywhere() {
    let (_home, mut library, _work, _docs) = filed();
    let all = show(&mut library, ScopeName::All);
    let deleted = act(
        &mut library,
        Intent::Remove {
            target: TargetName::Ids {
                ids: vec![id_of(&all, "C")],
            },
        },
    );
    assert_eq!(
        (deleted.count, deleted.sidebar.trash),
        (2, 0),
        "A and B stay inside Work, and nothing waits in the trash"
    );
    assert_eq!(
        deleted.notice.map(|notice| notice.text),
        Some("Deleted 1 shortcut".to_owned())
    );
}

#[test]
fn a_folder_in_the_trash_opens_no_column() {
    let (_home, mut library, _work, docs) = filed();
    trash(&mut library, &[&docs]);
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::Columns,
        },
    );
    let bin = show(&mut library, ScopeName::Trash);
    assert!(
        entries(&bin, 0)
            .first()
            .is_some_and(|entry| entry.name == "Docs" && !entry.opens)
    );
    let picked = act(
        &mut library,
        Intent::Column {
            depth: 0,
            entry: EntryName::Folder { id: docs },
            place: Some(0),
            extend: Extend::Replace,
        },
    );
    assert_eq!(picked.columns.len(), 1, "Docs opens once it is put back");
    assert!(picked.here.is_none());
}

fn request(library: &mut Library, value: serde_json::Value) -> View {
    act(library, serde_json::from_value(value).unwrap())
}

#[test]
fn back_and_forward_walk_places_and_a_new_visit_drops_the_places_ahead() {
    let (_home, mut library) = library();
    let first = collection(&mut library, "First", None);
    let second = collection(&mut library, "Second", None);
    show(&mut library, ScopeName::All);
    show(&mut library, ScopeName::Collection { id: first.clone() });
    show(&mut library, ScopeName::Collection { id: second });
    let back = request(&mut library, serde_json::json!({"kind": "back"}));
    assert_eq!(
        back.sidebar.scope,
        ScopeName::Collection { id: first.clone() }
    );
    let at_top = request(&mut library, serde_json::json!({"kind": "back"}));
    assert_eq!(at_top.sidebar.scope, ScopeName::All);
    let forward = request(&mut library, serde_json::json!({"kind": "forward"}));
    assert_eq!(forward.sidebar.scope, ScopeName::Collection { id: first });
    show(&mut library, ScopeName::Trash);
    let stayed = request(&mut library, serde_json::json!({"kind": "forward"}));
    assert_eq!(stayed.sidebar.scope, ScopeName::Trash);
}

#[test]
fn show_in_folder_clears_a_search_and_reveals_its_hit() {
    let (_home, mut library, work, child) = filed();
    let id = holder_of(&library, LINK_B, Some(&child));
    for layout in [LayoutName::Cards, LayoutName::List, LayoutName::Columns] {
        act(&mut library, Intent::Layout { layout });
        show(&mut library, ScopeName::Recent);
        act(
            &mut library,
            Intent::Search {
                text: "a".to_owned(),
            },
        );
        let revealed = request(
            &mut library,
            serde_json::json!({"kind": "showInFolder", "id": id}),
        );
        assert_eq!(
            revealed.sidebar.scope,
            ScopeName::Collection { id: child.clone() }
        );
        assert_eq!(revealed.search, "");
        assert_eq!(revealed.focus.as_deref(), Some(id.as_str()));
        assert_eq!(revealed.selection.count, 1);
        if layout == LayoutName::Columns {
            assert_eq!(
                revealed.preview.as_ref().map(|item| item.id.as_str()),
                Some(id.as_str())
            );
        } else {
            assert!(revealed.trail.iter().any(|step| step.id == work));
        }
    }
}

#[test]
fn shift_arrows_extend_and_shrink_from_the_same_anchor_across_a_page() {
    let (_home, mut library) = many(MANY);
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    let page = library.page(PAGE - 1, 1).unwrap();
    let RowView::Shortcut { item, .. } = page.rows.first().unwrap() else {
        panic!("a shortcut")
    };
    act(
        &mut library,
        Intent::Select {
            id: item.id.clone(),
            extend: Extend::Replace,
            place: Some(PAGE - 1),
        },
    );
    for count in [2, 3] {
        let extended = request(
            &mut library,
            serde_json::json!({"kind": "step", "key": "down", "across": 1, "extend": true}),
        );
        assert_eq!(extended.selection.count, count);
    }
    let shrunk = request(
        &mut library,
        serde_json::json!({"kind": "step", "key": "up", "across": 1, "extend": true}),
    );
    assert_eq!(shrunk.selection.count, 2);
    let moved = request(
        &mut library,
        serde_json::json!({"kind": "step", "key": "down", "across": 1}),
    );
    assert_eq!(moved.selection.count, 1);
}

#[test]
fn deep_disclosure_opens_and_closes_every_folder_below_one_row() {
    let (_home, mut library) = library();
    let outer = collection(&mut library, "Outer", None);
    let inner = collection(&mut library, "Inner", Some(outer.clone()));
    collection(&mut library, "Deep", Some(inner));
    show(&mut library, ScopeName::All);
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    let opened = request(
        &mut library,
        serde_json::json!({"kind": "disclose", "id": outer, "open": true, "deep": true}),
    );
    assert_eq!(folder_rows(&opened).len(), 3);
    let closed = request(
        &mut library,
        serde_json::json!({"kind": "disclose", "id": outer, "open": false, "deep": true}),
    );
    assert_eq!(folder_rows(&closed).len(), 1);
}

#[test]
fn undo_and_redo_restore_the_fields_of_an_edit_and_new_edits_drop_redo() {
    let (_home, mut library) = library();
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    let added = add(&mut library, LINK_A, "Before");
    let id = id_of(&added, "Before");
    act(
        &mut library,
        Intent::Edit {
            fields: Fields {
                id: id.clone(),
                title: "After".to_owned(),
                alias: "new-alias".to_owned(),
                note: "new note".to_owned(),
                collection: None,
            },
        },
    );
    let undone = request(&mut library, serde_json::json!({"kind": "undo"}));
    let restored = items(&undone)
        .into_iter()
        .find(|item| item.id == id)
        .unwrap();
    assert_eq!(
        (
            restored.title.as_str(),
            restored.alias.as_str(),
            restored.note.as_str()
        ),
        ("Before", "", "")
    );
    let redone = request(&mut library, serde_json::json!({"kind": "redo"}));
    let changed = items(&redone)
        .into_iter()
        .find(|item| item.id == id)
        .unwrap();
    assert_eq!(
        (
            changed.title.as_str(),
            changed.alias.as_str(),
            changed.note.as_str()
        ),
        ("After", "new-alias", "new note")
    );
    request(&mut library, serde_json::json!({"kind": "undo"}));
    act(
        &mut library,
        Intent::Retitle {
            id,
            title: "New branch".to_owned(),
        },
    );
    let stayed = request(&mut library, serde_json::json!({"kind": "redo"}));
    assert_eq!(titles(&stayed), ["New branch"]);
}

#[test]
fn undo_a_move_restores_the_home_and_position_with_new_stamps() {
    let (_home, mut library, work, child) = filed();
    let id = holder_of(&library, LINK_A, Some(&work));
    let before = library
        .session(|session| session.item(item_of(&id)))
        .unwrap()
        .unwrap();
    act(
        &mut library,
        Intent::File {
            target: TargetName::Ids {
                ids: vec![id.clone()],
            },
            collection: Some(child.clone()),
        },
    );
    request(&mut library, serde_json::json!({"kind": "undo"}));
    let restored = library
        .session(|session| session.item(item_of(&id)))
        .unwrap()
        .unwrap();
    assert_eq!(restored.folder.value, before.folder.value);
    assert_eq!(restored.position.value, before.position.value);
    assert!(restored.folder.stamp > before.folder.stamp);
    request(&mut library, serde_json::json!({"kind": "redo"}));
    assert_eq!(holder_of(&library, LINK_A, Some(&child)), id);
}

#[test]
fn an_added_item_undoes_to_the_trash_and_redoes_with_the_same_identifier() {
    let (_home, mut library) = library();
    let added = add(&mut library, LINK_A, "Added");
    let id = id_of(&added, "Added");
    let undone = request(&mut library, serde_json::json!({"kind": "undo"}));
    assert_eq!((undone.count, undone.sidebar.trash), (0, 1));
    let redone = request(&mut library, serde_json::json!({"kind": "redo"}));
    assert_eq!(id_of(&redone, "Added"), id);
    assert_eq!(redone.sidebar.trash, 0);
}

#[test]
fn collection_edits_undo_one_field_at_a_time_and_an_addition_keeps_its_identifier() {
    let (_home, mut library) = library();
    let id = collection(&mut library, "Before", None);
    act(
        &mut library,
        Intent::RenameCollection {
            id: id.clone(),
            name: "After".to_owned(),
        },
    );
    act(
        &mut library,
        Intent::MarkCollection {
            id: id.clone(),
            mark: ":star:".to_owned(),
        },
    );
    act(&mut library, Intent::Undo);
    let changed = library
        .session(|session| Ok(session.folders().folder(item_of(&id)).cloned()))
        .unwrap()
        .unwrap();
    assert_eq!(
        (changed.name.value.as_str(), changed.mark.value.as_str()),
        ("After", "")
    );
    act(&mut library, Intent::Undo);
    let renamed = library
        .session(|session| Ok(session.folders().folder(item_of(&id)).cloned()))
        .unwrap()
        .unwrap();
    assert_eq!(renamed.name.value, "Before");
    let trashed = act(&mut library, Intent::Undo);
    assert_eq!(trashed.sidebar.trash, 1);
    let restored = act(&mut library, Intent::Redo);
    assert_eq!(restored.sidebar.trash, 0);
    assert_eq!(paths(&library), ["Before"]);
    assert!(
        library
            .session(|session| Ok(session.folders().holds(item_of(&id))))
            .unwrap()
    );
}

#[test]
fn a_failed_undo_keeps_its_step_and_overrides_a_later_write_when_retried() {
    let (_home, mut library) = library();
    let added = add(&mut library, LINK_A, "Before");
    let id = id_of(&added, "Before");
    let entry = youarel_core::state::Entry::Shortcut(item_of(&id));
    act(
        &mut library,
        Intent::Retitle {
            id: id.clone(),
            title: "Local".to_owned(),
        },
    );
    library
        .session(|session| session.edit(Edit::Trash { entry }))
        .unwrap();
    let refused = act(&mut library, Intent::Undo);
    assert!(refused.notice.is_some_and(|notice| notice.error));
    assert_eq!(refused.undo.as_deref(), Some("Rename"));
    assert_eq!(refused.redo, None);
    library
        .session(|session| {
            session.edit(Edit::PutBack { entry })?;
            session.edit(Edit::Retitle {
                id: item_of(&id),
                title: "Later".to_owned(),
            })
        })
        .unwrap();
    act(&mut library, Intent::Undo);
    let restored = library
        .session(|session| session.item(item_of(&id)))
        .unwrap()
        .unwrap();
    assert_eq!(restored.title.value, "Before");
    act(&mut library, Intent::Redo);
    assert_eq!(titles(&library.view().unwrap()), ["Local"]);
}

#[test]
fn putting_back_can_be_undone_and_a_permanent_deletion_clears_history() {
    let (_home, mut library) = library();
    let added = add(&mut library, LINK_A, "A");
    let id = id_of(&added, "A");
    trash(&mut library, &[&id]);
    act(
        &mut library,
        Intent::PutBack {
            target: TargetName::Ids {
                ids: vec![id.clone()],
            },
        },
    );
    let back_in_trash = act(&mut library, Intent::Undo);
    assert_eq!((back_in_trash.count, back_in_trash.sidebar.trash), (0, 1));
    let live = act(&mut library, Intent::Redo);
    assert_eq!((live.count, live.sidebar.trash), (1, 0));
    let gone = act(
        &mut library,
        Intent::Remove {
            target: TargetName::Ids { ids: vec![id] },
        },
    );
    assert_eq!((gone.count, gone.undo, gone.redo), (0, None, None));
    assert_eq!(act(&mut library, Intent::Undo).count, 0);
}

#[test]
fn cut_moves_the_original_selection_into_the_folder_shown_and_undo_restores_it() {
    let (_home, mut library, work, child) = filed();
    let id = holder_of(&library, LINK_A, Some(&work));
    act(
        &mut library,
        Intent::Select {
            id: id.clone(),
            extend: Extend::Replace,
            place: None,
        },
    );
    let cutting = request(&mut library, serde_json::json!({"kind": "cut"}));
    assert_eq!(cutting.cut, 1);
    show(&mut library, ScopeName::Collection { id: child.clone() });
    let pasted = request(
        &mut library,
        serde_json::json!({"kind": "paste", "text": "https://ignored.example/"}),
    );
    assert_eq!(pasted.cut, 0);
    assert_eq!(holder_of(&library, LINK_A, Some(&child)), id);
    assert!(
        library
            .holder(
                &Link::parse("https://ignored.example/").unwrap(),
                Some(item_of(&child))
            )
            .unwrap()
            .is_none()
    );
    act(&mut library, Intent::Undo);
    assert_eq!(holder_of(&library, LINK_A, Some(&work)), id);
}

#[test]
fn pasted_text_admits_web_links_reports_duplicates_and_undoes_as_one_step() {
    let (_home, mut library) = library();
    let pasted = request(
        &mut library,
        serde_json::json!({"kind": "paste", "text": "https://a.example/\nhttps://b.example/ https://a.example/ javascript:alert(1)"}),
    );
    assert_eq!(pasted.count, 2);
    assert_eq!(
        pasted.notice.unwrap().text,
        "Added 2 shortcuts; 1 already here; 1 ignored"
    );
    let undone = act(&mut library, Intent::Undo);
    assert_eq!((undone.count, undone.sidebar.trash), (0, 2));
    assert_eq!(act(&mut library, Intent::Redo).count, 2);
}

#[test]
fn letters_select_a_name_in_every_layout_without_changing_the_search() {
    let (_home, mut library) = library();
    let abacus = id_of(&add(&mut library, LINK_A, "Abacus"), "Abacus");
    let alpha = id_of(&add(&mut library, LINK_B, "Alpha"), "Alpha");
    let zed = id_of(&add(&mut library, LINK_C, "Zed"), "Zed");
    for layout in [LayoutName::Cards, LayoutName::List, LayoutName::Columns] {
        act(&mut library, Intent::Layout { layout });
        show(&mut library, ScopeName::All);
        assert_eq!(
            act(&mut library, Intent::Type { key: 'a' }).focus,
            Some(abacus.clone())
        );
        let joined = act(&mut library, Intent::Type { key: 'l' });
        assert_eq!(
            (joined.focus, joined.search),
            (Some(alpha.clone()), String::new())
        );
        library.typed.as_mut().unwrap().at = 0;
        assert_eq!(
            act(&mut library, Intent::Type { key: 'z' }).focus,
            Some(zed.clone())
        );
        assert_eq!(
            act(&mut library, Intent::Type { key: 'x' }).focus,
            Some(zed.clone())
        );
    }
}

#[test]
fn a_column_selects_folders_and_extends_or_selects_all_its_rows() {
    let (_home, mut library) = library();
    let folder = collection(&mut library, "Folder", None);
    add(&mut library, LINK_A, "A");
    add(&mut library, LINK_B, "B");
    show(&mut library, ScopeName::All);
    let selected = move_focus(&mut library, KeyName::First, 1);
    assert_eq!(
        (
            selected.focus.as_deref(),
            selected.selection.count,
            selected.selection.folders
        ),
        (Some(folder.as_str()), 1, 1)
    );
    let extended = act(
        &mut library,
        Intent::Step {
            key: KeyName::Down,
            across: 1,
            extend: true,
        },
    );
    assert_eq!(
        (extended.selection.count, extended.selection.folders),
        (2, 1)
    );
    assert_eq!(
        entries(&extended, 0)
            .iter()
            .filter(|entry| entry.selected)
            .count(),
        2
    );
    let all = act(&mut library, Intent::SelectAll);
    assert_eq!((all.selection.count, all.selection.folders), (3, 1));
    assert_eq!(act(&mut library, Intent::Cut).cut, 3);
    let cancelled = act(&mut library, Intent::Cancel);
    assert_eq!((cancelled.cut, cancelled.selection.count), (0, 0));
}

#[test]
fn cut_marks_paged_rows_and_a_refused_paste_keeps_the_cut_selection() {
    let (_home, mut library) = many(MANY);
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    act(&mut library, Intent::SelectAll);
    act(&mut library, Intent::Cut);
    let later = library.page(PAGE, PAGE).unwrap();
    assert!(
        later
            .rows
            .iter()
            .all(|row| matches!(row, RowView::Shortcut { cut: true, .. }))
    );
    show(&mut library, ScopeName::Trash);
    let refused = act(
        &mut library,
        Intent::Paste {
            text: String::new(),
        },
    );
    assert!(refused.notice.is_some_and(|notice| notice.error));
    assert_eq!(refused.cut, MANY);
    act(&mut library, Intent::Cancel);
    show(&mut library, ScopeName::All);
    let kept = library.page(PAGE, PAGE).unwrap();
    assert!(
        kept.rows
            .iter()
            .all(|row| matches!(row, RowView::Shortcut { cut: false, .. }))
    );
}

#[test]
fn opening_a_multiple_selection_opens_its_links_and_counts_only_successful_visits() {
    let (_home, mut library, work, _child) = filed();
    act(&mut library, Intent::SelectAll);
    let opened = std::cell::RefCell::new(Vec::new());
    let notice = library
        .open_with(None, &|link| {
            opened.borrow_mut().push(link.to_owned());
            Ok(())
        })
        .unwrap();
    assert_eq!(notice.as_deref(), Some("Opened 1 shortcut"));
    assert_eq!(*opened.borrow(), [LINK_C]);
    let a = holder_of(&library, LINK_A, Some(&work));
    let c = holder_of(&library, LINK_C, None);
    library.selection = youarel_core::selection_rules::Selection::of([item_of(&a), item_of(&c)]);
    let failed = library
        .open_with(None, &|link| {
            if link == LINK_C {
                anyhow::bail!("opener refused")
            }
            Ok(())
        })
        .unwrap_err();
    assert!(format!("{failed:#}").contains("opener refused"));
    let visits = library
        .session(|session| {
            Ok((
                session.visits_of(item_of(&a))?,
                session.visits_of(item_of(&c))?,
            ))
        })
        .unwrap();
    assert_eq!(visits.0.unwrap().count, 1);
    assert_eq!(visits.1.unwrap().count, 1, "the failed open adds no visit");
}

#[test]
fn every_opener_receives_only_an_admitted_web_link() {
    let (_home, mut library) = library();
    let id = library
        .session(|session| {
            let id = session.fresh_id()?;
            session.edit(Edit::AddShortcut {
                id,
                link: "javascript:alert(1)".to_owned(),
                title: "Invalid".to_owned(),
                folder: None,
            })?;
            Ok(id)
        })
        .unwrap();
    let reached = std::cell::Cell::new(false);
    assert!(
        library
            .open_with(Some(id), &|_link| {
                reached.set(true);
                Ok(())
            })
            .is_err()
    );
    assert!(!reached.get());
    assert_eq!(
        library.session(|session| session.visits_of(id)).unwrap(),
        None
    );
}

#[test]
fn an_export_is_a_bookmark_file_of_the_live_library() {
    let (_home, mut library, _work, child) = filed();
    trash(&mut library, &[&child]);
    let directory = tempfile::tempdir().unwrap();
    let exported = library.export_to(directory.path()).unwrap();
    let files = youarel_store::fs::list(directory.path()).unwrap();
    assert_eq!(files.len(), 1);
    let path = directory.path().join(files.first().unwrap());
    assert_eq!(
        exported.notice.unwrap().text,
        format!("Exported {}", path.display())
    );
    let imported_home = tempfile::tempdir().unwrap();
    let store = Store::new(imported_home.path().to_path_buf());
    youarel_cli::commands::import(
        &store,
        youarel_cli::clock::now().unwrap(),
        &youarel_store::fs::read_bytes_if_present(&path)
            .unwrap()
            .unwrap(),
        None,
    )
    .unwrap();
    let mut imported = Library::new(store);
    assert_eq!(imported.view().unwrap().count, 2);
    assert_eq!(paths(&imported), ["Work"]);
}

#[test]
fn a_drag_keeps_its_entries_when_spring_loading_changes_the_selection() {
    let (_home, mut library, work, child) = filed();
    let a = holder_of(&library, LINK_A, Some(&work));
    let c = holder_of(&library, LINK_C, None);
    library.selection = youarel_core::selection_rules::Selection::of([item_of(&a), item_of(&c)]);
    let drag = library.prepare_drag(&TargetName::Selection).unwrap();
    assert_eq!(drag.count, 2);
    let mut links: Vec<&str> = drag.links.lines().collect();
    links.sort_unstable();
    assert_eq!(links, [LINK_A, LINK_C]);
    let token = item_of(&drag.token);
    assert_eq!(
        library.hover(token, Some(item_of(&child))).unwrap(),
        Some(600)
    );
    library.spring_after(token, item_of(&child), 599).unwrap();
    assert_ne!(
        library.view().unwrap().sidebar.scope,
        ScopeName::Collection { id: child.clone() }
    );
    library.spring_after(token, item_of(&child), 600).unwrap();
    let opened = library.view().unwrap();
    assert_eq!(
        opened.sidebar.scope,
        ScopeName::Collection { id: child.clone() }
    );
    assert_eq!(opened.selection.count, 0);
    act(
        &mut library,
        Intent::File {
            target: TargetName::Drag {
                token: drag.token.clone(),
            },
            collection: Some(child.clone()),
        },
    );
    assert_eq!(holder_of(&library, LINK_A, Some(&child)), a);
    assert_eq!(holder_of(&library, LINK_C, Some(&child)), c);
    act(
        &mut library,
        Intent::EndDrag {
            token: drag.token.clone(),
        },
    );
    let refused = act(
        &mut library,
        Intent::File {
            target: TargetName::Drag { token: drag.token },
            collection: None,
        },
    );
    assert!(refused.notice.is_some_and(|notice| notice.error));
}

#[test]
fn cancelled_and_stale_hover_completions_never_open_a_folder() {
    let (_home, mut library, _work, child) = filed();
    let c = holder_of(&library, LINK_C, None);
    let target = TargetName::Ids { ids: vec![c] };
    let first = library.prepare_drag(&target).unwrap();
    library
        .hover(item_of(&first.token), Some(item_of(&child)))
        .unwrap();
    library.hover(item_of(&first.token), None).unwrap();
    library
        .spring_after(item_of(&first.token), item_of(&child), u64::MAX)
        .unwrap();
    assert_eq!(library.view().unwrap().sidebar.scope, ScopeName::All);
    library
        .hover(item_of(&first.token), Some(item_of(&child)))
        .unwrap();
    let next = library.prepare_drag(&target).unwrap();
    library.end_drag(item_of(&first.token));
    library
        .spring_after(item_of(&first.token), item_of(&child), u64::MAX)
        .unwrap();
    assert_eq!(library.view().unwrap().sidebar.scope, ScopeName::All);
    library
        .hover(item_of(&next.token), Some(item_of(&child)))
        .unwrap();
    library
        .spring_after(item_of(&next.token), item_of(&child), 600)
        .unwrap();
    assert_eq!(
        library.view().unwrap().sidebar.scope,
        ScopeName::Collection { id: child }
    );
}

#[test]
fn a_hover_rechecks_a_destination_that_went_away() {
    let (_home, mut library, work, child) = filed();
    let drag = library
        .prepare_drag(&TargetName::Ids {
            ids: vec![work.clone()],
        })
        .unwrap();
    assert_eq!(
        library
            .hover(item_of(&drag.token), Some(item_of(&child)))
            .unwrap(),
        None,
        "a folder cannot spring into its descendant"
    );
    let c = holder_of(&library, LINK_C, None);
    let ready = library
        .prepare_drag(&TargetName::Ids { ids: vec![c] })
        .unwrap();
    library
        .hover(item_of(&ready.token), Some(item_of(&child)))
        .unwrap();
    trash(&mut library, &[&work]);
    assert!(
        library
            .spring_after(item_of(&ready.token), item_of(&child), 600)
            .is_err()
    );
    assert_eq!(library.view().unwrap().sidebar.scope, ScopeName::All);
}

#[test]
fn moves_refuse_a_hidden_destination_before_each_batch_and_keep_completed_history() {
    let (_home, mut library, work, child) = filed();
    let a = holder_of(&library, LINK_A, Some(&work));
    let c = holder_of(&library, LINK_C, None);
    let Begun::Working(job) = library
        .begin(Intent::File {
            target: TargetName::Ids {
                ids: vec![a.clone(), c.clone()],
            },
            collection: Some(child.clone()),
        })
        .unwrap()
    else {
        panic!("a move works in batches")
    };
    let mut job = (*job).in_batches_of(1);
    assert!(library.advance(&mut job).is_some());
    library
        .session(|session| {
            session.edit(Edit::Trash {
                entry: youarel_core::state::Entry::Folder(item_of(&work)),
            })
        })
        .unwrap();
    assert!(library.advance(&mut job).is_none());
    let finished = library.finish(&mut job).unwrap();
    assert!(
        finished
            .notice
            .is_some_and(|notice| notice.error && notice.text.contains("Stopped after 1 of 2"))
    );
    assert_eq!(holder_of(&library, LINK_C, None), c);
    let refused = act(
        &mut library,
        Intent::File {
            target: TargetName::Ids { ids: vec![c] },
            collection: Some(child),
        },
    );
    assert!(refused.notice.is_some_and(|notice| notice.error));
    library
        .session(|session| {
            session.edit(Edit::PutBack {
                entry: youarel_core::state::Entry::Folder(item_of(&work)),
            })
        })
        .unwrap();
    let undone = act(&mut library, Intent::Undo);
    assert!(undone.notice.is_some_and(|notice| !notice.error));
    assert_eq!(holder_of(&library, LINK_A, Some(&work)), a);
    assert_eq!(undone.redo.as_deref(), Some("Move"));
}

fn arriving(library: &Library, id: ItemId, title: &str) {
    let mut state = youarel_core::state::State::default();
    state
        .apply(
            Edit::AddShortcut {
                id,
                link: format!("https://{}.example/", crate::intent::text(id)),
                title: title.to_owned(),
                folder: None,
            },
            youarel_core::clock_rules::Stamp {
                wall: youarel_cli::clock::now().unwrap(),
                counter: 0,
                device: youarel_core::clock_rules::DeviceId([9; 16]),
            },
        )
        .unwrap();
    assert!(library.session(|session| session.merge(&state)).unwrap());
}

#[test]
fn a_merge_keeps_the_identifiers_of_a_range_across_pages() {
    let (_home, mut library) = many(MANY);
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    let before = library.page(PAGE - 2, 4).unwrap();
    let ids: Vec<String> = before
        .rows
        .iter()
        .filter_map(|row| match row {
            RowView::Shortcut { item, .. } => Some(item.id.clone()),
            RowView::Folder { .. } | RowView::Gone => None,
        })
        .collect();
    act(
        &mut library,
        Intent::Select {
            id: ids.first().unwrap().clone(),
            extend: Extend::Replace,
            place: Some(PAGE - 2),
        },
    );
    act(
        &mut library,
        Intent::Select {
            id: ids.last().unwrap().clone(),
            extend: Extend::Range,
            place: Some(PAGE + 1),
        },
    );
    arriving(&library, ItemId([211; 16]), "! before every selected row");
    let refreshed = library.view().unwrap();
    assert_eq!(refreshed.selection.count, 4);
    assert_eq!(refreshed.focus.as_ref(), ids.last());
    assert_eq!(refreshed.listing.reveal, None);
    let page = library.page(PAGE - 2, 6).unwrap();
    let selected: Vec<String> = page
        .rows
        .iter()
        .filter_map(|row| match row {
            RowView::Shortcut {
                item,
                selected: true,
                ..
            } => Some(item.id.clone()),
            RowView::Folder { .. } | RowView::Shortcut { .. } | RowView::Gone => None,
        })
        .collect();
    assert_eq!(selected, ids);
}

#[test]
fn new_rows_never_join_the_selection_that_preceded_a_merge_in_any_layout() {
    let (_home, mut library) = many(4);
    for (number, layout) in [
        (221, LayoutName::Cards),
        (222, LayoutName::List),
        (223, LayoutName::Columns),
    ] {
        act(&mut library, Intent::Layout { layout });
        show(&mut library, ScopeName::All);
        let selected = act(&mut library, Intent::SelectAll);
        let id = ItemId([number; 16]);
        arriving(&library, id, "! newly arrived");
        let refreshed = library.view().unwrap();
        assert_eq!(refreshed.selection.count, selected.selection.count);
        if layout == LayoutName::Columns {
            assert!(
                refreshed
                    .columns
                    .first()
                    .unwrap()
                    .entries
                    .iter()
                    .any(|entry| entry.id == crate::intent::text(id) && !entry.selected)
            );
        } else {
            assert!(refreshed.listing.rows.iter().any(|row| matches!(row, RowView::Shortcut { item, selected: false, .. } if item.id == crate::intent::text(id))));
        }
    }
}

#[test]
fn a_refresh_finds_the_same_first_visible_row_and_rejects_another_scope_s_anchor() {
    let (_home, mut library) = many(MANY);
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    let page = library.page(PAGE, PAGE).unwrap();
    let RowView::Shortcut { item, .. } = page.rows.first().unwrap() else {
        panic!("a shortcut")
    };
    let anchor = crate::view::ViewportAnchor {
        identity: library.view().unwrap().listing.identity,
        id: item.id.clone(),
        column: None,
    };
    arriving(&library, ItemId([231; 16]), "! first");
    let refreshed = library
        .view_anchored(std::slice::from_ref(&anchor))
        .unwrap();
    assert_eq!(
        refreshed.listing.anchor,
        Some(crate::view::AnchoredRow {
            id: anchor.id.clone(),
            place: PAGE + 1
        })
    );
    assert_eq!(refreshed.listing.first, PAGE + 1);
    assert!(
        matches!(refreshed.listing.rows.first(), Some(RowView::Shortcut { item: visible, .. }) if visible.id == anchor.id)
    );
    show(&mut library, ScopeName::Recent);
    assert_eq!(
        library.view_anchored(&[anchor]).unwrap().listing.anchor,
        None
    );
}

#[test]
fn each_column_keeps_its_visible_anchor_after_a_merge() {
    let (_home, mut library) = many(MANY);
    let page = library.column_page(0, PAGE, PAGE).unwrap();
    let id = page.entries.first().unwrap().id.clone();
    let anchor = crate::view::ViewportAnchor {
        identity: library
            .view()
            .unwrap()
            .columns
            .first()
            .unwrap()
            .identity
            .clone(),
        id: id.clone(),
        column: Some(0),
    };
    arriving(&library, ItemId([232; 16]), "! first");
    let refreshed = library.view_anchored(&[anchor]).unwrap();
    let column = refreshed.columns.first().unwrap();
    assert_eq!(
        column.anchor,
        Some(crate::view::AnchoredRow {
            id: id.clone(),
            place: PAGE + 1
        })
    );
    assert_eq!(column.first, PAGE + 1);
    assert_eq!(column.entries.first().unwrap().id, id);
}

#[test]
fn deletion_uses_the_first_visible_place_of_an_unordered_identifier_selection() {
    let (_home, mut library) = library();
    library
        .session(|session| {
            for number in 0..20_u8 {
                session.edit(Edit::AddShortcut {
                    id: ItemId([number; 16]),
                    link: format!("https://{number}.example/"),
                    title: format!("{:02}", 19 - number),
                    folder: None,
                })?;
            }
            Ok(())
        })
        .unwrap();
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    library.select_ids((0..8_u8).chain([19]).map(|number| ItemId([number; 16])));
    let removed = act(
        &mut library,
        Intent::Trash {
            target: TargetName::Selection,
        },
    );
    assert_eq!(removed.focus, Some(crate::intent::text(ItemId([18; 16]))));
    assert_eq!(
        (removed.listing.focus, removed.selection.count),
        (Some(0), 1)
    );
}

#[test]
fn deletion_follows_the_first_selected_folder_in_cards_and_the_outline() {
    for layout in [LayoutName::Cards, LayoutName::List] {
        let (_home, mut library) = library();
        library
            .session(|session| {
                for number in 0..12_u8 {
                    session.edit(Edit::AddFolder {
                        id: ItemId([number; 16]),
                        name: format!("{:02}", 11 - number),
                        parent: None,
                    })?;
                }
                Ok(())
            })
            .unwrap();
        act(&mut library, Intent::Layout { layout });
        library.select_ids((0..8_u8).chain([11]).map(|number| ItemId([number; 16])));
        let removed = act(
            &mut library,
            Intent::Trash {
                target: TargetName::Selection,
            },
        );
        assert_eq!(removed.focus, Some(crate::intent::text(ItemId([10; 16]))));
        assert_eq!(
            (removed.listing.focus, removed.selection.count),
            (Some(0), 1)
        );
    }
}

#[test]
fn deleting_the_last_row_clears_selection_and_a_column_trail_is_not_a_selection() {
    let (_home, mut library, work, _child) = filed();
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::Columns,
        },
    );
    show(&mut library, ScopeName::All);
    move_focus(&mut library, KeyName::First, 1);
    let inside = move_focus(&mut library, KeyName::Right, 1);
    assert_eq!(inside.selection.count, 1);
    assert!(
        inside
            .columns
            .first()
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.id == work && !entry.selected)
    );
    show(&mut library, ScopeName::Unsorted);
    act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    act(&mut library, Intent::SelectAll);
    let empty = act(
        &mut library,
        Intent::Trash {
            target: TargetName::Selection,
        },
    );
    assert_eq!(
        (empty.listing.total, empty.selection.count, empty.focus),
        (0, 0, None)
    );
}
