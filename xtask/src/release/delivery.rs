use super::{DIST, candidate, desktop, ensure_enabled, required};
use crate::fs;
use crate::tool::{self, Tool};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use youarel_release::manifest::{Digest, Name};
use youarel_release::trust::{self, Signatures};

pub(super) const PROVENANCE: &str = "artifact-provenance.jsonl";
const REPOSITORY: &str = "P4suta/youarel";
const REMOTE: &str = "https://github.com/P4suta/youarel.git";
const WORKFLOW: &str = "P4suta/youarel/.github/workflows/release.yml";

pub(super) fn plan(operation: &str) -> Result<()> {
    ensure!(
        ["draft", "publish"].contains(&operation),
        "unknown publication operation"
    );
    let source = required("RELEASE_SOURCE_SHA")?;
    let version = required("RELEASE_VERSION")?;
    let run = required("RELEASE_BUILD_RUN")?;
    ensure!(
        candidate::valid_source(&source),
        "source is not a full commit SHA"
    );
    ensure!(
        youarel_core::update_rules::Version::parse(&version).is_some() && version != "0.0.0",
        "version is not reserved"
    );
    ensure!(
        !run.is_empty() && run.bytes().all(|byte| byte.is_ascii_digit()),
        "build run is not numeric"
    );
    println!(
        "Operation: {operation}\nSource: {source}\nVersion: {version}\nOriginal build run: {run}\nRequired checks: successful original release workflow, signed asset inventory, all target packages, original provenance, immutable tag binding, remote asset digests"
    );
    println!(
        "External effects: {}",
        if operation == "draft" {
            "create a draft release and upload missing verified assets"
        } else {
            "publish the complete existing draft and create its immutable version tag"
        }
    );
    ensure_enabled()
}
const SIGNATURE_FILES: [&str; 3] = [
    "manifest.json",
    "manifest.json.minisig",
    "manifest.json.mldsa",
];

pub(super) fn distributed(name: &str) -> bool {
    !name.starts_with("asset-") && !name.ends_with("-unsigned.pkg")
}

pub(super) fn manifested(name: &str) -> bool {
    distributed(name)
        && name != "SHA256SUMS"
        && name != PROVENANCE
        && !SIGNATURE_FILES.contains(&name)
}

fn files(root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut found = BTreeMap::new();
    for path in fs::entries(&root.join(DIST))? {
        let metadata = fs::entry_metadata(&path)?;
        ensure!(!metadata.is_symlink(), "release contains a symbolic link");
        if metadata.is_file() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .context("release file name is not UTF-8")?;
            ensure!(Name::parse(name).is_some(), "unsafe release asset name");
            if distributed(name) {
                found.insert(name.to_owned(), path);
            }
        }
    }
    Ok(found)
}

#[derive(Debug)]
struct Identity {
    source: String,
    version: String,
    run: String,
}

fn identity(root: &Path) -> Result<Identity> {
    let expected = Identity {
        source: required("RELEASE_SOURCE_SHA")?,
        version: required("RELEASE_VERSION")?,
        run: required("RELEASE_BUILD_RUN")?,
    };
    ensure!(
        candidate::valid_source(&expected.source),
        "release source is not a full commit SHA"
    );
    ensure!(
        !expected.run.is_empty() && expected.run.bytes().all(|byte| byte.is_ascii_digit()),
        "release build run is not numeric"
    );
    ensure!(
        expected.version != "0.0.0"
            && youarel_core::update_rules::Version::parse(&expected.version).is_some(),
        "release version is not reserved"
    );
    for target in desktop::TARGETS {
        let record: candidate::Candidate = serde_json::from_slice(&fs::read(
            &root.join(DIST).join(format!("candidate-{target}.json")),
        )?)?;
        ensure!(
            record.source == expected.source
                && record.version == expected.version
                && record.run == expected.run
                && record
                    .attempt
                    .parse::<u32>()
                    .is_ok_and(|attempt| attempt > 0)
                && record.builder == "P4suta/youarel/.github/workflows/release.yml@refs/heads/main"
                && record.target == target,
            "{target} belongs to another candidate"
        );
        for name in [
            super::asset_name(target),
            format!("youarel-{target}.cdx.json"),
            format!("youarel-launcher-{target}.cdx.json"),
        ] {
            ensure!(
                fs::is_file(&root.join(DIST).join(&name))?,
                "release lacks {name}"
            );
        }
        let package = if target.contains("apple") {
            format!("youarel-{}-{target}.pkg", expected.version)
        } else if target.contains("windows") {
            format!("youarel-{}-{target}.zip", expected.version)
        } else {
            format!("youarel-{}-{target}.tar.gz", expected.version)
        };
        ensure!(
            fs::is_file(&root.join(DIST).join(&package))?,
            "release lacks {package}"
        );
        if target.contains("linux") {
            let architecture =
                super::debian_architecture(target).context("unsupported Debian target")?;
            ensure!(
                fs::is_file(
                    &root
                        .join(DIST)
                        .join(format!("youarel_{}_{architecture}.deb", expected.version))
                )?,
                "release lacks the Debian installer"
            );
        }
    }
    Ok(expected)
}

