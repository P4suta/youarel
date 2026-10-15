//! Page titles never stop the reader, and a title is tidy: nonempty, trimmed, single-spaced, and within the length limit.
#![no_main]

use youarel_html::title::{LONGEST, title_of};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Some(title) = title_of(text) {
        assert!(!title.is_empty(), "a title has text");
        assert_eq!(title.trim(), title, "a title is trimmed");
        assert!(!title.contains("  "), "a title is single-spaced");
        assert!(title.chars().count() <= LONGEST, "a title fits the limit");
    }
});
