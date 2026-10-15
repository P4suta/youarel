//! Text is a key that reads back as itself or is refused, and between two keys of any length, or past either end, there is another key.
//!
//! The input is two texts split at the first `|`; a text that is not a key stands for the open end on its side.
#![no_main]

use youarel_core::position_rules::{Key, Unordered, between};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let (first, second) = text.split_once('|').unwrap_or((text, ""));
    let [low, high] = [first, second].map(|part| {
        let parsed = Key::parse(part);
        if let Some(key) = &parsed {
            assert_eq!(key.as_str(), part, "a key reads back as its text");
        }
        parsed
    });
    let ordered = match (&low, &high) {
        (Some(low), Some(high)) => low < high,
        _ => true,
    };
    match between(low.as_ref(), high.as_ref()) {
        Ok(made) => {
            assert!(ordered, "only ordered bounds make a key");
            assert_eq!(
                Key::parse(made.as_str()).as_ref(),
                Some(&made),
                "a made key is a key"
            );
            assert!(
                low.as_ref().is_none_or(|low| *low < made),
                "the key is above its lower bound"
            );
            assert!(
                high.as_ref().is_none_or(|high| made < *high),
                "the key is below its upper bound"
            );
        }
        Err(Unordered) => assert!(!ordered, "ordered bounds always make a key"),
    }
});
