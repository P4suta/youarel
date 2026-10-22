//! What the window draws: every text as text, every identifier in hexadecimal, and every time in milliseconds for the window to format.

use crate::intent::{BrowserChoice, ColumnName, GroupName, LayoutName, ScopeName, SortName, text};
use crate::syncing::SyncView;
use crate::themes::Worn;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use youarel_core::host_rules::{self, HostParts};
use youarel_core::library_rules::{self, Counts};
use youarel_core::rank_rules::Visits;
use youarel_core::state::{ItemId, Shortcut};
use youarel_link::Link;
use youarel_store::folders::Folders;

pub use youarel_store::PATH_SEPARATOR;

/// What the scheme of a web link shows as.
const SCHEME_MARK: &str = "://";

/// A link in the runs the window sets apart: the scheme, the subdomains, the registrable name, the public suffix, the port, and what follows.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlView {
    pub scheme: String,
    pub sub: String,
    pub name: String,
    pub suffix: String,
    pub port: String,
    pub rest: String,
}

/// The runs of a stored link, or the link whole as the name when it does not parse.
#[must_use]
pub fn url(link: &str) -> UrlView {
    let Ok(parsed) = Link::parse(link) else {
        return UrlView {
            name: link.to_owned(),
            ..UrlView::default()
        };
    };
    let scheme = parsed
        .as_str()
        .split_once(SCHEME_MARK)
        .map_or("", |(scheme, _)| scheme);
    let HostParts { sub, name, suffix } = host_rules::split(parsed.host());
    UrlView {
        scheme: format!("{scheme}{SCHEME_MARK}"),
        sub: sub.to_owned(),
        name: name.to_owned(),
        suffix: suffix.to_owned(),
        port: parsed
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default(),
        rest: parsed.rest(),
    }
}

/// One shortcut as the window draws it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub title: String,
    pub link: String,
    pub url: UrlView,
    /// The shortcut's icon as a `data:` address of its PNG, which the page's policy admits as an image alone.
    pub icon: Option<String>,
    /// The path of the collection the shortcut sits in, or none at the top.
    pub collection: Option<String>,
    pub collection_id: Option<String>,
    pub alias: String,
    pub note: String,
    /// When the shortcut was added, in milliseconds since the Unix epoch.
    pub added: u64,
    /// When this device last opened it, or none.
    pub opened: Option<u64>,
    pub visits: u32,
}

/// The facts an item reads beyond its shortcut.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    pub folders: &'a Folders,
    pub visits: Option<&'a Visits>,
}

impl Item {
    /// The item of a live shortcut, with its icon drawn already.
    #[must_use]
    pub fn new(
        context: Context<'_>,
        id: ItemId,
        shortcut: &Shortcut,
        icon: Option<String>,
    ) -> Self {
        let folder = context.folders.home(shortcut);
        let path = folder.map(|folder| context.folders.path(Some(folder)).join(PATH_SEPARATOR));
        Self {
            id: text(id),
            title: shortcut.title.value.clone(),
            link: shortcut.link.value.clone(),
            url: url(&shortcut.link.value),
            icon,
            collection: path,
            collection_id: folder.map(text),
            alias: shortcut.alias.value.clone(),
            note: shortcut.note.value.clone(),
            added: library_rules::added(id, shortcut),
            opened: context.visits.map(|visits| visits.last),
            visits: context.visits.map_or(0, |visits| visits.count),
        }
    }
}

/// A collection in the sidebar with its count.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionCount {
    pub id: String,
    pub name: String,
    /// What the folder shows in place of its glyph, or empty for the glyph.
    pub mark: String,
    pub count: usize,
}

/// A heading over the rows of a listing: its label, the place of the first row under it, and how many rows follow under it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub label: String,
    pub place: usize,
    pub count: usize,
}

/// One row of a listing as the window draws it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RowView {
    /// A folder, the path of the folder it came from while it waits in the trash, how deep the outline sets it, and whether the outline shows its rows, or none outside the outline.
    Folder {
        folder: Box<CollectionCount>,
        origin: Option<String>,
        depth: usize,
        open: Option<bool>,
        selected: bool,
        cut: bool,
    },
    /// A shortcut, and how deep the outline sets it.
    Shortcut {
        item: Box<Item>,
        depth: usize,
        selected: bool,
        cut: bool,
    },
    /// A row whose entry went while the page was read.
    Gone,
}

