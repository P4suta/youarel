//! Text is a recovery key that reads back from its own spelling or is refused, and every secret's spelling reads back as that secret.
#![no_main]

use youarel_crypto::recovery::{RecoveryKey, SECRET};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Some(secret) = data.first_chunk::<SECRET>() {
        let key = RecoveryKey::from_secret(*secret);
        assert_eq!(
            RecoveryKey::parse(&key.text()),
            Some(key),
            "a secret reads back from its spelling"
        );
    }
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Some(key) = RecoveryKey::parse(text) {
        let spelled = key.text();
        assert_eq!(
            RecoveryKey::parse(&spelled),
            Some(key.clone()),
            "a key reads back from its spelling"
        );
        assert_eq!(
            RecoveryKey::parse(&spelled.to_lowercase()),
            Some(key),
            "a key ignores case"
        );
    }
});
