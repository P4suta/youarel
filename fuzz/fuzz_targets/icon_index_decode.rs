//! Decoding an index of icons never panics, every index it accepts writes a document that reads back as itself, and each image's name reads back as its identifier.
#![no_main]

use youarel_store::icons::{IconId, decode, encode};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(index) = decode(text) else {
        return;
    };
    let written = encode(&index).expect("a decoded index encodes");
    let again = decode(&written).expect("an encoded index decodes");
    assert_eq!(again, index, "the document reads back as the index");
    assert_eq!(
        encode(&again).expect("a decoded index encodes"),
        written,
        "the document is stable"
    );
    for (host, icon) in &index {
        assert!(!host.is_empty(), "a host has a name");
        assert_eq!(
            IconId::parse(&icon.image.name()).expect("an image's name reads back"),
            icon.image,
            "an image keeps its name"
        );
    }
});
