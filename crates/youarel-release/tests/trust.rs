//! A manifest is trusted only with both signatures under one trusted key, and anything else is refused with its reason.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::io::Cursor;
use youarel_core::update_rules::Version;
use youarel_release::manifest::{self, Asset, Digest, Manifest, Name};
use youarel_release::trust::{
    LONGEST_MINISIGN, ML_DSA_CONTEXT, ReleaseKey, Signatures, TrustError, ml_dsa_key_pair,
    ml_dsa_sign, verify,
};

struct Signer {
    minisign_public: String,
    minisign_secret: minisign::SecretKey,
    ml_dsa_public: Vec<u8>,
    ml_dsa_secret: Vec<u8>,
}

impl Signer {
    fn new(seed: u8) -> Result<Self> {
        let pair = minisign::KeyPair::generate_unencrypted_keypair()?;
        let (ml_dsa_secret, ml_dsa_public) = ml_dsa_key_pair([seed; 32]);
        Ok(Self {
            minisign_public: pair.pk.to_base64(),
            minisign_secret: pair.sk,
            ml_dsa_public,
            ml_dsa_secret,
        })
    }

    fn key(&self) -> ReleaseKey<'_> {
        ReleaseKey {
            minisign: &self.minisign_public,
            ml_dsa: &self.ml_dsa_public,
        }
    }

    fn sign(&self, bytes: &[u8]) -> Result<(String, Vec<u8>)> {
        self.sign_with(bytes, "youarel")
    }

    /// Both signatures, with `comment` as the trusted comment of the minisign one.
    fn sign_with(&self, bytes: &[u8], comment: &str) -> Result<(String, Vec<u8>)> {
        let minisig = minisign::sign(
            None,
            &self.minisign_secret,
            Cursor::new(bytes),
            Some(comment),
            None,
        )?
        .to_string();
        let ml_dsa = ml_dsa_sign(&self.ml_dsa_secret, bytes, [7; 32])?;
        Ok((minisig, ml_dsa))
    }
}

fn written() -> Result<String> {
    let mut assets = BTreeMap::new();
    assets.insert(
        Name::parse("aarch64-apple-darwin").context("a target")?,
        Asset {
            name: Name::parse("youarel-aarch64-apple-darwin").context("an asset")?,
            size: 3,
            sha256: Digest::of(b"new"),
        },
    );
    let release = Manifest {
        version: Version::parse("0.2.0").context("a version")?,
        assets,
    };
    Ok(manifest::encode(&release)?)
}

