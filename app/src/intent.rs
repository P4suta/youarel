//! What the window asks for, as its script sends it: plain words and identifiers in hexadecimal, which the library reads into the core's values.

use crate::library::Place;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use youarel_core::library_rules::{Grouping, Layout, Move, Scope};
use youarel_core::state::{Entry, ItemId};
use youarel_store::codec;

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
}

/// The item identifier spelled by hexadecimal text.
///
/// # Errors
///
/// Returns an error when the text spells no identifier.
pub fn item(text: &str) -> Result<ItemId> {
    codec::unhex(text)
        .map(ItemId)
        .with_context(|| format!("`{text}` is no identifier"))
}

/// The identifiers spelled by hexadecimal texts.
///
/// # Errors
///
/// Returns an error for the first text that spells no identifier.
pub fn items(texts: &[String]) -> Result<Vec<ItemId>> {
    texts.iter().map(|text| item(text)).collect()
}

/// The identifier spelled by optional hexadecimal text.
///
/// # Errors
///
/// Returns an error when the text spells no identifier.
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

    #[test]
    fn retired_product_intents_are_rejected() {
        for text in [
            r#"{"kind":"syncNow"}"#,
            r#"{"kind":"syncSettings","enabled":true,"target":"/shared"}"#,
            r#"{"kind":"import","bytes":[]}"#,
            r#"{"kind":"disclose","id":"ab","open":true}"#,
            r#"{"kind":"discloseSidebar","id":"ab","open":true}"#,
            r#"{"kind":"importBrowser","browser":"chrome"}"#,
            r#"{"kind":"layout","layout":"cards"}"#,
            r#"{"kind":"theme","follow":true,"light":"github-light","dark":"github-dark"}"#,
        ] {
            assert!(serde_json::from_str::<Intent>(text).is_err(), "{text}");
        }
    }
    use super::{
        EntryName, Extend, GroupName, Intent, KeyName, LayoutName, ScopeName, TargetName, item,
    };
    use crate::library::Place;
    use youarel_core::library_rules::{Grouping, Layout, Move, Scope};
    use youarel_core::state::{Entry, ItemId};

    #[test]
    fn retired_header_intents_are_rejected() {
        for request in [
            r#"{"kind":"sort","sort":"title"}"#,
            r#"{"kind":"search","text":"bookmark"}"#,
        ] {
            assert!(serde_json::from_str::<Intent>(request).is_err());
        }
    }

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
    }

    #[test]
    fn names_and_values_convert_both_ways() {
        for layout in [Layout::Cards, Layout::List, Layout::Columns] {
            assert_eq!(Layout::from(LayoutName::from(layout)), layout);
        }
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
    }
}
