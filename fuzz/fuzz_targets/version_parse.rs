//! Text is a release version that prints back as itself or is refused, and a version reads back from its print.
#![no_main]

use youarel_core::update_rules::Version;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Some(version) = Version::parse(text) {
        assert_eq!(version.to_string(), text, "a version prints as its text");
        assert_eq!(
            Version::parse(&version.to_string()),
            Some(version),
            "a version reads back"
        );
    }
});
