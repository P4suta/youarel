use super::{DIST, built_program, desktop, proposal, required, source_date, variable};
use crate::fs;
use crate::package::{self, EXECUTABLE, Entry, REGULAR};
use crate::tool::{self, Tool};
use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeSet;
use std::path::Path;
use youarel_release::manifest::Digest;

pub(super) const DIRECTORY: &str = "release-candidate";
pub(super) const BUILDER: &str = "P4suta/youarel/.github/workflows/release.yml@refs/heads/main";
const RECORD: &str = "candidate.json";
const PAYLOAD: &str = "payload.tar.gz";
const SOURCE_LENGTH: usize = 40;
const CLI_BOM: &str = "crates/youarel-cli/youarel_bin.cdx.json";
const APP_BOM: &str = "app/youarel-launcher_bin.cdx.json";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Candidate {
    pub source: String,
    pub version: String,
    pub run: String,
    pub attempt: String,
    pub builder: String,
    pub target: String,
    pub payload_sha256: String,
}

pub(super) fn valid_source(source: &str) -> bool {
    source.len() == SOURCE_LENGTH
        && source
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn source(root: &Path) -> Result<String> {
    let head = tool::capture(
        Tool::Git.command(root).args(["rev-parse", "HEAD"]),
        "read candidate source",
    )?
    .trim()
    .to_owned();
    ensure!(
        valid_source(&head),
        "the candidate source is not a full commit SHA"
    );
    if let Some(requested) = variable("RELEASE_SOURCE_SHA")? {
        ensure!(
            requested == head,
            "requested source differs from the checked-out candidate"
        );
    }
    if let Some(workflow) = variable("GITHUB_SHA")? {
        ensure!(
            workflow == head,
            "the release controller and candidate source differ"
        );
        let changes = tool::capture(
            Tool::Git
                .command(root)
                .args(["status", "--porcelain", "--untracked-files=all"]),
            "check the immutable candidate checkout",
        )?;
        ensure!(
            changes.trim().is_empty(),
            "the candidate checkout changed after selecting its source"
        );
    }
    Ok(head)
}

pub(super) fn valid_workflow_source(root: &Path) -> Result<()> {
    source(root).map(drop)
}

fn identity(root: &Path, target: &str) -> Result<Candidate> {
    desktop::supported(target)?;
    let version = proposal::versions(root)?.to_string();
    if let Some(requested) = variable("RELEASE_VERSION")? {
        ensure!(
            requested == version,
            "requested version differs from the product version"
        );
    }
    Ok(Candidate {
        source: source(root)?,
        version,
        run: variable("GITHUB_RUN_ID")?.unwrap_or_else(|| "local".to_owned()),
        attempt: variable("GITHUB_RUN_ATTEMPT")?.unwrap_or_else(|| "1".to_owned()),
        builder: variable("GITHUB_WORKFLOW_REF")?.unwrap_or_else(|| "local".to_owned()),
        target: target.to_owned(),
        payload_sha256: String::new(),
    })
}

fn restore_identity(root: &Path, target: &str) -> Result<Candidate> {
    let mut expected = identity(root, target)?;
    if let Some(run) = variable("RELEASE_CANDIDATE_RUN")?.filter(|run| !run.is_empty()) {
        let original = super::runs::verify(
            root,
            &run,
            &expected.source,
            super::runs::Purpose::Candidate,
        )?;
        expected.run = original.run;
        expected.attempt = original.attempt.to_string();
        BUILDER.clone_into(&mut expected.builder);
    }
    Ok(expected)
}

fn sbom(root: &Path, target: &str) -> Result<Vec<Entry>> {
    let version = proposal::versions(root)?.to_string();
    let locks = ["Cargo.lock", "app/Cargo.lock"];
    let before: Vec<Vec<u8>> = locks
        .iter()
        .map(|path| fs::read(&root.join(path)))
        .collect::<Result<_>>()?;
    for manifest in ["crates/youarel-cli/Cargo.toml", "app/Cargo.toml"] {
        let mut command = Tool::Cargo.command(root);
        if manifest == "app/Cargo.toml" {
            command.args(["cyclonedx", "--features", desktop::PRODUCTION_FEATURES]);
        } else {
            command.arg("cyclonedx");
        }
        tool::succeed(
            command.args([
                "--manifest-path",
                manifest,
                "--format",
                "json",
                "--describe",
                "binaries",
                "--target",
                target,
                "--all",
                "--spec-version",
                "1.6",
            ]),
            "inventory production dependencies",
        )?;
    }
    for (path, original) in locks.iter().zip(before) {
        ensure!(
            fs::read(&root.join(path))? == original,
            "SBOM generation changed {path}"
        );
    }
    let mut entries = Vec::new();
    for (name, path) in [("youarel", CLI_BOM), (desktop::LAUNCHER, APP_BOM)] {
        let bytes = fs::read(&root.join(path))?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        ensure!(
            value.get("bomFormat").and_then(serde_json::Value::as_str) == Some("CycloneDX"),
            "{path} is not a CycloneDX inventory"
        );
        ensure!(
            value
                .pointer("/metadata/component/version")
                .and_then(serde_json::Value::as_str)
                == Some(version.as_str()),
            "{path} describes another version"
        );
        ensure!(
            value
                .pointer("/metadata/component/name")
                .and_then(serde_json::Value::as_str)
                == Some(name),
            "{path} describes another executable"
        );
        ensure!(
            value
                .get("components")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|components| !components.is_empty()),
            "{path} contains no dependencies"
        );
        entries.push(Entry {
            path: format!("{name}-{target}.cdx.json"),
            mode: REGULAR,
            bytes,
        });
    }
    Ok(entries)
}

