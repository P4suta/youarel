//! The themes of the window: the bundled palettes, and the palette a VS Code color theme names.
//!
//! A VS Code theme is JSON with comments and trailing commas, whose `colors` name the parts of the editor and whose `tokenColors` name the colors of code.
//! The window reads the editor's colors for its own parts and the colors of comments, functions, keywords, numbers, and strings for the runs of a link, and [`theme_rules::resolve`] fills what the theme leaves out.

pub mod bundled;
pub mod jsonc;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use youarel_core::theme_rules::{self, Color, Kind, Palette, SLOTS, Slot};

/// The longest theme file the window reads.
pub const LONGEST_FILE: usize = 1024 * 1024;
/// The longest name a theme keeps.
pub const LONGEST_NAME: usize = 64;
/// The name of a theme that names none.
pub const UNNAMED: &str = "Imported theme";
/// The prefix of an imported theme's identifier, which keeps it apart from every bundled one.
pub const IMPORTED_PREFIX: &str = "imported-";

/// The editor colors each slot reads, the first that the theme names winning.
const COLOR_KEYS: [(Slot, &[&str]); 14] = [
    (Slot::Background, &["editor.background"]),
    (
        Slot::Sidebar,
        &["sideBar.background", "activityBar.background"],
    ),
    (
        Slot::Surface,
        &[
            "editorGroupHeader.tabsBackground",
            "sideBarSectionHeader.background",
        ],
    ),
    (Slot::Raised, &["dropdown.background", "input.background"]),
    (
        Slot::Border,
        &[
            "panel.border",
            "sideBar.border",
            "editorGroup.border",
            "contrastBorder",
        ],
    ),
    (Slot::Text, &["editor.foreground", "foreground"]),
    (
        Slot::Muted,
        &[
            "descriptionForeground",
            "editorLineNumber.activeForeground",
            "editorLineNumber.foreground",
        ],
    ),
    (
        Slot::Accent,
        &["button.background", "focusBorder", "textLink.foreground"],
    ),
    (Slot::AccentText, &["button.foreground"]),
    (
        Slot::Selection,
        &[
            "list.activeSelectionBackground",
            "editor.selectionBackground",
        ],
    ),
    (
        Slot::Go,
        &[
            "extensionButton.prominentBackground",
            "terminal.ansiGreen",
            "gitDecoration.addedResourceForeground",
        ],
    ),
    (Slot::GoText, &["extensionButton.prominentForeground"]),
    (
        Slot::Danger,
        &[
            "errorForeground",
            "editorError.foreground",
            "terminal.ansiRed",
        ],
    ),
    (
        Slot::Thumbnail,
        &["editorWidget.background", "sideBar.background"],
    ),
];

/// The code scopes each run of a link reads, the first that a token rule colors winning.
const TOKEN_SCOPES: [(Slot, &[&str]); 6] = [
    (Slot::UrlScheme, &["comment.block", "comment"]),
    (Slot::UrlSub, &["comment.line", "comment"]),
    (
        Slot::UrlName,
        &["entity.name.function", "support.function", "entity.name"],
    ),
    (Slot::UrlSuffix, &["keyword.control", "keyword", "storage"]),
    (Slot::UrlPort, &["constant.numeric", "constant"]),
    (Slot::UrlRest, &["string.quoted", "string"]),
];

/// A theme the window can wear.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Theme {
    /// The identifier settings name it by.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub palette: Palette,
}

/// Why a file is not a theme the window can read.
#[derive(Debug)]
pub enum ThemeError {
    /// The file is larger than [`LONGEST_FILE`].
    Large(usize),
    /// The file is not the JSON of a color theme.
    Json(serde_json::Error),
    /// The file's JSON is no object, as every theme is.
    Shape,
}

impl core::fmt::Display for ThemeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Large(length) => write!(
                formatter,
                "the theme file holds {length} bytes, more than the {LONGEST_FILE} a theme may"
            ),
            Self::Json(_) | Self::Shape => formatter.write_str("not a VS Code color theme"),
        }
    }
}

impl std::error::Error for ThemeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Large(_) | Self::Shape => None,
        }
    }
}

/// The scopes a token rule names: one, a list, or a comma-separated list in one string.
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum Scopes {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct TokenSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    foreground: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TokenRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<Scopes>,
    #[serde(default)]
    settings: TokenSettings,
}

/// The parts of a VS Code color theme the window reads; it ignores the rest.
#[derive(Debug, Default, Deserialize, Serialize)]
struct Document {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default)]
    colors: BTreeMap<String, serde_json::Value>,
    #[serde(default, rename = "tokenColors")]
    token_colors: Vec<TokenRule>,
}

impl Scopes {
    fn names(&self) -> Vec<&str> {
        match self {
            Self::One(text) => text.split(',').map(str::trim).collect(),
            Self::Many(list) => list.iter().map(|scope| scope.trim()).collect(),
        }
    }
}

