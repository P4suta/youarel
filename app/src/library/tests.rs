use super::{Addition, Begun, Library, MOST_CHOICES, PAGE};
use crate::intent::{Extend, Fields, Intent, KeyName, LayoutName, ScopeName, TargetName};
use crate::view::{CollectionCount, Item, RowView, View};
use youarel_core::state::{Edit, ItemId};
use youarel_link::Link;
use youarel_store::Store;
use youarel_store::icons::IconId;

fn library() -> (tempfile::TempDir, Library) {
    let home = tempfile::tempdir().unwrap();
    let library = Library::new(Store::new(home.path().to_path_buf()));
    (home, library)
}

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
    items(view)
        .into_iter()
        .map(|item| item.title.as_str())
        .collect()
}

fn id_of(view: &View, name: &str) -> String {
    items(view)
        .into_iter()
        .find(|item| item.title == name)
        .map(|item| item.id.clone())
        .or_else(|| {
            folder_rows(view)
                .into_iter()
                .find(|folder| folder.name == name)
                .map(|folder| folder.id.clone())
        })
        .unwrap()
}

#[test]
fn opening_a_folder_shows_only_its_contents_and_preserves_the_path() {
    let (_home, mut library) = library();
    let parent = collection(&mut library, "Parent", None);
    let child = collection(&mut library, "Child", Some(parent.clone()));
    library
        .add(addition(
            "https://nested.example/",
            "Nested",
            Some(item_of(&child)),
        ))
        .unwrap();
    add(&mut library, "https://root.example/", "Root");
    let root = show(&mut library, ScopeName::All);
    assert_eq!(titles(&root), ["Root"]);
    assert_eq!(folder_rows(&root).len(), 1);
    let opened = act(
        &mut library,
        Intent::Open {
            id: Some(parent.clone()),
        },
    );
    assert_eq!(opened.scope, ScopeName::Collection { id: parent });
    assert_eq!(titles(&opened), Vec::<&str>::new());
    assert_eq!(folder_rows(&opened)[0].id, child);
    let nested = act(&mut library, Intent::Open { id: Some(child) });
    assert_eq!(titles(&nested), ["Nested"]);
    assert_eq!(
        nested
            .trail
            .iter()
            .map(|step| step.name.as_str())
            .collect::<Vec<_>>(),
        ["Parent", "Child"]
    );
}