fn check(
    bytes: &[u8],
    minisign: &str,
    ml_dsa: &[u8],
    keys: &[ReleaseKey<'_>],
) -> Result<Manifest, TrustError> {
    verify(bytes, &Signatures { minisign, ml_dsa }, keys)
}

#[test]
fn both_signatures_under_one_trusted_key_admit_the_manifest() {
    let signer = Signer::new(1).unwrap();
    let other = Signer::new(2).unwrap();
    let bytes = written().unwrap();
    let (minisig, ml_dsa) = signer.sign(bytes.as_bytes()).unwrap();
    let admitted = check(
        bytes.as_bytes(),
        &minisig,
        &ml_dsa,
        &[other.key(), signer.key()],
    )
    .unwrap();
    assert_eq!(admitted.version, Version::parse("0.2.0").unwrap());
}

#[test]
fn one_signature_alone_or_halves_of_two_keys_admit_nothing() {
    let signer = Signer::new(1).unwrap();
    let other = Signer::new(2).unwrap();
    let bytes = written().unwrap();
    let (minisig, ml_dsa) = signer.sign(bytes.as_bytes()).unwrap();
    let (other_minisig, other_ml_dsa) = other.sign(bytes.as_bytes()).unwrap();
    let keys = [signer.key(), other.key()];
    for (minisign, post_quantum) in [
        (minisig.as_str(), other_ml_dsa.as_slice()),
        (other_minisig.as_str(), ml_dsa.as_slice()),
        (minisig.as_str(), &[][..]),
        ("", ml_dsa.as_slice()),
    ] {
        assert!(matches!(
            check(bytes.as_bytes(), minisign, post_quantum, &keys),
            Err(TrustError::Unsigned)
        ));
    }
    assert!(matches!(
        check(bytes.as_bytes(), &minisig, &ml_dsa, &[other.key()]),
        Err(TrustError::Unsigned)
    ));
}

#[test]
fn a_changed_manifest_or_another_context_fails_verification() {
    let signer = Signer::new(1).unwrap();
    let bytes = written().unwrap();
    let (minisig, ml_dsa) = signer.sign(bytes.as_bytes()).unwrap();
    let changed = bytes.replace("0.2.0", "0.3.0");
    assert!(matches!(
        check(changed.as_bytes(), &minisig, &ml_dsa, &[signer.key()]),
        Err(TrustError::Unsigned)
    ));
    assert_ne!(ML_DSA_CONTEXT, b"youarel/v1/state");
    let key: [u8; 4032] = signer.ml_dsa_secret.as_slice().try_into().unwrap();
    let foreign = libcrux_ml_dsa::ml_dsa_65::sign(
        &libcrux_ml_dsa::ml_dsa_65::MLDSA65SigningKey::new(key),
        bytes.as_bytes(),
        b"youarel/v1/state",
        [7; 32],
    )
    .unwrap();
    assert!(matches!(
        check(
            bytes.as_bytes(),
            &minisig,
            foreign.as_slice(),
            &[signer.key()]
        ),
        Err(TrustError::Unsigned)
    ));
}

#[test]
fn a_build_without_keys_or_with_a_bad_key_trusts_nothing() {
    let signer = Signer::new(1).unwrap();
    let bytes = written().unwrap();
    let (minisig, ml_dsa) = signer.sign(bytes.as_bytes()).unwrap();
    assert!(matches!(
        check(bytes.as_bytes(), &minisig, &ml_dsa, &[]),
        Err(TrustError::NoKey)
    ));
    assert!(matches!(
        check(
            bytes.as_bytes(),
            &minisig,
            &ml_dsa,
            youarel_release::trust::RELEASE_KEYS
        ),
        Err(TrustError::NoKey)
    ));
    let short = ReleaseKey {
        minisign: &signer.minisign_public,
        ml_dsa: &signer.ml_dsa_public[1..],
    };
    assert!(matches!(
        check(bytes.as_bytes(), &minisig, &ml_dsa, &[short]),
        Err(TrustError::MalformedKey)
    ));
    let garbled = ReleaseKey {
        minisign: "not a key",
        ml_dsa: &signer.ml_dsa_public,
    };
    assert!(matches!(
        check(bytes.as_bytes(), &minisig, &ml_dsa, &[garbled]),
        Err(TrustError::MalformedKey)
    ));
}

#[test]
fn signed_bytes_that_are_no_manifest_are_refused_after_verification() {
    let signer = Signer::new(1).unwrap();
    let bytes = b"{\"format\": 1}";
    let (minisig, ml_dsa) = signer.sign(bytes).unwrap();
    assert!(matches!(
        check(bytes, &minisig, &ml_dsa, &[signer.key()]),
        Err(TrustError::Manifest(_))
    ));
    assert!(matches!(
        ml_dsa_sign(&signer.ml_dsa_secret[1..], bytes, [0; 32]),
        Err(TrustError::MalformedKey)
    ));
    let refused = check(bytes, &minisig, &ml_dsa, &[signer.key()]).unwrap_err();
    assert!(std::error::Error::source(&refused).is_some());
}

#[test]
fn a_manifest_and_a_signature_of_their_longest_lengths_still_verify() {
    let signer = Signer::new(1).unwrap();
    let mut longest = written().unwrap().into_bytes();
    longest.resize(manifest::LONGEST_MANIFEST, b' ');
    let (minisig, post_quantum) = signer.sign(&longest).unwrap();
    check(&longest, &minisig, &post_quantum, &[signer.key()]).unwrap();
    let bytes = written().unwrap();
    let (short, _) = signer.sign_with(bytes.as_bytes(), "").unwrap();
    let padding = "x".repeat(LONGEST_MINISIGN - short.len());
    let (long, long_quantum) = signer.sign_with(bytes.as_bytes(), &padding).unwrap();
    assert_eq!(long.len(), LONGEST_MINISIGN);
    check(bytes.as_bytes(), &long, &long_quantum, &[signer.key()]).unwrap();
    let (longer, longer_quantum) = signer
        .sign_with(bytes.as_bytes(), &format!("{padding}x"))
        .unwrap();
    assert!(matches!(
        check(bytes.as_bytes(), &longer, &longer_quantum, &[signer.key()]),
        Err(TrustError::Unsigned)
    ));
}
