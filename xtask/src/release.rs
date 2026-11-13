//! Native candidates, isolated signing, and immutable desktop publication.
//!
//! Unsigned rehearsals use temporary manifest keys.
//! Signing, notarization, drafts, and publication require `RELEASES_ENABLED=true`.

use crate::fs;
use crate::package::{self, EXECUTABLE, Entry, REGULAR};
use crate::tool::{self, Tool};
use crate::{apple, portable};
use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use youarel_core::update_rules::Version;
use youarel_release::manifest::{self, Asset, Digest, Manifest, Name};
use youarel_release::trust::{self, ReleaseKey, Signatures};

mod access;
mod artifacts;
mod candidate;
mod credentials;
mod delivery;
mod desktop;
mod distribution;
mod esigner;
mod esigner_authorization;
mod install;
mod proposal;
mod runs;

pub use credentials::names as credential_names;

/// Checks product version synchronization and the pinned proposal updaters.
///
/// # Errors
///
/// Reports drift, an unavailable checker, or a rejected updater result.
pub fn check_proposal(root: &Path) -> Result<()> {
    proposal::check(root)
}

pub(crate) fn source_tree(root: &Path) -> Result<String> {
    let changes = tool::capture(
        tool::repository_git(root).args(["status", "--porcelain", "--untracked-files=all"]),
        "check the committed release source",
    )?;
    ensure!(
        changes.is_empty(),
        "release checks require a clean committed checkout"
    );
    let tree = tool::capture(
        tool::repository_git(root).args(["rev-parse", "HEAD:"]),
        "read release source tree",
    )?
    .trim()
    .to_owned();
    ensure!(
        candidate::valid_source(&tree),
        "the release tree is not a full object identifier"
    );
    Ok(tree)
}

/// Writes the application payload for the macOS installer.
///
/// # Errors
///
/// Rejects unsafe paths or a failed payload write.
pub(crate) fn write_application(base: &Path, entries: &[Entry]) -> Result<()> {
    desktop::write_entries(base, entries)
}

/// The directory a release assembles its artifacts in.
pub const DIST: &str = "dist";
/// The variable that enables releasing, which stays unset while releases are prohibited.
pub const ENABLED: &str = "RELEASES_ENABLED";
pub(super) const MACOS_APPLICATION_IDENTITY: &str = "MACOS_APPLICATION_IDENTITY_SHA1";
pub(super) const MACOS_INSTALLER_IDENTITY: &str = "MACOS_INSTALLER_IDENTITY_SHA1";
/// The package that builds the program.
const PACKAGE: &str = "youarel-cli";
/// The program's name.
const PROGRAM: &str = "youarel";
/// The files every archive carries beside the program.
const DOCUMENTS: [&str; 3] = ["README.md", "LICENSE-MIT", "LICENSE-APACHE"];
/// Who maintains the Debian package.
const MAINTAINER: &str = "P4suta <noreply@github.com>";
/// The project's home.
const HOMEPAGE: &str = "https://github.com/P4suta/youarel";
/// The machine-readable format of a Debian package's copyright file.
const COPYRIGHT_FORMAT: &str = "https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/";
/// What the Debian package says it is.
const SUMMARY: &str = "Save, organize, find, and open web bookmarks";
/// Where the release keeps the receipts of its notarization submissions.
const NOTARIZATION: &str = "notarization";
/// The trusted comment of a manifest's minisign signature.
const TRUSTED_COMMENT: &str = "youarel release manifest";
const KEY_RANDOMNESS_BYTES: usize = 32;
const KEY_INPUTS: &str = "manifest-secrets.json";
const MINISIGN_PUBLIC_KEY: &str = "minisign.public";
const ML_DSA_SECRET_KEY: &str = "ml-dsa.secret";
const ML_DSA_PUBLIC_KEY: &str = "ml-dsa.public";
const ML_DSA_PUBLIC_KEY_BYTES: &str = "ml-dsa-public-key.bin";
const SIGNATURE_PATH: &str = "YOUAREL_SIGNATURE_PROGRAM";
const WINDOWS_PROGRAM_COUNT: usize = 2;
const WINDOWS_SDK_QUERY: &str = r"$ErrorActionPreference = 'Stop'; $root = Get-ItemPropertyValue -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots' -Name KitsRoot10; $tool = Get-ChildItem -LiteralPath (Join-Path $root 'bin') -Directory | Where-Object Name -Match '^\d+\.\d+\.\d+\.\d+$' | Sort-Object { [version]$_.Name } -Descending | ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1; if (!$tool) { throw 'The Windows SDK SignTool verifier is missing.' }; $signature = Get-AuthenticodeSignature -LiteralPath $tool; if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notlike '*O=Microsoft Corporation*') { throw 'The Windows SDK verifier is not trusted Microsoft code.' }; $tool";

const USAGE: &str = "usage: cargo xtask release version | check-versions | propose | plan candidate|sign | plan-publication draft|publish | build TARGET | seal TARGET | restore TARGET | inspect-candidate | record-distribution | bundle TARGET | check-bundle TARGET | manifest | sign | checksums | verify-dist | verify-provenance | attach-provenance FILE | attach-build-provenance TARGET FILE | verify-build-run | credential-inputs PURPOSE | verify-credentials PURPOSE | access inventory|sync-inventory|sync-prepare AUTH_SCOPE | sign-macos program|package TARGET | notarize submit|finish | stage-windows TARGET | accept-windows TARGET | prepare-windows | sign-windows TARGET | verify-windows TARGET | install-check TARGET | resume-build TARGET | resume-windows TARGET | resume-notarization | resume-distribution | draft | publish | candidate | keygen DIRECTORY";

