//! `youarel self-update`: replace this program with the latest release when its signed manifest offers a newer one.
//!
//! Whether to install is `update_rules`'s decision; this module fetches the manifest and the asset, checks them, and replaces the program.
//! The mark of the highest version this device reached sits beside the store, locked from the check through the replacement.

use crate::web;
use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use youarel_core::update_rules::{self, Consent, Verdict, Version};
use youarel_release::manifest::{Digest, LONGEST_MANIFEST, Name};
use youarel_release::trust::{
    self, LONGEST_MINISIGN, ML_DSA_SIGNATURE_BYTES, ReleaseKey, Signatures,
};
use youarel_store::fs;

/// Where the releases are published.
pub const RELEASES: &str = "https://github.com/P4suta/youarel/releases";
/// The variable that names a directory holding a release instead, such as a mirror or a candidate build.
pub const SOURCE_OVERRIDE: &str = "YOUAREL_RELEASES";
/// The file name of a release's manifest.
pub const MANIFEST: &str = "manifest.json";
/// The file name of the manifest's minisign signature.
pub const MINISIGN: &str = "manifest.json.minisig";
/// The file name of the manifest's ML-DSA-65 signature.
pub const ML_DSA: &str = "manifest.json.mldsa";
/// The file beside the store that records the highest version this device reached.
const MARK: &str = "release-mark.json";
/// The lock that keeps two updates of one device apart.
const LOCK: &str = "update.lock";
/// The longest mark file a reader accepts.
const LONGEST_MARK: u64 = 1024;

/// The target triple this program was built for, which names its asset in a manifest.
pub const TARGET: &str = if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
    "aarch64-apple-darwin"
} else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
    "x86_64-apple-darwin"
} else if cfg!(all(target_arch = "aarch64", windows)) {
    "aarch64-pc-windows-msvc"
} else if cfg!(all(target_arch = "x86_64", windows)) {
    "x86_64-pc-windows-msvc"
} else if cfg!(all(target_arch = "aarch64", target_os = "linux")) {
    "aarch64-unknown-linux-gnu"
} else if cfg!(all(target_arch = "x86_64", target_os = "linux")) {
    "x86_64-unknown-linux-gnu"
} else {
    "unsupported"
};

/// Where a release comes from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Source {
    /// The GitHub releases under this address: the latest manifest, and each version's assets.
    Web(String),
    /// A directory that holds one release's manifest, signatures, and assets side by side.
    Directory(PathBuf),
}

impl Source {
    /// The directory [`SOURCE_OVERRIDE`] names, or the published releases.
    ///
    /// # Errors
    ///
    /// Returns an error when the variable is not UTF-8 or names a relative path.
    pub fn from_environment() -> Result<Self> {
        match std::env::var_os(SOURCE_OVERRIDE) {
            None => Ok(Self::Web(String::from(RELEASES))),
            Some(value) => {
                let path = PathBuf::from(value);
                ensure!(
                    path.is_absolute(),
                    "{SOURCE_OVERRIDE} names {}, which is not an absolute directory",
                    path.display()
                );
                Ok(Self::Directory(path))
            }
        }
    }

    /// One of the latest release's manifest files, at most `limit` bytes.
    fn manifest_file(&self, name: &str, limit: u64) -> Result<Vec<u8>> {
        match self {
            Self::Web(base) => web::fetch_bytes(&format!("{base}/latest/download/{name}"), limit),
            Self::Directory(directory) => Ok(fs::read_bounded(&directory.join(name), limit)?),
        }
    }

    /// The asset `name` of release `version`, at most `limit` bytes.
    fn asset(&self, version: Version, name: &Name, limit: u64) -> Result<Vec<u8>> {
        match self {
            Self::Web(base) => web::fetch_bytes(
                &format!("{base}/download/v{version}/{}", name.as_str()),
                limit,
            ),
            Self::Directory(directory) => {
                Ok(fs::read_bounded(&directory.join(name.as_str()), limit)?)
            }
        }
    }
}

/// Everything an update reads besides the release itself.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub source: &'a Source,
    pub keys: &'a [ReleaseKey<'a>],
    pub running: Version,
    pub target: &'a str,
    /// The program to replace.
    pub executable: &'a Path,
    /// The directory of the store, which holds the mark.
    pub home: &'a Path,
    pub consent: Consent,
}

