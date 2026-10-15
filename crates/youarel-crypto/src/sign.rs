//! Signatures under Ed25519 and ML-DSA-65 at once: a signature verifies only when both halves do, so a break of either scheme forges nothing.
//!
//! Every signed message is a label, a zero byte, and the payload, so a signature made for one purpose never verifies for another.

use crate::{CryptoError, Label, random};
use libcrux_ml_dsa::ml_dsa_65::{
    self, MLDSA65Signature, MLDSA65SigningKey, MLDSA65VerificationKey,
};
use zeroize::Zeroizing;

/// The bytes of an Ed25519 key or seed.
pub const ED25519_KEY: usize = 32;
/// The bytes of an Ed25519 signature.
pub const ED25519_SIGNATURE: usize = 64;
/// The bytes of an ML-DSA-65 verification key (FIPS 204, table 2).
pub const ML_DSA_KEY: usize = 1952;
/// The bytes of an ML-DSA-65 signing key (FIPS 204, table 2).
pub const ML_DSA_SIGNING_KEY: usize = 4032;
/// The bytes of an ML-DSA-65 signature (FIPS 204, table 2).
pub const ML_DSA_SIGNATURE: usize = 3309;
/// The bytes of the randomness ML-DSA-65 key generation and signing each take.
const ML_DSA_RANDOMNESS: usize = 32;

/// The public half of a device's signing keys.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifyingKeys {
    pub ed25519: [u8; ED25519_KEY],
    pub ml_dsa: Box<[u8; ML_DSA_KEY]>,
}

/// A device's signing keys, which leave memory zeroed.
pub struct SigningKeys {
    ed25519: Zeroizing<[u8; ED25519_KEY]>,
    ml_dsa: Zeroizing<[u8; ML_DSA_SIGNING_KEY]>,
    public: VerifyingKeys,
}

impl core::fmt::Debug for SigningKeys {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SigningKeys")
            .field("public", &self.public)
            .finish_non_exhaustive()
    }
}

/// A signature under both schemes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Signature {
    pub ed25519: [u8; ED25519_SIGNATURE],
    pub ml_dsa: Box<[u8; ML_DSA_SIGNATURE]>,
}

/// The message a signature covers: the label, a zero byte, and the payload.
fn message(label: Label, payload: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(
        label
            .text()
            .len()
            .saturating_add(1)
            .saturating_add(payload.len()),
    );
    message.extend_from_slice(label.text().as_bytes());
    message.push(0);
    message.extend_from_slice(payload);
    message
}

impl SigningKeys {
    /// New signing keys from the operating system's randomness.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Random`] when the operating system gives no randomness, and [`CryptoError::Primitive`] when a key cannot be made.
    pub fn generate() -> Result<Self, CryptoError> {
        Ok(Self::from_seeds(
            &Zeroizing::new(random::<ED25519_KEY>()?),
            &Zeroizing::new(random::<ML_DSA_RANDOMNESS>()?),
        ))
    }

    /// The signing keys that `ed25519`, the Ed25519 secret, and `ml_dsa`, the randomness of the ML-DSA-65 key generation, grow into.
    #[must_use]
    pub fn from_seeds(ed25519: &[u8; ED25519_KEY], ml_dsa: &[u8; ML_DSA_RANDOMNESS]) -> Self {
        let mut ed25519_public = [0_u8; ED25519_KEY];
        libcrux_ed25519::secret_to_public(&mut ed25519_public, ed25519);
        let pair = ml_dsa_65::generate_key_pair(*ml_dsa);
        Self {
            ed25519: Zeroizing::new(*ed25519),
            ml_dsa: Zeroizing::new(*pair.signing_key.as_ref()),
            public: VerifyingKeys {
                ed25519: ed25519_public,
                ml_dsa: Box::new(*pair.verification_key.as_ref()),
            },
        }
    }

    /// The signing keys whose secret halves `ed25519` and `ml_dsa` hold.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Malformed`] when a half has the wrong length.
    pub fn from_secret(
        ed25519: &[u8],
        ml_dsa: &[u8],
        public: VerifyingKeys,
    ) -> Result<Self, CryptoError> {
        let ed25519: [u8; ED25519_KEY] = ed25519
            .try_into()
            .map_err(|_wrong| CryptoError::Malformed("an Ed25519 key"))?;
        let ml_dsa: [u8; ML_DSA_SIGNING_KEY] = ml_dsa
            .try_into()
            .map_err(|_wrong| CryptoError::Malformed("an ML-DSA-65 signing key"))?;
        let mut derived = [0_u8; ED25519_KEY];
        libcrux_ed25519::secret_to_public(&mut derived, &ed25519);
        if derived != public.ed25519 {
            return Err(CryptoError::Malformed(
                "an Ed25519 key that does not match its public half",
            ));
        }
        Ok(Self {
            ed25519: Zeroizing::new(ed25519),
            ml_dsa: Zeroizing::new(ml_dsa),
            public,
        })
    }

