#![no_main]

use libfuzzer_sys::fuzz_target;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use youarel_import::icons::{Format, Image, Snapshot};

const MOST_INPUT: usize = 64 * 1024;
const PAGE: &str = "https://cache.example/page";

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > MOST_INPUT {
        return;
    }
    let directory = tempfile::tempdir().expect("create an isolated cache fixture");
    let path = directory.path().join("icons.sqlite");
    youarel_store::fs::replace(&path, bytes).expect("write the isolated fixture");
    let link = youarel_link::Link::parse(PAGE).expect("the fixed admitted page");
    for format in [Format::Chromium, Format::Firefox, Format::Safari] {
        let Ok(snapshot) = Snapshot::open(&path, format, Arc::new(AtomicBool::new(false))) else {
            continue;
        };
        let Ok(candidates) = snapshot.candidates(&link) else {
            continue;
        };
        assert!(candidates.len() <= youarel_core::icon_rules::MOST_CANDIDATES);
        for candidate in candidates {
            match candidate.image {
                Image::Bytes(image) => {
                    drop(youarel_icon::normalize(&image));
                }
                Image::SafariFile(name) => {
                    assert_eq!(name.len(), 32);
                    assert!(name.bytes().all(|byte| byte.is_ascii_hexdigit()));
                }
            }
        }
    }
});
