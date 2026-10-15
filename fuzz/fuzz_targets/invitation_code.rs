//! Text is an invitation code that reads back from its own spelling or is refused.
#![no_main]

use youarel_crypto::pairing::Code;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Some(code) = Code::parse(text) {
        let spelled = code.text();
        assert_eq!(
            Code::parse(&spelled),
            Some(code),
            "a code reads back from its spelling"
        );
        assert_eq!(
            Code::parse(&spelled.to_lowercase()),
            Some(code),
            "a code ignores case"
        );
    }
});
