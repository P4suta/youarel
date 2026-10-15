//! The cryptography of youarel, as `specs/CRYPTO.md` specifies it: hybrid signatures, X-Wing key slots, sealed states, and the pairing channel.
//!
//! Every primitive comes from libcrux, whose implementations carry machine-checked proofs, apart from the PAKE, which is the SPAKE2 of `RustCrypto`.
//! Secrets leave memory zeroed, and no function here reads a file, a clock, or the network.

pub mod kem;
pub mod pairing;
pub mod seal;
pub mod sign;
pub mod slot;

use sha2::{Digest as _, Sha512, Sha512_256};

/// The bytes of a digest that names a device, an account, or an operation.
pub const DIGEST: usize = 32;
/// The bytes of a symmetric key.
pub const KEY: usize = 32;

/// The purpose a signature, a derivation, or a digest serves, which `specs/CRYPTO.md` lists.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Label {
    Device,
    Account,
    Operation,
    Bundle,
    State,
    Commit,
    Slot,
    Pairing,
    Welcomed,
    Joined,
}

impl Label {
    /// The label's text, which every value made for this purpose binds.
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::Device => "youarel/v1/device",
            Self::Account => "youarel/v1/account",
            Self::Operation => "youarel/v1/operation",
            Self::Bundle => "youarel/v1/bundle",
            Self::State => "youarel/v1/state",
            Self::Commit => "youarel/v1/commit",
            Self::Slot => "youarel/v1/slot",
            Self::Pairing => "youarel/v1/pairing",
            Self::Welcomed => "youarel/v1/welcomed",
            Self::Joined => "youarel/v1/joined",
        }
    }
}

/// Why a cryptographic step failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CryptoError {
    /// The operating system gave no randomness.
    Random,
    /// A key, a ciphertext, or a message has the wrong shape.
    Malformed(&'static str),
    /// A primitive refused its inputs.
    Primitive(&'static str),
    /// A ciphertext, a commitment, or a confirmation does not authenticate.
    Forged(&'static str),
}

impl core::fmt::Display for CryptoError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Random => formatter.write_str("the operating system gave no randomness"),
            Self::Malformed(what) => write!(formatter, "{what} has the wrong shape"),
            Self::Primitive(what) => write!(formatter, "{what} failed"),
            Self::Forged(what) => write!(formatter, "{what} does not authenticate"),
        }
    }
}

impl std::error::Error for CryptoError {}

/// `N` bytes of the operating system's randomness.
///
/// # Errors
///
/// Returns [`CryptoError::Random`] when the operating system gives none.
pub fn random<const N: usize>() -> Result<[u8; N], CryptoError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|_unavailable| CryptoError::Random)?;
    Ok(bytes)
}

/// The SHA-512/256 digest of `parts` for the purpose `label`: the label, a zero byte, then each part.
#[must_use]
pub fn digest(label: Label, parts: &[&[u8]]) -> [u8; DIGEST] {
    let mut hasher = Sha512_256::new();
    hasher.update(label.text().as_bytes());
    hasher.update([0]);
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

/// The SHA-512 digest of `bytes`, for a pairing transcript.
#[must_use]
pub fn sha512(bytes: &[u8]) -> [u8; 64] {
    Sha512::digest(bytes).into()
}

/// `N` bytes of HKDF-SHA-512 over `ikm` with `salt`, for the purpose `label` followed by `context`.
///
/// # Errors
///
/// Returns [`CryptoError::Primitive`] when the derivation refuses its inputs.
pub fn derive<const N: usize>(
    salt: &[u8],
    ikm: &[u8],
    label: Label,
    context: &[&[u8]],
) -> Result<zeroize::Zeroizing<[u8; N]>, CryptoError> {
    let mut info = Vec::from(label.text().as_bytes());
    for part in context {
        info.extend_from_slice(part);
    }
    let mut okm = zeroize::Zeroizing::new([0_u8; N]);
    libcrux_hkdf::hkdf(
        libcrux_hkdf::Algorithm::Sha512,
        okm.as_mut_slice(),
        salt,
        ikm,
        &info,
    )
    .map_err(|_refused| CryptoError::Primitive("HKDF-SHA-512"))?;
    Ok(okm)
}

/// The HMAC-SHA-512 of `parts` under `key`, for the purpose `label`.
#[must_use]
pub fn mac(key: &[u8], label: Label, parts: &[&[u8]]) -> [u8; 64] {
    let mut data = Vec::from(label.text().as_bytes());
    data.push(0);
    for part in parts {
        data.extend_from_slice(part);
    }
    let mut tag = [0_u8; 64];
    libcrux_hmac::hmac(libcrux_hmac::Algorithm::Sha512, key, &data, &mut tag);
    tag
}

/// Whether two byte strings are equal, in time that depends only on their lengths.
#[must_use]
pub fn equal(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::{Label, derive, digest, equal, mac};

    #[test]
    fn labels_separate_every_purpose() {
        let labels = [
            Label::Device,
            Label::Account,
            Label::Operation,
            Label::Bundle,
            Label::State,
            Label::Commit,
            Label::Slot,
            Label::Pairing,
            Label::Welcomed,
            Label::Joined,
        ];
        for (index, label) in labels.iter().enumerate() {
            assert!(label.text().starts_with("youarel/v1/"));
            for other in labels.iter().skip(index + 1) {
                assert_ne!(digest(*label, &[b"x"]), digest(*other, &[b"x"]));
            }
        }
        assert_ne!(
            digest(Label::Device, &[b"ab", b"c"]),
            digest(Label::Device, &[b"a"])
        );
        assert_eq!(
            digest(Label::Device, &[b"ab", b"c"]),
            digest(Label::Device, &[b"abc"])
        );
    }

    #[test]
    fn derivations_and_macs_depend_on_every_input() {
        let one = derive::<32>(b"salt", b"key", Label::Slot, &[b"a"]).unwrap();
        assert_ne!(
            *one,
            *derive::<32>(b"salt", b"key", Label::Slot, &[b"b"]).unwrap()
        );
        assert_ne!(
            *one,
            *derive::<32>(b"pepper", b"key", Label::Slot, &[b"a"]).unwrap()
        );
        assert_ne!(
            *one,
            *derive::<32>(b"salt", b"key", Label::Commit, &[b"a"]).unwrap()
        );
        assert_ne!(
            mac(b"k", Label::Joined, &[b"t"]),
            mac(b"k", Label::Welcomed, &[b"t"])
        );
        assert!(equal(b"same", b"same"));
        assert!(!equal(b"same", b"Same"));
        assert!(!equal(b"same", b"sam"));
        // Two bytes that differ by the same bits must not cancel each other.
        assert!(!equal(&[1, 1], &[0, 0]));
    }
}