    /// The secret halves, for the sealed file that keeps them.
    #[must_use]
    pub fn secret(&self) -> (Zeroizing<Vec<u8>>, Zeroizing<Vec<u8>>) {
        (
            Zeroizing::new(self.ed25519.to_vec()),
            Zeroizing::new(self.ml_dsa.to_vec()),
        )
    }

    #[must_use]
    pub const fn public(&self) -> &VerifyingKeys {
        &self.public
    }

    /// The signature of `payload` for the purpose `label`.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::Random`] or [`CryptoError::Primitive`] when a half cannot be made.
    pub fn sign(&self, label: Label, payload: &[u8]) -> Result<Signature, CryptoError> {
        let message = message(label, payload);
        let ed25519 = libcrux_ed25519::sign(&message, &self.ed25519)
            .map_err(|_failed| CryptoError::Primitive("Ed25519 signing"))?;
        let ml_dsa = ml_dsa_65::sign(
            &MLDSA65SigningKey::new(*self.ml_dsa),
            &message,
            b"",
            random::<ML_DSA_RANDOMNESS>()?,
        )
        .map_err(|_failed| CryptoError::Primitive("ML-DSA-65 signing"))?;
        Ok(Signature {
            ed25519,
            ml_dsa: Box::new(*ml_dsa.as_ref()),
        })
    }
}

impl VerifyingKeys {
    /// Whether both halves of `signature` verify `payload` for the purpose `label`.
    #[must_use]
    pub fn verify(&self, label: Label, payload: &[u8], signature: &Signature) -> bool {
        let message = message(label, payload);
        let classical =
            libcrux_ed25519::verify(&message, &self.ed25519, &signature.ed25519).is_ok();
        let post_quantum = ml_dsa_65::verify(
            &MLDSA65VerificationKey::new(*self.ml_dsa),
            &message,
            b"",
            &MLDSA65Signature::new(*signature.ml_dsa),
        )
        .is_ok();
        classical && post_quantum
    }
}

#[cfg(test)]
mod tests {
    use super::SigningKeys;
    use crate::Label;

    #[test]
    fn a_signature_verifies_only_for_its_payload_purpose_and_keys() {
        let keys = SigningKeys::generate().unwrap();
        let other = SigningKeys::generate().unwrap();
        let signature = keys.sign(Label::Operation, b"payload").unwrap();
        assert!(
            keys.public()
                .verify(Label::Operation, b"payload", &signature)
        );
        assert!(!keys.public().verify(Label::Bundle, b"payload", &signature));
        assert!(
            !keys
                .public()
                .verify(Label::Operation, b"payloaD", &signature)
        );
        assert!(
            !other
                .public()
                .verify(Label::Operation, b"payload", &signature)
        );
        let mut half = signature.clone();
        half.ed25519[0] ^= 1;
        assert!(!keys.public().verify(Label::Operation, b"payload", &half));
        let mut other_half = signature;
        other_half.ml_dsa[0] ^= 1;
        assert!(
            !keys
                .public()
                .verify(Label::Operation, b"payload", &other_half)
        );
    }

    #[test]
    fn signing_keys_return_from_their_secret_halves() {
        let keys = SigningKeys::generate().unwrap();
        let (ed25519, ml_dsa) = keys.secret();
        let restored = SigningKeys::from_secret(&ed25519, &ml_dsa, keys.public().clone()).unwrap();
        let signature = restored.sign(Label::Bundle, b"x").unwrap();
        assert!(keys.public().verify(Label::Bundle, b"x", &signature));
        let stranger = SigningKeys::generate().unwrap();
        SigningKeys::from_secret(&ed25519, &ml_dsa, stranger.public().clone()).unwrap_err();
        SigningKeys::from_secret(&ed25519[1..], &ml_dsa, keys.public().clone()).unwrap_err();
        assert!(!format!("{keys:?}").contains(&format!("{:?}", &ed25519[..4])));
    }
}
