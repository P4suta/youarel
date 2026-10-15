//! The keys a build trusts and the two signatures a manifest needs under one of them.
//!
//! A minisign signature is Ed25519 over the `BLAKE2b` digest of the manifest, and the ML-DSA-65 signature covers the manifest under [`ML_DSA_CONTEXT`].
//! Either alone proves nothing, so a break of one scheme forges no release.

use crate::manifest::{self, Manifest, ManifestError};
use libcrux_ml_dsa::ml_dsa_65::{
    self, MLDSA65Signature, MLDSA65SigningKey, MLDSA65VerificationKey,
};
use libcrux_ml_dsa::{KEY_GENERATION_RANDOMNESS_SIZE, SIGNING_RANDOMNESS_SIZE};

/// The bytes of an ML-DSA-65 verification key (FIPS 204, table 2).
pub const ML_DSA_KEY_BYTES: usize = 1952;
/// The bytes of an ML-DSA-65 signing key (FIPS 204, table 2).
pub const ML_DSA_SIGNING_KEY_BYTES: usize = 4032;
/// The bytes of an ML-DSA-65 signature (FIPS 204, table 2).
pub const ML_DSA_SIGNATURE_BYTES: usize = 3309;
/// The longest minisign signature file a reader accepts, which holds two comment lines and two base64 lines.
pub const LONGEST_MINISIGN: usize = 4096;
/// The context of every ML-DSA-65 release signature, which binds it to this purpose.
pub const ML_DSA_CONTEXT: &[u8] = b"youarel/v1/release";

/// A release key: the minisign public key in its base64 form and the ML-DSA-65 verification key.
#[derive(Clone, Copy, Debug)]
pub struct ReleaseKey<'a> {
    pub minisign: &'a str,
    pub ml_dsa: &'a [u8],
}

/// The keys this build trusts: none until the owner makes the first release key, so no download is trusted yet.
pub const RELEASE_KEYS: &[ReleaseKey<'static>] = &[];

/// The two signatures of a manifest: the text of its minisign file and the bytes of its ML-DSA-65 file.
#[derive(Clone, Copy, Debug)]
pub struct Signatures<'a> {
    pub minisign: &'a str,
    pub ml_dsa: &'a [u8],
}

/// Why a manifest is not trusted.
#[derive(Debug)]
pub enum TrustError {
    /// The build trusts no key, so it trusts no release.
    NoKey,
    /// A key compiled into the build is malformed.
    MalformedKey,
    /// No trusted key verifies both signatures.
    Unsigned,
    /// Both signatures verify, but the bytes are no manifest.
    Manifest(ManifestError),
}

impl core::fmt::Display for TrustError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoKey => formatter.write_str("this build trusts no release key, so it trusts no downloaded release"),
            Self::MalformedKey => formatter.write_str("a release key compiled into this build is malformed"),
            Self::Unsigned => formatter.write_str(
                "the release manifest is not signed by both minisign and ML-DSA-65 under one trusted key",
            ),
            Self::Manifest(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for TrustError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Manifest(error) => Some(error),
            Self::NoKey | Self::MalformedKey | Self::Unsigned => None,
        }
    }
}

/// Whether the minisign signature verifies `bytes` under the key.
fn classical(key: &ReleaseKey<'_>, bytes: &[u8], signature: &str) -> Result<bool, TrustError> {
    let public = minisign_verify::PublicKey::from_base64(key.minisign)
        .map_err(|_wrong| TrustError::MalformedKey)?;
    if signature.len() > LONGEST_MINISIGN {
        return Ok(false);
    }
    let Ok(signature) = minisign_verify::Signature::decode(signature) else {
        return Ok(false);
    };
    Ok(public.verify(bytes, &signature, false).is_ok())
}

/// Whether the ML-DSA-65 signature verifies `bytes` under the key.
fn post_quantum(key: &ReleaseKey<'_>, bytes: &[u8], signature: &[u8]) -> Result<bool, TrustError> {
    let public: [u8; ML_DSA_KEY_BYTES] = key
        .ml_dsa
        .try_into()
        .map_err(|_wrong| TrustError::MalformedKey)?;
    let Ok(signature) = <[u8; ML_DSA_SIGNATURE_BYTES]>::try_from(signature) else {
        return Ok(false);
    };
    let verified = ml_dsa_65::verify(
        &MLDSA65VerificationKey::new(public),
        bytes,
        ML_DSA_CONTEXT,
        &MLDSA65Signature::new(signature),
    );
    Ok(verified.is_ok())
}

/// The manifest `bytes` hold, when both signatures verify under one of `keys`.
///
/// # Errors
///
/// Returns a [`TrustError`] when no key is trusted, a key is malformed, no key verifies both signatures, or the signed bytes are no manifest.
pub fn verify(
    bytes: &[u8],
    signatures: &Signatures<'_>,
    keys: &[ReleaseKey<'_>],
) -> Result<Manifest, TrustError> {
    if keys.is_empty() {
        return Err(TrustError::NoKey);
    }
    if bytes.len() > manifest::LONGEST_MANIFEST {
        return Err(TrustError::Manifest(ManifestError::TooLong(bytes.len())));
    }
    for key in keys {
        if classical(key, bytes, signatures.minisign)?
            && post_quantum(key, bytes, signatures.ml_dsa)?
        {
            return manifest::decode(bytes).map_err(TrustError::Manifest);
        }
    }
    Err(TrustError::Unsigned)
}

/// An ML-DSA-65 key pair from 32 random bytes: the signing key and the verification key.
#[must_use]
pub fn ml_dsa_key_pair(randomness: [u8; KEY_GENERATION_RANDOMNESS_SIZE]) -> (Vec<u8>, Vec<u8>) {
    let pair = ml_dsa_65::generate_key_pair(randomness);
    (
        pair.signing_key.as_slice().to_vec(),
        pair.verification_key.as_slice().to_vec(),
    )
}

/// The ML-DSA-65 signature of `bytes` under a signing key, drawn with 32 random bytes.
///
/// # Errors
///
/// Returns [`TrustError::MalformedKey`] when the signing key has the wrong length or the signer refuses it.
pub fn ml_dsa_sign(
    signing_key: &[u8],
    bytes: &[u8],
    randomness: [u8; SIGNING_RANDOMNESS_SIZE],
) -> Result<Vec<u8>, TrustError> {
    let key: [u8; ML_DSA_SIGNING_KEY_BYTES] = signing_key
        .try_into()
        .map_err(|_wrong| TrustError::MalformedKey)?;
    ml_dsa_65::sign(
        &MLDSA65SigningKey::new(key),
        bytes,
        ML_DSA_CONTEXT,
        randomness,
    )
    .map(|signature| signature.as_slice().to_vec())
    .map_err(|_wrong| TrustError::MalformedKey)
}
