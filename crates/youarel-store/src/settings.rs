//! The window's preferences on one device, which never sync.
//!
//! Only the window writes them, so they live beside the store outside its lock, and a lost write loses a preference and nothing of the state.
//! Decoding accepts only what encoding writes: known fields, known words, and every color of an imported theme.

use crate::codec;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use youarel_core::library_rules::{Grouping, Layout, Sort};
use youarel_core::state::ItemId;
use youarel_core::theme_rules::{self, ALL, Color, Kind, Palette, SLOTS, Slot};

/// The file of the window's preferences.
pub const SETTINGS_FILE: &str = "settings.json";
/// The format version this module writes and reads.
pub const FORMAT: u32 = 1;
/// The theme the window wears in light, until a person chooses one.
pub const DEFAULT_LIGHT: &str = "github-light";
/// The theme the window wears in dark, until a person chooses one.
pub const DEFAULT_DARK: &str = "github-dark";
/// The longest identifier or name of a theme the settings keep.
pub const LONGEST_THEME_TEXT: usize = 128;
/// The longest sync target the settings keep, a path or an address.
pub const LONGEST_TARGET: usize = 4096;

/// The words that name each layout in the document.
const LAYOUTS: [(Layout, &str); 3] = [
    (Layout::Cards, "cards"),
    (Layout::List, "list"),
    (Layout::Columns, "columns"),
];
/// The words that name each order in the document.
const SORTS: [(Sort, &str); 5] = [
    (Sort::Newest, "newest"),
    (Sort::Oldest, "oldest"),
    (Sort::Title, "title"),
    (Sort::Link, "link"),
    (Sort::Opened, "opened"),
];
/// The words that name each grouping in the document.
const GROUPINGS: [(Grouping, &str); 4] = [
    (Grouping::None, "none"),
    (Grouping::Site, "site"),
    (Grouping::Collection, "collection"),
    (Grouping::Added, "added"),
];
/// The words that name each column of the list in the document.
const COLUMNS: [(Column, &str); 3] = [
    (Column::Link, "link"),
    (Column::Collection, "collection"),
    (Column::Added, "added"),
];
/// The words that name each kind of theme in the document.
const KINDS: [(Kind, &str); 2] = [(Kind::Light, "light"), (Kind::Dark, "dark")];
/// The words that name each slot of a palette in the document.
const SLOT_WORDS: [(Slot, &str); SLOTS] = [
    (Slot::Background, "background"),
    (Slot::Sidebar, "sidebar"),
    (Slot::Surface, "surface"),
    (Slot::Raised, "raised"),
    (Slot::Border, "border"),
    (Slot::Text, "text"),
    (Slot::Muted, "muted"),
    (Slot::Accent, "accent"),
    (Slot::AccentText, "accent-text"),
    (Slot::Selection, "selection"),
    (Slot::Go, "go"),
    (Slot::GoText, "go-text"),
    (Slot::Danger, "danger"),
    (Slot::UrlScheme, "url-scheme"),
    (Slot::UrlSub, "url-sub"),
    (Slot::UrlName, "url-name"),
    (Slot::UrlSuffix, "url-suffix"),
    (Slot::UrlPort, "url-port"),
    (Slot::UrlRest, "url-rest"),
    (Slot::Thumbnail, "thumbnail"),
];

/// Which themes the window wears.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThemeChoice {
    /// Whether the window follows the system's light and dark appearance, or always wears the light choice.
    pub follow: bool,
    pub light: String,
    pub dark: String,
}

impl Default for ThemeChoice {
    fn default() -> Self {
        Self {
            follow: true,
            light: DEFAULT_LIGHT.to_owned(),
            dark: DEFAULT_DARK.to_owned(),
        }
    }
}

/// A theme a person imported from a file, kept whole so the file need not stay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Imported {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub palette: Palette,
}

/// How the window syncs on its own.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SyncChoice {
    /// Whether the window syncs on its own.
    pub enabled: bool,
    /// The shared folder or the relay's address the window syncs through, which `youarel sync` takes as well.
    pub target: String,
    /// When the last sync succeeded, in milliseconds since the Unix epoch.
    pub last: Option<u64>,
}