pub(super) fn verify(root: &Path) -> Result<()> {
    let identity = identity(root)?;
    let dist = root.join(DIST);
    let manifest_bytes = fs::read(&dist.join("manifest.json"))?;
    let minisign = fs::read_to_string(&dist.join("manifest.json.minisig"))?;
    let ml_dsa = fs::read(&dist.join("manifest.json.mldsa"))?;
    let manifest = trust::verify(
        &manifest_bytes,
        &Signatures {
            minisign: &minisign,
            ml_dsa: &ml_dsa,
        },
        trust::RELEASE_KEYS,
    )?;
    ensure!(
        manifest.version.to_string() == identity.version,
        "signed manifest version mismatch"
    );
    let files = files(root)?;
    let mut expected = BTreeMap::new();
    for asset in manifest.assets.values() {
        ensure!(
            expected
                .insert(asset.name.as_str().to_owned(), asset.sha256.hex())
                .is_none(),
            "the manifest repeats an asset"
        );
        let bytes = fs::read(
            files
                .get(asset.name.as_str())
                .context("a signed asset is missing")?,
        )?;
        ensure!(
            asset.sha256 == Digest::of(&bytes) && asset.size == u64::try_from(bytes.len())?,
            "signed asset digest mismatch: {}",
            asset.name.as_str()
        );
    }
    for name in files.keys().filter(|name| manifested(name)) {
        ensure!(
            expected.contains_key(name),
            "unmanifested release asset: {name}"
        );
    }
    let checksums = fs::read_to_string(&dist.join("SHA256SUMS"))?;
    let actual: BTreeMap<String, String> = checksums
        .lines()
        .map(|line| {
            let (digest, name) = line.split_once("  ").context("invalid checksum line")?;
            ensure!(
                Digest::parse(digest).is_some() && Name::parse(name).is_some(),
                "invalid checksum identity"
            );
            Ok((name.to_owned(), digest.to_owned()))
        })
        .collect::<Result<_>>()?;
    let wanted: BTreeMap<String, String> = files
        .iter()
        .filter(|(name, _)| name.as_str() != "SHA256SUMS" && name.as_str() != PROVENANCE)
        .map(|(name, path)| Ok((name.clone(), Digest::of(&fs::read(path)?).hex())))
        .collect::<Result<_>>()?;
    ensure!(
        actual.len() == checksums.lines().count() && actual == wanted,
        "checksums omit, repeat, or mismatch release assets"
    );
    Ok(())
}

pub(super) fn verify_provenance(root: &Path) -> Result<()> {
    verify(root)?;
    let identity = identity(root)?;
    for target in desktop::TARGETS {
        let bundle = root
            .join(DIST)
            .join(format!("build-provenance-{target}.jsonl"));
        for name in [
            format!("candidate-{target}.json"),
            format!("youarel-{target}.cdx.json"),
            format!("youarel-launcher-{target}.cdx.json"),
        ] {
            attest(root, &root.join(DIST).join(name), &bundle, &identity.source)?;
        }
    }
    for (name, path) in files(root)? {
        if name == "SHA256SUMS" || name == PROVENANCE {
            continue;
        }
        attest(
            root,
            &path,
            &root.join(DIST).join(PROVENANCE),
            &identity.source,
        )?;
    }
    Ok(())
}

