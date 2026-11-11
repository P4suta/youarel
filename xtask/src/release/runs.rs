use super::{candidate, desktop};
use crate::tool::{self, Tool};
use anyhow::{Result, ensure};
use std::path::Path;

const REPOSITORY: &str = "P4suta/youarel";
pub(super) const MAX_ATTEMPTS: u32 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Purpose {
    Candidate,
    Distribution,
}

#[derive(Debug, serde::Deserialize)]
struct Job {
    #[serde(rename = "run_id")]
    run: u64,
    #[serde(rename = "run_attempt")]
    attempt: u32,
    name: String,
    status: String,
    conclusion: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct JobPage {
    jobs: Vec<Job>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    database_id: u64,
    attempt: u32,
    head_sha: String,
    event: String,
    status: String,
    conclusion: String,
    workflow_database_id: u64,
    #[serde(skip)]
    jobs: Vec<Job>,
}

#[derive(Debug)]
pub(super) struct Verified {
    pub(super) run: String,
    pub(super) attempt: u32,
}

#[derive(Clone, Copy, Debug)]
struct Request<'a> {
    run: &'a str,
    source: &'a str,
    workflow: u64,
}

fn validate_run(summary: &Summary, run: &str, source: &str, workflow: u64) -> Result<()> {
    ensure!(
        summary.database_id.to_string() == run,
        "release run identity mismatch"
    );
    ensure!(
        (1..=MAX_ATTEMPTS).contains(&summary.attempt),
        "release run has an unsupported attempt"
    );
    ensure!(
        candidate::valid_source(source) && summary.head_sha == source,
        "release run source mismatch"
    );
    ensure!(
        summary.event == "workflow_dispatch",
        "the release run was not dispatched"
    );
    ensure!(
        summary.status == "completed" && summary.conclusion == "success",
        "the release run did not succeed"
    );
    ensure!(
        summary.workflow_database_id == workflow,
        "the run came from another workflow"
    );
    Ok(())
}

fn validate(summary: &Summary, request: Request<'_>, purpose: Purpose) -> Result<()> {
    validate_run(summary, request.run, request.source, request.workflow)?;
    require_job(summary, "Verify the requested release")?;
    for target in desktop::TARGETS {
        match purpose {
            Purpose::Candidate => require_job(summary, &format!("Build {target}"))?,
            Purpose::Distribution => {
                require_job(summary, &format!("Install {target}"))?;
                if target.contains("apple") || target.contains("windows") {
                    require_job(summary, &format!("Sign {target}"))?;
                }
            }
        }
    }
    if purpose == Purpose::Distribution {
        require_job(summary, "Notarize signed installers")?;
        require_job(summary, "Assemble verified distributions")?;
    }
    Ok(())
}

fn require_job(summary: &Summary, name: &str) -> Result<()> {
    let latest = summary
        .jobs
        .iter()
        .filter(|job| job.name == name)
        .map(|job| job.attempt)
        .max();
    let matching: Vec<&Job> = summary
        .jobs
        .iter()
        .filter(|job| job.name == name && Some(job.attempt) == latest)
        .collect();
    ensure!(
        matching.len() == 1
            && matching
                .first()
                .is_some_and(|job| job.run == summary.database_id
                    && job.attempt > 0
                    && job.attempt <= summary.attempt
                    && job.status == "completed"
                    && job.conclusion.as_deref() == Some("success")),
        "the run lacks one successful latest job: {name}"
    );
    Ok(())
}

pub(super) fn verify(root: &Path, run: &str, source: &str, purpose: Purpose) -> Result<Verified> {
    ensure!(valid_id(run), "release run is not a canonical identifier");
    let reply = tool::capture(
        Tool::Gh.command(root).args([
            "run",
            "view",
            run,
            "--repo",
            REPOSITORY,
            "--json",
            "databaseId,attempt,headSha,event,conclusion,status,workflowDatabaseId",
        ]),
        "inspect the selected release run",
    )?;
    let mut summary: Summary = serde_json::from_str(&reply)?;
    let workflow = tool::capture(
        Tool::Gh.command(root).args([
            "api",
            "repos/P4suta/youarel/actions/workflows/release.yml",
            "--jq",
            ".id",
        ]),
        "inspect the protected release workflow",
    )?;
    let workflow = workflow.trim().parse()?;
    validate_run(&summary, run, source, workflow)?;
    let path = format!("repos/{REPOSITORY}/actions/runs/{run}/jobs?filter=all&per_page=100");
    let jobs = tool::capture(
        Tool::Gh
            .command(root)
            .args(["api", &path, "--paginate", "--slurp"]),
        "inspect every original release job attempt",
    )?;
    let pages: Vec<JobPage> = serde_json::from_str(&jobs)?;
    summary.jobs = pages.into_iter().flat_map(|page| page.jobs).collect();
    validate(
        &summary,
        Request {
            run,
            source,
            workflow,
        },
        purpose,
    )?;
    Ok(Verified {
        run: run.to_owned(),
        attempt: summary.attempt,
    })
}

