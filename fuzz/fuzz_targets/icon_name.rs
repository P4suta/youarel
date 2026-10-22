//! Reading an image's name never panics, and every name it reads is the one form its identifier writes again.
#![no_main]

use youarel_store::icons::IconId;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(image) = IconId::parse(text) {
        assert_eq!(image.name(), text, "a name has one form");
        assert_eq!(
            IconId::parse(&image.name()).expect("a written name reads back"),
            image,
            "an image keeps its name"
        );
    }
});
