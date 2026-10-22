//! No image stops the icon adapter, and an icon it draws fits the limit and draws again as itself.
#![no_main]

use youarel_icon::{LONGEST_ICON, normalize};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(icon) = normalize(data) {
        assert!(icon.len() <= LONGEST_ICON, "an icon fits the limit");
        match normalize(&icon) {
            Ok(again) => assert_eq!(again, icon, "an icon draws again as itself"),
            Err(error) => panic!("an icon reads back: {error}"),
        }
    }
});
