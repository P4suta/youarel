//! The hosted entry points, which run the same checks as the local commands.

use crate::ci_plan::job_name;
use crate::{check, ci_plan, fs, proofs, tool};
use anyhow::{Context, Result, bail, ensure};
use std::fmt::Write as _;
use std::path::Path;
use youarel_core::verification_rules::{self, Job, Outcome};

/// The variable a workflow sets to the JSON of its `needs` context.
pub const NEEDS_VARIABLE: &str = "NEEDS_JSON";
const PLAN_VARIABLE: &str = "CI_PLAN";
const BASE_VARIABLE: &str = "CI_BASE";
const FULL_VARIABLE: &str = "CI_FULL";
const WORKFLOW_JOBS: [&str; 6] = ["plan", "check", "proofs", "app", "macos", "windows"];

const fn workflow_job(job: Job, campaign: bool) -> &'static str {
    match job {
        Job::App if !campaign => "check",
        Job::Kani | Job::Quint | Job::Proverif | Job::Miri | Job::Fuzz | Job::Mutants => "proofs",
        Job::Check | Job::App | Job::Macos | Job::Windows => job_name(job),
    }
}

fn proof_jobs(plan: &ci_plan::Plan) -> Vec<proofs::Step> {
    proofs::Step::ALL
        .into_iter()
        .filter(|step| {
            Job::ALL
                .into_iter()
                .any(|job| job_name(job) == step.name() && plan.jobs().includes(job))
        })
        .collect()
}

fn full(value: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "" | "false" => Ok(false),
        _ => bail!("CI_FULL must be true or false"),
    }
}

fn event() -> Result<(String, bool)> {
    let read = |name| match std::env::var(name) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("read {name}")),
    };
    Ok((read(BASE_VARIABLE)?, full(&read(FULL_VARIABLE)?)?))
}

fn selected(root: &Path) -> Result<ci_plan::Plan> {
    let (base, full) = event()?;
    let plan = std::env::var(PLAN_VARIABLE).context("the selected job requires CI_PLAN")?;
    ci_plan::validate(root, &base, full, &plan)
}

fn plan(root: &Path, base: Option<&str>) -> Result<()> {
    let (event_base, full) = event()?;
    let plan = ci_plan::build(root, base.unwrap_or(&event_base), full)?;
    println!("{}", serde_json::to_string_pretty(&plan)?);
    if let Some(output) = std::env::var_os("GITHUB_OUTPUT") {
        let mut lines = format!("plan={}\n", serde_json::to_string(&plan)?);
        writeln!(lines, "proofs={}", !proof_jobs(&plan).is_empty())?;
        for job in Job::ALL {
            writeln!(lines, "{}={}", job_name(job), plan.jobs().includes(job))?;
        }
        fs::append(Path::new(&output), lines.as_bytes())?;
    }
    Ok(())
}

fn outcome(value: &str) -> Outcome {
    match value {
        "success" => Outcome::Success,
        "failure" => Outcome::Failure,
        "cancelled" => Outcome::Cancelled,
        "skipped" => Outcome::Skipped,
        _ => Outcome::Incomplete,
    }
}

/// Requires a plan and an outcome for every job in the workflow inventory.
///
/// # Errors
///
/// Returns an error for missing jobs, unknown outcomes, or incomplete selected work.
pub fn require_selected(needs: &str, plan: &ci_plan::Plan) -> Result<()> {
    let needs: serde_json::Value = serde_json::from_str(needs)?;
    let jobs = needs
        .as_object()
        .context("the needs context must be an object")?;
    ensure!(
        jobs.len() == WORKFLOW_JOBS.len()
            && WORKFLOW_JOBS.iter().all(|name| jobs.contains_key(*name))
            && jobs
                .get("plan")
                .and_then(|job| job.get("result"))
                .and_then(serde_json::Value::as_str)
                == Some("success"),
        "the CI gate requires a successful plan and every known job"
    );
    for name in WORKFLOW_JOBS {
        let result = jobs[name]
            .get("result")
            .and_then(serde_json::Value::as_str)
            .with_context(|| format!("the job {name} reports no result"))?;
        ensure!(
            outcome(result) != Outcome::Incomplete,
            "the job {name} reports an unknown result"
        );
    }
    let supplied = jobs
        .get("plan")
        .and_then(|job| job.get("outputs"))
        .and_then(|outputs| outputs.get("plan"))
        .and_then(serde_json::Value::as_str)
        .context("the planning job produced no plan")?;
    ensure!(
        serde_json::from_str::<ci_plan::Plan>(supplied)? == *plan,
        "the gate and selected jobs require the same plan"
    );
    for job in Job::ALL {
        let result = jobs
            .get(workflow_job(job, plan.campaign))
            .and_then(|job| job.get("result"))
            .and_then(serde_json::Value::as_str)
            .context("a known job reports no result")?;
        ensure!(
            verification_rules::accepts(plan.jobs(), job, outcome(result)),
            "{}: {result}; selected work must complete successfully",
            job_name(job)
        );
    }
    Ok(())
}

