use crate::fs;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

const CANDIDATE_ONLY: &str = "inputs.operation == 'candidate'";
const ORIGINAL_RUN: &str = "${{ needs.check.outputs.candidate_run }}";
const TOKEN: &str = "${{ github.token }}";

#[derive(Debug, Deserialize)]
struct Workflow {
    #[serde(default)]
    env: BTreeMap<String, String>,
    jobs: BTreeMap<String, Job>,
}

#[derive(Debug, Deserialize)]
struct Job {
    #[serde(default, rename = "if")]
    condition: String,
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default)]
    permissions: BTreeMap<String, String>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
struct Step {
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    run: String,
    #[serde(default, rename = "if")]
    condition: String,
    #[serde(default)]
    uses: String,
    #[serde(default, rename = "with")]
    inputs: BTreeMap<String, Value>,
}

impl Step {
    fn input(&self, name: &str) -> Option<&str> {
        self.inputs.get(name).and_then(Value::as_str)
    }
}

/// Checks hosted campaign placement and original candidate transfers.
///
/// # Errors
///
/// Rejects malformed workflows or a changed execution boundary.
pub fn check(root: &Path) -> Result<()> {
    for name in ["ci", "release"] {
        check_text(
            name,
            &fs::read_to_string(&root.join(format!(".github/workflows/{name}.yml")))?,
        )?;
    }
    Ok(())
}

fn check_text(name: &str, text: &str) -> Result<()> {
    let workflow: Workflow =
        serde_saphyr::from_str(text).context("parse hosted workflow policy")?;
    for job in workflow.jobs.values() {
        for step in &job.steps {
            for line in step.run.lines() {
                let words: Vec<&str> = line.split_whitespace().collect();
                if let [_, "release", "credential-inputs", purpose]
                | ["cargo", "xtask", "release", "credential-inputs", purpose] = words.as_slice()
                {
                    for secret in crate::release::credential_names(purpose)? {
                        ensure!(
                            step.env.get(*secret) == Some(&format!("${{{{ secrets.{secret} }}}}")),
                            "the {purpose} credential check lacks its {secret} secret binding"
                        );
                    }
                }
            }
            for command in [
                step.run.as_str(),
                step.input("install_args").unwrap_or_default(),
            ] {
                ensure!(
                    !campaign(command),
                    "hosted workflows must leave full campaigns to local release-check"
                );
            }
        }
    }
    if name == "release" {
        check_release(&workflow)?;
    }
    Ok(())
}

fn campaign(command: &str) -> bool {
    command
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && !['-', '_'].contains(&character)
        })
        .any(|word| {
            matches!(
                word,
                "kani"
                    | "miri"
                    | "mutants"
                    | "fuzz"
                    | "proofs"
                    | "proof-source"
                    | "proofs-native"
                    | "proofs-ci"
                    | "release-check"
                    | "push-check"
                    | "assurance"
                    | "kani-verifier"
                    | "cargo-mutants"
                    | "cargo-fuzz"
            )
        })
}

fn job<'a>(workflow: &'a Workflow, name: &str) -> Result<&'a Job> {
    workflow
        .jobs
        .get(name)
        .with_context(|| format!("the release workflow lacks {name}"))
}