/// Runs one release task.
///
/// # Errors
///
/// Returns an error naming the step that failed.
pub fn run(root: &Path, words: &[&str]) -> Result<()> {
    match words {
        ["version"] => version(root).map(drop),
        ["plan", operation] => candidate::plan(root, operation),
        ["seal", target] => candidate::seal(root, target),
        ["restore", target] => candidate::restore(root, target),
        ["verify-build-run"] => candidate::verify_run(root),
        ["record-distribution"] => distribution::write(root),
        ["inspect-candidate"] => candidate::inspect(root),
        ["verify-dist"] => delivery::verify(root),
        ["credential-inputs", purpose] => credentials::inputs(purpose),
        ["verify-credentials", purpose] => credentials::verify(root, purpose),
        ["access", operation, scope] => access::run(root, operation, scope),
        ["verify-provenance"] => delivery::verify_provenance(root),
        ["plan-publication", operation] => delivery::plan(operation),
        ["attach-provenance", path] => delivery::copy_provenance(root, Path::new(path)),
        ["attach-build-provenance", target, path] => {
            candidate::attach_provenance(root, target, Path::new(path))
        }
        ["check-bundle", target] => check_bundle(
            root,
            target,
            workspace_version(root)?,
            &fs::read(&built_program(root, target))?,
        ),
        ["draft"] => delivery::draft(root),
        ["check-versions"] => check_proposal(root),
        ["propose"] => proposal::propose(root),
        ["build", target] => build(root, target),
        ["bundle", target] => bundle(root, target),
        ["manifest"] => write_manifest(root),
        ["sign"] => sign(root),
        ["checksums"] => checksums(root),
        ["sign-macos", "program", target] => sign_macos_program(root, target),
        ["sign-macos", "package", target] => sign_macos_package(root, target),
        ["notarize", "submit"] => notarize_submit(root),
        ["notarize", "finish"] => notarize_finish(root),
        ["verify-windows", target] => verify_windows(root, target),
        ["prepare-windows"] => esigner::prepare(root),
        ["sign-windows", target] => esigner::sign(root, target),
        ["stage-windows", target] => stage_windows(root, target),
        ["accept-windows", target] => accept_windows(root, target),
        ["install-check", target] => install::check(root, target),
        ["resume-windows", target] => artifacts::windows(root, target),
        ["resume-build", target] => artifacts::build(root, target),
        ["resume-notarization"] => artifacts::notarization(root),
        ["resume-distribution"] => artifacts::distribution(root),
        ["publish"] => delivery::publish(root),
        ["candidate"] => candidate(root),
        ["keygen", directory] => keygen(Path::new(directory)),
        _ => bail!("{USAGE}"),
    }
}

/// A variable's value, or `None` when it is unset or empty.
fn variable(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => bail!("{name} is not UTF-8"),
    }
}

/// A variable the step requires.
fn required(name: &str) -> Result<String> {
    variable(name)?.with_context(|| format!("{name} is unset; the release environment provides it"))
}

fn ensure_enabled() -> Result<()> {
    ensure!(
        variable(ENABLED)?.as_deref() == Some("true"),
        "releases are disabled: {ENABLED} is not `true`"
    );
    Ok(())
}

/// The workspace version, which every release artifact names.
fn workspace_version(root: &Path) -> Result<Version> {
    let manifest: toml::Table = toml::from_str(&fs::read_to_string(&root.join("Cargo.toml"))?)?;
    let text = manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .context("Cargo.toml names no workspace version")?;
    Version::parse(text)
        .with_context(|| format!("the workspace version `{text}` is not a release version"))
}

/// Checks that a tag build names the workspace version, and hands the version to later workflow steps.
fn version(root: &Path) -> Result<Version> {
    let version = workspace_version(root)?;
    if variable("GITHUB_REF_TYPE")?.as_deref() == Some("tag") {
        let tag = required("GITHUB_REF_NAME")?;
        ensure!(
            tag == format!("v{version}"),
            "the tag {tag} does not name the workspace version {version}"
        );
    }
    if let Some(output) = variable("GITHUB_OUTPUT")? {
        fs::append(
            Path::new(&output),
            format!("version={version}\n").as_bytes(),
        )?;
    }
    println!("{version}");
    Ok(version)
}

/// The file name of the program on a target.
fn program_name(target: &str) -> String {
    if target.contains("windows") {
        format!("{PROGRAM}.exe")
    } else {
        String::from(PROGRAM)
    }
}

/// Where Cargo writes the release program of a target.
fn built_program(root: &Path, target: &str) -> PathBuf {
    root.join("target")
        .join(target)
        .join("release")
        .join(program_name(target))
}

/// The name of a target's bare program asset, which the updater downloads.
fn asset_name(target: &str) -> String {
    if target.contains("windows") {
        format!("{PROGRAM}-{target}.exe")
    } else {
        format!("{PROGRAM}-{target}")
    }
}

/// Builds the release program for a target from the locked dependencies.
fn build(root: &Path, target: &str) -> Result<()> {
    desktop::supported(target)?;
    tool::succeed(
        Tool::Cargo.command(root).args([
            "build",
            "--release",
            "--locked",
            "--package",
            PACKAGE,
            "--target",
            target,
        ]),
        "cargo build --release",
    )?;
    desktop::build(root, target)
}

/// The time every archive entry carries: `SOURCE_DATE_EPOCH` when the workflow sets it, and the epoch otherwise.
fn source_date() -> Result<u64> {
    variable("SOURCE_DATE_EPOCH")?.map_or(Ok(0), |text| {
        text.parse()
            .with_context(|| format!("SOURCE_DATE_EPOCH holds `{text}`, which is not seconds"))
    })
}

/// The Debian architecture of a Linux target.
fn debian_architecture(target: &str) -> Option<&'static str> {
    match target.split('-').next() {
        Some("x86_64") => Some("amd64"),
        Some("aarch64") => Some("arm64"),
        _ => None,
    }
}