/// The page of a listing the window shows: what tells it apart, how many rows it holds, the rows around the place in sight, and where the focus and the headings stand.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListingView {
    pub identity: String,
    pub anchor: Option<AnchoredRow>,
    /// What tells the listing apart, with the state of the store it was read in, which a page the window asks for later answers with.
    pub key: String,
    pub total: usize,
    /// The place of the first of the rows.
    pub first: usize,
    pub rows: Vec<RowView>,
    /// The place of the focused row, when the listing holds it.
    pub focus: Option<usize>,
    /// The place of a row the page brings into sight, after an intent that moved the focus to it.
    pub reveal: Option<usize>,
    /// Whether the listing is another than the one the window showed before, so the page starts from its top.
    pub fresh: bool,
    /// How many of the first rows are folders.
    pub folders: usize,
    /// The headings over the rows, in their order.
    pub groups: Vec<GroupView>,
    /// Whether the rows form the list's outline, whose folders open in place.
    pub outline: bool,
}

/// The rows of a listing a page asks for: what tells the listing apart, how many rows it holds, and the rows from a place on.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowsView {
    pub key: String,
    pub total: usize,
    pub first: usize,
    pub rows: Vec<RowView>,
}

/// A column of the folder walk as the window draws it: one page of its entries, the place of the one selected, and its headings.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnView {
    pub identity: String,
    pub anchor: Option<AnchoredRow>,
    pub key: String,
    pub total: usize,
    pub first: usize,
    pub entries: Vec<ColumnEntry>,
    pub selected: Option<usize>,
    pub groups: Vec<GroupView>,
}

/// The entries of a column a page asks for.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntriesView {
    pub key: String,
    pub total: usize,
    pub first: usize,
    pub entries: Vec<ColumnEntry>,
}

/// The first visible entry of a listing or a column before a refresh.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewportAnchor {
    pub identity: String,
    pub id: String,
    pub column: Option<usize>,
}

/// Where the same visible entry stands in the refreshed listing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnchoredRow {
    pub id: String,
    pub place: usize,
}

/// How far a move or a removal came, which the window shows while it works.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub text: String,
}

/// The immutable entries of a drag, named by its token, with the text its native data store carries.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DragMaterial {
    pub token: String,
    pub links: String,
    pub count: usize,
}

/// What a person selected, counted: how many rows, and how many of them are folders.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionView {
    pub count: usize,
    pub folders: usize,
}

/// One folder of the trail above the columns, which a click selects again.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: String,
    pub name: String,
    /// The column the folder sits in, from the left.
    pub depth: usize,
}

/// A folder of the sidebar's tree: its count, how deep it sits, whether it holds folders, and whether it shows them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarFolder {
    pub id: String,
    pub name: String,
    /// What the folder shows in place of its glyph, or empty for the glyph.
    pub mark: String,
    pub count: usize,
    pub depth: usize,
    pub opens: bool,
    pub open: bool,
}

/// The sidebar: the chosen place, the counts of the fixed scopes and of the trash, and the tree of the collections.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sidebar {
    pub scope: ScopeName,
    pub all: usize,
    pub unsorted: usize,
    pub recent: usize,
    /// The folders and the shortcuts that wait in the trash themselves.
    pub trash: usize,
    /// The collections from the top, each open one followed by the folders inside it.
    pub collections: Vec<SidebarFolder>,
}

/// The folders that show directly in a folder, or at the top, by their names.
#[must_use]
pub fn folders_in(folders: &Folders, folder: Option<ItemId>) -> Vec<ItemId> {
    folders.named(folder).to_vec()
}

impl Sidebar {
    /// The sidebar over the counts of every scope, with the folders `open` holds open in its tree.
    #[must_use]
    pub fn new(
        folders: &Folders,
        scope: ScopeName,
        counts: &Counts,
        open: &BTreeSet<ItemId>,
    ) -> Self {
        let mut collections = Vec::new();
        let mut pending: Vec<(ItemId, usize)> = folders_in(folders, None)
            .into_iter()
            .rev()
            .map(|id| (id, 0))
            .collect();
        while let Some((id, depth)) = pending.pop() {
            let inside = folders_in(folders, Some(id));
            let shows = open.contains(&id) && !inside.is_empty();
            if let Some(folder) = folders.folder(id) {
                collections.push(SidebarFolder {
                    id: text(id),
                    name: folder.name.value.clone(),
                    mark: folder.mark.value.clone(),
                    count: counts.collections.get(&id).copied().unwrap_or_default(),
                    depth,
                    opens: !inside.is_empty(),
                    open: shows,
                });
            }
            if shows {
                let below = depth.saturating_add(1);
                pending.extend(inside.into_iter().rev().map(|child| (child, below)));
            }
        }
        Self {
            scope,
            all: counts.all,
            unsorted: counts.unsorted,
            recent: counts.recent,
            trash: counts.trash,
            collections,
        }
    }
}

