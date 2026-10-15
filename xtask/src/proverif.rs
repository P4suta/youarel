//! The ProVerif gate over the symbolic models of `specs/proverif`.
//!
//! Each model must prove every query it states, as many as the manifest records, and its probe, which removes one safeguard, must have a query refuted.
//! ProVerif runs in a pinned image with no network, limited memory, and the checkout mounted read-only.

use crate::obligations::{self, Obligation, ProverifModel};
use crate::quint;
use crate::tool::{self, Tool};
use anyhow::{Context, Result, ensure};
use std::ffi::OsString;
use std::path::Path;

/// The ProVerif release the image builds.
pub const PROVERIF_VERSION: &str = "2.05";

/// The directory that defines the image, and the files whose bytes name it.
const IMAGE_DIRECTORY: &str = "xtask/proofs/proverif";
const IMAGE_INPUTS: [&str; 1] = ["Dockerfile"];
const REPOSITORY: &str = "youarel-proverif";

/// Where the container mounts the read-only checkout.
const SOURCE: &str = "/source";
const WORKDIR: &str = "/tmp";

/// The memory a run may use, with no swap beyond it, and the processors.
const MEMORY: &str = "4g";
const PROCESSORS: &str = "1";

/// What ProVerif concluded about one query.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    Proved,
    Refuted,
    /// ProVerif could neither prove nor refute the query.
    Open,
}

/// Every query result of a ProVerif report, in order.
#[must_use]
pub fn results(report: &str) -> Vec<(&str, Verdict)> {
    report
        .lines()
        .filter_map(|line| {
            let claim = line.strip_prefix("RESULT ")?;
            let verdict = if claim.ends_with(" is true.") {
                Verdict::Proved
            } else if claim.ends_with(" is false.") {
                Verdict::Refuted
            } else {
                Verdict::Open
            };
            Some((claim, verdict))
        })
        .collect()
}

/// The arguments that run ProVerif on `model` with `library` in the image, with no network, limited memory, and the checkout read-only.
fn run_arguments(root: &Path, image: &str, library: &str, model: &str) -> Vec<OsString> {
    let mut docker: Vec<OsString> = [
        "run",
        "--rm",
        "--network",
        "none",
        "--memory",
        MEMORY,
        "--memory-swap",
        MEMORY,
        "--cpus",
        PROCESSORS,
        "--mount",
    ]
    .map(OsString::from)
    .into();
    docker.push(quint::mount(root, SOURCE, true));
    docker.extend(["--workdir", WORKDIR, image, "proverif", "-lib"].map(OsString::from));
    docker.push(format!("{SOURCE}/{library}").into());
    docker.push(format!("{SOURCE}/{model}").into());
    docker
}

/// ProVerif's report on `model`, which must run to its end.
fn report(root: &Path, image: &str, library: &str, model: &str) -> Result<String> {
    let mut docker = Tool::Docker.command(root);
    docker.args(run_arguments(root, image, library, model));
    let ran = tool::output(&mut docker, "docker run of ProVerif")?;
    let text = String::from_utf8_lossy(&ran.stdout).into_owned();
    ensure!(
        ran.status.success(),
        "ProVerif failed on {model}: {}\n{text}{}",
        ran.status,
        String::from_utf8_lossy(&ran.stderr)
    );
    Ok(text)
}

/// Requires every query of the model proved, as many as the manifest records, and a query of the probe refuted.
fn check(root: &Path, image: &str, model: &ProverifModel) -> Result<()> {
    let proved = report(root, image, &model.library, &model.model)?;
    let found = results(&proved);
    ensure!(
        found.len() == model.queries
            && found.iter().all(|(_, verdict)| *verdict == Verdict::Proved),
        "ProVerif proved {} of the {} queries of {}, while {} records {}; review the model, then record the new count:\n{proved}",
        found
            .iter()
            .filter(|(_, verdict)| *verdict == Verdict::Proved)
            .count(),
        found.len(),
        model.model,
        obligations::MANIFEST,
        model.queries
    );
    let refuted = report(root, image, &model.library, &model.probe)?;
    ensure!(
        results(&refuted)
            .iter()
            .any(|(_, verdict)| *verdict == Verdict::Refuted),
        "ProVerif refuted no query of the probe {}, so nothing shows the model can fail:\n{refuted}",
        model.probe
    );
    Ok(())
}

/// Discharges every ProVerif obligation of the manifest in the pinned image, offline.
///
/// # Errors
///
/// Returns an error when Docker cannot build or run the image, the manifest declares no model, or a model or its probe fails.
pub fn verify(root: &Path) -> Result<()> {
    let root = std::fs::canonicalize(root).context("resolve the checkout")?;
    let manifest = obligations::load(&root)?;
    let models: Vec<&ProverifModel> = manifest
        .obligation
        .iter()
        .filter_map(|obligation| match obligation {
            Obligation::Proverif(model) => Some(model),
            Obligation::Kani { .. } | Obligation::Quint(_) => None,
        })
        .collect();
    ensure!(
        !models.is_empty(),
        "the manifest declares no ProVerif obligation"
    );
    let image = quint::pinned_image(&root, IMAGE_DIRECTORY, &IMAGE_INPUTS, REPOSITORY)?;
    quint::build_pinned(&root, IMAGE_DIRECTORY, &image)?;
    for model in &models {
        check(&root, &image, model)?;
    }
    eprintln!(
        "proverif: {} models prove every query, and each probe has a query refuted",
        models.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Verdict, results, run_arguments};
    use std::path::Path;

    #[test]
    fn the_report_yields_each_verdict_and_the_run_mounts_the_checkout_read_only() {
        let report = "Process 0\nRESULT not attacker(epochkey[]) is true.\nRESULT Weak secret code is false.\nRESULT event(A) ==> event(B) cannot be proved.\n";
        assert_eq!(
            results(report),
            [
                ("not attacker(epochkey[]) is true.", Verdict::Proved),
                ("Weak secret code is false.", Verdict::Refuted),
                ("event(A) ==> event(B) cannot be proved.", Verdict::Open),
            ]
        );
        assert_eq!(results("no results"), []);
        let words: Vec<String> = run_arguments(Path::new("/r"), "image", "lib.pvl", "m.pv")
            .iter()
            .map(|word| word.to_string_lossy().into_owned())
            .collect();
        assert!(words.windows(2).any(|pair| pair == ["--network", "none"]));
        assert!(
            words
                .iter()
                .any(|word| word.ends_with("target=/source,readonly"))
        );
        assert_eq!(
            words.get(words.len().saturating_sub(2)..),
            Some(["/source/lib.pvl".to_owned(), "/source/m.pv".to_owned()].as_slice())
        );
    }
}
