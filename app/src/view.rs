//! What the window draws: every text as text, every identifier in hexadecimal, and every time in milliseconds for the window to format.

use crate::intent::{BrowserChoice, ColumnName, GroupName, LayoutName, ScopeName, SortName, text};
use crate::syncing::SyncView;
use crate::themes::Worn;
use serde::Serialize;
use std::collections::BTreeSet;
use youarel_core::host_rules::{self, HostParts};
use youarel_core::library_rules::{self, Counts};
use youarel_core::rank_rules::Visits;
use youarel_core::state::{Entry, ItemId, Shortcut, State, Tree};
use youarel_link::Link;

/// The separator between the folders of a collection's path.
pub const PATH_SEPARATOR: &str = " / ";
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
    pub state: &'a State,
    pub tree: &'a Tree,
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
        let folder = context.state.folder_of(context.tree, id);
        let path = folder.map(|folder| {
            context
                .state
                .path(context.tree, Some(folder))
                .join(PATH_SEPARATOR)
        });
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

/// A heading the cards and the list gather shortcuts under, with how many of the items that follow it fall under it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub label: String,
    pub count: usize,
}

/// The site of a link: its registrable domain, or its whole host when it has none, such as an address or `localhost`.
#[must_use]
pub fn site(link: &str) -> String {
    let parts = url(link);
    format!("{}{}", parts.name, parts.suffix)
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

/// The sidebar: the chosen scope, the counts of the fixed scopes, and the tree of the collections.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sidebar {
    pub scope: ScopeName,
    pub all: usize,
    pub unsorted: usize,
    pub recent: usize,
    /// The collections from the top, each open one followed by the folders inside it.
    pub collections: Vec<SidebarFolder>,
}

/// The live folders directly in a folder, or at the top, in their order.
#[must_use]
pub fn folders_in(state: &State, tree: &Tree, folder: Option<ItemId>) -> Vec<ItemId> {
    state
        .entries(tree, folder)
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Folder(id) => Some(id),
            Entry::Shortcut(_) => None,
        })
        .collect()
}

impl Sidebar {
    /// The sidebar over the counts of every scope, with the folders `open` holds open in its tree.
    #[must_use]
    pub fn new(
        (state, tree): (&State, &Tree),
        scope: ScopeName,
        counts: &Counts,
        open: &BTreeSet<ItemId>,
    ) -> Self {
        let mut collections = Vec::new();
        let mut pending: Vec<(ItemId, usize)> = library_rules::collections(state, tree)
            .into_iter()
            .rev()
            .map(|id| (id, 0))
            .collect();
        while let Some((id, depth)) = pending.pop() {
            let inside = folders_in(state, tree, Some(id));
            let shows = open.contains(&id) && !inside.is_empty();
            if let Some(folder) = state.folder(id) {
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
}

/// One entry of a column.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnEntry {
    pub kind: EntryKind,
    pub id: String,
    pub name: String,
    pub selected: bool,
    /// Whether the entry is a folder that holds entries.
    pub opens: bool,
    pub icon: Option<String>,
    /// The registrable name of a shortcut's site, which an entry without an icon shows as a tile, or empty for a folder.
    pub site: String,
    /// What a folder shows in place of its glyph, or empty for the glyph and for a shortcut.
    pub mark: String,
    /// The heading this entry and the shortcuts after it fall under, on the first of each, when the column gathers under more than one.
    pub heading: Option<String>,
}

/// One row of the list's outline: a folder or a shortcut, how deep it sits below the folder the list stands in, and whether a folder shows its rows.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineRow {
    pub kind: EntryKind,
    pub id: String,
    pub depth: usize,
    pub open: bool,
}

/// A collection a picker offers, named by its whole path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub id: String,
    pub path: String,
}

/// Every live collection by its path, in the order of the folders.
#[must_use]
pub fn choices(state: &State, tree: &Tree) -> Vec<Choice> {
    let mut found = Vec::new();
    let mut pending: Vec<ItemId> = library_rules::collections(state, tree);
    pending.reverse();
    while let Some(id) = pending.pop() {
        found.push(Choice {
            id: text(id),
            path: state.path(tree, Some(id)).join(PATH_SEPARATOR),
        });
        let mut inside: Vec<ItemId> = state
            .entries(tree, Some(id))
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Folder(folder) => Some(folder),
                Entry::Shortcut(_) => None,
            })
            .collect();
        inside.reverse();
        pending.extend(inside);
    }
    found
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
    pub sidebar: Sidebar,
    /// The heading of the scope: all, unsorted, recent, or a collection's path.
    pub title: String,
    pub count: usize,
    pub layout: LayoutName,
    pub sort: SortName,
    pub grouping: GroupName,
    pub search: String,
    /// The folders the cards and the list show before the shortcuts while they stand in a folder, each with the shortcuts it holds.
    pub folders: Vec<CollectionCount>,
    /// The rows of the list's outline, in their order, or none when the list shows the folders and the items as they come.
    pub outline: Vec<OutlineRow>,
    /// The columns the list draws after the name, in their order.
    pub list_columns: Vec<ColumnName>,
    /// The columns of the list a person hid.
    pub hidden_columns: Vec<ColumnName>,
    /// The shortcuts of the cards and the list, in their order, which the groups follow when there are any.
    pub items: Vec<Item>,
    /// The headings the items fall under in turn, or none when the window gathers nothing.
    pub groups: Vec<GroupView>,
    /// The columns of the folder walk, from the left.
    pub columns: Vec<Vec<ColumnEntry>>,
    /// The shortcut the columns preview.
    pub preview: Option<Item>,
    pub focus: Option<String>,
    pub selected: Vec<String>,
    pub notice: Option<Notice>,
    pub collections: Vec<Choice>,
    /// The folder that a new folder or a new link goes into, or none for the top.
    pub here: Option<String>,
    /// The folders the columns open, from the left, or, in the cards and the list, the folders from the top down to the one they stand in.
    pub trail: Vec<Step>,
    /// The themes the window wears, or none when no theme is known at all.
    pub theme: Option<Worn>,
    /// The browsers that keep bookmarks on this kind of host.
    pub browsers: Vec<BrowserChoice>,
    pub sync: SyncView,
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