fn attest(root: &Path, artifact: &Path, bundle: &Path, source: &str) -> Result<()> {
    tool::succeed(
        Tool::Gh
            .command(root)
            .args(["attestation", "verify"])
            .arg(artifact)
            .args(["--bundle"])
            .arg(bundle)
            .args([
                "--repo",
                REPOSITORY,
                "--signer-workflow",
                WORKFLOW,
                "--source-digest",
                source,
                "--source-ref",
                "refs/heads/main",
                "--deny-self-hosted-runners",
            ]),
        "verify original artifact provenance",
    )
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteAsset {
    name: String,
    digest: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Release {
    is_draft: bool,
    tag_name: String,
    target_commitish: String,
    assets: Vec<RemoteAsset>,
}

fn current_release(root: &Path, tag: &str) -> Result<Option<Release>> {
    let listing = tool::capture(
        Tool::Gh.command(root).args([
            "api",
            "repos/P4suta/youarel/releases",
            "--paginate",
            "--slurp",
        ]),
        "inspect releases before mutation",
    )?;
    let pages: Vec<Vec<serde_json::Value>> = serde_json::from_str(&listing)?;
    let found = pages
        .into_iter()
        .flatten()
        .find(|release| release.get("tag_name").and_then(serde_json::Value::as_str) == Some(tag));
    if found.is_none() {
        return Ok(None);
    }
    let reply = tool::capture(
        Tool::Gh.command(root).args([
            "release",
            "view",
            tag,
            "--repo",
            REPOSITORY,
            "--json",
            "isDraft,tagName,targetCommitish,assets",
        ]),
        "inspect the existing release",
    )?;
    Ok(Some(serde_json::from_str(&reply)?))
}

fn check_tag(root: &Path, tag: &str, source: &str) -> Result<()> {
    let direct = format!("refs/tags/{tag}");
    let peeled = format!("{direct}^{{}}");
    let reply = tool::capture(
        Tool::Git
            .command(root)
            .args(["ls-remote", "--tags", REMOTE, &direct, &peeled]),
        "inspect the immutable release tag",
    )?;
    let mut refs = BTreeMap::new();
    for line in reply.lines() {
        let (sha, name) = line
            .split_once('\t')
            .context("invalid remote tag listing")?;
        ensure!(
            candidate::valid_source(sha) && [direct.as_str(), peeled.as_str()].contains(&name),
            "unexpected tag identity"
        );
        ensure!(refs.insert(name, sha).is_none(), "ambiguous tag identity");
    }
    if let Some(commit) = refs
        .get(peeled.as_str())
        .or_else(|| refs.get(direct.as_str()))
    {
        ensure!(
            *commit == source,
            "the version tag identifies another source"
        );
    }
    Ok(())
}

fn missing_assets(existing: &Release, expected: &BTreeMap<String, String>) -> Result<Vec<String>> {
    ensure!(
        existing.is_draft,
        "the release is already published and immutable"
    );
    let mut seen = BTreeMap::new();
    for asset in &existing.assets {
        let digest = asset
            .digest
            .as_deref()
            .context("the existing asset has no verifiable digest")?;
        ensure!(
            expected
                .get(&asset.name)
                .is_some_and(|expected| digest == format!("sha256:{expected}")),
            "the draft contains a mismatched or unexpected asset: {}",
            asset.name
        );
        ensure!(
            seen.insert(asset.name.clone(), digest).is_none(),
            "the draft repeats an asset"
        );
    }
    Ok(expected
        .keys()
        .filter(|name| !seen.contains_key(*name))
        .cloned()
        .collect())
}

fn expected_assets(root: &Path) -> Result<BTreeMap<String, String>> {
    files(root)?
        .into_iter()
        .map(|(name, path)| Ok((name, Digest::of(&fs::read(&path)?).hex())))
        .collect()
}

fn check_identity(release: &Release, identity: &Identity) -> Result<()> {
    ensure!(
        release.tag_name == format!("v{}", identity.version)
            && release.target_commitish == identity.source,
        "the release draft identifies another source or version"
    );
    Ok(())
}

pub(super) fn draft(root: &Path) -> Result<()> {
    ensure_enabled()?;
    repository_policy(root)?;
    verify_provenance(root)?;
    let identity = identity(root)?;
    let tag = format!("v{}", identity.version);
    check_tag(root, &tag, &identity.source)?;
    let expected = expected_assets(root)?;
    if current_release(root, &tag)?.is_none() {
        let notes = format!(
            "youarel {}\n\nSource: {}\nBuild: https://github.com/{REPOSITORY}/actions/runs/{}\n\nVerify downloads against the signed manifest, SHA256SUMS, and artifact-provenance.jsonl.\n",
            identity.version, identity.source, identity.run
        );
        tool::succeed(
            Tool::Gh.command(root).args([
                "release",
                "create",
                &tag,
                "--repo",
                REPOSITORY,
                "--draft",
                "--target",
                &identity.source,
                "--title",
                &tag,
                "--notes",
                &notes,
            ]),
            "create the verified release draft",
        )?;
    }
    let release =
        current_release(root, &tag)?.context("the release draft is missing after creation")?;
    check_identity(&release, &identity)?;
    for name in missing_assets(&release, &expected)? {
        tool::succeed(
            Tool::Gh
                .command(root)
                .args(["release", "upload", &tag, "--repo", REPOSITORY])
                .arg(root.join(DIST).join(name)),
            "upload a missing verified asset",
        )?;
    }
    let observed = current_release(root, &tag)?.context("the release draft disappeared")?;
    check_identity(&observed, &identity)?;
    ensure!(
        missing_assets(&observed, &expected)?.is_empty(),
        "the draft is incomplete"
    );
    println!(
        "Draft {tag} contains every verified asset; publication remains a separate operation."
    );
    Ok(())
}

pub(super) fn publish(root: &Path) -> Result<()> {
    ensure_enabled()?;
    repository_policy(root)?;
    verify_provenance(root)?;
    let identity = identity(root)?;
    let tag = format!("v{}", identity.version);
    check_tag(root, &tag, &identity.source)?;
    let release =
        current_release(root, &tag)?.context("prepare the complete draft before publication")?;
    check_identity(&release, &identity)?;
    ensure!(
        missing_assets(&release, &expected_assets(root)?)?.is_empty(),
        "the release draft is incomplete"
    );
    tool::succeed(
        Tool::Gh.command(root).args([
            "release",
            "edit",
            &tag,
            "--repo",
            REPOSITORY,
            "--draft=false",
        ]),
        "publish the complete immutable release",
    )?;
    let observed = current_release(root, &tag)?.context("the published release is missing")?;
    ensure!(!observed.is_draft, "GitHub has not confirmed publication");
    check_tag(root, &tag, &identity.source)?;
    println!(
        "Published {tag} from {} with build {}.",
        identity.source, identity.run
    );
    Ok(())
}

pub(super) fn copy_provenance(root: &Path, input: &Path) -> Result<()> {
    let bytes = fs::read(input)?;
    ensure!(!bytes.is_empty(), "the provenance bundle is empty");
    fs::write(&root.join(DIST).join(PROVENANCE), &bytes)
}

fn protected_version_tags(value: &serde_json::Value) -> bool {
    value.get("target").and_then(serde_json::Value::as_str) == Some("tag")
        && value.get("enforcement").and_then(serde_json::Value::as_str) == Some("active")
        && value
            .get("bypass_actors")
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty)
        && value
            .pointer("/conditions/ref_name/include")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|patterns| {
                patterns
                    .iter()
                    .any(|pattern| pattern.as_str() == Some("refs/tags/v*"))
            })
        && value
            .pointer("/conditions/ref_name/exclude")
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty)
        && ["update", "deletion"].iter().all(|required| {
            value
                .get("rules")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|rules| {
                    rules.iter().any(|rule| {
                        rule.get("type").and_then(serde_json::Value::as_str) == Some(*required)
                    })
                })
        })
}

