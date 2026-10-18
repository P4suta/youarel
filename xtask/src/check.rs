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
    /// The portable fast gate, with native window checks in their selected job.
    Hosted,
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
/// The relay's Worker, its own workspace, and the WebAssembly target it builds for.
const WORKER: &str = "server";
const WORKER_TARGET: &str = "wasm32-unknown-unknown";
/// The window launcher's workspace, and its script's TypeScript configuration.
pub const APP: &str = "app";
const APP_SCRIPT: &str = "app/tsconfig.json";
/// The window launcher's manifest and its build's directory, relative to the checkout.
const APP_MANIFEST: &str = "app/Cargo.toml";
const APP_TARGET: &str = "target";
/// The advisories the window launcher's lockfile accepts, with their reasons.
///
/// Tauri's Linux webview and tray reach GTK 3 through `gtk` 0.18, and no release of Tauri reaches GTK 4 yet.
/// Every crate of the lockfile, for every target, was searched for a use of the unsound iterator, and only `glib` itself names it.
/// Reconsider both when Tauri leaves GTK 3.
///
/// The launcher's Linux build reaches `zbus` 5 through the credential store and through `tauri-plugin-single-instance`, whose newest 2.x release still requires it.
/// `zbus` 6 took in `zvariant`, `zbus_names`, and `zvariant_derive`, which left their own crates without maintenance, and their advisories name no flaw.
/// `.cargo/audit.toml` accepts them for every lockfile because of the credential store, and this list keeps them for the plugin.
/// Reconsider them when the plugin or the credential store releases on a maintained `zbus`, or an advisory names a vulnerability.
const APP_ACCEPTED_ADVISORIES: [(&str, &str); 5] = [
    (
        "RUSTSEC-2024-0370",
        "proc-macro-error is unmaintained, and it runs only while glib-macros compiles",
    ),
    (
        "RUSTSEC-2024-0429",
        "glib 0.18's VariantStrIter is unsound, and no crate the launcher builds iterates one",
    ),
    (
        "RUSTSEC-2026-0370",
        "zvariant went into zbus 6, and only the Linux build reaches the unmaintained crate",
    ),
    (
        "RUSTSEC-2026-0371",
        "zbus_names went into zbus 6, and only the Linux build reaches the unmaintained crate",
    ),
    (
        "RUSTSEC-2026-0372",
        "zvariant_derive went into zbus 6, and only the Linux build reaches the unmaintained crate",
    ),
];

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

/// The relay's Worker, which builds only for WebAssembly: its formatting and its lints.
fn worker(root: &Path) -> Result<()> {
    let manifest = root.join(WORKER).join("Cargo.toml");
    succeed(
        Tool::Cargo
            .command(root)
            .args(["fmt", "--check", "--manifest-path"])
            .arg(&manifest),
        "cargo fmt --check of the Worker",
    )?;
    succeed(
        Tool::Cargo
            .command(root)
            .args(["clippy", "--locked", "--manifest-path"])
            .arg(&manifest)
            .args(["--target", WORKER_TARGET, "--", "-D", "warnings"]),
        "cargo clippy of the Worker",
    )
}

/// The window launcher, in its own workspace: its script under strict TypeScript, then its formatting, its lints, and its tests.
///
/// The compiled script lands beside its source, where the launcher's build embeds it.
/// Each command names the launcher's manifest relative to the checkout, since the checkout's resolved path is a verbatim `\\?\` path on Windows, which reaches the linker through Cargo and fails it.
/// The launcher builds into the checkout's own target directory, the one place a remote runner lets a build write.
fn app(root: &Path) -> Result<()> {
    app_script(root)?;
    app_native(root)
}

fn app_script(root: &Path) -> Result<()> {
    succeed(
        Tool::Tsc.command(root).args(["--project", APP_SCRIPT]),
        "tsc of the window launcher's script",
    )?;
    succeed(
        Tool::Cargo
            .command(root)
            .args(["fmt", "--check", "--manifest-path", APP_MANIFEST]),
        "cargo fmt --check of the window launcher",
    )?;
    Ok(())
}

