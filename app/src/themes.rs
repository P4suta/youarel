//! The themes the window can wear, as the page reads them: each palette by the custom properties of its style sheet.

use serde::Serialize;
use std::collections::BTreeMap;
use youarel_core::theme_rules::{Kind, Palette, SLOTS, Slot};
use youarel_store::settings::{DEFAULT_DARK, DEFAULT_LIGHT, Settings};

/// The custom property of the style sheet that each slot sets.
const PROPERTIES: [(Slot, &str); SLOTS] = [
    (Slot::Background, "--background"),
    (Slot::Sidebar, "--sidebar"),
    (Slot::Surface, "--surface"),
    (Slot::Raised, "--raised"),
    (Slot::Border, "--border"),
    (Slot::Text, "--text"),
    (Slot::Muted, "--muted"),
    (Slot::Accent, "--accent"),
    (Slot::AccentText, "--accent-text"),
    (Slot::Selection, "--selection"),
    (Slot::Go, "--go"),
    (Slot::GoText, "--go-text"),
    (Slot::Danger, "--danger"),
    (Slot::UrlScheme, "--url-scheme"),
    (Slot::UrlSub, "--url-sub"),
    (Slot::UrlName, "--url-name"),
    (Slot::UrlSuffix, "--url-suffix"),
    (Slot::UrlPort, "--url-port"),
    (Slot::UrlRest, "--url-rest"),
    (Slot::Thumbnail, "--thumbnail"),
];

/// A kind of theme as the page names it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KindName {
    Light,
    Dark,
}

impl From<Kind> for KindName {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Light => Self::Light,
            Kind::Dark => Self::Dark,
        }
    }
}

/// One theme as the page draws it and wears it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeOption {
    pub id: String,
    pub name: String,
    pub kind: KindName,
    /// Whether a person imported it from a file, and so may remove it.
    pub imported: bool,
    /// Each custom property of the style sheet with its color as `#rrggbbaa`.
    pub colors: BTreeMap<String, String>,
}

fn option(id: &str, name: &str, kind: Kind, palette: &Palette) -> ThemeOption {
    ThemeOption {
        id: id.to_owned(),
        name: name.to_owned(),
        kind: kind.into(),
        imported: false,
        colors: PROPERTIES
            .iter()
            .map(|(slot, property)| {
                (
                    (*property).to_owned(),
                    palette.color(*slot).hex().to_string(),
                )
            })
            .collect(),
    }
}

/// Every theme: the bundled ones, then the imported ones.
#[must_use]
pub fn all(settings: &Settings) -> Vec<ThemeOption> {
    let bundled = youarel_theme::bundled()
        .into_iter()
        .map(|theme| option(&theme.id, &theme.name, theme.kind, &theme.palette));
    let imported = settings.imported.iter().map(|theme| ThemeOption {
        imported: true,
        ..option(&theme.id, &theme.name, theme.kind, &theme.palette)
    });
    bundled.chain(imported).collect()
}

/// The themes the window wears, and whether it follows the system's appearance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Worn {
    pub follow: bool,
    /// The theme of a light appearance, which the window wears always when it does not follow.
    pub light: ThemeOption,
    pub dark: ThemeOption,
}

/// The theme of `id`, else the one of `default`, else the first one.
fn chosen(themes: &[ThemeOption], id: &str, default: &str) -> Option<ThemeOption> {
    themes
        .iter()
        .find(|theme| theme.id == id)
        .or_else(|| themes.iter().find(|theme| theme.id == default))
        .or_else(|| themes.first())
        .cloned()
}

/// The themes the settings choose, each falling back to its default when it is gone.
#[must_use]
pub fn worn(settings: &Settings) -> Option<Worn> {
    let themes = all(settings);
    Some(Worn {
        follow: settings.theme.follow,
        light: chosen(&themes, &settings.theme.light, DEFAULT_LIGHT)?,
        dark: chosen(&themes, &settings.theme.dark, DEFAULT_DARK)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{KindName, PROPERTIES, all, worn};
    use youarel_core::theme_rules::ALL;
    use youarel_store::settings::{DEFAULT_DARK, DEFAULT_LIGHT, Settings, ThemeChoice};

    #[test]
    fn every_slot_sets_its_own_property() {
        for (slot, (named, property)) in ALL.into_iter().zip(PROPERTIES) {
            assert_eq!(slot, named);
            assert!(property.starts_with("--"));
        }
    }

    #[test]
    fn the_worn_themes_fall_back_to_the_defaults() {
        let settings = Settings::default();
        let themes = all(&settings);
        assert_eq!(themes.len(), 16);
        assert!(
            themes
                .iter()
                .all(|theme| theme.colors.len() == PROPERTIES.len() && !theme.imported)
        );
        let shown = worn(&settings).unwrap();
        assert_eq!(
            (shown.light.id.as_str(), shown.dark.id.as_str()),
            (DEFAULT_LIGHT, DEFAULT_DARK)
        );
        assert_eq!(
            (shown.light.kind, shown.dark.kind),
            (KindName::Light, KindName::Dark)
        );
        let gone = Settings {
            theme: ThemeChoice {
                follow: false,
                light: "removed".to_owned(),
                dark: "dracula".to_owned(),
            },
            ..Settings::default()
        };
        let fallen = worn(&gone).unwrap();
        assert_eq!(
            (
                fallen.follow,
                fallen.light.id.as_str(),
                fallen.dark.id.as_str()
            ),
            (false, DEFAULT_LIGHT, "dracula")
        );
    }
}
