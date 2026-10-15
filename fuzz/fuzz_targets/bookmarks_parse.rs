//! Bookmark files never stop the reader, and what it reads writes a file that reads back the same.
#![no_main]

use youarel_html::bookmarks::{parse, write};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let items = parse(text);
    assert_eq!(parse(&write(&items)), items, "the written file reads back");
});
