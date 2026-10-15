//! The hosted entry points, which run the same checks as the local commands.

use crate::{check, proofs, tool};
use anyhow::{Context, Result, bail, ensure};
use std::path::Path;

/// The variable a workflow sets to the JSON of its `needs` context.
pub const NEEDS_VARIABLE: &str = "NEEDS_JSON";

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

fn gate(name: &str) -> Result<()> {
    let needs = std::env::var(NEEDS_VARIABLE)
        .with_context(|| format!("the {name} gate reads {NEEDS_VARIABLE}"))?;
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
        ["check"] => check::run(root, check::Scope::Everything),
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
            },
        ),
        ["gate", name] => gate(name),
        _ => bail!("usage: cargo xtask ci check|test|proofs|gate NAME"),
    }
}

#[cfg(test)]
mod tests {
    use super::unsuccessful;

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
}