/// What a target's bundle holds: every file name and its bytes, apart from the macOS package.
fn bundle_files(
    root: &Path,
    target: &str,
    version: Version,
    program: &[u8],
) -> Result<Vec<(String, Vec<u8>)>> {
    if target.contains("apple") {
        return Ok(vec![(asset_name(target), program.to_vec())]);
    }
    let time = source_date()?;
    let folder = format!("{PROGRAM}-{version}-{target}");
    let mut entries = vec![Entry {
        path: format!("{folder}/{}", program_name(target)),
        mode: EXECUTABLE,
        bytes: program.to_vec(),
    }];
    for mut entry in desktop::entries(root, target)? {
        entry.path = format!("{folder}/{}", entry.path);
        entries.push(entry);
    }
    for document in DOCUMENTS {
        entries.push(Entry {
            path: format!("{folder}/{document}"),
            mode: REGULAR,
            bytes: fs::read(&root.join(document))?,
        });
    }
    let mut files = vec![(asset_name(target), program.to_vec())];
    if target.contains("windows") {
        files.push((format!("{folder}.zip"), package::zip(&entries)?));
    } else {
        files.push((format!("{folder}.tar.gz"), package::tar_gz(&entries, time)?));
    }
    if target.contains("linux") {
        let architecture = debian_architecture(target)
            .with_context(|| format!("{target} has no Debian architecture"))?;
        let mut copyright = format!(
            "Format: {COPYRIGHT_FORMAT}\nUpstream-Name: {PROGRAM}\nSource: {HOMEPAGE}\n\nFiles: *\nLicense: MIT or Apache-2.0\n\n"
        );
        for license in ["LICENSE-MIT", "LICENSE-APACHE"] {
            copyright.push_str(&fs::read_to_string(&root.join(license))?);
            copyright.push('\n');
        }
        let control = package::Control {
            package: PROGRAM,
            version: &version.to_string(),
            architecture,
            maintainer: MAINTAINER,
            homepage: HOMEPAGE,
            summary: SUMMARY,
            depends: desktop::LINUX_DEPENDS,
        };
        let mut installed = vec![
            Entry {
                path: format!("usr/bin/{PROGRAM}"),
                mode: EXECUTABLE,
                bytes: program.to_vec(),
            },
            Entry {
                path: format!("usr/share/doc/{PROGRAM}/copyright"),
                mode: REGULAR,
                bytes: copyright.into_bytes(),
            },
        ];
        installed.extend(desktop::linux_entries(
            root,
            &fs::read(&desktop::launcher(root, target))?,
        )?);
        files.push((
            format!("{PROGRAM}_{version}_{architecture}.deb"),
            package::deb(&control, &installed, time)?,
        ));
    }
    Ok(files)
}

/// The record of a target's asset that `manifest` gathers.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetRecord {
    target: String,
    name: String,
    size: u64,
    sha256: String,
}

/// Packages a target's built program into `dist`: the bare asset, the archive, the Debian package on Linux, the unsigned installer on macOS, and the asset record.
fn bundle(root: &Path, target: &str) -> Result<()> {
    desktop::supported(target)?;
    let version = workspace_version(root)?;
    let program_path = built_program(root, target);
    let program = fs::read(&program_path)?;
    let dist = root.join(DIST);
    fs::create_dir_all(&dist)?;
    for (name, bytes) in bundle_files(root, target, version, &program)? {
        fs::write(&dist.join(&name), &bytes)?;
    }
    if target.contains("apple") {
        let unsigned = dist.join(format!("{PROGRAM}-{version}-{target}-unsigned.pkg"));
        apple::build_package(
            root,
            &desktop::app_entries(root, target)?,
            &version.to_string(),
            &unsigned,
        )?;
    }
    let record = AssetRecord {
        target: String::from(target),
        name: asset_name(target),
        size: u64::try_from(program.len())?,
        sha256: Digest::of(&program).hex(),
    };
    fs::write(
        &dist.join(format!("asset-{target}.json")),
        serde_json::to_string_pretty(&record)?.as_bytes(),
    )
}

/// The manifest of the assets that `dist` records, for the workspace version.
fn gathered(root: &Path) -> Result<Manifest> {
    let version = workspace_version(root)?;
    let mut assets = BTreeMap::new();
    for path in fs::entries(&root.join(DIST))? {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if !(name.starts_with("asset-")
            && Path::new(name)
                .extension()
                .is_some_and(|extension| extension == "json"))
        {
            continue;
        }
        let record: AssetRecord = serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("{} is not an asset record", portable(&path)))?;
        let target = Name::parse(&record.target)
            .with_context(|| format!("`{}` is not a target", record.target))?;
        let asset = Asset {
            name: Name::parse(&record.name)
                .with_context(|| format!("`{}` is not an asset name", record.name))?,
            size: record.size,
            sha256: Digest::parse(&record.sha256)
                .with_context(|| format!("`{}` is not a digest", record.sha256))?,
        };
        let bytes = fs::read(&root.join(DIST).join(asset.name.as_str()))?;
        ensure!(
            Digest::of(&bytes) == asset.sha256 && u64::try_from(bytes.len())? == asset.size,
            "{} no longer matches its record",
            asset.name.as_str()
        );
        assets.insert(target, asset);
    }
    ensure!(
        !assets.is_empty(),
        "{DIST} records no asset; run `release bundle` first"
    );
    for path in fs::entries(&root.join(DIST))? {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("release file name is not UTF-8")?;
        if !fs::is_file(&path)?
            || !delivery::manifested(name)
            || assets.values().any(|asset| asset.name.as_str() == name)
        {
            continue;
        }
        let bytes = fs::read(&path)?;
        let name = Name::parse(name).context("unsafe release asset name")?;
        let asset = Asset {
            name: name.clone(),
            size: u64::try_from(bytes.len())?,
            sha256: Digest::of(&bytes),
        };
        ensure!(
            assets.insert(name, asset).is_none(),
            "release asset collides with a target key"
        );
    }
    Ok(Manifest { version, assets })
}

/// Writes `dist/manifest.json` from the asset records.
fn write_manifest(root: &Path) -> Result<()> {
    let text = manifest::encode(&gathered(root)?)?;
    fs::write(&root.join(DIST).join("manifest.json"), text.as_bytes())
}

