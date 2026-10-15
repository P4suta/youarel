//! The authoritative check: formatting, lints, the structural gates, the records, the tests, and the dependency checks.

use crate::report::Report;
use crate::tool::{Tool, capture, succeed};
use crate::{adr, fs, proofs, structure};
use anyhow::{Context, Result};
use std::io::ErrorKind;
use std::path::Path;

/// Which files a check covers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    /// The whole checkout, as the push gate and the hosted runners see it.
    Everything,
    /// The staged files alone, as the pre-commit hook sees them.
    Staged,
    /// The checks whose outcome depends on the operating system, for a host that receives the files without their Git history.
    Platform,
}

const WORKFLOWS: &str = ".github/workflows";
const RUST_EDITION: &str = "2024";
const PROOF_FIXTURES: &str = "xtask/proofs";
const CORE_PACKAGE: &str = "youarel-core";
/// A bare-metal target with no operating system, where only a crate without `std` and I/O builds.
pub const BARE_TARGET: &str = "thumbv7em-none-eabi";

fn staged_paths(root: &Path) -> Result<Vec<String>> {
    let listing = capture(
        Tool::Git
            .command(root)
            .args(["diff", "--cached", "--name-only", "--diff-filter=ACMR"]),
        "git diff --cached --name-only",
    )?;
    Ok(listing.lines().map(str::to_owned).collect())
}

fn with_suffix<'a>(paths: &'a [String], suffix: &str) -> Vec<&'a str> {
    paths
        .iter()
        .map(String::as_str)
        .filter(|path| path.ends_with(suffix))
        .collect()
}

fn rustfmt(root: &Path, files: Option<&[&str]>) -> Result<()> {
    match files {
        Some([]) => Ok(()),
        Some(files) => succeed(
            Tool::Rustfmt
                .command(root)
                .args(["--edition", RUST_EDITION, "--check"])
                .args(files),
            "rustfmt --check",
        ),
        None => succeed(
            Tool::Cargo
                .command(root)
                .args(["fmt", "--all", "--", "--check"]),
            "cargo fmt --check",
        ),
    }
}

fn taplo(root: &Path, files: Option<&[&str]>) -> Result<()> {
    match files {
        Some([]) => Ok(()),
        Some(files) => succeed(
            Tool::Taplo
                .command(root)
                .args(["fmt", "--check"])
                .args(files),
            "taplo fmt --check",
        ),
        None => {
            succeed(
                Tool::Taplo.command(root).args(["fmt", "--check"]),
                "taplo fmt --check",
            )?;
            succeed(Tool::Taplo.command(root).arg("lint"), "taplo lint")
        }
    }
}

fn typos(root: &Path, files: Option<&[&str]>) -> Result<()> {
    match files {
        Some([]) => Ok(()),
        Some(files) => succeed(
            Tool::Typos.command(root).arg("--force-exclude").args(files),
            "typos",
        ),
        None => succeed(&mut Tool::Typos.command(root), "typos"),
    }
}

fn clippy(root: &Path) -> Result<()> {
    succeed(
        Tool::Cargo.command(root).args([
            "clippy",
            "--locked",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ]),
        "cargo clippy",
    )
}

/// Builds the core for a target without an operating system, which fails the moment the core reaches `std`.
fn bare(root: &Path) -> Result<()> {
    succeed(
        Tool::Cargo.command(root).args([
            "build",
            "--locked",
            "--package",
            CORE_PACKAGE,
            "--target",
            BARE_TARGET,
        ]),
        "cargo build for the bare target",
    )
}

fn tests(root: &Path) -> Result<()> {
    succeed(
        Tool::Cargo.command(root).args([
            "nextest",
            "run",
            "--locked",
            "--workspace",
            "--all-targets",
            "--no-tests=fail",
        ]),
        "cargo nextest",
    )
}

/// Every manifest with its own lockfile: the workspace, the fuzz workspace, and each proof fixture.
fn manifests(root: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut found = vec![root.join("Cargo.toml")];
    let fuzz = root.join(proofs::FUZZ).join("Cargo.toml");
    if fs::is_file(&fuzz)? {
        found.push(fuzz);
    }
    for fixture in fs::entries(&root.join(PROOF_FIXTURES))? {
        let manifest = fixture.join("Cargo.toml");
        if fs::is_file(&manifest)? {
            found.push(manifest);
        }
    }
    Ok(found)
}

/// Licenses, bans, and sources of every lockfile; advisories are `cargo audit`'s, which fetches its own database.
fn deny(root: &Path) -> Result<()> {
    for manifest in manifests(root)? {
        succeed(
            Tool::Cargo
                .command(root)
                .args(["deny", "--locked", "--manifest-path"])
                .arg(&manifest)
                .args([
                    "check",
                    "--hide-inclusion-graph",
                    "licenses",
                    "bans",
                    "sources",
                ]),
            "cargo deny",
        )?;
    }
    Ok(())
}

