use super::{
    IMPORTED_PREFIX, LONGEST_FILE, Theme, ThemeError, UNNAMED, bundled, imported_id, parse, write,
};
use youarel_core::theme_rules::{Color, Kind, Slot};

/// The size of the longest theme file the window promises to read.
const MEBIBYTE: usize = 1 << 20;

fn hex(color: Color) -> String {
    color.hex().to_string()
}

#[test]
fn every_bundled_theme_writes_each_color_and_keeps_its_own_identifier() {
    let themes = bundled();
    assert_eq!(themes.len(), 16);
    let mut ids: Vec<&str> = themes.iter().map(|theme| theme.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), themes.len(), "the identifiers differ");
    for (id, name, kind, colors) in bundled::THEMES {
        for color in colors {
            assert!(Color::from_hex(color).is_some(), "{id}: {color}");
        }
        let theme = themes.iter().find(|theme| theme.id == id).unwrap();
        assert_eq!((theme.name.as_str(), theme.kind), (name, kind));
        assert_eq!(
            hex(theme.palette.color(Slot::Background)),
            format!("{}ff", colors[0])
        );
    }
    let light = themes
        .iter()
        .filter(|theme| theme.kind == Kind::Light)
        .count();
    assert_eq!(light, 8);
    assert!(
        !themes
            .iter()
            .any(|theme| theme.id.starts_with(IMPORTED_PREFIX))
    );
}

#[test]
fn a_written_theme_reads_back_as_itself() {
    for theme in bundled() {
        let text = write(&theme).unwrap();
        let read = parse(&text).unwrap();
        assert_eq!(
            Theme {
                id: theme.id.clone(),
                ..read
            },
            theme,
            "{}",
            theme.name
        );
    }
}

#[test]
fn a_vs_code_theme_names_the_window_s_colors() {
    let text = r##"{
        // A theme as VS Code writes one, with comments and trailing commas.
        "name": "  Midnight\u0007 ",
        "type": "dark",
        "colors": {
            "editor.background": "#101010",
            "editor.foreground": "#e0e0e0",
            "focusBorder": "#3399ff80",
            "sideBar.background": 7, /* not a color */
            "terminal.ansiGreen": "#00ff00",
        },
        "tokenColors": [
            { "scope": "comment", "settings": { "foreground": "#777777" } },
            { "scope": ["keyword", "storage.type"], "settings": { "foreground": "#ff0000", "fontStyle": "bold" } },
            { "scope": "keyword.control, keyword.other", "settings": { "foreground": "#ff8800" } },
            { "scope": "entity.name.function", "settings": { "foreground": "#00aaff" } },
            { "settings": { "foreground": "#ffffff", "background": "#000000" } },
            { "scope": "string", "settings": {} },
        ],
    }"##;
    let theme = parse(text).unwrap();
    assert_eq!(theme.name, "Midnight");
    assert_eq!(theme.id, "imported-midnight");
    assert_eq!(theme.kind, Kind::Dark);
    let color = |slot| hex(theme.palette.color(slot));
    assert_eq!(color(Slot::Background), "#101010ff");
    assert_eq!(
        color(Slot::Sidebar),
        "#101010ff",
        "a value that is no color falls back"
    );
    assert_eq!(color(Slot::Accent), "#3399ff80");
    assert_eq!(color(Slot::Go), "#00ff00ff");
    assert_eq!(color(Slot::UrlScheme), "#777777ff");
    assert_eq!(
        color(Slot::UrlSuffix),
        "#ff8800ff",
        "the most specific rule wins"
    );
    assert_eq!(color(Slot::UrlName), "#00aaffff");
    assert_eq!(
        color(Slot::UrlRest),
        "#e0e0e0ff",
        "a rule without a color falls back to the text"
    );
    assert_eq!(color(Slot::UrlPort), "#3399ff80");
}

#[test]
fn a_theme_without_a_kind_takes_its_background_s_and_without_a_name_gets_one() {
    let light = parse(r##"{ "colors": { "editor.background": "#fafafa" } }"##).unwrap();
    assert_eq!((light.kind, light.name.as_str()), (Kind::Light, UNNAMED));
    let unknown = parse(r#"{ "type": "vibrant" }"#).unwrap();
    assert_eq!(unknown.kind, Kind::Dark);
    let hc_light = parse(r#"{ "type": "hcLight" }"#).unwrap();
    assert_eq!(hc_light.kind, Kind::Light);
    let long = parse(&format!(r#"{{ "name": "{}" }}"#, "n".repeat(200))).unwrap();
    assert_eq!(long.name.chars().count(), super::LONGEST_NAME);
}

#[test]
fn what_is_no_theme_is_refused() {
    assert!(matches!(parse("not json"), Err(ThemeError::Json(_))));
    assert!(matches!(parse("[]"), Err(ThemeError::Shape)));
    assert!(matches!(
        parse(r#"{ "colors": [] }"#),
        Err(ThemeError::Json(_))
    ));
    assert!(std::error::Error::source(&ThemeError::Shape).is_none());
    assert_ne!(ThemeError::Shape.to_string(), "");
    let large = " ".repeat(LONGEST_FILE + 1);
    assert!(matches!(parse(&large), Err(ThemeError::Large(_))));
    // A theme of a whole mebibyte, the longest the window promises to read, still reads.
    let theme = r#"{ "name": "Wide", "type": "dark" }"#;
    let longest = format!("{theme}{}", " ".repeat(MEBIBYTE - theme.len()));
    assert_eq!(longest.len(), MEBIBYTE);
    assert_eq!(parse(&longest).unwrap().name, "Wide");
    for error in [parse("{").unwrap_err(), ThemeError::Large(1)] {
        assert_ne!(error.to_string(), "");
    }
    assert!(std::error::Error::source(&parse("{").unwrap_err()).is_some());
    assert!(std::error::Error::source(&ThemeError::Large(1)).is_none());
}

#[test]
fn an_imported_theme_s_identifier_keeps_its_words() {
    assert_eq!(
        imported_id("One Dark Pro (Flat)"),
        "imported-one-dark-pro-flat"
    );
    assert_eq!(imported_id("東京 Night"), "imported-東京-night");
    assert_eq!(imported_id("!!!"), IMPORTED_PREFIX);
}
