//! Settings decoding never panics, and every accepted document writes settings that read back as themselves in a stable document.
#![no_main]

use youarel_store::settings::{decode, encode};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(settings) = decode(text) else {
        return;
    };
    let written = encode(&settings).expect("decoded settings encode");
    let again = decode(&written).expect("encoded settings decode");
    assert_eq!(again, settings, "the document reads back as the settings");
    assert_eq!(
        encode(&again).expect("decoded settings encode"),
        written,
        "the document is stable"
    );
});