/// What kind of entry a column shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Folder,
    Shortcut,
    /// An entry whose folder or shortcut went while the column was read.
    Gone,
}

/// One entry of a column.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnEntry {
    pub kind: EntryKind,
    pub id: String,
    pub name: String,
    pub selected: bool,
    pub cut: bool,
    /// Whether the entry is a folder that holds entries.
    pub opens: bool,
    pub icon: Option<String>,
    /// The registrable name of a shortcut's site, which an entry without an icon shows as a tile, or empty for a folder.
    pub site: String,
    /// What a folder shows in place of its glyph, or empty for the glyph and for a shortcut.
    pub mark: String,
}

/// A collection a picker offers, named by its whole path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub id: String,
    pub path: String,
}

/// A collection as a picker offers it, named by its whole path.
#[must_use]
pub fn choice(folders: &Folders, id: ItemId) -> Choice {
    Choice {
        id: text(id),
        path: folders.path(Some(id)).join(PATH_SEPARATOR),
    }
}

/// Every collection that shows in the order of the tree: each one by its name among its siblings, then the folders inside it.
#[must_use]
pub fn tree_order(folders: &Folders) -> Vec<ItemId> {
    let mut found = Vec::new();
    let mut pending: Vec<ItemId> = folders_in(folders, None);
    pending.reverse();
    while let Some(id) = pending.pop() {
        found.push(id);
        let mut inside = folders_in(folders, Some(id));
        inside.reverse();
        pending.extend(inside);
    }
    found
}

/// What the window opens with, which the settings show: a layout, and a collection or every shortcut for none.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpeningView {
    pub layout: LayoutName,
    pub start: Option<Choice>,
}

/// The last notice, and whether it reports a failure.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

/// Everything the window draws.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub cut: usize,
    pub sidebar: Sidebar,
    /// The heading of the scope: all, unsorted, recent, or a collection's path.
    pub title: String,
    pub count: usize,
    pub layout: LayoutName,
    pub sort: SortName,
    pub grouping: GroupName,
    pub search: String,
    /// The page of the listing the cards or the list show.
    pub listing: ListingView,
    /// The columns the list draws after the name, in their order.
    pub list_columns: Vec<ColumnName>,
    /// The columns of the list a person hid.
    pub hidden_columns: Vec<ColumnName>,
    /// The columns of the folder walk, from the left.
    pub columns: Vec<ColumnView>,
    /// The shortcut the columns preview.
    pub preview: Option<Item>,
    pub focus: Option<String>,
    /// What a person selected, counted.
    pub selection: SelectionView,
    pub notice: Option<Notice>,
    /// The folder that a new folder or a new link goes into, with its path, or none for the top.
    pub here: Option<Choice>,
    /// The folders the columns open, from the left, or, in the cards and the list, the folders from the top down to the one they stand in.
    pub trail: Vec<Step>,
    /// The themes the window wears, or none when no theme is known at all.
    pub theme: Option<Worn>,
    /// The browsers that keep bookmarks on this kind of host.
    pub browsers: Vec<BrowserChoice>,
    pub sync: SyncView,
    pub opening: OpeningView,
    pub back: bool,
    pub forward: bool,
    pub undo: Option<String>,
    pub redo: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{UrlView, url};

    #[test]
    fn a_link_shows_in_its_runs() {
        assert_eq!(
            url("https://url.spec.whatwg.org/"),
            UrlView {
                scheme: "https://".to_owned(),
                sub: "url.spec.".to_owned(),
                name: "whatwg".to_owned(),
                suffix: ".org".to_owned(),
                port: String::new(),
                rest: "/".to_owned(),
            }
        );
        let local = url("http://localhost:5173/app?x#y");
        assert_eq!(
            (
                local.scheme.as_str(),
                local.name.as_str(),
                local.port.as_str(),
                local.rest.as_str()
            ),
            ("http://", "localhost", ":5173", "/app?x#y")
        );
        assert_eq!(url("not a link").name, "not a link");
    }
}