pub(super) fn seal(root: &Path, target: &str) -> Result<()> {
    let mut record = identity(root, target)?;
    desktop::check_programs(root, target, &record.version)?;
    let mut entries = vec![Entry {
        path: super::program_name(target),
        mode: EXECUTABLE,
        bytes: fs::read(&built_program(root, target))?,
    }];
    entries.extend(desktop::entries(root, target)?);
    entries.extend(sbom(root, target)?);
    let dist = root.join(DIST);
    fs::create_dir_all(&dist)?;
    for entry in &entries {
        if entry.path.ends_with(".cdx.json") {
            fs::write(&dist.join(&entry.path), &entry.bytes)?;
        }
    }
    let payload = package::tar_gz(&entries, source_date()?)?;
    record.payload_sha256 = Digest::of(&payload).hex();
    fs::write(
        &dist.join(format!("candidate-{target}.json")),
        &serde_json::to_vec_pretty(&record)?,
    )?;
    let directory = root.join(DIRECTORY);
    fs::create_dir_all(&directory)?;
    fs::write(&directory.join(PAYLOAD), &payload)?;
    fs::write(
        &directory.join(RECORD),
        &serde_json::to_vec_pretty(&record)?,
    )
}

fn validate(record: &Candidate, expected: &Candidate, payload: &[u8]) -> Result<()> {
    ensure!(
        record.source == expected.source && valid_source(&record.source),
        "candidate source mismatch"
    );
    ensure!(
        record.version == expected.version,
        "candidate version mismatch"
    );
    ensure!(record.run == expected.run, "candidate build run mismatch");
    let original: u32 = record
        .attempt
        .parse()
        .context("malformed original build attempt")?;
    let current: u32 = expected
        .attempt
        .parse()
        .context("malformed current build attempt")?;
    ensure!(
        original > 0 && original <= current && record.builder == expected.builder,
        "candidate builder or attempt mismatch"
    );
    ensure!(
        record.target == expected.target,
        "candidate target mismatch"
    );
    ensure!(
        record.payload_sha256 == Digest::of(payload).hex(),
        "candidate payload digest mismatch"
    );
    Ok(())
}

