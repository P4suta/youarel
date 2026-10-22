//! What the window asks for, as its script sends it: plain words and identifiers in hexadecimal, which the library reads into the core's values.

use crate::library::Place;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use youarel_core::browser_rules::Browser;
use youarel_core::library_rules::{Grouping, Layout, Move, Scope, Sort};
use youarel_core::state::{Entry, ItemId};
use youarel_store::codec;
use youarel_store::settings::Column;

/// A layout as the window names it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutName {
    Cards,
    List,
    Columns,
}

impl From<LayoutName> for Layout {
    fn from(name: LayoutName) -> Self {
        match name {
            LayoutName::Cards => Self::Cards,
            LayoutName::List => Self::List,
            LayoutName::Columns => Self::Columns,
        }
    }
}

impl From<Layout> for LayoutName {
    fn from(layout: Layout) -> Self {
        match layout {
            Layout::Cards => Self::Cards,
            Layout::List => Self::List,
            Layout::Columns => Self::Columns,
        }
    }
}

/// An order as the window names it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SortName {
    Newest,
    Oldest,
    Title,
    Link,
    Opened,
}

impl From<SortName> for Sort {
    fn from(name: SortName) -> Self {
        match name {
            SortName::Newest => Self::Newest,
            SortName::Oldest => Self::Oldest,
            SortName::Title => Self::Title,
            SortName::Link => Self::Link,
            SortName::Opened => Self::Opened,
        }
    }
}

impl From<Sort> for SortName {
    fn from(sort: Sort) -> Self {
        match sort {
            Sort::Newest => Self::Newest,
            Sort::Oldest => Self::Oldest,
            Sort::Title => Self::Title,
            Sort::Link => Self::Link,
            Sort::Opened => Self::Opened,
        }
    }
}

/// A grouping as the window names it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupName {
    None,
    Site,
    Collection,
    Added,
}

impl From<GroupName> for Grouping {
    fn from(name: GroupName) -> Self {
        match name {
            GroupName::None => Self::None,
            GroupName::Site => Self::Site,
            GroupName::Collection => Self::Collection,
            GroupName::Added => Self::Added,
        }
    }
}

impl From<Grouping> for GroupName {
    fn from(grouping: Grouping) -> Self {
        match grouping {
            Grouping::None => Self::None,
            Grouping::Site => Self::Site,
            Grouping::Collection => Self::Collection,
            Grouping::Added => Self::Added,
        }
    }
}

/// A column of the list that a person may hide, as the window names it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ColumnName {
    Link,
    Collection,
    Added,
}

impl From<ColumnName> for Column {
    fn from(name: ColumnName) -> Self {
        match name {
            ColumnName::Link => Self::Link,
            ColumnName::Collection => Self::Collection,
            ColumnName::Added => Self::Added,
        }
    }
}

impl From<Column> for ColumnName {
    fn from(column: Column) -> Self {
        match column {
            Column::Link => Self::Link,
            Column::Collection => Self::Collection,
            Column::Added => Self::Added,
        }
    }
}

/// A key that moves the selection, as the window names it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum KeyName {
    Up,
    Down,
    Left,
    Right,
    First,
    Last,
}

impl From<KeyName> for Move {
    fn from(name: KeyName) -> Self {
        match name {
            KeyName::Up => Self::Up,
            KeyName::Down => Self::Down,
            KeyName::Left => Self::Left,
            KeyName::Right => Self::Right,
            KeyName::First => Self::First,
            KeyName::Last => Self::Last,
        }
    }
}

/// How a click changes the selection.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Extend {
    /// The clicked shortcut alone.
    #[default]
    Replace,
    /// The clicked shortcut joins the selection or leaves it.
    Toggle,
    /// Every shortcut from the focused one to the clicked one.
    Range,
}

/// A place as the window names it, a scope of the library or the trash, which the view also sends back.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScopeName {
    All,
    Unsorted,
    Recent,
    Collection { id: String },
    Trash,
}

impl ScopeName {
    /// The place the name stands for.
    ///
    /// # Errors
    ///
    /// Returns an error when the identifier is no identifier.
    pub fn place(&self) -> Result<Place> {
        Ok(match self {
            Self::All => Place::Scope(Scope::All),
            Self::Unsorted => Place::Scope(Scope::Unsorted),
            Self::Recent => Place::Scope(Scope::Recent),
            Self::Collection { id } => Place::Scope(Scope::Collection(item(id)?)),
            Self::Trash => Place::Trash,
        })
    }
}