/// The two signatures of the manifest: minisign under the secret key the variables hold, and ML-DSA-65 under its signing key.
fn signatures(
    bytes: &[u8],
    minisign_key: &minisign::SecretKey,
    ml_dsa_key: &[u8],
) -> Result<(String, Vec<u8>)> {
    let minisig = minisign::sign(
        None,
        minisign_key,
        Cursor::new(bytes),
        Some(TRUSTED_COMMENT),
        None,
    )
    .context("sign the manifest with minisign")?
    .to_string();
    let mut randomness = [0_u8; 32];
    getrandom::fill(&mut randomness)
        .map_err(|error| anyhow::anyhow!("draw signing randomness: {error}"))?;
    let ml_dsa = trust::ml_dsa_sign(ml_dsa_key, bytes, randomness)?;
    Ok((minisig, ml_dsa))
}

/// Writes the two signature files beside a manifest, and checks them under `keys`.
fn write_signatures(
    dist: &Path,
    minisign_key: &minisign::SecretKey,
    ml_dsa_key: &[u8],
    keys: &[ReleaseKey<'_>],
) -> Result<Manifest> {
    let bytes = fs::read(&dist.join("manifest.json"))?;
    let (minisig, ml_dsa) = signatures(&bytes, minisign_key, ml_dsa_key)?;
    fs::write(&dist.join("manifest.json.minisig"), minisig.as_bytes())?;
    fs::write(&dist.join("manifest.json.mldsa"), &ml_dsa)?;
    let signed = Signatures {
        minisign: &minisig,
        ml_dsa: &ml_dsa,
    };
    Ok(trust::verify(&bytes, &signed, keys)?)
}

/// Signs the manifest with the release keys the variables hold, and checks it under the keys the program trusts.
fn sign(root: &Path) -> Result<()> {
    ensure_enabled()?;
    let minisign_box = minisign::SecretKeyBox::from_string(&required("RELEASE_MINISIGN_KEY")?)
        .context("RELEASE_MINISIGN_KEY is not a minisign secret key")?;
    let minisign_key = minisign_box
        .into_secret_key(Some(required("RELEASE_MINISIGN_PASSWORD")?))
        .context("RELEASE_MINISIGN_PASSWORD does not open the minisign key")?;
    let ml_dsa_key =
        unhex(&required("RELEASE_ML_DSA_KEY")?).context("RELEASE_ML_DSA_KEY is not hexadecimal")?;
    write_signatures(
        &root.join(DIST),
        &minisign_key,
        &ml_dsa_key,
        trust::RELEASE_KEYS,
    )
    .context("the signed manifest does not verify under the keys the program trusts")?;
    Ok(())
}

/// Writes `dist/SHA256SUMS` over every distributed file, after the last step that changes one.
fn checksums(root: &Path) -> Result<()> {
    let dist = root.join(DIST);
    let mut lines = String::new();
    for path in fs::entries(&dist)? {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        if name == "SHA256SUMS"
            || name == delivery::PROVENANCE
            || name.starts_with("asset-")
            || name.ends_with("-unsigned.pkg")
            || !fs::is_file(&path)?
        {
            continue;
        }
        lines.push_str(&Digest::of(&fs::read(&path)?).hex());
        lines.push_str("  ");
        lines.push_str(&name);
        lines.push('\n');
    }
    fs::write(&dist.join("SHA256SUMS"), lines.as_bytes())
}

/// The certificates the macOS signing steps import.
fn apple_imports() -> Result<Vec<(String, String)>> {
    Ok(vec![
        (
            required("APPLE_CERTIFICATE")?,
            required("APPLE_CERTIFICATE_PASSWORD")?,
        ),
        (
            required("APPLE_INSTALLER_CERTIFICATE")?,
            required("APPLE_INSTALLER_CERTIFICATE_PASSWORD")?,
        ),
    ])
}

fn with_keychain(root: &Path, work: impl FnOnce(&apple::Keychain) -> Result<()>) -> Result<()> {
    let imports = apple_imports()?;
    let imports: Vec<apple::Import<'_>> = imports
        .iter()
        .map(|(base64, password)| apple::Import { base64, password })
        .collect();
    let mut keychain = apple::Keychain::create(root, &imports)?;
    let result = work(&keychain);
    keychain.close()?;
    result
}

/// Signs a macOS target's built program with the Developer ID Application identity, before it is bundled.
fn sign_macos_program(root: &Path, target: &str) -> Result<()> {
    ensure_enabled()?;
    desktop::supported(target)?;
    ensure!(target.contains("apple"), "{target} is not a macOS target");
    let identity = required("APPLE_SIGNING_IDENTITY")?;
    let sha1 = required(MACOS_APPLICATION_IDENTITY)?;
    let identity = apple::Identity {
        sha1: &sha1,
        authority: &identity,
    };
    with_keychain(root, |keychain| {
        let program = built_program(root, target);
        let launcher = desktop::launcher(root, target);
        keychain.sign_program(identity, &program)?;
        keychain.sign_program(identity, &launcher)?;
        let app = desktop::application(root, target);
        fs::write(&app.join("Contents/MacOS/youarel"), &fs::read(&program)?)?;
        fs::write(
            &app.join("Contents/MacOS/youarel-launcher"),
            &fs::read(&launcher)?,
        )?;
        keychain.sign_program(identity, &app)
    })
}

/// Signs a macOS target's bundled installer with the Developer ID Installer identity.
fn sign_macos_package(root: &Path, target: &str) -> Result<()> {
    ensure_enabled()?;
    desktop::supported(target)?;
    ensure!(target.contains("apple"), "{target} is not a macOS target");
    let version = workspace_version(root)?;
    let identity = required("APPLE_INSTALLER_IDENTITY")?;
    let sha1 = required(MACOS_INSTALLER_IDENTITY)?;
    let identity = apple::Identity {
        sha1: &sha1,
        authority: &identity,
    };
    let dist = root.join(DIST);
    let unsigned = dist.join(format!("{PROGRAM}-{version}-{target}-unsigned.pkg"));
    let signed = dist.join(format!("{PROGRAM}-{version}-{target}.pkg"));
    with_keychain(root, |keychain| {
        keychain.sign_package(identity, &unsigned, &signed)
    })
}

/// A notarization submission the release persisted, bound to the package bytes it submitted.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: Option<String>,
    package: String,
    sha256: String,
}