/// What an update did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Installed(Version),
    Current(Version),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkDocument {
    version: String,
}

fn read_mark(path: &Path) -> Result<Option<Version>> {
    let Some(bytes) = fs::read_bounded_if_present(path, LONGEST_MARK)? else {
        return Ok(None);
    };
    let document: MarkDocument = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not a release mark", path.display()))?;
    let version = Version::parse(&document.version).with_context(|| {
        format!(
            "{} records `{}`, which is not a version",
            path.display(),
            document.version
        )
    })?;
    Ok(Some(version))
}

fn write_mark(path: &Path, version: Version) -> Result<()> {
    let document = MarkDocument {
        version: version.to_string(),
    };
    let mut text = serde_json::to_string(&document)?;
    text.push('\n');
    Ok(fs::replace(path, text.as_bytes())?)
}

/// Checks the latest release and installs it when the decision says so.
///
/// # Errors
///
/// Returns an error when the release cannot be fetched, is not signed by a trusted key, lies below the mark without consent, has no asset for the target, or does not match its digest, and when the program cannot be replaced.
pub fn update(request: &Request<'_>) -> Result<Outcome> {
    fs::create_dir_all(request.home)?;
    let _lock = fs::Lock::acquire(&request.home.join(LOCK))?;
    let mark_path = request.home.join(MARK);
    let mark = read_mark(&mark_path)?;
    let source = request.source;
    let manifest_bytes = source.manifest_file(MANIFEST, u64::try_from(LONGEST_MANIFEST)?)?;
    let minisign = source.manifest_file(MINISIGN, u64::try_from(LONGEST_MINISIGN)?)?;
    let ml_dsa = source.manifest_file(ML_DSA, u64::try_from(ML_DSA_SIGNATURE_BYTES)?)?;
    let signatures = Signatures {
        minisign: std::str::from_utf8(&minisign).context("the minisign signature is not text")?,
        ml_dsa: &ml_dsa,
    };
    let manifest = trust::verify(&manifest_bytes, &signatures, request.keys)?;
    let highest = update_rules::reached(mark, request.running);
    if mark != Some(highest) {
        write_mark(&mark_path, highest)?;
    }
    let offered = manifest.version;
    match update_rules::decide(request.running, mark, offered, request.consent) {
        Verdict::Install => {}
        Verdict::Current => return Ok(Outcome::Current(request.running)),
        Verdict::Refuse { mark: reached } => bail!(
            "the latest release {offered} lies below {reached}, the highest version this device reached; pass --allow-downgrade to install it"
        ),
    }
    let target = Name::parse(request.target)
        .ok_or_else(|| anyhow!("`{}` is not a target", request.target))?;
    let asset = manifest
        .assets
        .get(&target)
        .with_context(|| format!("release {offered} has no build for {}", request.target))?;
    let binary = source.asset(offered, &asset.name, asset.size)?;
    ensure!(
        u64::try_from(binary.len())? == asset.size,
        "{} holds {} bytes, not the {} its manifest names",
        asset.name.as_str(),
        binary.len(),
        asset.size
    );
    ensure!(
        Digest::of(&binary) == asset.sha256,
        "{} does not match the digest its manifest names",
        asset.name.as_str()
    );
    fs::replace_executable(request.executable, &binary)?;
    write_mark(&mark_path, update_rules::reached(Some(highest), offered))?;
    Ok(Outcome::Installed(offered))
}

/// Updates the running program from the published releases, or the directory [`SOURCE_OVERRIDE`] names.
///
/// # Errors
///
/// Returns an error as [`update`] does, and when the running program cannot be found.
pub fn run(home: &Path, consent: Consent) -> Result<String> {
    let running = Version::parse(env!("CARGO_PKG_VERSION"))
        .context("the build's own version is not a version")?;
    let executable = std::env::current_exe().context("cannot find the running program")?;
    let source = Source::from_environment()?;
    let request = Request {
        source: &source,
        keys: trust::RELEASE_KEYS,
        running,
        target: TARGET,
        executable: &executable,
        home,
        consent,
    };
    Ok(match update(&request)? {
        Outcome::Installed(version) => format!("updated youarel from {running} to {version}"),
        Outcome::Current(version) => format!("youarel {version} is the latest release"),
    })
}
