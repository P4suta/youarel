//! Panics on the one input its corpus holds.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    assert!(data != b"boom", "the fixture input reached the target");
});