fn check_release(workflow: &Workflow) -> Result<()> {
    for (name, input) in [
        ("RELEASE_CHECKED_TREE", "${{ inputs.checked_tree }}"),
        ("RELEASE_CANDIDATE_RUN", "${{ inputs.candidate_run }}"),
    ] {
        ensure!(
            workflow.env.get(name).map(String::as_str) == Some(input),
            "release input binding differs: {name}"
        );
    }
    let check = job(workflow, "check")?;
    ensure!(
        check
            .steps
            .iter()
            .any(|step| step.run == "cargo xtask ci check" && step.condition == CANDIDATE_ONLY),
        "portable candidate checks must run once before native builds"
    );
    let build = job(workflow, "build")?;
    ensure!(
        build.condition == CANDIDATE_ONLY && build.needs == ["check"],
        "signing must consume an existing candidate without rebuilding"
    );
    ensure!(
        build
            .steps
            .iter()
            .any(|step| step.run == "cargo xtask check --desktop"),
        "native candidates require desktop behavior checks"
    );
    let sign = job(workflow, "sign")?;
    ensure!(
        sign.needs == ["check"],
        "signing must depend on the verified original run"
    );
    for name in ["sign", "install", "assemble"] {
        let consumer = job(workflow, name)?;
        ensure!(
            consumer.permissions.get("actions").map(String::as_str) == Some("read"),
            "{name} needs read access to the original artifact run"
        );
        ensure!(
            !consumer
                .needs
                .iter()
                .any(|dependency| dependency == "build"),
            "candidate consumers must not depend on a skipped build"
        );
        let downloads: Vec<&Step> = consumer
            .steps
            .iter()
            .filter(|step| {
                step.uses.starts_with("actions/download-artifact@")
                    && step.input("run-id").is_some()
            })
            .collect();
        ensure!(
            downloads.len() == 1
                && downloads
                    .first()
                    .is_some_and(|step| step.input("run-id") == Some(ORIGINAL_RUN)
                        && step.input("github-token") == Some(TOKEN)),
            "{name} must download the original candidate run"
        );
    }
    ensure!(
        job(workflow, "assemble")?.steps.iter().any(|step| step
            .run
            .lines()
            .any(|line| line == "./target/debug/xtask release record-distribution")),
        "the signed inventory must bind candidate and distribution runs"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check_text;

    const CI: &str = include_str!("../../.github/workflows/ci.yml");
    const RELEASE: &str = include_str!("../../.github/workflows/release.yml");

    #[test]
    fn release_steps_bind_every_declared_credential_input() {
        assert_credential_bindings(RELEASE);
    }

    #[test]
    fn cargo_credential_checks_require_the_same_secret_bindings() {
        let fixture = RELEASE.replace(
            "./target/debug/xtask release credential-inputs",
            "cargo xtask release credential-inputs",
        );
        assert_ne!(fixture, RELEASE);
        assert_credential_bindings(&fixture);
    }

    fn assert_credential_bindings(complete: &str) {
        check_text("release", complete).unwrap();
        for name in [
            "APPLE_INSTALLER_IDENTITY",
            "APPLE_SIGNING_IDENTITY",
            "APPLE_CERTIFICATE_PASSWORD",
            "SSLDOTCOM_CREDENTIAL_ID",
        ] {
            let broken = complete.replace(
                &format!("{name}: ${{{{ secrets.{name} }}}}"),
                &format!("{name}_MISSING: ${{{{ secrets.{name} }}}}"),
            );
            check_text("release", &broken).unwrap_err();
        }
    }

    #[test]
    fn hosted_workflows_keep_campaigns_local_and_consume_one_candidate() {
        check_text("ci", CI).unwrap();
        check_text("release", RELEASE).unwrap();
        for command in [
            "cargo kani",
            "cargo +nightly miri test",
            "cargo mutants",
            "cargo xtask proofs",
            "just release-check",
            "cargo xtask ci assurance",
        ] {
            let broken = CI.replace("cargo xtask ci run check", command);
            check_text("ci", &broken).unwrap_err();
        }
        for (before, after) in [
            (
                "if: inputs.operation == 'candidate'",
                "if: inputs.operation == 'sign'",
            ),
            ("needs: [check]", "needs: [build]"),
            (
                "run-id: ${{ needs.check.outputs.candidate_run }}",
                "run-id: ${{ github.run_id }}",
            ),
            (
                "RELEASE_CHECKED_TREE: ${{ inputs.checked_tree }}",
                "RELEASE_CHECKED_TREE: stale",
            ),
            (
                "cargo xtask check --desktop",
                "cargo xtask check --platform",
            ),
            ("./target/debug/xtask release record-distribution", "true"),
            ("actions: read", "actions: none"),
        ] {
            let broken = RELEASE.replace(before, after);
            assert_ne!(broken, RELEASE);
            check_text("release", &broken).unwrap_err();
        }
    }
}
