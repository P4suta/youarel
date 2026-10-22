//! The browser readers never panic, and every tree a reader reads writes back as the browser's own file that reads as the same tree.
//! The first byte chooses the reader: Chrome's JSON, Firefox's compressed backup, or Safari's property list.
#![no_main]

use youarel_import::{chrome, firefox, safari};

/// How many readers the first byte chooses among.
const READERS: u8 = 3;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((choice, bytes)) = data.split_first() else {
        return;
    };
    match choice % READERS {
        0 => {
            let Ok(text) = std::str::from_utf8(bytes) else {
                return;
            };
            if let Ok(items) = chrome::parse(text) {
                assert_eq!(
                    chrome::parse(&chrome::write(&items)).expect("a written file reads"),
                    items
                );
            }
        }
        1 => {
            if let Ok(items) = firefox::decode(bytes) {
                let encoded = firefox::encode(&items).expect("a decoded tree encodes");
                assert_eq!(
                    firefox::decode(&encoded).expect("an encoded backup decodes"),
                    items
                );
            }
        }
        _ => {
            if let Ok(items) = safari::parse(bytes) {
                let written = safari::write(&items).expect("a read list writes");
                assert_eq!(
                    safari::parse(&written).expect("a written list reads"),
                    items
                );
            }
        }
    }
});