/// A column of the list that a person may hide; the name always shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Column {
    Link,
    /// The path of the collection that holds the shortcut, which only a listing across the folders shows.
    Collection,
    /// When the shortcut was added.
    Added,
}

/// Every column of the list that a person may hide, in the order the list shows them.
pub const HIDEABLE: [Column; 3] = [Column::Link, Column::Collection, Column::Added];

/// The window's preferences, which a person sets and the window never sets behind their back.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Settings {
    /// The layout the window opens with, whatever layout it showed last.
    pub layout: Layout,
    /// The collection the window opens at, or every shortcut for none, whatever place it showed last.
    pub start: Option<ItemId>,
    /// The order of a listing without a search.
    pub sort: Sort,
    /// How the cards and the list gather their shortcuts under headings.
    pub grouping: Grouping,
    /// The columns of the list that a person hid.
    pub hidden: BTreeSet<Column>,
    pub theme: ThemeChoice,
    /// The themes imported from files, which the bundled ones join.
    pub imported: Vec<Imported>,
    pub sync: SyncChoice,
}

/// Why a document is not the window's preferences.
#[derive(Debug)]
pub enum DecodeError {
    /// The text is not the document's JSON shape.
    Json(serde_json::Error),
    /// The document names another format version.
    Format(u32),
    /// A field holds a word this youarel does not know, or a color it cannot read.
    Word(String),
}