/// Runs `work` with the notary key the variables hold, written to a private file that is gone afterward.
fn with_notary_key(work: impl FnOnce(&apple::NotaryKey<'_>) -> Result<()>) -> Result<()> {
    let directory = tempfile::tempdir().context("create the notary key directory")?;
    let file = directory.path().join("notary.p8");
    fs::write_private(&file, required("APPLE_NOTARY_KEY")?.as_bytes())?;
    let id = required("APPLE_NOTARY_KEY_ID")?;
    let issuer = required("APPLE_NOTARY_ISSUER_ID")?;
    work(&apple::NotaryKey {
        file: &file,
        id: &id,
        issuer: &issuer,
    })
}

/// The signed macOS packages in `dist`.
fn signed_packages(dist: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for path in fs::entries(dist)? {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if Path::new(name)
            .extension()
            .is_some_and(|extension| extension == "pkg")
            && !name.ends_with("-unsigned.pkg")
        {
            found.push(path);
        }
    }
    Ok(found)
}

/// Submits every signed package and persists each submission before anything waits on it.
fn notarize_submit(root: &Path) -> Result<()> {
    ensure_enabled()?;
    let dist = root.join(DIST);
    let receipts = dist.join(NOTARIZATION);
    fs::create_dir_all(&receipts)?;
    with_notary_key(|key| {
        for package in signed_packages(&dist)? {
            let name = package
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned();
            let receipt_path = receipts.join(format!("{name}.json"));
            if fs::is_file(&receipt_path)? {
                let receipt: Receipt = serde_json::from_slice(&fs::read(&receipt_path)?)?;
                ensure!(
                    receipt.package == name
                        && Digest::of(&fs::read(&package)?).hex() == receipt.sha256,
                    "the preserved submission does not identify the current package"
                );
                ensure!(
                    receipt.id.is_some(),
                    "the previous submission outcome is unknown; reconcile Apple history before any retry"
                );
                println!("{name} was submitted already; the finish step resumes it");
                continue;
            }
            let sha256 = Digest::of(&fs::read(&package)?).hex();
            let mut receipt = Receipt {
                id: None,
                package: name,
                sha256,
            };
            fs::write(&receipt_path, &serde_json::to_vec_pretty(&receipt)?)?;
            receipt.id = Some(apple::submit(root, key, &package)?);
            fs::write(
                &receipt_path,
                serde_json::to_string_pretty(&receipt)?.as_bytes(),
            )?;
        }
        Ok(())
    })
}

fn check_notary_receipts(root: &Path) -> Result<()> {
    let dist = root.join(DIST);
    let paths = fs::entries(&dist.join(NOTARIZATION))?;
    ensure!(
        !paths.is_empty(),
        "no preserved notarization receipt exists; reconcile Apple history before submitting"
    );
    for path in paths {
        let receipt: Receipt = serde_json::from_slice(&fs::read(&path)?)?;
        ensure!(
            Name::parse(&receipt.package).is_some(),
            "unsafe preserved package name"
        );
        ensure!(
            receipt.id.is_some(),
            "the previous notarization outcome is unknown; reconcile Apple history before another submission"
        );
        ensure!(
            Digest::of(&fs::read(&dist.join(&receipt.package))?).hex() == receipt.sha256,
            "the preserved notarization package changed"
        );
    }
    Ok(())
}

/// Waits for every persisted submission, then staples and checks its package.
fn notarize_finish(root: &Path) -> Result<()> {
    ensure_enabled()?;
    let dist = root.join(DIST);
    with_notary_key(|key| {
        for path in fs::entries(&dist.join(NOTARIZATION))? {
            let receipt: Receipt = serde_json::from_slice(&fs::read(&path)?)
                .with_context(|| format!("{} is not a notarization receipt", portable(&path)))?;
            ensure!(
                Name::parse(&receipt.package).is_some()
                    && Path::new(&receipt.package)
                        .extension()
                        .is_some_and(|extension| extension == "pkg"),
                "unsafe notarization package name"
            );
            let package = dist.join(&receipt.package);
            ensure!(
                Digest::of(&fs::read(&package)?).hex() == receipt.sha256,
                "{} changed after its submission",
                receipt.package
            );
            let id = receipt.id.as_deref().context(
                "the submission outcome is unknown; reconcile Apple history before resuming",
            )?;
            apple::finish(root, key, id, &package)?;
        }
        Ok(())
    })
}

/// Checks the Authenticode signature of a Windows target's program: valid, timestamped, and from the expected publisher.
fn verify_windows(root: &Path, target: &str) -> Result<()> {
    desktop::supported(target)?;
    ensure!(
        target.contains("windows"),
        "{target} is not a Windows target"
    );
    let expected = required("WINDOWS_SIGNING_IDENTITY_SHA256")?.to_ascii_lowercase();
    ensure!(
        Digest::parse(&expected).is_some(),
        "the expected Windows publisher fingerprint is malformed"
    );
    for program in [built_program(root, target), desktop::launcher(root, target)] {
        verify_windows_program(root, &program, &expected)?;
    }
    Ok(())
}

fn stage_windows(root: &Path, target: &str) -> Result<()> {
    desktop::supported(target)?;
    if !target.contains("windows") {
        return Ok(());
    }
    let expected = required("WINDOWS_SIGNING_IDENTITY_SHA256")?.to_ascii_lowercase();
    ensure!(
        Digest::parse(&expected).is_some(),
        "configure the Windows publisher fingerprint before requesting signatures"
    );
    windows_sdk(root)?;
    let input = root.join("signing-input");
    ensure!(
        !fs::is_directory(&input)?,
        "the Windows signing input already exists"
    );
    fs::create_dir_all(&input)?;
    for path in [built_program(root, target), desktop::launcher(root, target)] {
        let name = path.file_name().context("program file has no name")?;
        fs::write(&input.join(name), &fs::read(&path)?)?;
    }
    Ok(())
}

fn accept_windows(root: &Path, target: &str) -> Result<()> {
    ensure_enabled()?;
    desktop::supported(target)?;
    ensure!(
        target.contains("windows"),
        "{target} is not a Windows target"
    );
    let expected = required("WINDOWS_SIGNING_IDENTITY_SHA256")?.to_ascii_lowercase();
    ensure!(
        Digest::parse(&expected).is_some(),
        "the expected Windows publisher fingerprint is malformed"
    );
    let input = root.join("signing-input");
    ensure!(
        fs::entries(&input)?.len() == WINDOWS_PROGRAM_COUNT,
        "the Windows signing input has unexpected files"
    );
    for path in [built_program(root, target), desktop::launcher(root, target)] {
        let name = path.file_name().context("program file has no name")?;
        let signed = input.join(name);
        verify_windows_program(root, &signed, &expected)?;
        fs::write(&path, &fs::read(&signed)?)?;
    }
    verify_windows(root, target)
}

fn windows_sdk(root: &Path) -> Result<PathBuf> {
    let path = tool::capture(
        Tool::Pwsh.command(root).args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            WINDOWS_SDK_QUERY,
        ]),
        "locate the trusted Windows SDK verifier",
    )?;
    let path = PathBuf::from(path.trim());
    ensure!(
        path.is_absolute()
            && path.file_name().is_some_and(|name| name == "signtool.exe")
            && fs::is_file(&path)?,
        "the Windows SDK verifier path is invalid"
    );
    Ok(path)
}