fn audit(root: &Path) -> Result<()> {
    for manifest in manifests(root)? {
        let lock = manifest.with_file_name("Cargo.lock");
        succeed(
            Tool::Cargo
                .command(root)
                .args(["audit", "--deny", "warnings", "--file"])
                .arg(&lock),
            "cargo audit",
        )?;
    }
    Ok(())
}

fn workflows(root: &Path) -> Result<()> {
    if !fs::is_directory(&root.join(WORKFLOWS))? {
        return Ok(());
    }
    succeed(&mut Tool::Actionlint.command(root), "actionlint")?;
    succeed(
        Tool::Zizmor
            .command(root)
            .args(["--no-online-audits", WORKFLOWS]),
        "zizmor",
    )
}

/// Runs `dotfiles repository <word>`, or notes its absence on a host without the owner's tools.
fn repository(root: &Path, word: &str) -> Result<()> {
    let mut command = Tool::Dotfiles.command(root);
    command.args(["repository", word]);
    match command.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(anyhow::anyhow!(
            "dotfiles repository {word} failed with {status}"
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!(
                "repository: the `dotfiles` tool is absent on this host, so the generated files are checked only where it is installed"
            );
            Ok(())
        }
        Err(error) => Err(error).context("start dotfiles repository"),
    }
}

fn everything(root: &Path) -> Result<()> {
    let mut report = Report::default();
    report.stage("rustfmt", || rustfmt(root, None));
    report.stage("taplo", || taplo(root, None));
    report.stage("typos", || typos(root, None));
    report.stage("clippy", || clippy(root));
    report.stage("structure", || structure::check(root));
    report.stage("bare", || bare(root));
    report.stage("adr", || adr::check(root));
    report.stage("repository", || repository(root, "check"));
    report.stage("workflows", || workflows(root));
    report.stage("tests", || tests(root));
    report.stage("deny", || deny(root));
    report.stage("audit", || audit(root));
    report.finish("check")
}

fn staged(root: &Path) -> Result<()> {
    let paths = staged_paths(root)?;
    let mut report = Report::default();
    report.stage("rustfmt", || {
        rustfmt(root, Some(&with_suffix(&paths, ".rs")))
    });
    report.stage("taplo", || taplo(root, Some(&with_suffix(&paths, ".toml"))));
    report.stage("typos", || {
        let all: Vec<&str> = paths.iter().map(String::as_str).collect();
        typos(root, Some(&all))
    });
    report.stage("structure", || structure::check(root));
    if paths.iter().any(|path| path.starts_with(adr::DIRECTORY)) {
        report.stage("adr", || adr::check(root));
    }
    if paths.iter().any(|path| path.starts_with(WORKFLOWS)) {
        report.stage("workflows", || workflows(root));
    }
    report.finish("check --staged")
}

/// The compiler, the lints under the host's configuration, the structure, the bare build, and the tests.
///
/// Formatting, spelling, the records, the generated files, and the workflows read the same bytes on every host and need the Git history, so the full check covers them.
fn platform(root: &Path) -> Result<()> {
    let mut report = Report::default();
    report.stage("clippy", || clippy(root));
    report.stage("structure", || structure::check(root));
    report.stage("bare", || bare(root));
    report.stage("tests", || tests(root));
    report.finish("check --platform")
}

/// Runs the checks of one scope and reports every failure together.
///
/// # Errors
///
/// Returns an error naming each failed check.
pub fn run(root: &Path, scope: Scope) -> Result<()> {
    match scope {
        Scope::Everything => everything(root),
        Scope::Staged => staged(root),
        Scope::Platform => platform(root),
    }
}

/// Every check and then every proof, which a push requires; the proofs run in the memory-limited container.
///
/// # Errors
///
/// Returns an error when a check or a proof fails.
pub fn push_check(root: &Path) -> Result<()> {
    run(root, Scope::Everything)?;
    crate::container::run(root, &["proofs"])
}

/// Writes the generated files from their sources.
///
/// # Errors
///
/// Returns an error when a generator fails.
pub fn generate(root: &Path) -> Result<()> {
    crate::quint::generate(root)?;
    repository(root, "generate")
}

/// Refuses drift between the generated files and their sources, and a malformed record.
///
/// # Errors
///
/// Returns an error naming the drift or the record.
pub fn generated(root: &Path) -> Result<()> {
    let mut report = Report::default();
    report.stage("adr", || adr::check(root));
    report.stage("repository", || repository(root, "check"));
    report.finish("gen --check")
}

#[cfg(test)]
mod tests {
    use super::with_suffix;

    #[test]
    fn staged_files_are_selected_by_suffix() {
        let paths = ["a.rs".to_owned(), "b.toml".to_owned(), "c.md".to_owned()];
        assert_eq!(with_suffix(&paths, ".rs"), ["a.rs"]);
        assert_eq!(with_suffix(&paths, ".toml"), ["b.toml"]);
        assert_eq!(with_suffix(&paths, ".yml"), Vec::<&str>::new());
    }
}
