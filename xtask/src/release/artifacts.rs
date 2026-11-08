use super::{DIST, candidate, delivery, desktop, required, variable};
use crate::fs;
use crate::tool::{self, Tool};
use anyhow::{Context, Result, ensure};
use std::path::Path;

const MAX_ATTEMPTS: u32 = 100;
const WINDOWS_STEP: &str = "Sign and timestamp the Windows programs together";
const NOTARY_STEP: &str = "Submit each original signed package once";

#[derive(Debug, serde::Deserialize)]
struct Artifact {
    name: String,
    expired: bool,
}
#[derive(Debug, serde::Deserialize)]
struct ArtifactPage {
    artifacts: Vec<Artifact>,
}
#[derive(Debug, serde::Deserialize)]
struct Step {
    name: String,
    conclusion: Option<String>,
}
#[derive(Debug, serde::Deserialize)]
struct Job {
    name: String,
    conclusion: Option<String>,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Clone, Copy, Debug)]
struct Operation<'a> {
    job: &'a str,
    step: &'a str,
}
#[derive(Debug, serde::Deserialize)]
struct JobPage {
    jobs: Vec<Job>,
}

fn run(root: &Path) -> Result<(String, u32)> {
    let id = required("GITHUB_RUN_ID")?;
    ensure!(
        !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()),
        "invalid workflow run ID"
    );
    let attempt: u32 = required("GITHUB_RUN_ATTEMPT")?.parse()?;
    ensure!(
        (1..=MAX_ATTEMPTS).contains(&attempt),
        "unsupported workflow attempt"
    );
    ensure!(
        variable("RELEASE_BUILD_RUN")?.as_deref() == Some(id.as_str()),
        "artifact run differs from the selected build"
    );
    candidate::valid_workflow_source(root)?;
    Ok((id, attempt))
}

fn listing(root: &Path, run: &str) -> Result<Vec<Artifact>> {
    let path = format!("repos/P4suta/youarel/actions/runs/{run}/artifacts");
    let output = tool::capture(
        Tool::Gh
            .command(root)
            .args(["api", &path, "--paginate", "--slurp"]),
        "inspect preserved workflow artifacts",
    )?;
    let pages: Vec<ArtifactPage> = serde_json::from_str(&output)?;
    Ok(pages.into_iter().flat_map(|page| page.artifacts).collect())
}

fn previous<'a>(artifacts: &'a [Artifact], prefix: &str, attempt: u32) -> Result<Option<&'a str>> {
    let mut found = std::collections::BTreeMap::new();
    for artifact in artifacts {
        let Some(suffix) = artifact.name.strip_prefix(prefix) else {
            continue;
        };
        let original: u32 = suffix
            .parse()
            .context("malformed preserved artifact attempt")?;
        ensure!(original > 0, "invalid preserved artifact attempt");
        if original >= attempt {
            continue;
        }
        ensure!(
            !artifact.expired,
            "preserved signing state expired; reconcile the provider before starting another operation"
        );
        ensure!(
            found.insert(original, artifact.name.as_str()).is_none(),
            "ambiguous preserved artifact identity"
        );
    }
    Ok(found.last_key_value().map(|(_, name)| *name))
}

fn download(root: &Path, run: &str, name: &str, directory: &Path) -> Result<()> {
    tool::succeed(
        Tool::Gh
            .command(root)
            .args([
                "run",
                "download",
                run,
                "--repo",
                "P4suta/youarel",
                "--name",
                name,
                "--dir",
            ])
            .arg(directory),
        "restore the original workflow artifact",
    )
}

