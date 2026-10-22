//! The recovery key: thirty-two random bytes that a person prints and keeps.
//!
//! The key grows into the keys of an identity that the account lists as a member, so every rotation gives it a slot like any device's, and a new device that learns the key acts as that member to join the account.

use crate::{CryptoError, KEY, Label, crockford, derive, digest, random};
use zeroize::Zeroizing;

/// The bytes of the key's secret.
pub const SECRET: usize = 32;
/// The bytes of the checksum the printed key carries, which catches a mistyped symbol.
const CHECKSUM: usize = 2;
/// The bytes the printed key spells: the secret and its checksum.
const PRINTED: usize = SECRET + CHECKSUM;
/// The symbols the printed key spells them in, five bits each, and the symbols between two separators.
const SYMBOLS: usize = 55;
const GROUP: usize = 5;

/// A recovery key.
#[derive(Clone, PartialEq, Eq)]
pub struct RecoveryKey {
    secret: Zeroizing<[u8; SECRET]>,
}

impl core::fmt::Debug for RecoveryKey {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RecoveryKey")
            .finish_non_exhaustive()
    }
}

/// The seeds of an identity's keys: its Ed25519 key, its ML-DSA-65 key, and its X-Wing key.
pub struct Seeds {
    pub ed25519: Zeroizing<[u8; KEY]>,
    pub ml_dsa: Zeroizing<[u8; KEY]>,
    pub kem: Zeroizing<[u8; KEY]>,
}

impl core::fmt::Debug for Seeds {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("Seeds").finish_non_exhaustive()
    }
}

fn checksum(secret: &[u8; SECRET]) -> [u8; CHECKSUM] {
    let [first, second, ..] = digest(Label::RecoveryCheck, &[secret.as_slice()]);
    [first, second]
}

impl RecoveryKey {
    /// A new key from the operating system's randomness.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Random`] when the operating system gives no randomness.
    pub fn generate() -> Result<Self, CryptoError> {
        Ok(Self::from_secret(random::<SECRET>()?))
    }

    /// The key whose secret is `secret`.
    #[must_use]
    pub fn from_secret(secret: [u8; SECRET]) -> Self {
        Self {
            secret: Zeroizing::new(secret),
        }
    }

    /// The printed key: fifty-five symbols in eleven groups of five, which spell the secret and its checksum.
    #[must_use]
    pub fn text(&self) -> Zeroizing<String> {
        let mut printed = Zeroizing::new([0_u8; PRINTED]);
        for (target, source) in printed
            .iter_mut()
            .zip(self.secret.iter().chain(&checksum(&self.secret)))
        {
            *target = *source;
        }
        let symbols = Zeroizing::new(crockford::encode(printed.as_slice()));
        let mut text = Zeroizing::new(String::new());
        for (index, symbol) in symbols.chars().enumerate() {
            if index > 0 && index % GROUP == 0 {
                text.push('-');
            }
            text.push(symbol);
        }
        text
    }

    /// The key that `text` spells, ignoring dashes and spaces and forgiving case and the letters people confuse with digits, when its checksum holds.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let symbols: Zeroizing<Vec<u8>> = Zeroizing::new(
            text.bytes()
                .filter(|byte| !matches!(byte, b'-' | b' '))
                .collect(),
        );
        if symbols.len() != SYMBOLS {
            return None;
        }
        let printed = Zeroizing::new(crockford::decode::<PRINTED>(&symbols)?);
        let (secret, sum) = printed.split_first_chunk::<SECRET>()?;
        let key = Self::from_secret(*secret);
        (sum == checksum(&key.secret).as_slice()).then_some(key)
    }

    /// The seeds of the identity this key grows into.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Primitive`] when the derivation fails.
    pub fn seeds(&self) -> Result<Seeds, CryptoError> {
        let seed =
            |part: &[u8]| derive::<KEY>(&[], self.secret.as_slice(), Label::Recovery, &[part]);
        Ok(Seeds {
            ed25519: seed(b"ed25519")?,
            ml_dsa: seed(b"ml-dsa-65")?,
            kem: seed(b"x-wing")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{RecoveryKey, SECRET};

    #[test]
    fn a_key_reads_back_from_its_text_and_a_typo_breaks_its_checksum() {
        let fresh = RecoveryKey::generate().unwrap();
        assert_eq!(RecoveryKey::parse(&fresh.text()), Some(fresh));
        // A fixed key, so that the typo below breaks its checksum on every run rather than all but one in 65,536.
        let key = RecoveryKey::from_secret([7; SECRET]);
        let text = key.text();
        assert_eq!(text.len(), 65);
        assert_eq!(text.split('-').count(), 11);
        assert!(text.split('-').all(|group| group.len() == 5));
        assert_eq!(RecoveryKey::parse(&text), Some(key.clone()));
        assert_eq!(
            RecoveryKey::parse(&text.to_lowercase().replace('-', " ")),
            Some(key.clone())
        );
        let mut typo: Vec<u8> = text.bytes().collect();
        let first = typo.first_mut().unwrap();
        *first = if *first == b'0' { b'1' } else { b'0' };
        assert_eq!(
            RecoveryKey::parse(core::str::from_utf8(&typo).unwrap()),
            None
        );
        assert_eq!(RecoveryKey::parse(text.get(..60).unwrap()), None);
        assert!(!format!("{key:?}").contains(text.get(..5).unwrap()));
    }

    #[test]
    fn a_key_always_grows_the_same_seeds_and_two_keys_grow_different_ones() {
        let key = RecoveryKey::from_secret([7; SECRET]);
        let (once, again) = (key.seeds().unwrap(), key.seeds().unwrap());
        assert_eq!(once.ml_dsa, again.ml_dsa);
        assert_ne!(once.ed25519, once.kem);
        assert_ne!(once.ed25519, once.ml_dsa);
        let other = RecoveryKey::from_secret([8; SECRET]).seeds().unwrap();
        assert_ne!(other.kem, once.kem);
    }
}