fn allowed_entry(path: &str, target: &str) -> bool {
    desktop::safe_path(path)
        && (path == super::program_name(target)
            || path == desktop::launcher_name(target)
            || path == format!("youarel-{target}.cdx.json")
            || path == format!("{}-{target}.cdx.json", desktop::LAUNCHER)
            || (target.contains("apple") && path.starts_with("youarel.app/Contents/")))
}

struct Contents {
    record: Candidate,
    entries: Vec<Entry>,
}

fn read_payload(root: &Path, directory: &Path, expected: &Candidate) -> Result<Contents> {
    let target = expected.target.as_str();
    let record: Candidate = serde_json::from_slice(&fs::read(&directory.join(RECORD))?)?;
    let payload = fs::read(&directory.join(PAYLOAD))?;
    validate(&record, expected, &payload)?;
    if record.builder == BUILDER {
        verify_build_provenance(
            root,
            directory,
            &directory.join("build-provenance.jsonl"),
            &expected.source,
        )?;
    } else {
        ensure!(record.builder == "local", "unknown candidate builder");
    }
    let entries = package::read_tar_gz(&payload)?;
    let mut names = BTreeSet::new();
    for entry in &entries {
        ensure!(
            allowed_entry(&entry.path, target),
            "unexpected candidate path `{}`",
            entry.path
        );
        ensure!(
            names.insert(entry.path.as_str()),
            "duplicate candidate path `{}`",
            entry.path
        );
        ensure!(
            [REGULAR, EXECUTABLE].contains(&entry.mode),
            "unexpected payload permissions"
        );
    }
    for required in [
        super::program_name(target),
        desktop::launcher_name(target),
        format!("youarel-{target}.cdx.json"),
        format!("{}-{target}.cdx.json", desktop::LAUNCHER),
    ] {
        ensure!(
            names.contains(required.as_str()),
            "candidate lacks {required}"
        );
    }
    Ok(Contents { record, entries })
}

pub(super) fn restore(root: &Path, target: &str) -> Result<()> {
    let expected = restore_identity(root, target)?;
    let directory = root.join(DIRECTORY);
    let Contents { record, entries } = read_payload(root, &directory, &expected)?;
    if record.builder == BUILDER {
        fs::create_dir_all(&root.join(DIST))?;
        fs::write(
            &root
                .join(DIST)
                .join(format!("build-provenance-{target}.jsonl")),
            &fs::read(&directory.join("build-provenance.jsonl"))?,
        )?;
    }
    for entry in entries {
        let base = if entry.path == super::program_name(target) {
            built_program(root, target)
                .parent()
                .context("program has no parent")?
                .to_path_buf()
        } else if entry.path == desktop::launcher_name(target) {
            desktop::launcher(root, target)
                .parent()
                .context("launcher has no parent")?
                .to_path_buf()
        } else if entry.path.starts_with("youarel.app/") {
            desktop::application(root, target)
                .parent()
                .context("application has no parent")?
                .to_path_buf()
        } else {
            root.join(DIST)
        };
        desktop::write_entries(&base, &[entry])?;
    }
    fs::write(
        &root.join(DIST).join(format!("candidate-{target}.json")),
        &serde_json::to_vec_pretty(&record)?,
    )?;
    if target.contains("apple") {
        desktop::app_entries(root, target)?;
    }
    Ok(())
}

fn verify_build_provenance(
    root: &Path,
    directory: &Path,
    bundle: &Path,
    source: &str,
) -> Result<()> {
    for artifact in [PAYLOAD, RECORD] {
        tool::succeed(
            Tool::Gh
                .command(root)
                .args(["attestation", "verify"])
                .arg(directory.join(artifact))
                .arg("--bundle")
                .arg(bundle)
                .args([
                    "--repo",
                    "P4suta/youarel",
                    "--signer-workflow",
                    "P4suta/youarel/.github/workflows/release.yml",
                    "--source-digest",
                    source,
                    "--source-ref",
                    "refs/heads/main",
                    "--deny-self-hosted-runners",
                ]),
            "verify original unsigned build provenance",
        )?;
    }
    Ok(())
}

