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

/// The macOS tools of a release, by the fixed system paths that no `PATH` can redirect.
pub const SECURITY: &str = "/usr/bin/security";
pub const CODESIGN: &str = "/usr/bin/codesign";
pub const PRODUCTSIGN: &str = "/usr/bin/productsign";
pub const PKGBUILD: &str = "/usr/bin/pkgbuild";
pub const PRODUCTBUILD: &str = "/usr/bin/productbuild";
pub const PKGUTIL: &str = "/usr/sbin/pkgutil";
pub const XCRUN: &str = "/usr/bin/xcrun";
pub const SPCTL: &str = "/usr/sbin/spctl";

/// The TypeScript compiler as npm installs it: a script that Windows runs only by its `.cmd` name.
const TSC: &str = if cfg!(windows) { "tsc.cmd" } else { "tsc" };

mod raw {
    #![expect(
        clippy::disallowed_methods,
        reason = "the repository tasks' one process owner constructs each command in a single statement"
    )]

    use std::path::Path;
    use std::process::Command;

    pub(super) fn command(program: &str) -> Command {
        Command::new(program)
    }

    pub(super) fn command_path(program: &Path) -> Command {
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
    Rsync,
    Dotfiles,
    /// Docker, which hosts the proof gate with a memory limit on a workstation.
    Docker,
    Rustc,
    /// The GitHub CLI, which publishes a release.
    Gh,
    /// PowerShell, which checks an Authenticode signature on Windows.
    Pwsh,
    /// The TypeScript compiler, which checks the window launcher's script under strict rules.
    Tsc,
    /// The macOS tools of a release, by their fixed system paths.
    Security,
    Codesign,
    Productsign,
    Pkgbuild,
    Productbuild,
    Pkgutil,
    Xcrun,
    Spctl,
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
            Self::Rsync => "rsync",
            Self::Dotfiles => "dotfiles",
            Self::Docker => "docker",
            Self::Rustc => "rustc",
            Self::Gh => "gh",
            Self::Pwsh => "pwsh",
            Self::Tsc => TSC,
            Self::Security => SECURITY,
            Self::Codesign => CODESIGN,
            Self::Productsign => PRODUCTSIGN,
            Self::Pkgbuild => PKGBUILD,
            Self::Productbuild => PRODUCTBUILD,
            Self::Pkgutil => PKGUTIL,
            Self::Xcrun => XCRUN,
            Self::Spctl => SPCTL,
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

/// A command for a program the tasks built, such as a release candidate, run from `root`.
#[must_use]
pub fn built(root: &Path, program: &Path) -> Command {
    let mut command = raw::command_path(program);
    command.current_dir(root);
    command
}

const REPOSITORY_VARIABLES: [&str; 15] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

fn repository_context(mut command: Command) -> Command {
    for name in REPOSITORY_VARIABLES {
        command.env_remove(name);
    }
    command
}

/// Git for an explicitly selected repository, independent of the invoking hook's context.
#[must_use]
pub fn repository_git(root: &Path) -> Command {
    repository_context(Tool::Git.command(root))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn selected_repositories_ignore_the_parent_hooks_git_context() {
        let parent = tempfile::tempdir().unwrap();
        let selected = tempfile::tempdir().unwrap();
        for root in [parent.path(), selected.path()] {
            succeed(
                repository_git(root).args(["init", "--quiet"]),
                "fixture init",
            )
            .unwrap();
        }
        let mut inherited = raw::command(Tool::Git.program());
        inherited
            .current_dir(selected.path())
            .env("GIT_DIR", parent.path().join(".git"))
            .env("GIT_WORK_TREE", parent.path());
        let actual = capture(
            repository_context(inherited).args(["rev-parse", "--show-toplevel"]),
            "selected fixture repository",
        )
        .unwrap();
        assert_eq!(
            std::fs::canonicalize(actual.trim()).unwrap(),
            std::fs::canonicalize(selected.path()).unwrap()
        );
    }

    #[test]
    fn repository_variables_match_gits_local_environment_contract() {
        let root = tempfile::tempdir().unwrap();
        let actual = capture(
            repository_git(root.path()).args(["rev-parse", "--local-env-vars"]),
            "Git's repository variables",
        )
        .unwrap();
        assert_eq!(
            actual.lines().collect::<BTreeSet<_>>(),
            REPOSITORY_VARIABLES.into_iter().collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn the_current_hooks_git_keeps_its_index_context() {
        let selected = repository_git(Path::new("."));
        for name in REPOSITORY_VARIABLES {
            assert!(
                selected
                    .get_envs()
                    .any(|(key, value)| key == name && value.is_none())
            );
        }
        assert!(
            Tool::Git
                .command(Path::new("."))
                .get_envs()
                .next()
                .is_none()
        );
    }
}
