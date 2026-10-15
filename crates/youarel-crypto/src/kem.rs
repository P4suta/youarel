//! X-Wing, the hybrid of ML-KEM-768 and X25519: a key encapsulated to a device stays secret while either half stands.

use crate::{CryptoError, KEY, random};
use libcrux_kem::{Algorithm, Ct, PrivateKey, PublicKey};
use zeroize::Zeroizing;

/// The bytes of an X-Wing public key (draft-connolly-cfrg-xwing-kem-06).
pub const PUBLIC_KEY: usize = 1216;
/// The bytes of an X-Wing encapsulation.
pub const CIPHERTEXT: usize = 1120;
/// The bytes of the seed an X-Wing key pair grows from, which is its secret key.
pub const SEED: usize = 32;
/// The bytes of randomness one encapsulation takes.
const ENCAPSULATION_RANDOMNESS: usize = 64;

const ALGORITHM: Algorithm = Algorithm::XWingKemDraft06;

/// A device's X-Wing key pair: the seed that is its secret, and the public key it publishes.
pub struct KeyPair {
    seed: Zeroizing<[u8; SEED]>,
    public: Box<[u8; PUBLIC_KEY]>,
}

impl core::fmt::Debug for KeyPair {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("KeyPair").finish_non_exhaustive()
    }
}

impl KeyPair {
    /// A new key pair from the operating system's randomness.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Random`] or [`CryptoError::Primitive`] when no key can be made.
    pub fn generate() -> Result<Self, CryptoError> {
        Self::from_seed(&random::<SEED>()?)
    }

    /// The key pair that `seed` grows into.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Malformed`] for a seed of the wrong length, and [`CryptoError::Primitive`] when the key cannot be made.
    pub fn from_seed(seed: &[u8]) -> Result<Self, CryptoError> {
        let seed: [u8; SEED] = seed
            .try_into()
            .map_err(|_wrong| CryptoError::Malformed("an X-Wing seed"))?;
        let (_, public) = libcrux_kem::key_gen_derand(ALGORITHM, &seed)
            .map_err(|_refused| CryptoError::Primitive("X-Wing key generation"))?;
        let public: [u8; PUBLIC_KEY] = public
            .encode()
            .try_into()
            .map_err(|_wrong| CryptoError::Malformed("an X-Wing public key"))?;
        Ok(Self {
            seed: Zeroizing::new(seed),
            public: Box::new(public),
        })
    }

    /// The seed, for the sealed file that keeps it.
    #[must_use]
    pub fn seed(&self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(self.seed.to_vec())
    }

    #[must_use]
    pub fn public(&self) -> &[u8; PUBLIC_KEY] {
        &self.public
    }

    /// The shared secret an encapsulation to this key carries.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Malformed`] for an encapsulation of the wrong shape, and [`CryptoError::Primitive`] when it does not decapsulate.
    pub fn decapsulate(&self, ciphertext: &[u8]) -> Result<Zeroizing<[u8; KEY]>, CryptoError> {
        let (private, _) = libcrux_kem::key_gen_derand(ALGORITHM, self.seed.as_slice())
            .map_err(|_refused| CryptoError::Primitive("X-Wing key generation"))?;
        let private = PrivateKey::decode(ALGORITHM, &private.encode())
            .map_err(|_wrong| CryptoError::Malformed("an X-Wing secret key"))?;
        let ciphertext = Ct::decode(ALGORITHM, ciphertext)
            .map_err(|_wrong| CryptoError::Malformed("an X-Wing encapsulation"))?;
        let shared = ciphertext
            .decapsulate(&private)
            .map_err(|_refused| CryptoError::Primitive("X-Wing decapsulation"))?;
        let bytes: [u8; KEY] = shared
            .encode()
            .try_into()
            .map_err(|_wrong| CryptoError::Malformed("an X-Wing shared secret"))?;
        Ok(Zeroizing::new(bytes))
    }
}

/// A shared secret for the holder of `public`, and the encapsulation that carries it there.
///
/// # Errors
///
/// Returns [`CryptoError::Malformed`] for a public key of the wrong shape, and [`CryptoError::Random`] or [`CryptoError::Primitive`] when no encapsulation can be made.
pub fn encapsulate(public: &[u8]) -> Result<(Zeroizing<[u8; KEY]>, Vec<u8>), CryptoError> {
    let public = PublicKey::decode(ALGORITHM, public)
        .map_err(|_wrong| CryptoError::Malformed("an X-Wing public key"))?;
    let randomness = Zeroizing::new(random::<ENCAPSULATION_RANDOMNESS>()?);
    let (shared, ciphertext) = public
        .encapsulate_derand(randomness.as_slice())
        .map_err(|_refused| CryptoError::Primitive("X-Wing encapsulation"))?;
    let bytes: [u8; KEY] = shared
        .encode()
        .try_into()
        .map_err(|_wrong| CryptoError::Malformed("an X-Wing shared secret"))?;
    Ok((Zeroizing::new(bytes), ciphertext.encode()))
}

#[cfg(test)]
mod tests {
    use super::{CIPHERTEXT, KeyPair, PUBLIC_KEY, encapsulate};

    #[test]
    fn an_encapsulated_secret_reaches_only_its_key() {
        let keys = KeyPair::generate().unwrap();
        let other = KeyPair::generate().unwrap();
        assert_eq!(keys.public().len(), PUBLIC_KEY);
        let (shared, ciphertext) = encapsulate(keys.public()).unwrap();
        assert_eq!(ciphertext.len(), CIPHERTEXT);
        assert_eq!(*keys.decapsulate(&ciphertext).unwrap(), *shared);
        assert_ne!(*other.decapsulate(&ciphertext).unwrap(), *shared);
        let restored = KeyPair::from_seed(&keys.seed()).unwrap();
        assert_eq!(restored.public(), keys.public());
        assert_eq!(*restored.decapsulate(&ciphertext).unwrap(), *shared);
        keys.decapsulate(&ciphertext[1..]).unwrap_err();
        encapsulate(&keys.public()[1..]).unwrap_err();
        KeyPair::from_seed(&[0; 31]).unwrap_err();
    }
}