fn run_selected(root: &Path, name: &str) -> Result<()> {
    if name == "proofs" {
        let plan = selected(root)?;
        let steps = proof_jobs(&plan);
        ensure!(!steps.is_empty(), "the plan selected no proof work");
        let mut report = crate::report::Report::default();
        for step in steps {
            report.stage(step.name(), || proofs::scoped(root, step, &plan.proofs));
        }
        return report.finish("selected proofs");
    }
    let job = Job::ALL
        .into_iter()
        .find(|job| job_name(*job) == name)
        .context("unknown CI job")?;
    let plan = selected(root)?;
    ensure!(plan.jobs().includes(job), "the plan did not select {name}");
    match job {
        Job::Check => {
            check::run(root, check::Scope::Hosted)?;
            if !plan.campaign && plan.jobs().includes(Job::App) {
                crate::app::linux(root)?;
            }
            Ok(())
        }
        Job::App => crate::app::linux(root),
        Job::Macos | Job::Windows => {
            let expected = if job == Job::Macos {
                "macos"
            } else {
                "windows"
            };
            ensure!(
                std::env::consts::OS == expected,
                "the {name} check requires a native {expected} runner"
            );
            check::run(root, check::Scope::Platform)
        }
        Job::Kani | Job::Quint | Job::Proverif | Job::Miri | Job::Fuzz | Job::Mutants => {
            let step = proofs::Step::named(name).context("the proof job names a verifier")?;
            proofs::scoped(root, step, &plan.proofs)
        }
    }
}

/// The jobs that did not succeed, from the JSON of a workflow's `needs` context.
///
/// # Errors
///
/// Returns an error when the JSON is not an object of jobs with a `result`.
pub fn unsuccessful(needs: &str) -> Result<Vec<String>> {
    let needs: serde_json::Value =
        serde_json::from_str(needs).context("parse the needs context")?;
    let jobs = needs
        .as_object()
        .context("the needs context is an object of jobs")?;
    ensure!(!jobs.is_empty(), "the gate received no jobs");
    let mut failed = Vec::new();
    for (job, outcome) in jobs {
        let result = outcome
            .get("result")
            .and_then(serde_json::Value::as_str)
            .with_context(|| format!("the job {job} reports no result"))?;
        if result != "success" {
            failed.push(format!("{job}: {result}"));
        }
    }
    Ok(failed)
}

fn gate(root: &Path, name: &str) -> Result<()> {
    let needs = std::env::var(NEEDS_VARIABLE)
        .with_context(|| format!("the {name} gate reads {NEEDS_VARIABLE}"))?;
    if name == "ci" {
        return require_selected(&needs, &selected(root)?);
    }
    let failed = unsuccessful(&needs)?;
    ensure!(
        failed.is_empty(),
        "the {name} gate requires every selected job to succeed:\n{}",
        failed.join("\n")
    );
    Ok(())
}

