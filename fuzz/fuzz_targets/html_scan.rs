//! The scan never stops on any text, decoding undoes escaping, and escaped text scans back as itself.
#![no_main]

use youarel_html::scan::{Token, decode, escape, tokens};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let scanned = tokens(text);
    assert!(
        scanned
            .iter()
            .all(|token| !matches!(token, Token::Text(piece) if piece.is_empty())),
        "the scan yields no empty text"
    );
    let escaped = escape(text);
    assert_eq!(decode(&escaped), text, "decoding undoes escaping");
    let expected: Vec<Token> = if text.is_empty() {
        Vec::new()
    } else {
        vec![Token::Text(text.to_owned())]
    };
    assert_eq!(tokens(&escaped), expected, "escaped text scans as itself");
});
