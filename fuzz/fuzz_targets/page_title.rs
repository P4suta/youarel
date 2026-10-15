//! Page titles and icon links never stop the reader, a title is tidy: nonempty, trimmed, single-spaced, and within the length limit, and the icon links are few, distinct, and nonempty.
#![no_main]

use youarel_html::icon::{MOST_ICONS, icons_of};
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
    let icons = icons_of(text);
    assert!(icons.len() <= MOST_ICONS, "a page yields few icons");
    for (index, icon) in icons.iter().enumerate() {
        assert!(!icon.is_empty(), "an icon link has an address");
        assert_eq!(icon.trim(), icon, "an icon link is trimmed");
        assert!(
            !icons.iter().skip(index + 1).any(|other| other == icon),
            "an icon link comes back once"
        );
    }
});