fn repository_policy(root: &Path) -> Result<()> {
    let immutable_reply = tool::capture(
        Tool::Gh
            .command(root)
            .args(["api", "repos/P4suta/youarel/immutable-releases"]),
        "verify immutable release protection",
    )?;
    let immutable: serde_json::Value = serde_json::from_str(&immutable_reply)?;
    ensure!(
        immutable
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            == Some(true),
        "enable Immutable Releases before publication"
    );
    let ruleset_listing = tool::capture(
        Tool::Gh.command(root).args([
            "api",
            "repos/P4suta/youarel/rulesets",
            "--paginate",
            "--slurp",
        ]),
        "inspect version tag protections",
    )?;
    let pages: Vec<Vec<serde_json::Value>> = serde_json::from_str(&ruleset_listing)?;
    for ruleset in pages
        .into_iter()
        .flatten()
        .filter(|ruleset| ruleset.get("target").and_then(serde_json::Value::as_str) == Some("tag"))
    {
        let id = ruleset
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .context("ruleset ID is malformed")?;
        let path = format!("repos/P4suta/youarel/rulesets/{id}");
        let ruleset_detail = tool::capture(
            Tool::Gh.command(root).args(["api", &path]),
            "verify immutable version tags",
        )?;
        if protected_version_tags(&serde_json::from_str(&ruleset_detail)?) {
            return Ok(());
        }
    }
    anyhow::bail!("protect version tag updates and deletions without bypass before publication")
}

