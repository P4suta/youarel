use super::{
    Column, DEFAULT_DARK, DEFAULT_LIGHT, DecodeError, FORMAT, GROUPINGS, Imported, LAYOUTS, SORTS,
    Settings, SyncChoice, ThemeChoice, decode, encode,
};
use std::collections::BTreeSet;
use youarel_core::library_rules::{Grouping, Layout, Sort};
use youarel_core::state::ItemId;
use youarel_core::theme_rules::{self, Color, Kind, SLOTS, Slot};

/// The collection the chosen settings open at, and its spelling in the document.
const START: ItemId = ItemId([0xab; 16]);
const START_TEXT: &str = "abababababababababababababababab";

fn imported() -> Imported {
    let mut declared = [None; SLOTS];
    declared[Slot::Background.index()] = Color::from_hex("#101010");
    declared[Slot::Accent.index()] = Color::from_hex("#3399ff80");
    Imported {
        id: "imported-midnight".to_owned(),
        name: "Midnight".to_owned(),
        kind: Kind::Dark,
        palette: theme_rules::resolve(Kind::Dark, &declared),
    }
}

fn chosen() -> Settings {
    Settings {
        layout: Layout::Columns,
        start: Some(START),
        sort: Sort::Link,
        grouping: Grouping::Site,
        hidden: BTreeSet::from([Column::Collection, Column::Added]),
        theme: ThemeChoice {
            follow: false,
            light: "imported-midnight".to_owned(),
            dark: "dracula".to_owned(),
        },
        imported: vec![imported()],
        sync: SyncChoice {
            enabled: true,
            target: RELAY.to_owned(),
            last: Some(LAST_SYNC),
        },
    }
}

/// The relay and the time of the last sync the chosen settings name.
const RELAY: &str = "https://relay.example";
const LAST_SYNC: u64 = 1_700_000_000_000;

#[test]
fn every_setting_reads_back_from_its_document() {
    for (layout, _) in LAYOUTS {
        for (sort, _) in SORTS {
            for (grouping, _) in GROUPINGS {
                let settings = Settings {
                    layout,
                    sort,
                    grouping,
                    ..Settings::default()
                };
                let text = encode(&settings).unwrap();
                assert_eq!(decode(&text).unwrap(), settings);
                assert_eq!(encode(&decode(&text).unwrap()).unwrap(), text);
            }
        }
    }
    let text = encode(&chosen()).unwrap();
    assert_eq!(decode(&text).unwrap(), chosen());
    assert_eq!(encode(&decode(&text).unwrap()).unwrap(), text);
    let default = encode(&Settings::default()).unwrap();
    assert!(
        default.contains("\"columns\"") && default.contains("\"title\""),
        "{default}"
    );
    assert!(
        default.contains(DEFAULT_LIGHT) && default.contains(DEFAULT_DARK),
        "{default}"
    );
    assert!(
        !default.contains("\"themes\""),
        "no imported theme writes no list"
    );
    assert!(!default.contains("\"group\""), "no grouping writes none");
    assert!(
        !default.contains("\"start\""),
        "a window that opens at every shortcut writes no place"
    );
    assert!(
        text.contains(&format!("\"start\": \"{START_TEXT}\"")),
        "{text}"
    );
    assert!(
        !default.contains("\"hidden\""),
        "every column shown writes none"
    );
}

#[test]
fn a_document_from_before_themes_reads_with_the_default_themes() {
    let older = format!("{{\"format\": {FORMAT}, \"layout\": \"list\", \"sort\": \"title\"}}");
    let read = decode(&older).unwrap();
    assert_eq!(read.theme, ThemeChoice::default());
    assert_eq!(read.imported, []);
    assert_eq!(read.sync, SyncChoice::default());
    assert!(
        !encode(&read).unwrap().contains("\"sync\""),
        "no sync choice writes none"
    );
}

#[test]
fn a_document_the_window_would_not_write_is_refused() {
    let text = encode(&chosen()).unwrap();
    let words = [
        text.replace("\"columns\"", "\"grid\""),
        text.replace("\"link\"", "\"Link\""),
        text.replace("\"link\"", "\"manual\""),
        text.replace(START_TEXT, "zz"),
        text.replace(START_TEXT, &START_TEXT.to_uppercase()),
        text.replace("\"group\": \"site\"", "\"group\": \"domain\""),
        text.replace("\"added\"", "\"opened\""),
        text.replace("\"kind\": \"dark\"", "\"kind\": \"dim\""),
        text.replace("\"url-name\"", "\"url-title\""),
        text.replace("\"#101010ff\"", "\"#10101\""),
        text.replacen("\"accent\": \"#3399ff80\",", "", 1),
        text.replace("\"name\": \"Midnight\"", "\"name\": \"\""),
        text.replace("\"dark\": \"dracula\"", "\"dark\": \"a\\nb\""),
        text.replace(
            "\"target\": \"https://relay.example\"",
            "\"target\": \"a\\tb\"",
        ),
        text.replace(
            "\"light\": \"imported-midnight\"",
            &format!("\"light\": \"{}\"", "x".repeat(200)),
        ),
    ];
    for refused in words {
        assert!(
            matches!(decode(&refused), Err(DecodeError::Word(_))),
            "{refused}"
        );
    }
    assert!(matches!(
        decode(&text.replace(&format!("\"format\": {FORMAT}"), "\"format\": 2")),
        Err(DecodeError::Format(2))
    ));
    let extra = text.replace("\"format\"", "\"window\": 1, \"format\"");
    let refused = decode(&extra).unwrap_err();
    assert!(matches!(refused, DecodeError::Json(_)));
    assert!(std::error::Error::source(&refused).is_some());
    for error in [
        refused,
        DecodeError::Format(2),
        DecodeError::Word("x".to_owned()),
    ] {
        assert_ne!(error.to_string(), "");
    }
}