impl From<&Place> for ScopeName {
    fn from(place: &Place) -> Self {
        match place {
            Place::Scope(Scope::All) => Self::All,
            Place::Scope(Scope::Unsorted) => Self::Unsorted,
            Place::Scope(Scope::Recent) => Self::Recent,
            Place::Scope(Scope::Collection(id)) => Self::Collection { id: text(*id) },
            Place::Trash => Self::Trash,
        }
    }
}

/// An entry of a column as the window names it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EntryName {
    Folder { id: String },
    Shortcut { id: String },
}

impl EntryName {
    /// The entry the name stands for.
    ///
    /// # Errors
    ///
    /// Returns an error when the identifier is no identifier.
    pub fn entry(&self) -> Result<Entry> {
        Ok(match self {
            Self::Folder { id } => Entry::Folder(item(id)?),
            Self::Shortcut { id } => Entry::Shortcut(item(id)?),
        })
    }
}

/// What a move or a removal acts on, as the window names it: shortcuts and folders by their identifiers, or the selection as the library holds it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TargetName {
    Ids { ids: Vec<String> },
    Selection,
    Drag { token: String },
}

/// The fields of a shortcut as the edit sheet sends them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Fields {
    pub id: String,
    pub title: String,
    pub alias: String,
    pub note: String,
    /// The collection to file the shortcut in, or none for no collection.
    pub collection: Option<String>,
}

/// One thing the window asks for.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Intent {
    /// The view as the store holds it now.
    Show,
    Undo,
    Redo,
    Cut,
    CancelCut,
    Cancel,
    Paste {
        text: String,
    },
    EndDrag {
        token: String,
    },
    Spring {
        token: String,
        folder: String,
    },
    Scope {
        scope: ScopeName,
    },
    Back,
    Forward,
    ShowInFolder {
        id: Option<String>,
    },
    /// Shows the shortcuts in another layout until the window opens again.
    Layout {
        layout: LayoutName,
    },
    /// Chooses the layout and the collection the window opens with, or every shortcut for none, which the settings keep.
    Opening {
        layout: LayoutName,
        start: Option<String>,
    },
    Sort {
        sort: SortName,
    },
    /// Gathers the cards and the list under headings, or under none.
    Group {
        grouping: GroupName,
    },
    /// Shows or hides a column of the list.
    ShowColumn {
        column: ColumnName,
        shown: bool,
    },
    Search {
        text: String,
    },
    Type {
        key: char,
    },
    /// A click on a row, with the place the page shows it at, which the library checks before it trusts it.
    Select {
        id: String,
        #[serde(default)]
        extend: Extend,
        #[serde(default)]
        place: Option<usize>,
    },
    /// Selects every folder and shortcut the cards or the list show.
    SelectAll,
    /// Opens or closes a folder of the list's outline in place.
    Disclose {
        id: String,
        open: bool,
        #[serde(default)]
        deep: bool,
    },
    /// Opens or closes a collection of the sidebar's tree.
    DiscloseSidebar {
        id: String,
        open: bool,
    },
    /// Gives a collection a mark to show in place of its glyph, or clears it with an empty one.
    MarkCollection {
        id: String,
        mark: String,
    },
    /// A key that moves the selection, with how many cards a row holds.
    Step {
        key: KeyName,
        across: usize,
        #[serde(default)]
        extend: bool,
    },
    /// A click on an entry of the column at `depth`, with the place the page shows it at.
    Column {
        depth: usize,
        entry: EntryName,
        #[serde(default)]
        place: Option<usize>,
        #[serde(default)]
        extend: Extend,
    },
    /// Opens the shortcut, or the focused one.
    Open {
        id: Option<String>,
    },
    Edit {
        fields: Fields,
    },
    /// Gives a shortcut the title a person typed over its old one.
    Retitle {
        id: String,
        title: String,
    },
    /// Files the shortcuts and folders dropped on a collection, or on no collection.
    File {
        target: TargetName,
        collection: Option<String>,
    },
    /// Moves shortcuts and folders to the trash, where a folder hides everything inside it.
    Trash {
        target: TargetName,
    },
    /// Puts shortcuts and folders in the trash back where they were.
    PutBack {
        target: TargetName,
    },
    /// Deletes shortcuts and folders for good, a folder with everything inside it.
    Remove {
        target: TargetName,
    },
    /// Deletes everything in the trash for good.
    EmptyTrash,
    NewCollection {
        name: String,
        parent: Option<String>,
    },
    RenameCollection {
        id: String,
        name: String,
    },
    /// Imports a bookmark file of any kind a browser exports or keeps, as its bytes.
    Import {
        bytes: Vec<u8>,
    },
    /// Chooses the themes the window wears, and whether it follows the system's appearance.
    Theme {
        follow: bool,
        light: String,
        dark: String,
    },
    /// Imports a VS Code color theme file's text.
    ImportTheme {
        text: String,
    },
    /// Removes an imported theme.
    RemoveTheme {
        id: String,
    },
    /// Imports the bookmarks a browser keeps on this device.
    ImportBrowser {
        browser: BrowserChoice,
    },
    /// Chooses whether the window syncs on its own, and through which folder or relay.
    SyncSettings {
        enabled: bool,
        target: String,
    },
    /// Syncs at the next tick, whatever the schedule says.
    SyncNow,
}

