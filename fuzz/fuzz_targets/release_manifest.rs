//! Release manifests never stop the reader, and what it reads writes bytes that read back the same.
#![no_main]

use youarel_release::manifest::{decode, encode};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(manifest) = decode(data) else {
        return;
    };
    let written = encode(&manifest).expect("a decoded manifest encodes");
    let reread = decode(written.as_bytes()).expect("an encoded manifest decodes");
    assert_eq!(reread, manifest, "the written manifest reads back");
    assert_eq!(
        encode(&reread).expect("it encodes again"),
        written,
        "the encoding is canonical"
    );
});
