//! The release tasks: candidate builds and their packages, the signed manifest, the platform signatures, and publication.
//!
//! The release workflows run these only while the repository variable `RELEASES_ENABLED` is `true`, and `publish` refuses otherwise.
//! `release candidate` builds and checks the same artifacts on a workstation without any credential, signed with keys made for that run alone.

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

/// The directory a release assembles its artifacts in.
pub const DIST: &str = "dist";
/// The variable that enables releasing, which stays unset while releases are prohibited.
pub const ENABLED: &str = "RELEASES_ENABLED";
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
const SUMMARY: &str = "Open your web shortcuts in any browser, from a terminal or the command line";
/// Where the release keeps the receipts of its notarization submissions.
const NOTARIZATION: &str = "notarization";
/// The trusted comment of a manifest's minisign signature.
const TRUSTED_COMMENT: &str = "youarel release manifest";

const USAGE: &str = "usage: cargo xtask release version | build TARGET | bundle TARGET | manifest | sign | checksums | sign-macos program|package TARGET | notarize submit|finish | verify-windows TARGET | publish | candidate | keygen DIRECTORY";

/// Runs one release task.
///
/// # Errors
///
/// Returns an error naming the step that failed.
pub fn run(root: &Path, words: &[&str]) -> Result<()> {
    match words {
        ["version"] => version(root).map(drop),
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
        ["publish"] => publish(root),
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
    ensure!(
        Name::parse(target).is_some(),
        "`{target}` is not a target triple"
    );
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
    )
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
    let time = source_date()?;
    let folder = format!("{PROGRAM}-{version}-{target}");
    let mut entries = vec![Entry {
        path: format!("{folder}/{}", program_name(target)),
        mode: EXECUTABLE,
        bytes: program.to_vec(),
    }];
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
        };
        let installed = [
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
        apple::build_package(root, &program_path, &version.to_string(), &unsigned)?;
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
    let keychain = apple::Keychain::create(root, &imports)?;
    work(&keychain)
}

/// Signs a macOS target's built program with the Developer ID Application identity, before it is bundled.
fn sign_macos_program(root: &Path, target: &str) -> Result<()> {
    ensure!(target.contains("apple"), "{target} is not a macOS target");
    let identity = required("APPLE_SIGNING_IDENTITY")?;
    with_keychain(root, |keychain| {
        keychain.sign_program(&identity, &built_program(root, target))
    })
}

/// Signs a macOS target's bundled installer with the Developer ID Installer identity.
fn sign_macos_package(root: &Path, target: &str) -> Result<()> {
    let version = workspace_version(root)?;
    let identity = required("APPLE_INSTALLER_IDENTITY")?;
    let dist = root.join(DIST);
    let unsigned = dist.join(format!("{PROGRAM}-{version}-{target}-unsigned.pkg"));
    let signed = dist.join(format!("{PROGRAM}-{version}-{target}.pkg"));
    with_keychain(root, |keychain| {
        keychain.sign_package(&identity, &unsigned, &signed)
    })
}

/// A notarization submission the release persisted, bound to the package bytes it submitted.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
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
                println!("{name} was submitted already; the finish step resumes it");
                continue;
            }
            let sha256 = Digest::of(&fs::read(&package)?).hex();
            let id = apple::submit(root, key, &package)?;
            let receipt = Receipt {
                id,
                package: name,
                sha256,
            };
            fs::write(
                &receipt_path,
                serde_json::to_string_pretty(&receipt)?.as_bytes(),
            )?;
        }
        Ok(())
    })
}

/// Waits for every persisted submission, then staples and checks its package.
fn notarize_finish(root: &Path) -> Result<()> {
    let dist = root.join(DIST);
    with_notary_key(|key| {
        for path in fs::entries(&dist.join(NOTARIZATION))? {
            let receipt: Receipt = serde_json::from_slice(&fs::read(&path)?)
                .with_context(|| format!("{} is not a notarization receipt", portable(&path)))?;
            let package = dist.join(&receipt.package);
            ensure!(
                Digest::of(&fs::read(&package)?).hex() == receipt.sha256,
                "{} changed after its submission",
                receipt.package
            );
            apple::finish(root, key, &receipt.id, &package)?;
        }
        Ok(())
    })
}