impl Intent {
    /// Whether the intent changes the state, which the other devices then need.
    #[must_use]
    pub const fn edits(&self) -> bool {
        match self {
            Self::Edit { .. }
            | Self::Retitle { .. }
            | Self::File { .. }
            | Self::Trash { .. }
            | Self::PutBack { .. }
            | Self::Remove { .. }
            | Self::EmptyTrash
            | Self::NewCollection { .. }
            | Self::RenameCollection { .. }
            | Self::MarkCollection { .. }
            | Self::Import { .. }
            | Self::ImportBrowser { .. } => true,
            Self::Show
            | Self::Undo
            | Self::Redo
            | Self::Cut
            | Self::CancelCut
            | Self::Cancel
            | Self::Paste { .. }
            | Self::EndDrag { .. }
            | Self::Spring { .. }
            | Self::Back
            | Self::Forward
            | Self::ShowInFolder { .. }
            | Self::Scope { .. }
            | Self::Layout { .. }
            | Self::Opening { .. }
            | Self::Sort { .. }
            | Self::Group { .. }
            | Self::ShowColumn { .. }
            | Self::Search { .. }
            | Self::Type { .. }
            | Self::Select { .. }
            | Self::SelectAll
            | Self::Disclose { .. }
            | Self::DiscloseSidebar { .. }
            | Self::Step { .. }
            | Self::Column { .. }
            | Self::Open { .. }
            | Self::Theme { .. }
            | Self::ImportTheme { .. }
            | Self::RemoveTheme { .. }
            | Self::SyncSettings { .. }
            | Self::SyncNow => false,
        }
    }
}

/// A browser as the window names it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BrowserChoice {
    Safari,
    Chrome,
    Firefox,
}

impl From<BrowserChoice> for Browser {
    fn from(choice: BrowserChoice) -> Self {
        match choice {
            BrowserChoice::Safari => Self::Safari,
            BrowserChoice::Chrome => Self::Chrome,
            BrowserChoice::Firefox => Self::Firefox,
        }
    }
}

impl From<Browser> for BrowserChoice {
    fn from(browser: Browser) -> Self {
        match browser {
            Browser::Safari => Self::Safari,
            Browser::Chrome => Self::Chrome,
            Browser::Firefox => Self::Firefox,
        }
    }
}

/// The identifier `text` spells in hexadecimal.
///
/// # Errors
///
/// Returns an error for any other text.
pub fn item(text: &str) -> Result<ItemId> {
    codec::unhex(text)
        .map(ItemId)
        .with_context(|| format!("`{text}` is no identifier"))
}

/// The identifiers `texts` spell.
///
/// # Errors
///
/// Returns an error for the first text that spells none.
pub fn items(texts: &[String]) -> Result<Vec<ItemId>> {
    texts.iter().map(|text| item(text)).collect()
}

/// The identifier, if any, `text` spells.
///
/// # Errors
///
/// Returns an error for a text that spells none.
pub fn optional(text: Option<&str>) -> Result<Option<ItemId>> {
    text.map(item).transpose()
}

/// An identifier in hexadecimal, as the window names it.
#[must_use]
pub fn text(id: ItemId) -> String {
    codec::hex(&id.0)
}

#[cfg(test)]
mod tests {
    use super::{
        EntryName, Extend, GroupName, Intent, KeyName, LayoutName, ScopeName, SortName, TargetName,
        item,
    };
    use crate::library::Place;
    use youarel_core::library_rules::{Grouping, Layout, Move, Scope, Sort};
    use youarel_core::state::{Entry, ItemId};