fn windows_signature(root: &Path, program: &Path) -> std::process::Command {
    let script = "$ErrorActionPreference = 'Stop'; $s = Get-AuthenticodeSignature -LiteralPath $env:YOUAREL_SIGNATURE_PROGRAM; $c = $s.SignerCertificate; [pscustomobject]@{ status = [string]$s.Status; signer = if ($c) { [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($c.RawData)) } else { '' } } | ConvertTo-Json -Compress";
    let mut command = Tool::Pwsh.command(root);
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env(SIGNATURE_PATH, program);
    command
}

fn verify_windows_program(root: &Path, program: &Path, expected: &str) -> Result<()> {
    let reply = tool::capture(
        &mut windows_signature(root, program),
        "Get-AuthenticodeSignature",
    )?;
    let value: serde_json::Value =
        serde_json::from_str(reply.trim()).context("the signature check printed no JSON")?;
    let field = |name: &str| value.get(name).cloned().unwrap_or_default();
    ensure!(
        field("status") == "Valid",
        "{} has no valid signature: {reply}",
        program.display()
    );
    ensure!(
        field("signer")
            .as_str()
            .map(str::to_ascii_lowercase)
            .as_deref()
            == Some(expected),
        "{} is signed by another certificate",
        program.display()
    );
    let sdk = windows_sdk(root)?;
    tool::succeed(
        tool::built(root, &sdk)
            .args(["verify", "/all", "/pa", "/tw", "/v"])
            .arg(program),
        "verify every Authenticode signature and timestamp with the Windows SDK",
    )
}

/// The value of a letter digit of hexadecimal, past the ten decimal ones.
const HEX_LETTER: u8 = 10;
/// The bits one hexadecimal digit writes.
const NIBBLE_BITS: u32 = 4;
/// The hexadecimal digits that write one byte.
const DIGITS_PER_BYTE: usize = 2;

/// The value of one hexadecimal digit, in either case.
const fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit.wrapping_sub(b'0')),
        b'a'..=b'f' => Some(digit.wrapping_sub(b'a').wrapping_add(HEX_LETTER)),
        b'A'..=b'F' => Some(digit.wrapping_sub(b'A').wrapping_add(HEX_LETTER)),
        _ => None,
    }
}

/// The bytes that hexadecimal digits spell, white space ignored, or `None` for any other text.
fn unhex(text: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    let (pairs, rest) = digits.as_chunks::<DIGITS_PER_BYTE>();
    if !rest.is_empty() {
        return None;
    }
    pairs
        .iter()
        .map(|[high, low]| Some(hex_value(*high)?.wrapping_shl(NIBBLE_BITS) | hex_value(*low)?))
        .collect()
}

/// Creates both manifest keys in a fresh directory with private Doppler inputs and matching public trust anchors.
fn keygen(directory: &Path) -> Result<()> {
    let directory = fs::create_private_directory(directory)?;
    let mut seed = [0_u8; KEY_RANDOMNESS_BYTES];
    getrandom::fill(&mut seed).map_err(|error| anyhow::anyhow!("draw a release key: {error}"))?;
    let (secret, public) = trust::ml_dsa_key_pair(seed);
    let mut password_bytes = [0_u8; KEY_RANDOMNESS_BYTES];
    getrandom::fill(&mut password_bytes)
        .map_err(|error| anyhow::anyhow!("draw a minisign password: {error}"))?;
    let password = crate::hex(&password_bytes);
    let minisign = minisign::KeyPair::generate_encrypted_keypair(Some(password.clone()))
        .context("create an encrypted minisign key")?;
    let inputs = serde_json::json!({
        "RELEASE_MINISIGN_KEY": minisign.sk.to_box(None)?.into_string(),
        "RELEASE_MINISIGN_PASSWORD": password,
        "RELEASE_ML_DSA_KEY": crate::hex(&secret),
    });
    fs::write_private(&directory.join(KEY_INPUTS), &serde_json::to_vec(&inputs)?)?;
    let secret_path = directory.join(ML_DSA_SECRET_KEY);
    fs::write_private(&secret_path, crate::hex(&secret).as_bytes())?;
    fs::write(
        &directory.join(ML_DSA_PUBLIC_KEY),
        crate::hex(&public).as_bytes(),
    )?;
    fs::write(&directory.join(ML_DSA_PUBLIC_KEY_BYTES), &public)?;
    fs::write(
        &directory.join(MINISIGN_PUBLIC_KEY),
        minisign.pk.to_base64().as_bytes(),
    )?;
    println!(
        "Created private manifest inputs in {} and matching public trust anchors without signing or publication.",
        directory.display()
    );
    Ok(())
}

/// The target triple of this host, as `rustc -vV` names it.
fn host_target(root: &Path) -> Result<String> {
    let text = tool::capture(Tool::Rustc.command(root).arg("-vV"), "rustc -vV")?;
    text.lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::to_owned)
        .context("rustc names no host")
}