pub(super) fn valid_id(run: &str) -> bool {
    run.parse::<u64>()
        .is_ok_and(|id| id > 0 && id.to_string() == run)
}

#[cfg(test)]
mod tests {
    use super::{Job, Purpose, Request, Summary, desktop, validate};

    const fn request(source: &str) -> Request<'_> {
        Request {
            run: "1234",
            source,
            workflow: 17,
        }
    }

    fn job(name: &str) -> Job {
        Job {
            run: 1234,
            attempt: 1,
            name: name.to_owned(),
            status: "completed".to_owned(),
            conclusion: Some("success".to_owned()),
        }
    }

    fn candidate() -> Summary {
        let mut jobs = vec![job("Verify the requested release")];
        jobs.extend(desktop::TARGETS.map(|target| job(&format!("Build {target}"))));
        Summary {
            database_id: 1234,
            attempt: 2,
            head_sha: "a".repeat(40),
            event: "workflow_dispatch".to_owned(),
            status: "completed".to_owned(),
            conclusion: "success".to_owned(),
            workflow_database_id: 17,
            jobs,
        }
    }

    #[test]
    fn reuse_requires_every_native_target_to_finish_successfully() {
        let mut summary = candidate();
        let source = summary.head_sha.clone();
        validate(&summary, request(&source), Purpose::Candidate).unwrap();
        summary.jobs.pop().unwrap();
        validate(&summary, request(&source), Purpose::Candidate).unwrap_err();
    }

    #[test]
    fn reuse_refuses_skipped_failed_duplicate_or_unrelated_builds() {
        let source = candidate().head_sha;
        for conclusion in ["failure", "cancelled", "skipped", "", "unknown"] {
            let mut summary = candidate();
            summary.jobs.last_mut().unwrap().conclusion = Some(conclusion.to_owned());
            validate(&summary, request(&source), Purpose::Candidate).unwrap_err();
        }
        let mut duplicate = candidate();
        duplicate.jobs.push(job("Verify the requested release"));
        validate(&duplicate, request(&source), Purpose::Candidate).unwrap_err();
        for (run, revision, workflow) in [
            ("1235", source.clone(), 17),
            ("1234", "b".repeat(40), 17),
            ("1234", source.clone(), 18),
        ] {
            validate(
                &candidate(),
                Request {
                    run,
                    source: &revision,
                    workflow,
                },
                Purpose::Candidate,
            )
            .unwrap_err();
        }
        let mut pending = candidate();
        pending.status = "in_progress".to_owned();
        validate(&pending, request(&source), Purpose::Candidate).unwrap_err();
    }

    #[test]
    fn publication_requires_signatures_installations_notarization_and_assembly() {
        let mut summary = candidate();
        let source = summary.head_sha.clone();
        validate(&summary, request(&source), Purpose::Distribution).unwrap_err();
        summary.jobs = vec![
            job("Verify the requested release"),
            job("Notarize signed installers"),
            job("Assemble verified distributions"),
        ];
        for target in desktop::TARGETS {
            summary.jobs.push(job(&format!("Install {target}")));
            if target.contains("apple") || target.contains("windows") {
                summary.jobs.push(job(&format!("Sign {target}")));
            }
        }
        validate(&summary, request(&source), Purpose::Distribution).unwrap();
        for missing in 0..summary.jobs.len() {
            let mut incomplete = candidate();
            incomplete.jobs = summary
                .jobs
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != missing)
                .map(|(_, found)| job(&found.name))
                .collect();
            validate(&incomplete, request(&source), Purpose::Distribution).unwrap_err();
        }
    }

    #[test]
    fn partial_retries_keep_successful_jobs_but_never_ignore_a_later_failure() {
        let mut summary = candidate();
        let source = summary.head_sha.clone();
        let mut retry = job(&format!("Build {}", desktop::TARGETS.first().unwrap()));
        retry.attempt = 2;
        summary.jobs.push(retry);
        validate(&summary, request(&source), Purpose::Candidate).unwrap();
        summary.jobs.last_mut().unwrap().conclusion = Some("failure".to_owned());
        validate(&summary, request(&source), Purpose::Candidate).unwrap_err();
        summary.jobs.last_mut().unwrap().conclusion = Some("success".to_owned());
        summary.jobs.last_mut().unwrap().attempt = 3;
        validate(&summary, request(&source), Purpose::Candidate).unwrap_err();
    }
}