    #[test]
    fn the_window_s_words_read_as_intents() {
        let read = |json: &str| serde_json::from_str::<Intent>(json).unwrap();
        assert_eq!(read(r#"{"kind":"show"}"#), Intent::Show);
        assert_eq!(
            read(r#"{"kind":"select","id":"ab"}"#),
            Intent::Select {
                id: "ab".to_owned(),
                extend: Extend::Replace,
                place: None,
            }
        );
        assert_eq!(
            read(r#"{"kind":"select","id":"ab","extend":"range","place":7}"#),
            Intent::Select {
                id: "ab".to_owned(),
                extend: Extend::Range,
                place: Some(7),
            }
        );
        assert_eq!(
            read(r#"{"kind":"remove","target":{"kind":"selection"}}"#),
            Intent::Remove {
                target: TargetName::Selection
            }
        );
        assert_eq!(
            read(r#"{"kind":"step","key":"down","across":3}"#),
            Intent::Step {
                key: KeyName::Down,
                across: 3,
                extend: false,
            }
        );
        assert_eq!(
            read(r#"{"kind":"scope","scope":{"kind":"collection","id":"ab"}}"#),
            Intent::Scope {
                scope: ScopeName::Collection {
                    id: "ab".to_owned()
                }
            }
        );
        assert_eq!(
            read(r#"{"kind":"scope","scope":{"kind":"trash"}}"#),
            Intent::Scope {
                scope: ScopeName::Trash
            }
        );
        assert!(serde_json::from_str::<Intent>(r#"{"kind":"tag","ids":[],"tag":"x"}"#).is_err());
        assert!(serde_json::from_str::<Intent>(r#"{"kind":"launch"}"#).is_err());
        assert!(
            serde_json::from_str::<Intent>(r#"{"kind":"removeCollection","id":"ab"}"#).is_err()
        );
        for edit in [
            r#"{"kind":"remove","target":{"kind":"ids","ids":[]}}"#,
            r#"{"kind":"trash","target":{"kind":"selection"}}"#,
            r#"{"kind":"putBack","target":{"kind":"selection"}}"#,
            r#"{"kind":"emptyTrash"}"#,
        ] {
            assert!(read(edit).edits(), "{edit}");
        }
        assert!(!read(r#"{"kind":"syncNow"}"#).edits());
    }

    #[test]
    fn names_and_values_convert_both_ways() {
        for layout in [Layout::Cards, Layout::List, Layout::Columns] {
            assert_eq!(Layout::from(LayoutName::from(layout)), layout);
        }
        for sort in [
            Sort::Newest,
            Sort::Oldest,
            Sort::Title,
            Sort::Link,
            Sort::Opened,
        ] {
            assert_eq!(Sort::from(SortName::from(sort)), sort);
        }
        assert!(
            serde_json::from_str::<Intent>(r#"{"kind":"sort","sort":"manual"}"#).is_err(),
            "the window sorts only automatically"
        );
        for grouping in [
            Grouping::None,
            Grouping::Site,
            Grouping::Collection,
            Grouping::Added,
        ] {
            assert_eq!(Grouping::from(GroupName::from(grouping)), grouping);
        }
        let keys = [
            (KeyName::Up, Move::Up),
            (KeyName::Down, Move::Down),
            (KeyName::Left, Move::Left),
            (KeyName::Right, Move::Right),
            (KeyName::First, Move::First),
            (KeyName::Last, Move::Last),
        ];
        for (name, key) in keys {
            assert_eq!(Move::from(name), key);
        }
        let id = ItemId([0xab; 16]);
        for place in [
            Place::Scope(Scope::All),
            Place::Scope(Scope::Unsorted),
            Place::Scope(Scope::Recent),
            Place::Scope(Scope::Collection(id)),
            Place::Trash,
        ] {
            assert_eq!(ScopeName::from(&place).place().unwrap(), place);
        }
        assert_eq!(item(&super::text(id)).unwrap(), id);
        assert!(item("zz").is_err());
        let folder = EntryName::Folder {
            id: super::text(id),
        };
        assert_eq!(folder.entry().unwrap(), Entry::Folder(id));
        let shortcut = EntryName::Shortcut {
            id: super::text(id),
        };
        assert_eq!(shortcut.entry().unwrap(), Entry::Shortcut(id));
        assert_eq!(super::items(&[super::text(id)]).unwrap(), [id]);
        assert_eq!(super::optional(None).unwrap(), None);
        for browser in youarel_core::browser_rules::BROWSERS {
            let choice = super::BrowserChoice::from(browser);
            assert_eq!(youarel_core::browser_rules::Browser::from(choice), browser);
        }
    }
}