fn app_native(root: &Path) -> Result<()> {
    succeed(
        Tool::Cargo.command(root).args([
            "clippy",
            "--locked",
            "--all-targets",
            "--manifest-path",
            APP_MANIFEST,
            "--target-dir",
            APP_TARGET,
            "--",
            "-D",
            "warnings",
        ]),
        "cargo clippy of the window launcher",
    )?;
    succeed(
        Tool::Cargo.command(root).args([
            "nextest",
            "run",
            "--locked",
            "--no-tests=fail",
            "--manifest-path",
            APP_MANIFEST,
            "--target-dir",
            APP_TARGET,
        ]),
        "cargo nextest of the window launcher",
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

/// Every manifest with its own lockfile: the workspace, the fuzz workspace, the Worker, the window launcher, and each proof fixture.
fn manifests(root: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut found = vec![root.join("Cargo.toml")];
    for workspace in [proofs::FUZZ, WORKER, APP] {
        let manifest = root.join(workspace).join("Cargo.toml");
        if fs::is_file(&manifest)? {
            found.push(manifest);
        }
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

/// The advisories of every lockfile, refusing each warning but the ones the window launcher's lockfile accepts.
fn audit(root: &Path) -> Result<()> {
    let app = root.join(APP).join("Cargo.toml");
    for manifest in manifests(root)? {
        let lock = manifest.with_file_name("Cargo.lock");
        let mut command = Tool::Cargo.command(root);
        command
            .args(["audit", "--deny", "warnings", "--file"])
            .arg(&lock);
        if manifest == app {
            for (advisory, _reason) in APP_ACCEPTED_ADVISORIES {
                command.args(["--ignore", advisory]);
            }
        }
        succeed(&mut command, "cargo audit")?;
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

fn everything(root: &Path, native_app: bool) -> Result<()> {
    let mut report = Report::default();
    report.stage("rustfmt", || rustfmt(root, None));
    report.stage("taplo", || taplo(root, None));
    report.stage("typos", || typos(root, None));
    report.stage("clippy", || clippy(root));
    report.stage("worker", || worker(root));
    report.stage("app", || {
        if native_app {
            app(root)
        } else {
            app_script(root)
        }
    });
    report.stage("icons", || crate::icons::check(root));
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
/// A Linux host builds the window launcher only with the `WebKitGTK` development packages, so there the container of `just app-linux` covers it instead.
fn platform(root: &Path) -> Result<()> {
    let mut report = Report::default();
    report.stage("clippy", || clippy(root));
    report.stage("structure", || structure::check(root));
    report.stage("bare", || bare(root));
    report.stage("tests", || tests(root));
    if !cfg!(target_os = "linux") {
        report.stage("app", || app(root));
    }
    report.finish("check --platform")
}

/// Runs the checks of one scope and reports every failure together.
///
/// # Errors
///
/// Returns an error naming each failed check.
pub fn run(root: &Path, scope: Scope) -> Result<()> {
    match scope {
        Scope::Everything => everything(root, true),
        Scope::Hosted => everything(root, false),
        Scope::Staged => staged(root),
        Scope::Platform => platform(root),
    }
}

/// The explicit full campaign: every check, the Linux window, and every proof.
/// The Linux check and the proofs run in memory-limited containers.
///
/// # Errors
///
/// Returns an error when a check or a proof fails.
pub fn push_check(root: &Path) -> Result<()> {
    run(root, Scope::Everything)?;
    crate::app::linux(root)?;
    crate::container::run(root, &["proofs"])
}

/// Writes the generated files from their sources.
///
/// # Errors
///
/// Returns an error when a generator fails.
pub fn generate(root: &Path) -> Result<()> {
    crate::quint::generate(root)?;
    crate::icons::generate(root)?;
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
    report.stage("icons", || crate::icons::check(root));
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