/// Checks the Authenticode signature of a Windows target's program: valid, timestamped, and from the expected publisher.
fn verify_windows(root: &Path, target: &str) -> Result<()> {
    let expected = required("WINDOWS_SIGNING_IDENTITY_SHA256")?.to_ascii_lowercase();
    let program = built_program(root, target);
    let script = "$s = Get-AuthenticodeSignature -LiteralPath $args[0]; $c = $s.SignerCertificate; [pscustomobject]@{ status = [string]$s.Status; signer = if ($c) { [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($c.RawData)) } else { '' }; timestamped = [bool]$s.TimeStamperCertificate } | ConvertTo-Json -Compress";
    let reply = tool::capture(
        Tool::Pwsh
            .command(root)
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .arg(&program),
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
        field("timestamped") == true,
        "{} carries no timestamp",
        program.display()
    );
    ensure!(
        field("signer")
            .as_str()
            .map(str::to_ascii_lowercase)
            .as_deref()
            == Some(expected.as_str()),
        "{} is signed by another certificate",
        program.display()
    );
    Ok(())
}

/// Publishes the verified `dist` as an immutable GitHub release, after a draft holds every asset with its digest.
fn publish(root: &Path) -> Result<()> {
    ensure!(
        variable(ENABLED)?.as_deref() == Some("true"),
        "releases are disabled: {ENABLED} is not `true`"
    );
    let version = version(root)?;
    let tag = format!("v{version}");
    let dist = root.join(DIST);
    let mut files = Vec::new();
    for path in fs::entries(&dist)? {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if fs::is_file(&path)? && !name.starts_with("asset-") && !name.ends_with("-unsigned.pkg") {
            files.push(path);
        }
    }
    let notes = format!(
        "youarel {version}\n\nVerify downloads with SHA256SUMS, or with the signed manifest that `youarel self-update` checks.\n"
    );
    tool::succeed(
        Tool::Gh
            .command(root)
            .args([
                "release",
                "create",
                &tag,
                "--draft",
                "--verify-tag",
                "--title",
                &tag,
                "--notes",
                &notes,
            ])
            .args(&files),
        "gh release create --draft",
    )?;
    let listing = tool::capture(
        Tool::Gh
            .command(root)
            .args(["release", "view", &tag, "--json", "assets"]),
        "gh release view",
    )?;
    let value: serde_json::Value = serde_json::from_str(&listing).context("gh printed no JSON")?;
    let uploaded: BTreeMap<String, String> = value
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            Some((
                asset.get("name")?.as_str()?.to_owned(),
                asset.get("digest")?.as_str()?.to_owned(),
            ))
        })
        .collect();
    for path in &files {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let digest = format!("sha256:{}", Digest::of(&fs::read(path)?).hex());
        ensure!(
            uploaded.get(name) == Some(&digest),
            "the draft's {name} does not match the verified file"
        );
    }
    tool::succeed(
        Tool::Gh
            .command(root)
            .args(["release", "edit", &tag, "--draft=false"]),
        "gh release edit --draft=false",
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

/// Makes a release ML-DSA-65 key pair in `directory`: the signing key, which only its owner may read, for the release environment's secret, and the verification key for the keys the program trusts.
///
/// The minisign half comes from `minisign -G`, which protects its secret key with a password.
fn keygen(directory: &Path) -> Result<()> {
    let mut seed = [0_u8; 32];
    getrandom::fill(&mut seed).map_err(|error| anyhow::anyhow!("draw a release key: {error}"))?;
    let (secret, public) = trust::ml_dsa_key_pair(seed);
    fs::create_dir_all(directory)?;
    let secret_path = directory.join("ml-dsa.secret");
    fs::write_private(&secret_path, crate::hex(&secret).as_bytes())?;
    fs::write(
        &directory.join("ml-dsa.public"),
        crate::hex(&public).as_bytes(),
    )?;
    println!(
        "wrote {} for the RELEASE_ML_DSA_KEY secret and ml-dsa.public for trust::RELEASE_KEYS; make the minisign half with `minisign -G`",
        secret_path.display()
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
    let entries = if target.contains("windows") {
        package::read_zip(&fs::read(&dist.join(format!("{folder}.zip")))?)?
    } else {
        package::read_tar_gz(&fs::read(&dist.join(format!("{folder}.tar.gz")))?)?
    };
    let program_entry = format!("{folder}/{}", program_name(target));
    ensure!(
        entries.iter().any(|entry| entry.path == program_entry
            && entry.bytes == program
            && entry.mode == EXECUTABLE),
        "the archive lacks the executable program"
    );
    for document in DOCUMENTS {
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
        ensure!(
            installed == [".", "./youarel"],
            "the installer installs more or less than the program: {files:?}"
        );
    }
    Ok(())
}

/// Runs the candidate program: it names its version, and refuses the rehearsal release because a build trusts only the compiled keys.
fn check_program(root: &Path, target: &str, dist: &Path) -> Result<()> {
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
    ensure!(
        !refused.status.success() && message.contains("trusts no release key"),
        "a candidate program accepted a release signed by rehearsal keys: {message}"
    );
    Ok(())
}