pub(super) fn attach_provenance(root: &Path, target: &str, bundle: &Path) -> Result<()> {
    let expected = identity(root, target)?;
    verify_build_provenance(root, &root.join(DIRECTORY), bundle, &expected.source)?;
    let bytes = fs::read(bundle)?;
    fs::write(&root.join(DIRECTORY).join("build-provenance.jsonl"), &bytes)?;
    fs::write(
        &root
            .join(DIST)
            .join(format!("build-provenance-{target}.jsonl")),
        &bytes,
    )
}

pub(super) fn inspect(root: &Path) -> Result<()> {
    required("RELEASE_CANDIDATE_RUN")?;
    let first = desktop::TARGETS.first().context("no release targets")?;
    let original = restore_identity(root, first)?;
    let temporary = tempfile::tempdir()?;
    for target in desktop::TARGETS {
        let directory = temporary.path().join(target);
        tool::succeed(
            Tool::Gh
                .command(root)
                .args([
                    "run",
                    "download",
                    &original.run,
                    "--repo",
                    "P4suta/youarel",
                    "--name",
                    &format!("candidate-{target}"),
                    "--dir",
                ])
                .arg(&directory),
            "download the selected native candidate",
        )?;
        let expected = Candidate {
            target: target.to_owned(),
            ..original.clone()
        };
        let contents = read_payload(root, &directory, &expected)?;
        println!(
            "Verified {target} from run {}: {} payload entries",
            contents.record.run,
            contents.entries.len()
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    Candidate,
    Sign,
}

impl Operation {
    fn named(value: &str) -> Result<Self> {
        match value {
            "candidate" => Ok(Self::Candidate),
            "sign" => Ok(Self::Sign),
            _ => bail!("unknown release operation"),
        }
    }
}

fn require_signing_inputs(identity: &Candidate) -> Result<()> {
    super::ensure_enabled()?;
    ensure!(
        identity.version != "0.0.0",
        "reserve a product version before signing"
    );
    ensure!(
        !youarel_release::trust::RELEASE_KEYS.is_empty(),
        "configure trusted product release keys before signing"
    );
    ensure!(
        Digest::parse(&required("WINDOWS_SIGNING_IDENTITY_SHA256")?.to_ascii_lowercase()).is_some(),
        "configure the Windows publisher fingerprint before signing"
    );
    for name in [
        super::MACOS_APPLICATION_IDENTITY,
        super::MACOS_INSTALLER_IDENTITY,
    ] {
        ensure!(
            crate::apple::valid_selector(&required(name)?),
            "configure {name} with the intended certificate fingerprint before signing"
        );
    }
    Ok(())
}

fn require_checked_tree(tree: &str, acknowledged: Option<&str>) -> Result<()> {
    ensure!(
        valid_source(tree) && acknowledged == Some(tree),
        "confirm successful local release-check for the current source tree with RELEASE_CHECKED_TREE"
    );
    Ok(())
}

pub(super) fn plan(root: &Path, operation: &str) -> Result<()> {
    let selected = Operation::named(operation)?;
    let identity = identity(
        root,
        desktop::TARGETS.first().context("no release targets")?,
    )?;
    let candidate = match selected {
        Operation::Candidate => {
            ensure!(
                variable("RELEASE_CANDIDATE_RUN")?.is_none_or(|run| run.is_empty()),
                "candidate preparation does not consume another run"
            );
            require_checked_tree(
                &super::source_tree(root)?,
                variable("RELEASE_CHECKED_TREE")?.as_deref(),
            )?;
            identity.run.clone()
        }
        Operation::Sign => {
            require_signing_inputs(&identity)?;
            super::runs::verify(
                root,
                &required("RELEASE_CANDIDATE_RUN")?,
                &identity.source,
                super::runs::Purpose::Candidate,
            )?
            .run
        }
    };
    println!(
        "Source: {}\nVersion: {}\nController run: {}\nCandidate run: {candidate}\nOperation: {operation}",
        identity.source, identity.version, identity.run
    );
    println!(
        "Required checks: product versions, lint, tests, native behavior, SBOMs, package contents, original artifact provenance"
    );
    println!("Targets: {}", desktop::TARGETS.join(", "));
    println!(
        "External effects: {}",
        match selected {
            Operation::Candidate => "read dependencies; upload unsigned workflow artifacts",
            Operation::Sign =>
                "sign with Developer ID and SSL.com; submit notarization; upload signed workflow artifacts",
        }
    );
    if let Some(output) = variable("GITHUB_OUTPUT")? {
        fs::append(
            Path::new(&output),
            format!(
                "version={}\nsource={}\ncandidate_run={candidate}\n",
                identity.version, identity.source
            )
            .as_bytes(),
        )?;
    }
    Ok(())
}

pub(super) fn verify_run(root: &Path) -> Result<()> {
    let run = required("RELEASE_BUILD_RUN")?;
    let source = required("RELEASE_SOURCE_SHA")?;
    super::runs::verify(root, &run, &source, super::runs::Purpose::Distribution).map(drop)
}

#[cfg(test)]
mod tests {
    use super::{Candidate, allowed_entry, require_checked_tree, valid_source, validate};
    use youarel_release::manifest::Digest;

    fn fixture() -> Candidate {
        Candidate {
            source: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            version: "0.4.2".to_owned(),
            run: "1234".to_owned(),
            attempt: "1".to_owned(),
            builder: "P4suta/youarel/.github/workflows/release.yml@refs/heads/main".to_owned(),
            target: "x86_64-pc-windows-msvc".to_owned(),
            payload_sha256: Digest::of(b"payload").hex(),
        }
    }

    #[test]
    fn candidate_preparation_requires_confirmation_for_the_current_source_tree() {
        let tree = "a".repeat(40);
        require_checked_tree(&tree, Some(&tree)).unwrap();
        for confirmation in [
            None,
            Some(""),
            Some("main"),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        ] {
            require_checked_tree(&tree, confirmation).unwrap_err();
        }
    }

    #[test]
    fn a_candidate_is_bound_to_source_version_run_target_and_bytes() {
        let expected = fixture();
        validate(&fixture(), &expected, b"payload").unwrap();
        for field in ["source", "version", "run", "target", "digest"] {
            let mut altered = fixture();
            match field {
                "source" => altered.source = "a".repeat(40),
                "version" => altered.version = "0.4.3".to_owned(),
                "run" => altered.run = "1235".to_owned(),
                "target" => altered.target = "aarch64-pc-windows-msvc".to_owned(),
                _ => altered.payload_sha256 = Digest::of(b"changed").hex(),
            }
            assert!(
                validate(&altered, &expected, b"payload").is_err(),
                "{field}"
            );
        }
        assert!(validate(&fixture(), &expected, b"changed").is_err());
    }

    #[test]
    fn a_candidate_cannot_restore_checkout_files_or_another_platform() {
        for path in [
            "Cargo.toml",
            "../Cargo.toml",
            "youarel.app/../../Cargo.toml",
            "youarel.exe:stream",
            "youarel.app/Contents/MacOS/youarel",
        ] {
            assert!(!allowed_entry(path, "x86_64-pc-windows-msvc"), "{path}");
        }
        assert!(allowed_entry("youarel.exe", "x86_64-pc-windows-msvc"));
        assert!(valid_source(&fixture().source));
        assert!(!valid_source("main"));
    }

    #[test]
    fn a_retry_keeps_the_original_build_attempt_and_refuses_future_or_other_builders() {
        let mut retry = fixture();
        retry.attempt = "2".to_owned();
        validate(&fixture(), &retry, b"payload").unwrap();
        let mut future = fixture();
        future.attempt = "3".to_owned();
        assert!(validate(&future, &retry, b"payload").is_err());
        let mut invalid = fixture();
        invalid.attempt = "0".to_owned();
        assert!(validate(&invalid, &retry, b"payload").is_err());
        invalid.attempt = "1".to_owned();
        invalid.builder = "another-workflow".to_owned();
        assert!(validate(&invalid, &retry, b"payload").is_err());
    }
}