impl core::fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Json(_) => formatter.write_str("not the window's settings"),
            Self::Format(format) => write!(
                formatter,
                "the settings have format {format}, but this youarel reads format {FORMAT}"
            ),
            Self::Word(word) => write!(formatter, "`{word}` is no setting this youarel knows"),
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Format(_) | Self::Word(_) => None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ThemeDocument {
    follow: bool,
    light: String,
    dark: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ImportedDocument {
    id: String,
    name: String,
    kind: String,
    colors: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: u32,
    layout: String,
    /// The collection the window opens at in hexadecimal, which a document from before the setting lacks, and which every shortcut leaves out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    start: Option<String>,
    sort: String,
    /// The grouping, which a document from before groupings lacks, and which no grouping leaves out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group: Option<String>,
    /// The columns of the list a person hid, which a document from before hiding them lacks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hidden: Vec<String>,
    /// The theme choice, which a document from before themes lacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    theme: Option<ThemeDocument>,
    /// The imported themes, which a document from before themes lacks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    themes: Vec<ImportedDocument>,
    /// How the window syncs, which a document from before syncing from the window lacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sync: Option<SyncDocument>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SyncDocument {
    enabled: bool,
    target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last: Option<u64>,
}

/// A sync target as the settings keep it: no control character, and not too long.
fn sync_target(target: String) -> Result<String, DecodeError> {
    let fits = target.chars().count() <= LONGEST_TARGET && !target.chars().any(char::is_control);
    if fits {
        Ok(target)
    } else {
        Err(DecodeError::Word(target))
    }
}

/// The word that names `value` in `words`.
fn word<T: Eq + Copy>(words: &[(T, &'static str)], value: T) -> &'static str {
    words
        .iter()
        .find_map(|(known, word)| (*known == value).then_some(*word))
        .unwrap_or_default()
}

/// The value that `text` names in `words`.
fn named<T: Copy>(words: &[(T, &'static str)], text: &str) -> Result<T, DecodeError> {
    words
        .iter()
        .find_map(|(value, word)| (*word == text).then_some(*value))
        .ok_or_else(|| DecodeError::Word(text.to_owned()))
}

/// A theme's identifier or name as the settings keep it: not empty, no control character, and not too long.
fn theme_text(text: String) -> Result<String, DecodeError> {
    let fits = !text.is_empty()
        && text.chars().count() <= LONGEST_THEME_TEXT
        && !text.chars().any(char::is_control);
    if fits {
        Ok(text)
    } else {
        Err(DecodeError::Word(text))
    }
}

fn imported_document(theme: &Imported) -> ImportedDocument {
    ImportedDocument {
        id: theme.id.clone(),
        name: theme.name.clone(),
        kind: word(&KINDS, theme.kind).to_owned(),
        colors: theme
            .palette
            .colors()
            .map(|(slot, color)| (word(&SLOT_WORDS, slot).to_owned(), color.hex().to_string()))
            .collect(),
    }
}

/// The imported theme a document holds, which names every slot once with a color.
fn imported(document: ImportedDocument) -> Result<Imported, DecodeError> {
    let kind = named(&KINDS, &document.kind)?;
    let mut declared = [None; SLOTS];
    for (name, written) in &document.colors {
        let slot = named(&SLOT_WORDS, name)?;
        let color = Color::from_hex(written).ok_or_else(|| DecodeError::Word(written.clone()))?;
        if let Some(place) = declared.get_mut(slot.index()) {
            *place = Some(color);
        }
    }
    if let Some(missing) = ALL
        .into_iter()
        .find(|slot| declared.get(slot.index()).copied().flatten().is_none())
    {
        return Err(DecodeError::Word(word(&SLOT_WORDS, missing).to_owned()));
    }
    Ok(Imported {
        id: theme_text(document.id)?,
        name: theme_text(document.name)?,
        kind,
        palette: theme_rules::resolve(kind, &declared),
    })
}

/// The preferences as the document the window writes.
///
/// # Errors
///
/// Returns the serializer's error, which a document of strings, numbers, and booleans never meets.
pub fn encode(settings: &Settings) -> Result<String, serde_json::Error> {
    let document = Document {
        format: FORMAT,
        layout: word(&LAYOUTS, settings.layout).to_owned(),
        start: settings.start.map(|ItemId(id)| codec::hex(&id)),
        sort: word(&SORTS, settings.sort).to_owned(),
        group: (settings.grouping != Grouping::None)
            .then(|| word(&GROUPINGS, settings.grouping).to_owned()),
        hidden: settings
            .hidden
            .iter()
            .map(|column| word(&COLUMNS, *column).to_owned())
            .collect(),
        theme: Some(ThemeDocument {
            follow: settings.theme.follow,
            light: settings.theme.light.clone(),
            dark: settings.theme.dark.clone(),
        }),
        themes: settings.imported.iter().map(imported_document).collect(),
        sync: (settings.sync != SyncChoice::default()).then(|| SyncDocument {
            enabled: settings.sync.enabled,
            target: settings.sync.target.clone(),
            last: settings.sync.last,
        }),
    };
    let mut text = serde_json::to_string_pretty(&document)?;
    text.push('\n');
    Ok(text)
}

/// The preferences a document holds.
///
/// # Errors
///
/// Returns a [`DecodeError`] naming the first field that encoding would not have written.
pub fn decode(text: &str) -> Result<Settings, DecodeError> {
    let document: Document = serde_json::from_str(text).map_err(DecodeError::Json)?;
    if document.format != FORMAT {
        return Err(DecodeError::Format(document.format));
    }
    let theme = match document.theme {
        Some(theme) => ThemeChoice {
            follow: theme.follow,
            light: theme_text(theme.light)?,
            dark: theme_text(theme.dark)?,
        },
        None => ThemeChoice::default(),
    };
    Ok(Settings {
        layout: named(&LAYOUTS, &document.layout)?,
        start: document
            .start
            .map(|spelled| {
                codec::from_hex(&spelled)
                    .map(ItemId)
                    .ok_or(DecodeError::Word(spelled))
            })
            .transpose()?,
        sort: named(&SORTS, &document.sort)?,
        grouping: match document.group {
            Some(group) => named(&GROUPINGS, &group)?,
            None => Grouping::None,
        },
        hidden: document
            .hidden
            .iter()
            .map(|column| named(&COLUMNS, column))
            .collect::<Result<_, _>>()?,
        theme,
        imported: document
            .themes
            .into_iter()
            .map(imported)
            .collect::<Result<_, _>>()?,
        sync: match document.sync {
            Some(sync) => SyncChoice {
                enabled: sync.enabled,
                target: sync_target(sync.target)?,
                last: sync.last,
            },
            None => SyncChoice::default(),
        },
    })
}

#[cfg(test)]
mod tests;