fn started(root: &Path, run: &str, attempt: u32, operation: Operation<'_>) -> Result<bool> {
    for original in 1..attempt {
        let path = format!(
            "repos/P4suta/youarel/actions/runs/{run}/attempts/{original}/jobs?per_page=100"
        );
        let output = tool::capture(
            Tool::Gh
                .command(root)
                .args(["api", &path, "--paginate", "--slurp"]),
            "reconcile earlier provider operation attempts",
        )?;
        let pages: Vec<JobPage> = serde_json::from_str(&output)?;
        if pages
            .into_iter()
            .flat_map(|page| page.jobs)
            .filter(|job| job.name == operation.job)
            .any(|job| operation_started(&job, operation.step))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn operation_started(job: &Job, step: &str) -> bool {
    if job.conclusion.as_deref() == Some("skipped") {
        return false;
    }
    job.steps
        .iter()
        .find(|candidate| candidate.name == step)
        .is_none_or(|found| found.conclusion.as_deref() != Some("skipped"))
}

pub(super) fn build(root: &Path, target: &str) -> Result<()> {
    desktop::supported(target)?;
    let (run, _) = run(root)?;
    let artifacts = listing(root, &run)?;
    let name = format!("candidate-{target}");
    let matching: Vec<&Artifact> = artifacts
        .iter()
        .filter(|artifact| artifact.name == name)
        .collect();
    ensure!(matching.len() <= 1, "ambiguous unsigned candidate artifact");
    let Some(candidate_artifact) = matching.first() else {
        return output(false);
    };
    ensure!(
        !candidate_artifact.expired,
        "the original unsigned candidate expired"
    );
    download(root, &run, &name, &root.join(candidate::DIRECTORY))?;
    candidate::restore(root, target)?;
    desktop::check_programs(root, target, &super::workspace_version(root)?.to_string())?;
    let distribution = format!("unsigned-dist-{target}");
    let existing: Vec<&Artifact> = artifacts
        .iter()
        .filter(|artifact| artifact.name == distribution)
        .collect();
    ensure!(
        existing.len() <= 1,
        "ambiguous unsigned distribution artifact"
    );
    if let Some(artifact) = existing.first() {
        ensure!(
            !artifact.expired,
            "the original unsigned distribution expired"
        );
        download(root, &run, &distribution, &root.join(DIST))?;
        super::check_bundle(
            root,
            target,
            super::workspace_version(root)?,
            &fs::read(&super::built_program(root, target))?,
        )?;
    }
    if let Some(path) = variable("GITHUB_OUTPUT")? {
        fs::append(
            Path::new(&path),
            format!("distribution={}\n", !existing.is_empty()).as_bytes(),
        )?;
    }
    output(true)
}

fn output(reused: bool) -> Result<()> {
    if let Some(path) = variable("GITHUB_OUTPUT")? {
        fs::append(Path::new(&path), format!("reused={reused}\n").as_bytes())?;
    }
    Ok(())
}

pub(super) fn windows(root: &Path, target: &str) -> Result<()> {
    desktop::supported(target)?;
    if !target.contains("windows") {
        return output(false);
    }
    let (run, attempt) = run(root)?;
    if !started(
        root,
        &run,
        attempt,
        Operation {
            job: &format!("Sign {target}"),
            step: WINDOWS_STEP,
        },
    )? {
        return output(false);
    }
    let artifacts = listing(root, &run)?;
    let prefix = format!("windows-signatures-{target}-{run}-");
    let name = previous(&artifacts, &prefix, attempt)?.context("the previous signing outcome is unknown and no preserved output exists; reconcile SSL.com before any new signature request")?;
    let restored = tempfile::tempdir()?;
    download(root, &run, name, restored.path())?;
    ensure!(
        fs::read(&restored.path().join("release-candidate/candidate.json"))?
            == fs::read(&root.join("release-candidate/candidate.json"))?,
        "preserved Windows signatures belong to another candidate"
    );
    let expected = required("WINDOWS_SIGNING_IDENTITY_SHA256")?.to_ascii_lowercase();
    ensure!(
        youarel_release::manifest::Digest::parse(&expected).is_some(),
        "invalid Windows publisher fingerprint"
    );
    let mut verified = Vec::new();
    for program in [super::program_name(target), desktop::launcher_name(target)] {
        let path = restored.path().join("signing-input").join(&program);
        super::verify_windows_program(root, &path, &expected)?;
        verified.push((program, fs::read(&path)?));
    }
    for (program, bytes) in verified {
        fs::write(&root.join("signing-input").join(program), &bytes)?;
    }
    output(true)
}

pub(super) fn notarization(root: &Path) -> Result<()> {
    let (run, attempt) = run(root)?;
    if !started(
        root,
        &run,
        attempt,
        Operation {
            job: "Notarize signed installers",
            step: NOTARY_STEP,
        },
    )? {
        return Ok(());
    }
    let artifacts = listing(root, &run)?;
    let name = previous(&artifacts, &format!("notarization-{run}-"), attempt)?.context("the notarization outcome is unknown and no receipt artifact exists; reconcile Apple history before any submission")?;
    download(root, &run, name, &root.join(DIST))?;
    super::check_notary_receipts(root)
}

pub(super) fn distribution(root: &Path) -> Result<()> {
    let (run, _) = run(root)?;
    let name = format!("release-{run}");
    let artifacts = listing(root, &run)?;
    let matches: Vec<&Artifact> = artifacts
        .iter()
        .filter(|artifact| artifact.name == name)
        .collect();
    ensure!(matches.len() <= 1, "ambiguous final distribution artifact");
    let Some(artifact) = matches.first() else {
        return output(false);
    };
    ensure!(
        !artifact.expired,
        "the original final distribution expired; restore its preserved bytes before continuing"
    );
    download(root, &run, &name, &root.join(DIST))?;
    delivery::verify_provenance(root)?;
    output(true)
}

#[cfg(test)]
mod tests {
    use super::{Artifact, Job, Step, operation_started, previous};

    #[test]
    fn missing_or_uncertain_provider_outcomes_never_authorize_another_request() {
        let mut job = Job {
            name: "sign".to_owned(),
            conclusion: Some("failure".to_owned()),
            steps: Vec::new(),
        };
        assert!(operation_started(&job, "provider"));
        job.steps.push(Step {
            name: "provider".to_owned(),
            conclusion: None,
        });
        assert!(operation_started(&job, "provider"));
        job.steps.first_mut().unwrap().conclusion = Some("success".to_owned());
        assert!(operation_started(&job, "provider"));
        job.steps.first_mut().unwrap().conclusion = Some("skipped".to_owned());
        assert!(!operation_started(&job, "provider"));
        job.conclusion = Some("skipped".to_owned());
        job.steps.clear();
        assert!(!operation_started(&job, "provider"));
    }

    #[test]
    fn recovery_selects_the_latest_original_attempt_without_using_future_or_ambiguous_state() {
        let artifacts = vec![
            Artifact {
                name: "state-1".to_owned(),
                expired: false,
            },
            Artifact {
                name: "state-2".to_owned(),
                expired: false,
            },
            Artifact {
                name: "state-3".to_owned(),
                expired: false,
            },
        ];
        assert_eq!(previous(&artifacts, "state-", 3).unwrap(), Some("state-2"));
        assert_eq!(previous(&artifacts, "unrelated-", 3).unwrap(), None);
        let duplicate = vec![
            Artifact {
                name: "state-1".to_owned(),
                expired: false,
            },
            Artifact {
                name: "state-1".to_owned(),
                expired: false,
            },
        ];
        previous(&duplicate, "state-", 2).unwrap_err();
        let expired = vec![Artifact {
            name: "state-1".to_owned(),
            expired: true,
        }];
        previous(&expired, "state-", 2).unwrap_err();
        let malformed = vec![Artifact {
            name: "state-unknown".to_owned(),
            expired: false,
        }];
        previous(&malformed, "state-", 2).unwrap_err();
    }
}