/// Builds, bundles, and signs a candidate for this host with keys made for this run, and checks every artifact without publishing anything.
fn candidate(root: &Path) -> Result<()> {
    let target = host_target(root)?;
    let dist = root.join(DIST);
    if fs::is_directory(&dist)? {
        fs::remove_dir_all(&dist)?;
    }
    build(root, &target)?;
    bundle(root, &target)?;
    write_manifest(root)?;
    let minisign_pair = minisign::KeyPair::generate_unencrypted_keypair()
        .context("make a rehearsal minisign key")?;
    let mut seed = [0_u8; 32];
    getrandom::fill(&mut seed).map_err(|error| anyhow::anyhow!("draw a rehearsal key: {error}"))?;
    let (ml_dsa_secret, ml_dsa_public) = trust::ml_dsa_key_pair(seed);
    let minisign_public = minisign_pair.pk.to_base64();
    let rehearsal = [ReleaseKey {
        minisign: &minisign_public,
        ml_dsa: &ml_dsa_public,
    }];
    let signed = write_signatures(&dist, &minisign_pair.sk, &ml_dsa_secret, &rehearsal)?;
    checksums(root)?;
    let version = workspace_version(root)?;
    ensure!(
        signed.version == version,
        "the manifest names {}, not {version}",
        signed.version
    );
    let program = fs::read(&built_program(root, &target))?;
    check_bundle(root, &target, version, &program)?;
    check_program(root, &target, &dist)?;
    println!(
        "candidate {version} for {target}: every artifact checked in {}",
        portable(&dist)
    );
    Ok(())
}

/// Reads every artifact of a candidate back: the archive holds the program and the documents, and the Debian package or the installer installs the program.
fn check_bundle(root: &Path, target: &str, version: Version, program: &[u8]) -> Result<()> {
    let dist = root.join(DIST);
    let folder = format!("{PROGRAM}-{version}-{target}");
    let entries = if target.contains("apple") {
        Vec::new()
    } else if target.contains("windows") {
        package::read_zip(&fs::read(&dist.join(format!("{folder}.zip")))?)?
    } else {
        package::read_tar_gz(&fs::read(&dist.join(format!("{folder}.tar.gz")))?)?
    };
    let program_entry = format!("{folder}/{}", program_name(target));
    ensure!(
        target.contains("apple")
            || entries.iter().any(|entry| entry.path == program_entry
                && entry.bytes == program
                && entry.mode == EXECUTABLE),
        "the archive lacks the executable program"
    );
    if !target.contains("apple") {
        let launcher = fs::read(&desktop::launcher(root, target))?;
        ensure!(
            entries.iter().any(|entry| entry.path
                == format!("{folder}/{}", desktop::launcher_name(target))
                && entry.bytes == launcher
                && entry.mode == EXECUTABLE),
            "the archive lacks the executable launcher"
        );
    }
    for document in DOCUMENTS.into_iter().filter(|_| !target.contains("apple")) {
        ensure!(
            entries
                .iter()
                .any(|entry| entry.path == format!("{folder}/{document}")),
            "the archive lacks {document}"
        );
    }
    if target.contains("linux") {
        let architecture = debian_architecture(target).unwrap_or_default();
        let deb = fs::read(&dist.join(format!("{PROGRAM}_{version}_{architecture}.deb")))?;
        let members = package::read_ar(&deb)?;
        let data = members
            .iter()
            .find(|(name, _)| name == "data.tar.gz")
            .context("the Debian package lacks its data")?;
        let installed = package::read_tar_gz(&data.1)?;
        ensure!(
            installed
                .iter()
                .any(|entry| entry.path == format!("usr/bin/{PROGRAM}") && entry.bytes == program),
            "the Debian package does not install the program"
        );
        let launcher = fs::read(&desktop::launcher(root, target))?;
        ensure!(
            installed
                .iter()
                .any(|entry| entry.path == "usr/bin/youarel-launcher" && entry.bytes == launcher),
            "the Debian package lacks the launcher"
        );
    }
    if target.contains("apple") {
        // A `._` entry holds the extended attributes of its file, which the installer applies to the file instead of installing it.
        // A workstation marks every file its sessions write with an attribute that no tool removes, and a hosted runner does not.
        let files = apple::payload_files(root, &dist.join(format!("{folder}-unsigned.pkg")))?;
        let installed: Vec<&str> = files
            .iter()
            .map(String::as_str)
            .filter(|file| !file.starts_with("./._"))
            .collect();
        for required in [
            "./Applications/youarel.app/Contents/MacOS/youarel",
            "./Applications/youarel.app/Contents/MacOS/youarel-launcher",
            "./usr/local/bin/youarel",
            "./usr/local/bin/youarel-launcher",
        ] {
            ensure!(
                installed.contains(&required),
                "the installer lacks {required}: {files:?}"
            );
        }
    }
    Ok(())
}

