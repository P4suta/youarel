//! State decoding never panics, every accepted state writes a document that reads back as itself, and a state merged with itself is unchanged.
#![no_main]

use youarel_store::codec::{decode, encode};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(state) = decode(text) else {
        return;
    };
    let written = encode(&state).expect("a decoded state encodes");
    let again = decode(&written).expect("an encoded state decodes");
    assert_eq!(again, state, "the document reads back as the state");
    assert_eq!(
        encode(&again).expect("a decoded state encodes"),
        written,
        "the document is stable"
    );
    assert_eq!(state.merge(&state), state, "the merge is idempotent");
    let tree = state.tree();
    for folder in state.folders.keys() {
        assert!(
            tree.lineage(Some(*folder)).len() <= state.folders.len(),
            "the tree has no cycle"
        );
    }
});
