//! `self-update` installs a release only when both signatures verify, the asset matches its digest, and the version lies above the device's mark.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use youarel_cli::update::{MANIFEST, MINISIGN, ML_DSA, Outcome, Request, Source, update};
use youarel_core::update_rules::{Consent, Version};
use youarel_release::manifest::{self, Asset, Digest, Manifest, Name};
use youarel_release::trust::{ReleaseKey, ml_dsa_key_pair, ml_dsa_sign};
use youarel_store::fs;

const TARGET: &str = "x86_64-unknown-linux-gnu";
const ASSET: &str = "youarel-x86_64-unknown-linux-gnu";
/// The longest program a test reads back.
const LONGEST: u64 = 64;

fn version(text: &str) -> Result<Version> {
    Version::parse(text).with_context(|| format!("`{text}` is a version"))
}

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

    /// Writes a release of `program` at `release` into `directory`, signed by both halves of this key.
    fn publish(&self, directory: &Path, release: &str, program: &[u8]) -> Result<()> {
        let mut assets = BTreeMap::new();
        assets.insert(
            Name::parse(TARGET).context("a target")?,
            Asset {
                name: Name::parse(ASSET).context("an asset")?,
                size: u64::try_from(program.len())?,
                sha256: Digest::of(program),
            },
        );
        let bytes = manifest::encode(&Manifest {
            version: version(release)?,
            assets,
        })?;
        let minisig = minisign::sign(
            None,
            &self.minisign_secret,
            Cursor::new(bytes.as_bytes()),
            None,
            None,
        )?
        .to_string();
        let ml_dsa = ml_dsa_sign(&self.ml_dsa_secret, bytes.as_bytes(), [9; 32])?;
        fs::replace(&directory.join(MANIFEST), bytes.as_bytes())?;
        fs::replace(&directory.join(MINISIGN), minisig.as_bytes())?;
        fs::replace(&directory.join(ML_DSA), &ml_dsa)?;
        fs::replace(&directory.join(ASSET), program)?;
        Ok(())
    }
}

struct Device {
    home: tempfile::TempDir,
    releases: tempfile::TempDir,
    program: PathBuf,
    _program_directory: tempfile::TempDir,
}

impl Device {
    fn new() -> Result<Self> {
        let program_directory = tempfile::tempdir()?;
        let program = program_directory.path().join("youarel");
        fs::replace(&program, b"old program")?;
        Ok(Self {
            home: tempfile::tempdir()?,
            releases: tempfile::tempdir()?,
            program,
            _program_directory: program_directory,
        })
    }

    /// Updates the program of this device, which runs `running` and has the asset of `target`.
    fn update_for(&self, target: &str, keys: &[ReleaseKey<'_>], running: &str) -> Result<Outcome> {
        let source = Source::Directory(self.releases.path().to_path_buf());
        update(&Request {
            source: &source,
            keys,
            running: version(running)?,
            target,
            executable: &self.program,
            home: self.home.path(),
            consent: Consent::Forward,
        })
    }

    fn update(&self, keys: &[ReleaseKey<'_>], running: &str, consent: Consent) -> Result<Outcome> {
        let source = Source::Directory(self.releases.path().to_path_buf());
        update(&Request {
            source: &source,
            keys,
            running: version(running)?,
            target: TARGET,
            executable: &self.program,
            home: self.home.path(),
            consent,
        })
    }

    fn program(&self) -> Result<Vec<u8>> {
        Ok(fs::read_bounded(&self.program, LONGEST)?)
    }
}

#[test]
fn a_newer_signed_release_replaces_the_program() {
    let signer = Signer::new(1).unwrap();
    let device = Device::new().unwrap();
    signer
        .publish(device.releases.path(), "0.2.0", b"new program")
        .unwrap();
    let outcome = device
        .update(&[signer.key()], "0.1.0", Consent::Forward)
        .unwrap();
    assert_eq!(outcome, Outcome::Installed(version("0.2.0").unwrap()));
    assert_eq!(device.program().unwrap(), b"new program");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&device.program)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }
    let again = device
        .update(&[signer.key()], "0.2.0", Consent::Forward)
        .unwrap();
    assert_eq!(again, Outcome::Current(version("0.2.0").unwrap()));
}

