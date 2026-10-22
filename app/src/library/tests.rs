use super::{Addition, Library};
use crate::intent::{
    ColumnName, EntryName, Extend, Fields, GroupName, Intent, KeyName, LayoutName, ScopeName,
    SortName,
};
use crate::syncing::SyncState;
use crate::view::View;
use std::path::PathBuf;
use youarel_core::folder_rules::Seen;
use youarel_core::state::{Edit, ItemId};
use youarel_core::sync_rules::{BACKOFF, QUIET, Sync};
use youarel_link::Link;
use youarel_store::Store;
use youarel_store::icons::IconId;

fn library() -> (tempfile::TempDir, Library) {
    let home = tempfile::tempdir().unwrap();
    let library = Library::new(Store::new(home.path().to_path_buf()));
    (home, library)
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

fn titles(view: &View) -> Vec<&str> {
    view.items.iter().map(|item| item.title.as_str()).collect()
}

fn id_of(view: &View, title: &str) -> String {
    view.items
        .iter()
        .find(|item| item.title == title)
        .map(|item| item.id.clone())
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
    let item = view.items.first().unwrap();
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
    view.groups
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
        view.items
            .iter()
            .find(|item| view.focus.as_ref() == Some(&item.id))
            .map(|item| item.title.clone())
    };
    let step = |shown: &mut Library, key: KeyName| act(shown, Intent::Step { key, across: 3 });
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
    view.folders
        .iter()
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
            },
        );
        let work = first.folders.first().map(|folder| folder.id.clone());
        assert_eq!(first.focus, work, "the keys reach the folders first");
        let below = act(
            &mut library,
            Intent::Step {
                key: KeyName::Down,
                across: 3,
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
        assert_eq!((titles(&inside), inside.folders.len()), (vec!["D"], 0));
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
            searched.folders.is_empty() && titles(&searched).contains(&"D"),
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

#[test]
fn the_list_opens_a_folder_in_place_and_the_arrows_open_and_close_it() {
    let (_home, mut library) = sites();
    let top = act(
        &mut library,
        Intent::Layout {
            layout: LayoutName::List,
        },
    );
    let work = top.folders.first().map(|folder| folder.id.clone()).unwrap();
    let rows = |view: &View| {
        view.outline
            .iter()
            .map(|row| (row.depth, row.open))
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&top), [(0, false); 4], "Work and the three shortcuts");
    let open = act(
        &mut library,
        Intent::Disclose {
            id: work.clone(),
            open: true,
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
        },
    );
    let left = |shown: &mut Library, key: KeyName| act(shown, Intent::Step { key, across: 1 });
    let up = left(&mut library, KeyName::Left);
    assert_eq!(
        up.focus.as_deref(),
        Some(work.as_str()),
        "the left arrow steps up to the folder"
    );
    let closed = left(&mut library, KeyName::Left);
    assert_eq!(closed.outline.len(), 4, "and then closes it");
    let reopened = left(&mut library, KeyName::Right);
    assert_eq!(reopened.outline.len(), 5, "the right arrow opens it");
    let opened = act(&mut library, Intent::Open { id: Some(work) });
    assert_eq!(
        (opened.outline.len(), opened.title.as_str()),
        (4, "All"),
        "opening a folder of the list closes it in place, and the list stays where it stands"
    );
    let again = act(&mut library, Intent::Open { id: None });
    assert_eq!(
        again.outline.len(),
        5,
        "the return key opens the focused folder again"
    );
    let gathered = act(
        &mut library,
        Intent::Group {
            grouping: GroupName::Site,
        },
    );
    assert_eq!(gathered.outline, [], "headings take the outline's place");
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
            ("Work".to_owned(), 0, true, true),
            ("Specs".to_owned(), 1, false, false),
            ("Life".to_owned(), 0, false, false)
        ]
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
            ("Work".to_owned(), 0, true, false),
            ("Life".to_owned(), 0, false, false)
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
    let column = marked.columns.first().and_then(|column| column.first());
    assert_eq!(column.map(|entry| entry.mark.as_str()), Some("📚"));
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
        cards.folders.first().map(|folder| folder.mark.as_str()),
        Some("📚"),
        "a refused mark leaves the old one"
    );
    let id = cards
        .folders
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
        coded.folders.first().map(|folder| folder.mark.as_str()),
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
        (all.selected.len(), all.focus.is_some()),
        (4, true),
        "Work and the three shortcuts at the top, with the first focused"
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
    let work = top.folders.first().map(|folder| folder.id.clone()).unwrap();
    let archive = collection(&mut library, "Archive", None);
    let moved = act(
        &mut library,
        Intent::File {
            ids: vec![work.clone(), id_of(&top, "A")],
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
    let removed = act(
        &mut library,
        Intent::Remove {
            ids: vec![work, id_of(&inside, "A")],
        },
    );
    assert_eq!(
        removed.notice.map(|notice| notice.text),
        Some("Removed 1 folder and 1 shortcut".to_owned())
    );
    assert_eq!((removed.count, removed.folders.len()), (0, 0));
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
    let first = columns
        .columns
        .first()
        .unwrap()
        .iter()
        .map(|entry| (entry.name.as_str(), entry.heading.as_deref()))
        .collect::<Vec<_>>();
    assert_eq!(
        first,
        [
            ("Work", None),
            ("B", Some("github.com")),
            ("A", Some("mozilla.org")),
            ("C", None)
        ],
        "a column shows its folders first and its shortcuts under their sites"
    );
    let ungrouped = act(
        &mut library,
        Intent::Group {
            grouping: GroupName::None,
        },
    );
    assert!(
        ungrouped
            .columns
            .first()
            .unwrap()
            .iter()
            .all(|entry| entry.heading.is_none())
    );
}

#[test]
fn a_folder_moves_into_another_but_never_into_itself() {
    let (_home, mut library) = library();
    let work = collection(&mut library, "Work", None);
    let specs = collection(&mut library, "Specs", None);
    let moved = act(
        &mut library,
        Intent::MoveFolders {
            ids: vec![specs.clone()],
            collection: Some(work.clone()),
        },
    );
    assert_eq!(
        moved.notice.map(|notice| notice.text),
        Some("Moved 1 folder to Work".to_owned())
    );
    assert_eq!(
        moved
            .collections
            .iter()
            .map(|choice| choice.path.as_str())
            .collect::<Vec<_>>(),
        ["Work", "Work / Specs"]
    );
    let refused = act(
        &mut library,
        Intent::MoveFolders {
            ids: vec![work.clone()],
            collection: Some(specs),
        },
    );
    assert!(refused.notice.is_some_and(|notice| notice.error));
    let top = act(
        &mut library,
        Intent::MoveFolders {
            ids: vec![work],
            collection: None,
        },
    );
    assert_eq!(
        top.notice.map(|notice| notice.text),
        Some("Moved 1 folder to the top".to_owned())
    );
}

/// A collection named `name` inside `parent`, by its identifier.
fn collection(library: &mut Library, name: &str, parent: Option<String>) -> String {
    let before = library.view().unwrap().collections;
    let created = act(
        library,
        Intent::NewCollection {
            name: name.to_owned(),
            parent,
        },
    );
    created
        .collections
        .iter()
        .find(|choice| !before.contains(choice))
        .map(|choice| choice.id.clone())
        .unwrap()
}

#[test]
fn a_new_collection_needs_a_name_and_opens_where_it_is_made() {
    let (_home, mut library) = library();
    let work = collection(&mut library, " Work ", None);
    let shown = library.view().unwrap();
    assert_eq!(
        (shown.title.as_str(), shown.here.as_deref()),
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
    assert_eq!(nested.here, Some(specs));
    assert_eq!(
        nested
            .collections
            .iter()
            .map(|choice| choice.path.as_str())
            .collect::<Vec<_>>(),
        ["Work", "Work / Specs"]
    );
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
            ids: vec![a],
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
    assert_eq!(
        work_view
            .items
            .first()
            .and_then(|item| item.collection.as_deref()),
        Some("Work / Specs")
    );
    act(
        &mut library,
        Intent::RenameCollection {
            id: work.clone(),
            name: "Job".to_owned(),
        },
    );
    let removed = act(&mut library, Intent::RemoveCollection { id: work });
    assert_eq!((removed.title.as_str(), removed.count), ("All", 0));
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
            scope: ScopeName::All,
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
    let alpha = saved.items.first().unwrap();
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
    assert_eq!(kept.items.first().unwrap().collection, None);
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
        },
    );
    assert_eq!(first.focus, Some(id_of(&first, "a")));
    let down = act(
        &mut library,
        Intent::Step {
            key: KeyName::Down,
            across: 3,
        },
    );
    assert_eq!(down.focus, Some(id_of(&down, "d")));
    let ranged = act(
        &mut library,
        Intent::Select {
            id: id_of(&down, "b"),
            extend: Extend::Range,
        },
    );
    assert_eq!(ranged.selected.len(), 3);
    let toggled = act(
        &mut library,
        Intent::Select {
            id: id_of(&down, "c"),
            extend: Extend::Toggle,
        },
    );
    assert_eq!(toggled.selected.len(), 2);
    let removed = act(
        &mut library,
        Intent::Remove {
            ids: toggled.selected,
        },
    );
    assert_eq!(titles(&removed), ["a", "c", "e"]);
    assert_eq!(removed.selected.len(), 0);
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
            ids: vec![id_of(&view, "A")],
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
    let step = |shown: &mut Library, key| act(shown, Intent::Step { key, across: 1 });
    let first = step(&mut library, KeyName::Down);
    assert_eq!(
        first.columns.len(),
        2,
        "the selected folder opens its column"
    );
    assert!(
        first
            .columns
            .first()
            .and_then(|column| column.first())
            .is_some_and(|entry| entry.selected && entry.opens)
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
fn the_layout_and_the_order_persist_and_a_bad_intent_becomes_a_notice() {
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
        (LayoutName::List, SortName::Oldest)
    );
    let refused = act(
        &mut library,
        Intent::Select {
            id: "zz".to_owned(),
            extend: Extend::Replace,
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
    assert_eq!(
        library.begin_sync(NOW),
        Some((Sync::Publish, target.clone()))
    );
    assert_eq!(library.view().unwrap().sync.state, SyncState::Syncing);
    library.end_sync(NOW, Err(anyhow::anyhow!("unreachable")));
    let failed = library.view().unwrap().sync;
    assert_eq!((failed.state, failed.last), (SyncState::Failed, None));
    act(&mut library, Intent::SyncNow);
    assert_eq!(
        library.begin_sync(NOW + 1),
        Some((Sync::Publish, target.clone())),
        "a request skips the backoff"
    );
    library.end_sync(NOW + 1, Ok(Vec::new()));
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

/// A library that syncs through `target` for a device that belongs to an account, and whose first sync, a publish, began at `now` and succeeded.
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
        Some((Sync::Publish, target.to_owned()))
    );
    library.end_sync(now, Ok(Vec::new()));
    (home, library)
}

#[test]
fn another_device_s_bundle_brings_a_merge_once_the_folder_rests_and_never_the_device_s_own() {
    const NOW: u64 = 1_700_000_000_000;
    let (_home, mut library) = synced_library("/shared", NOW);
    library.learned_own("mine.youarel".to_owned());
    assert!(!library.noticed(Seen::Touched("mine.youarel"), NOW + 1));
    assert!(!library.noticed(Seen::Read, NOW + 1));
    assert_eq!(library.begin_sync(NOW + 1 + QUIET), None, "its own write");
    assert!(library.noticed(Seen::Touched("other.youarel"), NOW + 1));
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
    assert!(library.noticed(Seen::Touched("other.youarel"), NOW + 2 + QUIET));
    assert_eq!(
        library.begin_sync(NOW + 2 + 2 * QUIET),
        None,
        "no merge during a backoff"
    );
    assert_eq!(
        library.begin_sync(NOW + 1 + QUIET + BACKOFF),
        Some((Sync::Publish, "/shared".to_owned()))
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
    let view = add(&mut library, "https://b.example/", "Other");
    let mut names: Vec<(&str, Option<String>)> = view
        .items
        .iter()
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
    let view = library.view().unwrap();
    assert_eq!(
        view.items.iter().filter(|item| item.icon.is_some()).count(),
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
                session.keep_icon(host, IconId([name; 32]), image.to_vec())?;
            }
            Ok(())
        })
        .unwrap();
    let mut library = Library::new(store);
    let view = library.view().unwrap();
    let mut names: Vec<String> = view
        .items
        .iter()
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
