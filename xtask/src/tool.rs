//! The one owner of child processes in the repository tasks, which names each pinned tool.

use anyhow::{Context, Result, ensure};
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// The nightly toolchain that Kani 0.68.0 pins, which Miri and cargo-fuzz share.
pub const NIGHTLY: &str = "nightly-2026-08-21";

/// The pinned versions the proof gate requires before it trusts a result.
pub const KANI_VERSION: &str = "0.68.0";
pub const MUTANTS_VERSION: &str = "27.1.0";
pub const FUZZ_VERSION: &str = "0.13.2";

mod raw {
    #![expect(
        clippy::disallowed_methods,
        reason = "the repository tasks' one process owner constructs each command in a single statement"
    )]

    use std::process::Command;

    pub(super) fn command(program: &str) -> Command {
        Command::new(program)
    }
}

/// A tool the tasks run, each resolved through the pinned `mise` environment on `PATH`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tool {
    Cargo,
    /// Cargo on the pinned nightly, for Miri and cargo-fuzz.
    CargoNightly,
    Rustfmt,
    Taplo,
    Typos,
    Actionlint,
    Zizmor,
    Git,
    Dotfiles,
}

impl Tool {
    const fn program(self) -> &'static str {
        match self {
            Self::Cargo | Self::CargoNightly => "cargo",
            Self::Rustfmt => "rustfmt",
            Self::Taplo => "taplo",
            Self::Typos => "typos",
            Self::Actionlint => "actionlint",
            Self::Zizmor => "zizmor",
            Self::Git => "git",
            Self::Dotfiles => "dotfiles",
        }
    }

    /// A command for the tool, run from `root`.
    #[must_use]
    pub fn command(self, root: &Path) -> Command {
        let mut command = raw::command(self.program());
        command.current_dir(root);
        if self == Self::CargoNightly {
            command.arg(format!("+{NIGHTLY}"));
        }
        command
    }
}

/// Runs a command with inherited streams and requires success.
///
/// # Errors
///
/// Returns an error naming `what` when the command cannot start or exits with failure.
pub fn succeed(command: &mut Command, what: &str) -> Result<()> {
    let status = command.status().with_context(|| format!("start {what}"))?;
    ensure!(status.success(), "{what} failed with {status}");
    Ok(())
}

/// Runs a command with inherited streams and returns whether it succeeded.
///
/// # Errors
///
/// Returns an error naming `what` when the command cannot start.
pub fn status(command: &mut Command, what: &str) -> Result<bool> {
    let status = command.status().with_context(|| format!("start {what}"))?;
    Ok(status.success())
}

/// Runs a command with captured streams.
///
/// # Errors
///
/// Returns an error naming `what` when the command cannot start.
pub fn output(command: &mut Command, what: &str) -> Result<Output> {
    command
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("start {what}"))
}

/// The standard output of a command that must succeed.
///
/// # Errors
///
/// Returns an error naming `what` when the command cannot start, fails, or prints invalid UTF-8.
pub fn capture(command: &mut Command, what: &str) -> Result<String> {
    let output = output(command, what)?;
    ensure!(
        output.status.success(),
        "{what} failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).with_context(|| format!("{what} printed invalid UTF-8"))
}

/// Requires that `command --version` names `version`.
///
/// # Errors
///
/// Returns an error when the tool is missing or reports another version.
pub fn require_version(command: &mut Command, what: &str, version: &str) -> Result<()> {
    let text = capture(command.arg("--version"), what)?;
    ensure!(
        text.contains(version),
        "the pinned {what} {version} is unavailable; found: {}",
        text.trim()
    );
    Ok(())
}

/// Appends arguments to a command and returns it for chaining.
pub fn with_args<I, S>(command: &mut Command, arguments: I) -> &mut Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    command.args(arguments);
    command
}