#[test]
fn a_replayed_release_below_the_mark_is_refused_without_consent() {
    let signer = Signer::new(1).unwrap();
    let device = Device::new().unwrap();
    signer
        .publish(device.releases.path(), "0.3.0", b"three")
        .unwrap();
    device
        .update(&[signer.key()], "0.2.0", Consent::Forward)
        .unwrap();
    signer
        .publish(device.releases.path(), "0.2.5", b"two and a half")
        .unwrap();
    let refused = device
        .update(&[signer.key()], "0.2.0", Consent::Forward)
        .unwrap_err();
    assert!(
        format!("{refused:#}").contains("lies below 0.3.0"),
        "{refused:#}"
    );
    assert_eq!(device.program().unwrap(), b"three");
    let downgraded = device
        .update(&[signer.key()], "0.3.0", Consent::Downgrade)
        .unwrap();
    assert_eq!(downgraded, Outcome::Installed(version("0.2.5").unwrap()));
    assert_eq!(device.program().unwrap(), b"two and a half");
    let current = device
        .update(&[signer.key()], "0.2.5", Consent::Forward)
        .unwrap();
    assert_eq!(current, Outcome::Current(version("0.2.5").unwrap()));
    signer
        .publish(device.releases.path(), "0.2.7", b"two and three quarters")
        .unwrap();
    let still_refused = device
        .update(&[signer.key()], "0.2.5", Consent::Forward)
        .unwrap_err();
    assert!(
        format!("{still_refused:#}").contains("lies below 0.3.0"),
        "{still_refused:#}"
    );
}

#[test]
fn an_unsigned_or_tampered_release_changes_nothing() {
    let signer = Signer::new(1).unwrap();
    let stranger = Signer::new(2).unwrap();
    let device = Device::new().unwrap();
    signer
        .publish(device.releases.path(), "0.2.0", b"new program")
        .unwrap();
    let untrusted = device
        .update(&[stranger.key()], "0.1.0", Consent::Forward)
        .unwrap_err();
    assert!(
        format!("{untrusted:#}").contains("not signed"),
        "{untrusted:#}"
    );
    let keyless = device.update(&[], "0.1.0", Consent::Forward).unwrap_err();
    assert!(
        format!("{keyless:#}").contains("trusts no release key"),
        "{keyless:#}"
    );
    fs::replace(&device.releases.path().join(ASSET), b"new pr0gram").unwrap();
    let tampered = device
        .update(&[signer.key()], "0.1.0", Consent::Forward)
        .unwrap_err();
    assert!(format!("{tampered:#}").contains("digest"), "{tampered:#}");
    fs::replace(&device.releases.path().join(ASSET), b"new program and more").unwrap();
    let longer = device
        .update(&[signer.key()], "0.1.0", Consent::Forward)
        .unwrap_err();
    assert!(format!("{longer:#}").contains("larger than"), "{longer:#}");
    fs::replace(&device.releases.path().join(ASSET), b"short").unwrap();
    let shorter = device
        .update(&[signer.key()], "0.1.0", Consent::Forward)
        .unwrap_err();
    assert!(
        format!("{shorter:#}").contains("holds 5 bytes"),
        "{shorter:#}"
    );
    assert_eq!(device.program().unwrap(), b"old program");
}

#[test]
fn a_release_without_this_target_or_with_a_broken_mark_is_refused() {
    let signer = Signer::new(1).unwrap();
    let device = Device::new().unwrap();
    signer
        .publish(device.releases.path(), "0.2.0", b"new program")
        .unwrap();
    let other = device
        .update_for("riscv64gc-unknown-linux-gnu", &[signer.key()], "0.1.0")
        .unwrap_err();
    assert!(format!("{other:#}").contains("no build for"), "{other:#}");
    let odd = device
        .update_for("not a target", &[signer.key()], "0.1.0")
        .unwrap_err();
    assert!(format!("{odd:#}").contains("is not a target"), "{odd:#}");
    fs::replace(
        &device.home.path().join("release-mark.json"),
        b"{\"version\": \"x\"}",
    )
    .unwrap();
    let broken = device
        .update(&[signer.key()], "0.1.0", Consent::Forward)
        .unwrap_err();
    assert!(
        format!("{broken:#}").contains("release-mark.json"),
        "{broken:#}"
    );
    assert_eq!(device.program().unwrap(), b"old program");
}

#[test]
fn the_source_names_the_published_releases_or_a_directory() {
    let base = String::from(youarel_cli::update::RELEASES);
    assert_eq!(Source::Web(base.clone()), Source::Web(base));
    assert!(youarel_cli::update::TARGET.contains('-'));
}
