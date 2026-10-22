//! Decoding records never panics, every body it accepts is the one form of its records, which writes the same bytes again, and the store reads a state from records only with every link admitted again.
//! A state received as records or in the JSON form of an older build merges into itself unchanged.
#![no_main]

use youarel_core::record_rules::{decode_body, encode_body};
use youarel_link::Link;
use youarel_store::records;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(body) = decode_body(data) {
        let written = encode_body(&body).expect("a decoded body encodes");
        assert_eq!(written, data, "a body has one form");
        let again = decode_body(&written).expect("an encoded body decodes");
        assert_eq!(again, body, "the records read back as the body");
        let state = body.into_state();
        assert_eq!(state.merge(&state), state, "the merge is idempotent");
    }
    if let Ok(state) = records::decode(data) {
        for shortcut in state.shortcuts.values() {
            let admitted = Link::parse(&shortcut.link.value).expect("a link the store read parses");
            assert_eq!(
                admitted.as_str(),
                shortcut.link.value,
                "a link is admitted as it stands"
            );
        }
        assert_eq!(
            records::encode(&state).expect("a state the store read encodes"),
            data,
            "the store reads only the one form of a state's records"
        );
    }
    if let Ok(state) = records::decode_received(data) {
        assert_eq!(
            state.merge(&state),
            state,
            "a received state merges into itself unchanged"
        );
    }
});
