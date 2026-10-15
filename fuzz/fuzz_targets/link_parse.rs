//! Link parsing never panics, admits only web links with a host, and reads its own serialization back as the same link.
#![no_main]

use youarel_core::link_rules::Scheme;
use youarel_link::Link;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(link) = Link::parse(text) else {
        return;
    };
    let serialized = link.as_str();
    let prefix = match link.scheme() {
        Scheme::Http => "http://",
        Scheme::Https => "https://",
    };
    assert!(
        serialized.starts_with(prefix),
        "an admitted link keeps its web scheme"
    );
    assert!(
        serialized.len() > prefix.len(),
        "an admitted link names a host"
    );
    let again = Link::parse(serialized);
    assert_eq!(
        again.as_ref(),
        Ok(&link),
        "the normalized form parses back to the same link"
    );
});
