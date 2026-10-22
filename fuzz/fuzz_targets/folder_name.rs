#![no_main]

use youarel_core::folder_rules::{Seen, counts, segment_name, segment_sequence};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(name) = core::str::from_utf8(data) {
        if let Some(sequence) = segment_sequence(name) {
            assert_eq!(
                segment_name(sequence).as_str(),
                name,
                "a sequence has one file name"
            );
        }
        let own = name.split('/').next();
        assert!(
            !counts(Seen::Touched(name), own),
            "an own chain never triggers a merge"
        );
        assert!(!counts(Seen::Read, own));
        assert!(counts(Seen::Rescan, own));
    }
});