#[test]
fn bookmarks_and_folders_keep_name_order() {
    let (_home, mut library) = library();
    add(&mut library, "https://z.example/", "Alpha");
    add(&mut library, "https://a.example/", "Zulu");
    for name in ["Beta", "Gamma"] {
        act(
            &mut library,
            Intent::NewCollection {
                name: name.to_owned(),
                parent: None,
            },
        );
    }
    act(
        &mut library,
        Intent::Scope {
            scope: ScopeName::All,
        },
    );
    let view = library.view().unwrap();
    assert_eq!(titles(&view), ["Alpha", "Zulu"]);
    assert_eq!(
        folder_rows(&view)
            .iter()
            .map(|folder| folder.name.as_str())
            .collect::<Vec<_>>(),
        ["Beta", "Gamma"]
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
        revealed.notice.as_ref().map(|notice| notice.text.as_str()),
        Some("Already in Work: A")
    );
    assert_eq!(revealed.scope, ScopeName::Collection { id: work });
    assert!(items(&revealed).iter().any(|item| item.title == "A"));
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

const MDN_PAGE: &str = "https://developer.mozilla.org/a";
const GITHUB_PAGE: &str = "https://github.com/b";
const MOZILLA_PAGE: &str = "https://www.mozilla.org/c";
const EXAMPLE_PAGE: &str = "https://example.com/d";

fn sites() -> (tempfile::TempDir, Library) {
    let (home, mut library) = library();
    add(&mut library, MDN_PAGE, "A");
    add(&mut library, GITHUB_PAGE, "B");
    add(&mut library, MOZILLA_PAGE, "C");
    let work = collection(&mut library, "Work", None);
    library
        .add(addition(EXAMPLE_PAGE, "D", Some(item_of(&work))))
        .unwrap();
    show(&mut library, ScopeName::All);
    (home, library)
}

fn folder_names(view: &View) -> Vec<(&str, usize)> {
    folder_rows(view)
        .into_iter()
        .map(|folder| (folder.name.as_str(), folder.count))
        .collect()
}

#[test]
fn select_all_takes_every_folder_and_shortcut_the_list_shows() {
    let (_home, mut library) = sites();
    library.view().unwrap();
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
    let refreshed = library.view().unwrap();
    assert_eq!(
        refreshed.selection.count, 3,
        "a refresh keeps every selected row"
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
    let top = library.view().unwrap();
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
    let moved_shortcut = id_of(&inside, "A");
    let trashed = act(
        &mut library,
        Intent::Trash {
            target: TargetName::Ids {
                ids: vec![work.clone(), moved_shortcut.clone()],
            },
        },
    );
    assert_eq!(
        (trashed.count, folder_names(&trashed), titles(&trashed)),
        (0, vec![], vec![])
    );
    assert_eq!(
        trashed.notice.map(|notice| notice.text),
        Some("Moved 1 folder and 1 shortcut to the Trash".to_owned())
    );
    assert!(
        trashed
            .focus
            .as_ref()
            .is_none_or(|id| id != &work && id != &moved_shortcut)
    );
    let root = show(&mut library, ScopeName::All);
    assert_eq!(titles(&root), ["B", "C"]);
    assert_eq!(folder_names(&root), [("Archive", 0)]);
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

fn paths(library: &Library) -> Vec<String> {
    library
        .choices("", 0, MOST_CHOICES)
        .unwrap()
        .into_iter()
        .map(|choice| choice.path)
        .collect()
}

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

const MANY: usize = 450;
const MANY_LINK: &str = "https://many.example/";

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
    let top = library.view().unwrap();
    assert_eq!(
        (top.listing.total, top.listing.first, top.listing.rows.len()),
        (MANY, 0, PAGE)
    );
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
fn a_range_moves_together_and_a_folder_never_moves_into_itself() {
    let (_home, mut library) = sites();
    let archive = collection(&mut library, "Archive", None);
    library.view().unwrap();
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

const PNG: &[u8] = include_bytes!("../../icons/32x32.png");

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
    let view = library.view().unwrap();
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

fn show(library: &mut Library, scope: ScopeName) -> View {
    act(library, Intent::Scope { scope })
}

fn holder_of(library: &Library, link: &str, collection: Option<&str>) -> String {
    let id = library
        .holder(&Link::parse(link).unwrap(), collection.map(item_of))
        .unwrap()
        .unwrap();
    crate::intent::text(id)
}

const LINK_A: &str = "https://a.example/";
const LINK_B: &str = "https://b.example/";
const LINK_C: &str = "https://c.example/";

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
    show(&mut library, ScopeName::All);
    (home, library, work, docs)
}

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
        (all.count, all.trash),
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
    assert_eq!((back.listing.total, back.trash), (0, 0));
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
        (emptied.notice.map(|notice| notice.text), emptied.trash),
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
        (deleted.count, deleted.trash),
        (2, 0),
        "A and B stay inside Work, and nothing waits in the trash"
    );
    assert_eq!(
        deleted.notice.map(|notice| notice.text),
        Some("Deleted 1 shortcut".to_owned())
    );
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
    assert_eq!(back.scope, ScopeName::Collection { id: first.clone() });
    let at_top = request(&mut library, serde_json::json!({"kind": "back"}));
    assert_eq!(at_top.scope, ScopeName::All);
    let forward = request(&mut library, serde_json::json!({"kind": "forward"}));
    assert_eq!(forward.scope, ScopeName::Collection { id: first });
    show(&mut library, ScopeName::Trash);
    let stayed = request(&mut library, serde_json::json!({"kind": "forward"}));
    assert_eq!(stayed.scope, ScopeName::Trash);
}

#[test]
fn shift_arrows_extend_and_shrink_from_the_same_anchor_across_a_page() {
    let (_home, mut library) = many(MANY);
    library.view().unwrap();
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
fn undo_and_redo_restore_the_fields_of_an_edit_and_new_edits_drop_redo() {
    let (_home, mut library) = library();
    library.view().unwrap();
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
    assert_eq!((undone.count, undone.trash), (0, 1));
    let redone = request(&mut library, serde_json::json!({"kind": "redo"}));
    assert_eq!(id_of(&redone, "Added"), id);
    assert_eq!(redone.trash, 0);
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
    assert_eq!(trashed.trash, 1);
    let restored = act(&mut library, Intent::Redo);
    assert_eq!(restored.trash, 0);
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
    assert_eq!((back_in_trash.count, back_in_trash.trash), (0, 1));
    let live = act(&mut library, Intent::Redo);
    assert_eq!((live.count, live.trash), (1, 0));
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
        serde_json::json!({"kind": "paste", "text": "a.example\nhttps://b.example/ https://a.example/ javascript:alert(1)"}),
    );
    assert_eq!(pasted.count, 2);
    assert!(
        library
            .holder(&Link::parse("https://a.example/").unwrap(), None)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        pasted.notice.unwrap().text,
        "Added 2 shortcuts; 1 already here; 1 ignored"
    );
    let undone = act(&mut library, Intent::Undo);
    assert_eq!((undone.count, undone.trash), (0, 2));
    assert_eq!(act(&mut library, Intent::Redo).count, 2);
}

#[test]
fn cut_marks_paged_rows_and_a_refused_paste_keeps_the_cut_selection() {
    let (_home, mut library) = many(MANY);
    library.view().unwrap();
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
    assert!(!titles(&library.view().unwrap()).contains(&"B"));
    library.spring_after(token, item_of(&child), 600).unwrap();
    let opened = library.view().unwrap();
    assert_eq!(opened.scope, ScopeName::Collection { id: child.clone() });
    assert!(titles(&opened).contains(&"B"));
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
    assert_eq!(library.view().unwrap().scope, ScopeName::All);
    assert!(!titles(&library.view().unwrap()).contains(&"B"));
    library
        .hover(item_of(&first.token), Some(item_of(&child)))
        .unwrap();
    let next = library.prepare_drag(&target).unwrap();
    library.end_drag(item_of(&first.token));
    library
        .spring_after(item_of(&first.token), item_of(&child), u64::MAX)
        .unwrap();
    assert_eq!(library.view().unwrap().scope, ScopeName::All);
    assert!(!titles(&library.view().unwrap()).contains(&"B"));
    library
        .hover(item_of(&next.token), Some(item_of(&child)))
        .unwrap();
    library
        .spring_after(item_of(&next.token), item_of(&child), 600)
        .unwrap();
    let opened = library.view().unwrap();
    assert_eq!(opened.scope, ScopeName::Collection { id: child });
    assert!(titles(&opened).contains(&"B"));
}

#[test]
fn an_empty_folder_opens_and_new_folders_stay_in_their_parent() {
    let (_home, mut library) = library();
    let parent = collection(&mut library, "Parent", None);
    let empty = act(
        &mut library,
        Intent::Open {
            id: Some(parent.clone()),
        },
    );
    assert_eq!(empty.listing.total, 0);
    assert_eq!(empty.scope, ScopeName::Collection { id: parent.clone() });
    let child = collection(&mut library, "Child", Some(parent.clone()));
    let created = library.view().unwrap();
    assert_eq!(created.scope, ScopeName::Collection { id: parent });
    assert_eq!(created.focus, Some(child));
    assert_eq!(folder_names(&created), [("Child", 0)]);
}

#[test]
fn a_saved_bookmark_reveals_its_containing_folder() {
    let (_home, mut library) = library();
    let parent = collection(&mut library, "Parent", None);
    let child = collection(&mut library, "Child", Some(parent));
    let added = library
        .add(addition(
            "https://nested.example/",
            "Nested bookmark",
            Some(item_of(&child)),
        ))
        .unwrap();
    let id = id_of(&added, "Nested bookmark");
    show(&mut library, ScopeName::All);
    let located = act(
        &mut library,
        Intent::ShowInFolder {
            id: Some(id.clone()),
        },
    );
    assert_eq!(located.scope, ScopeName::Collection { id: child });
    assert_eq!(located.focus, Some(id));
    assert_eq!(located.trail.len(), 2);
    assert_eq!(titles(&located), ["Nested bookmark"]);
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
    assert_eq!(library.view().unwrap().scope, ScopeName::All);
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
    library.view().unwrap();
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
fn a_refresh_finds_the_same_first_visible_row_and_rejects_another_scope_s_anchor() {
    let (_home, mut library) = many(MANY);
    library.view().unwrap();
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
    library.view().unwrap();
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
fn the_list_ignores_retired_preferences_and_preserves_existing_bookmarks() {
    const RETIRED_SETTINGS: &str = "settings.json";
    const RETIRED_ACCOUNT: &str = "account";
    const SENTINEL: &[u8] = b"retired configuration is not read";
    let (home, mut library) = library();
    add(&mut library, "https://preserved.example/", "Preserved");
    let store = library.store.clone();
    drop(library);
    for name in [RETIRED_SETTINGS, RETIRED_ACCOUNT] {
        youarel_store::fs::replace(&home.path().join(name), SENTINEL).unwrap();
    }
    let mut restarted = Library::new(store.clone());
    let shown = restarted.view().unwrap();
    assert_eq!(shown.layout, LayoutName::List);
    assert_eq!(titles(&shown), ["Preserved"]);
    assert_eq!(shown.notice, None);
    let edited = act(
        &mut restarted,
        Intent::Retitle {
            id: id_of(&shown, "Preserved"),
            title: "Kept".to_owned(),
        },
    );
    assert_eq!(titles(&edited), ["Kept"]);
    drop(restarted);
    let reopened = Library::new(store).view().unwrap();
    assert_eq!(reopened.layout, LayoutName::List);
    assert_eq!(titles(&reopened), ["Kept"]);
    for name in [RETIRED_SETTINGS, RETIRED_ACCOUNT] {
        assert_eq!(
            youarel_store::fs::read_bounded(
                &home.path().join(name),
                u64::try_from(SENTINEL.len()).unwrap(),
            )
            .unwrap(),
            SENTINEL
        );
    }
}
