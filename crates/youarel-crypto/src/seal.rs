//! Sealed bytes: XChaCha20-Poly1305 under a 256-bit key with a random nonce, and a commitment to the key that a reader checks first.
//!
//! The commitment binds the ciphertext to one key, so no ciphertext opens under two keys to two different texts.

use crate::{CryptoError, KEY, Label, derive, equal, random};
use libcrux_chacha20poly1305::TAG_LEN;
use libcrux_chacha20poly1305::xchacha20_poly1305::{self, NONCE_LEN};

/// The bytes of a sealing nonce.
pub const NONCE: usize = NONCE_LEN;
/// The bytes of the tag a sealed text carries.
pub const TAG: usize = TAG_LEN;
/// The bytes of a key commitment.
pub const COMMITMENT: usize = 32;

/// Sealed bytes: the commitment to the key, the nonce, and the ciphertext with its tag.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sealed {
    pub commitment: [u8; COMMITMENT],
    pub nonce: [u8; NONCE],
    pub ciphertext: Vec<u8>,
}

/// The commitment to a key.
///
/// # Errors
///
/// Returns [`CryptoError::Primitive`] when the derivation fails.
pub fn commitment(key: &[u8; KEY]) -> Result<[u8; COMMITMENT], CryptoError> {
    Ok(*derive::<COMMITMENT>(&[], key, Label::Commit, &[])?)
}

/// `plaintext` sealed under `key`, bound to `associated`.
///
/// # Errors
///
/// Returns [`CryptoError::Random`] or [`CryptoError::Primitive`] when the sealing fails.
pub fn seal(key: &[u8; KEY], associated: &[u8], plaintext: &[u8]) -> Result<Sealed, CryptoError> {
    let nonce = random::<NONCE>()?;
    let mut ciphertext = vec![0_u8; plaintext.len().saturating_add(TAG)];
    xchacha20_poly1305::encrypt(key, plaintext, &mut ciphertext, associated, &nonce)
        .map_err(|_refused| CryptoError::Primitive("XChaCha20-Poly1305 sealing"))?;
    Ok(Sealed {
        commitment: commitment(key)?,
        nonce,
        ciphertext,
    })
}

/// The plaintext of `sealed` under `key`, when the commitment names this key and the ciphertext authenticates with `associated`.
///
/// # Errors
///
/// Returns [`CryptoError::Forged`] when the commitment or the authentication fails.
pub fn open(
    key: &[u8; KEY],
    associated: &[u8],
    sealed: &Sealed,
) -> Result<zeroize::Zeroizing<Vec<u8>>, CryptoError> {
    if !equal(&commitment(key)?, &sealed.commitment) {
        return Err(CryptoError::Forged("the key commitment"));
    }
    let length = sealed
        .ciphertext
        .len()
        .checked_sub(TAG)
        .ok_or(CryptoError::Malformed("a sealed text"))?;
    let mut plaintext = zeroize::Zeroizing::new(vec![0_u8; length]);
    xchacha20_poly1305::decrypt(
        key,
        &mut plaintext,
        &sealed.ciphertext,
        associated,
        &sealed.nonce,
    )
    .map_err(|_refused| CryptoError::Forged("the sealed text"))?;
    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::{open, seal};
    use crate::CryptoError;

    #[test]
    fn a_sealed_text_opens_only_under_its_key_and_associated_data() {
        let key = [7_u8; 32];
        let sealed = seal(&key, b"account", b"state").unwrap();
        assert_eq!(
            open(&key, b"account", &sealed).unwrap().as_slice(),
            b"state"
        );
        assert_eq!(
            open(&[8; 32], b"account", &sealed),
            Err(CryptoError::Forged("the key commitment"))
        );
        assert_eq!(
            open(&key, b"other", &sealed),
            Err(CryptoError::Forged("the sealed text"))
        );
        let mut flipped = sealed.clone();
        flipped.ciphertext[0] ^= 1;
        assert_eq!(
            open(&key, b"account", &flipped),
            Err(CryptoError::Forged("the sealed text"))
        );
        let mut cut = sealed.clone();
        cut.ciphertext.truncate(3);
        assert_eq!(
            open(&key, b"account", &cut),
            Err(CryptoError::Malformed("a sealed text"))
        );
        assert_ne!(
            seal(&key, b"account", b"state").unwrap().nonce,
            sealed.nonce
        );
    }
}
