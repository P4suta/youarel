//! State decoding never panics, every accepted state writes a document that reads back as itself and names no icon, and a state merged with itself is unchanged.
#![no_main]

use youarel_store::codec::{decode, encode};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(decoded) = decode(text) else {
        return;
    };
    let state = decoded.state;
    let written = encode(&state).expect("a decoded state encodes");
    let again = decode(&written).expect("an encoded state decodes");
    assert_eq!(again.state, state, "the document reads back as the state");
    assert_eq!(again.icons, [], "a written document names no icon");
    assert_eq!(
        encode(&again.state).expect("a decoded state encodes"),
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
    for shown in &decoded.icons {
        assert!(
            state
                .shortcuts
                .values()
                .any(|shortcut| !shortcut.removed.value && shortcut.link.value == shown.link),
            "an icon belongs to a live shortcut's link"
        );
    }
});
