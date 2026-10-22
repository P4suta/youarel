//! Theme reading never panics, and every theme it reads writes a file that reads back as the same name, kind, and palette.
#![no_main]

use youarel_theme::{Theme, parse, write};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(theme) = parse(text) else {
        return;
    };
    let written = write(&theme).expect("a theme writes");
    let again = parse(&written).expect("a written theme reads");
    assert_eq!(
        Theme {
            id: theme.id.clone(),
            ..again
        },
        theme,
        "the theme reads back as itself"
    );
});