/// Runs one hosted entry point.
///
/// # Errors
///
/// Returns an error when the entry point is unknown or its checks fail.
pub fn run(root: &Path, words: &[&str]) -> Result<()> {
    match words {
        ["plan"] => plan(root, None),
        ["plan", base] => plan(root, Some(base)),
        ["cache", "app"] => crate::app::prepare_cache(root),
        ["run", job] => run_selected(root, job),
        ["assurance"] => {
            check::run(root, check::Scope::Hosted)?;
            crate::app::linux(root)?;
            proofs::run(
                root,
                proofs::Options {
                    ci: false,
                    only: None,
                    without: None,
                    inside: false,
                },
            )
        }
        ["check"] => check::run(root, check::Scope::Hosted),
        ["test"] => tool::succeed(
            tool::Tool::Cargo.command(root).args([
                "nextest",
                "run",
                "--locked",
                "--workspace",
                "--all-targets",
                "--no-tests=fail",
            ]),
            "cargo nextest",
        ),
        ["proofs"] => proofs::run(
            root,
            proofs::Options {
                ci: true,
                only: None,
                without: None,
                inside: false,
            },
        ),
        ["gate", name] => gate(root, name),
        _ => {
            bail!(
                "usage: cargo xtask ci plan [BASE]|cache app|run JOB|check|test|proofs|assurance|gate NAME"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_job_must_succeed() {
        let failed = unsuccessful(
            r#"{"check":{"result":"success"},"test":{"result":"failure"},"proofs":{"result":"skipped"}}"#,
        )
        .unwrap();
        assert_eq!(failed, ["proofs: skipped", "test: failure"]);
        unsuccessful(r#"{"check":{}}"#).unwrap_err();
        unsuccessful("[]").unwrap_err();
    }

    #[test]
    fn an_empty_gate_is_not_evidence() {
        unsuccessful("{}").unwrap_err();
    }

    fn gate_fixture() -> (ci_plan::Plan, serde_json::Value) {
        let plan = ci_plan::Plan {
            schema: 1,
            campaign: false,
            source: ci_plan::Source {
                requested_base: "base".to_owned(),
                base: Some("base".to_owned()),
                merge_base: Some("base".to_owned()),
                head: "head".to_owned(),
            },
            reason: ci_plan::Reason::Affected,
            changed: vec!["README.md".to_owned()],
            proofs: proofs::Scope::default(),
            app: false,
            platforms: false,
        };
        let mut needs = serde_json::Map::new();
        for name in WORKFLOW_JOBS {
            needs.insert(name.to_owned(), serde_json::json!({ "result": if matches!(name, "plan" | "check") { "success" } else { "skipped" } }));
        }
        needs.insert("plan".to_owned(), serde_json::json!({"result":"success", "outputs":{"plan":serde_json::to_string(&plan).unwrap()}}));
        (plan, serde_json::Value::Object(needs))
    }

    #[test]
    fn the_summary_requires_selected_jobs_and_refuses_missing_or_unplanned_results() {
        let (plan, needs) = gate_fixture();
        require_selected(&needs.to_string(), &plan).unwrap();
        for result in ["failure", "cancelled", "skipped", "", "inconclusive"] {
            let mut broken = needs.clone();
            broken["check"]["result"] = serde_json::json!(result);
            require_selected(&broken.to_string(), &plan).unwrap_err();
        }
        for name in WORKFLOW_JOBS {
            let mut broken = needs.clone();
            broken.as_object_mut().unwrap().remove(name);
            require_selected(&broken.to_string(), &plan).unwrap_err();
            for value in [serde_json::Value::Null, serde_json::json!("inconclusive")] {
                let mut malformed = needs.clone();
                malformed[name]["result"] = value;
                require_selected(&malformed.to_string(), &plan).unwrap_err();
            }
        }
        let mut broken = needs.clone();
        broken["unexpected"] = serde_json::json!({"result":"success"});
        require_selected(&broken.to_string(), &plan).unwrap_err();
        require_selected("{}", &plan).unwrap_err();
        require_selected("[]", &plan).unwrap_err();
        let mut missing_plan = needs;
        missing_plan["plan"]["outputs"]["plan"] = serde_json::json!("{}");
        require_selected(&missing_plan.to_string(), &plan).unwrap_err();
    }

    #[test]
    fn a_selected_proof_campaign_cannot_skip() {
        let (mut plan, mut needs) = gate_fixture();
        plan.campaign = true;
        plan.proofs.kani.push("rules.rs".to_owned());
        needs["plan"]["outputs"]["plan"] = serde_json::json!(serde_json::to_string(&plan).unwrap());
        require_selected(&needs.to_string(), &plan).unwrap_err();
        needs["proofs"]["result"] = serde_json::json!("success");
        require_selected(&needs.to_string(), &plan).unwrap();
        needs["proofs"]["result"] = serde_json::json!("cancelled");
        require_selected(&needs.to_string(), &plan).unwrap_err();
    }

    #[test]
    fn the_linux_fast_gate_covers_the_affected_window_without_a_heavy_job() {
        let (mut plan, mut needs) = gate_fixture();
        plan.app = true;
        plan.proofs.kani.push("rules.rs".to_owned());
        needs["plan"]["outputs"]["plan"] = serde_json::json!(serde_json::to_string(&plan).unwrap());
        require_selected(&needs.to_string(), &plan).unwrap();
        assert_eq!(proof_jobs(&plan), []);
        needs["check"]["result"] = serde_json::json!("skipped");
        require_selected(&needs.to_string(), &plan).unwrap_err();
    }
}