/// Runs the candidate program: it names its version, and refuses the rehearsal release because a build trusts only the compiled keys.
fn check_program(root: &Path, target: &str, dist: &Path) -> Result<()> {
    desktop::check_programs(root, target, &workspace_version(root)?.to_string())?;
    let program = built_program(root, target);
    let home = tempfile::tempdir().context("create a rehearsal home")?;
    let printed = tool::capture(
        tool::built(root, &program).arg("--version"),
        "youarel --version",
    )?;
    ensure!(
        printed.starts_with("youarel "),
        "youarel --version printed `{printed}`"
    );
    let refused = tool::output(
        tool::built(root, &program)
            .arg("self-update")
            .env("YOUAREL_HOME", home.path())
            .env("YOUAREL_RELEASES", dist),
        "youarel self-update",
    )?;
    let message = String::from_utf8_lossy(&refused.stderr);
    let rejection = if trust::RELEASE_KEYS.is_empty() {
        trust::TrustError::NoKey
    } else {
        trust::TrustError::Unsigned
    };
    ensure!(
        !refused.status.success() && message.contains(&rejection.to_string()),
        "a candidate program accepted a release signed by rehearsal keys: {message}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DOCUMENTS, SIGNATURE_PATH, bundle_files, fs, package, windows_signature};
    use youarel_core::update_rules::Version;

    #[cfg(unix)]
    #[test]
    fn generated_keys_prepare_matching_manifest_inputs_without_overwriting_an_identity() {
        use std::os::unix::fs::PermissionsExt as _;
        let workspace = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let directory = workspace.path().join("keys");
        super::keygen(&directory).unwrap();
        let original = fs::read(&directory.join(super::KEY_INPUTS)).unwrap();
        let inputs: serde_json::Value = serde_json::from_slice(&original).unwrap();
        let password = inputs["RELEASE_MINISIGN_PASSWORD"].as_str().unwrap();
        assert_eq!(
            password.len(),
            super::KEY_RANDOMNESS_BYTES.saturating_mul(2)
        );
        let secret =
            minisign::SecretKeyBox::from_string(inputs["RELEASE_MINISIGN_KEY"].as_str().unwrap())
                .unwrap();
        secret
            .clone()
            .into_secret_key(Some(String::new()))
            .unwrap_err();
        let secret = secret.into_secret_key(Some(password.to_owned())).unwrap();
        assert_eq!(
            minisign::PublicKey::from_secret_key(&secret)
                .unwrap()
                .to_base64(),
            fs::read_to_string(&directory.join(super::MINISIGN_PUBLIC_KEY)).unwrap()
        );
        let ml_dsa = super::unhex(inputs["RELEASE_ML_DSA_KEY"].as_str().unwrap()).unwrap();
        assert_eq!(
            ml_dsa.len(),
            youarel_release::trust::ML_DSA_SIGNING_KEY_BYTES
        );
        assert_eq!(
            fs::read_to_string(&directory.join(super::ML_DSA_SECRET_KEY)).unwrap(),
            inputs["RELEASE_ML_DSA_KEY"].as_str().unwrap()
        );
        assert_eq!(
            super::unhex(&fs::read_to_string(&directory.join(super::ML_DSA_PUBLIC_KEY)).unwrap())
                .unwrap(),
            fs::read(&directory.join(super::ML_DSA_PUBLIC_KEY_BYTES)).unwrap()
        );
        assert_prepared_signatures(&secret, &ml_dsa, &directory);
        let refusal = super::keygen(&directory).unwrap_err();
        assert_eq!(
            refusal.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            fs::read(&directory.join(super::KEY_INPUTS)).unwrap(),
            original
        );
        {
            assert_eq!(
                fs::entry_metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::entry_metadata(&directory.join(super::KEY_INPUTS))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    fn assert_prepared_signatures(
        secret: &minisign::SecretKey,
        ml_dsa: &[u8],
        directory: &std::path::Path,
    ) {
        let manifest = youarel_release::manifest::Manifest {
            version: Version::parse("1.0.0").unwrap(),
            assets: std::collections::BTreeMap::new(),
        };
        let encoded = youarel_release::manifest::encode(&manifest).unwrap();
        let (classical, post_quantum) =
            super::signatures(encoded.as_bytes(), secret, ml_dsa).unwrap();
        let public = fs::read_to_string(&directory.join(super::MINISIGN_PUBLIC_KEY)).unwrap();
        let ml_dsa_public = fs::read(&directory.join(super::ML_DSA_PUBLIC_KEY_BYTES)).unwrap();
        assert_eq!(
            youarel_release::trust::verify(
                encoded.as_bytes(),
                &youarel_release::trust::Signatures {
                    minisign: &classical,
                    ml_dsa: &post_quantum,
                },
                &[youarel_release::trust::ReleaseKey {
                    minisign: &public,
                    ml_dsa: &ml_dsa_public,
                }],
            )
            .unwrap(),
            manifest
        );
    }

    #[cfg(not(unix))]
    #[test]
    fn key_generation_refuses_hosts_without_private_directory_support() {
        let workspace = tempfile::tempdir().unwrap();
        let directory = workspace.path().join("keys");
        let refusal = super::keygen(&directory).unwrap_err();
        assert!(format!("{refusal:#}").contains("Unix"));
        assert_eq!(fs::kind(&directory).unwrap(), None);
    }

    #[test]
    fn windows_signature_paths_remain_literal_data_outside_the_powershell_program() {
        let root = tempfile::tempdir().unwrap();
        let path = std::path::Path::new("C:\\folder with spaces\\'$();[name].exe");
        let command = windows_signature(root.path(), path);
        assert!(
            command
                .get_envs()
                .any(|(name, value)| name == SIGNATURE_PATH && value == Some(path.as_os_str()))
        );
        assert!(
            !command
                .get_args()
                .any(|argument| argument == path.as_os_str())
        );
    }

    #[test]
    fn desktop_archives_ship_the_launcher_beside_the_cli() {
        for target in ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            for document in DOCUMENTS {
                fs::write(&root.join(document), b"license").unwrap();
            }
            fs::create_dir_all(&root.join("app/icons")).unwrap();
            fs::write(&root.join("app/icons/128x128.png"), b"png").unwrap();
            let extension = if target.contains("windows") {
                ".exe"
            } else {
                ""
            };
            let built = root.join("app/target").join(target).join("release");
            fs::create_dir_all(&built).unwrap();
            fs::write(&built.join(format!("youarel-launcher{extension}")), b"gui").unwrap();
            let version = Version::parse("0.4.2").unwrap();
            let files = bundle_files(root, target, version, b"cli").unwrap();
            let (_, archive) = files
                .iter()
                .find(|(name, _)| {
                    name.ends_with(if extension.is_empty() {
                        ".tar.gz"
                    } else {
                        ".zip"
                    })
                })
                .unwrap();
            let entries = if extension.is_empty() {
                package::read_tar_gz(archive)
            } else {
                package::read_zip(archive)
            }
            .unwrap();
            assert!(entries.iter().any(|entry| {
                entry
                    .path
                    .ends_with(&format!("/youarel-launcher{extension}"))
                    && entry.bytes == b"gui"
            }));
            assert!(entries.iter().any(
                |entry| entry.path.ends_with(&format!("/youarel{extension}"))
                    && entry.bytes == b"cli"
            ));
        }
    }
}