#[cfg(test)]
mod tests {
    use super::{Release, RemoteAsset, missing_assets, protected_version_tags};
    use std::collections::BTreeMap;

    fn draft() -> Release {
        Release {
            is_draft: true,
            tag_name: "v0.4.2".to_owned(),
            target_commitish: "a".repeat(40),
            assets: vec![RemoteAsset {
                name: "first".to_owned(),
                digest: Some("sha256:abc".to_owned()),
            }],
        }
    }

    #[test]
    fn publication_requires_active_version_tag_protection_without_bypass_or_exclusions() {
        let protected = serde_json::json!({"target":"tag", "enforcement":"active", "bypass_actors":[], "conditions":{"ref_name":{"include":["refs/tags/v*"], "exclude":[]}}, "rules":[{"type":"update"},{"type":"deletion"}]});
        assert!(protected_version_tags(&protected));
        for (field, value) in [
            ("enforcement", serde_json::json!("disabled")),
            ("bypass_actors", serde_json::json!([{"actor_id":1}])),
            ("rules", serde_json::json!([{"type":"deletion"}])),
        ] {
            let mut changed = protected.clone();
            changed[field] = value;
            assert!(!protected_version_tags(&changed));
        }
        let mut excluded = protected;
        excluded["conditions"]["ref_name"]["exclude"] = serde_json::json!(["refs/tags/v0.*"]);
        assert!(!protected_version_tags(&excluded));
    }

    #[test]
    fn a_matching_partial_draft_adds_only_missing_assets() {
        let expected = BTreeMap::from([
            ("first".to_owned(), "abc".to_owned()),
            ("second".to_owned(), "def".to_owned()),
        ]);
        assert_eq!(missing_assets(&draft(), &expected).unwrap(), ["second"]);
    }

    #[test]
    fn published_unknown_or_changed_assets_are_never_replaced() {
        let expected = BTreeMap::from([("first".to_owned(), "abc".to_owned())]);
        let mut published = draft();
        published.is_draft = false;
        missing_assets(&published, &expected).unwrap_err();
        let mut changed = draft();
        changed.assets.first_mut().unwrap().digest = Some("sha256:changed".to_owned());
        missing_assets(&changed, &expected).unwrap_err();
        let mut unknown = draft();
        unknown.assets.first_mut().unwrap().digest = None;
        missing_assets(&unknown, &expected).unwrap_err();
        missing_assets(&draft(), &BTreeMap::new()).unwrap_err();
    }
}