/// Whether a rule's scope selects a token of `scope`: the same scope, or one that it refines.
fn selects(rule: &str, scope: &str) -> bool {
    scope
        .strip_prefix(rule)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// The color of the most specific token rule that selects `scope`.
fn token_color(rules: &[TokenRule], scope: &str) -> Option<Color> {
    rules
        .iter()
        .filter_map(|rule| {
            let color = Color::from_hex(rule.settings.foreground.as_deref()?)?;
            let longest = rule
                .scope
                .as_ref()?
                .names()
                .into_iter()
                .filter(|name| selects(name, scope))
                .map(str::len)
                .max()?;
            Some((longest, color))
        })
        .max_by_key(|(longest, _)| *longest)
        .map(|(_, color)| color)
}

/// The editor color a key names, when it writes a color.
fn editor_color(colors: &BTreeMap<String, serde_json::Value>, key: &str) -> Option<Color> {
    colors.get(key)?.as_str().and_then(Color::from_hex)
}

/// The colors a document declares for each slot.
fn declared(document: &Document) -> [Option<Color>; SLOTS] {
    let mut declared = [None; SLOTS];
    for (slot, keys) in COLOR_KEYS {
        if let Some(place) = declared.get_mut(slot.index()) {
            *place = keys
                .iter()
                .find_map(|key| editor_color(&document.colors, key));
        }
    }
    for (slot, scopes) in TOKEN_SCOPES {
        if let Some(place) = declared.get_mut(slot.index()) {
            *place = scopes
                .iter()
                .find_map(|scope| token_color(&document.token_colors, scope));
        }
    }
    declared
}

/// The kind a document names, or the one its background implies.
fn kind(document: &Document, declared: &[Option<Color>; SLOTS]) -> Kind {
    match document.kind.as_deref() {
        Some("light" | "hcLight") => Kind::Light,
        Some("dark" | "hc" | "hcDark") => Kind::Dark,
        Some(_) | None => declared
            .get(Slot::Background.index())
            .copied()
            .flatten()
            .map_or(Kind::Dark, theme_rules::kind_of),
    }
}

/// A theme's name, trimmed, without control characters, and no longer than [`LONGEST_NAME`].
fn clean_name(name: Option<&str>) -> String {
    let cleaned: String = name
        .unwrap_or_default()
        .chars()
        .filter(|character| !character.is_control())
        .take(LONGEST_NAME)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        UNNAMED.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// The identifier of an imported theme of `name`: the prefix, then its letters and digits in lowercase joined by hyphens.
#[must_use]
pub fn imported_id(name: &str) -> String {
    let words: Vec<String> = name
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    format!("{IMPORTED_PREFIX}{}", words.join("-"))
}

/// The theme a VS Code color theme file describes.
///
/// # Errors
///
/// Returns [`ThemeError::Large`] for a file past [`LONGEST_FILE`], [`ThemeError::Json`] for text that is not a theme's JSON once its comments and trailing commas are gone, and [`ThemeError::Shape`] for JSON that is no object.
pub fn parse(text: &str) -> Result<Theme, ThemeError> {
    if text.len() > LONGEST_FILE {
        return Err(ThemeError::Large(text.len()));
    }
    let value: serde_json::Value =
        serde_json::from_str(&jsonc::strip(text)).map_err(ThemeError::Json)?;
    if !value.is_object() {
        return Err(ThemeError::Shape);
    }
    let document: Document = serde_json::from_value(value).map_err(ThemeError::Json)?;
    let declared = declared(&document);
    let kind = kind(&document, &declared);
    let name = clean_name(document.name.as_deref());
    Ok(Theme {
        id: imported_id(&name),
        name,
        kind,
        palette: theme_rules::resolve(kind, &declared),
    })
}

/// The theme as a VS Code color theme file, which [`parse`] reads back as the same name, kind, and palette.
///
/// # Errors
///
/// Returns the serializer's error, which a document of strings never meets.
pub fn write(theme: &Theme) -> Result<String, serde_json::Error> {
    let colors = COLOR_KEYS
        .iter()
        .filter_map(|(slot, keys)| {
            let key = keys.first()?;
            Some((
                (*key).to_owned(),
                serde_json::Value::String(theme.palette.color(*slot).hex().to_string()),
            ))
        })
        .collect();
    let token_colors = TOKEN_SCOPES
        .iter()
        .filter_map(|(slot, scopes)| {
            Some(TokenRule {
                scope: Some(Scopes::One((*scopes.first()?).to_owned())),
                settings: TokenSettings {
                    foreground: Some(theme.palette.color(*slot).hex().to_string()),
                },
            })
        })
        .collect();
    let document = Document {
        name: Some(theme.name.clone()),
        kind: Some(
            match theme.kind {
                Kind::Light => "light",
                Kind::Dark => "dark",
            }
            .to_owned(),
        ),
        colors,
        token_colors,
    };
    serde_json::to_string_pretty(&document)
}

/// The themes that ship with the window, the light ones first.
#[must_use]
pub fn bundled() -> Vec<Theme> {
    bundled::THEMES
        .iter()
        .map(|(id, name, kind, colors)| {
            let declared = colors.map(Color::from_hex);
            Theme {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
                kind: *kind,
                palette: theme_rules::resolve(*kind, &declared),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
