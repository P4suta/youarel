use super::{DIST, desktop, program_name, variable, workspace_version};
use crate::fs;
use crate::package;
use crate::tool::{self, Tool};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

const MACOS_PROGRAMS: [&str; 2] = ["/usr/local/bin/youarel", "/usr/local/bin/youarel-launcher"];
const LINUX_PROGRAMS: [&str; 2] = ["/usr/bin/youarel", "/usr/bin/youarel-launcher"];
const HOSTED: &str = "github-hosted";
const INSTALLATIONS: usize = 2;
const RELEASE_WORKFLOW: &str = "P4suta/youarel/.github/workflows/release.yml@refs/heads/main";

fn permitted(actions: Option<&str>, runner: Option<&str>, workflow: Option<&str>) -> bool {
    actions == Some("true") && runner == Some(HOSTED) && workflow == Some(RELEASE_WORKFLOW)
}

fn versions(root: &Path, version: &str, programs: &[PathBuf]) -> Result<()> {
    let home = tempfile::tempdir()?;
    for path in programs {
        desktop::check_version(root, path, version, home.path())?;
    }
    ensure!(
        fs::entries(home.path())?.is_empty(),
        "installed version reporting changed library files"
    );
    Ok(())
}

fn macos(root: &Path, target: &str, version: &str) -> Result<()> {
    let programs = MACOS_PROGRAMS.map(PathBuf::from);
    ensure!(
        !fs::is_directory(Path::new("/Applications/youarel.app"))?,
        "the fresh runner already has the product installed"
    );
    for path in &programs {
        ensure!(
            !fs::is_file(path)?,
            "the fresh runner already has {}",
            path.display()
        );
    }
    let package = root
        .join(DIST)
        .join(format!("youarel-{version}-{target}.pkg"));
    tool::succeed(
        Tool::Spctl
            .command(root)
            .args(["--assess", "--type", "install", "--verbose=2"])
            .arg(&package),
        "assess the notarized installer",
    )?;
    for _ in 0..INSTALLATIONS {
        tool::succeed(
            Tool::Sudo
                .command(root)
                .arg(Tool::Installer.command(root).get_program())
                .arg("-pkg")
                .arg(&package)
                .args(["-target", "/"]),
            "install the macOS package on the fresh hosted runner",
        )?;
        versions(root, version, &programs)?;
        tool::succeed(
            Tool::Codesign
                .command(root)
                .args(["--verify", "--strict", "--deep"])
                .arg("/Applications/youarel.app"),
            "verify the installed application's nested signatures",
        )?;
        tool::succeed(
            Tool::Pkgutil
                .command(root)
                .args(["--pkg-info", crate::apple::PACKAGE_IDENTIFIER]),
            "verify the installer receipt",
        )?;
    }
    Ok(())
}

fn linux(root: &Path, target: &str, version: &str) -> Result<()> {
    let programs = LINUX_PROGRAMS.map(PathBuf::from);
    for path in &programs {
        ensure!(
            !fs::is_file(path)?,
            "the fresh runner already has {}",
            path.display()
        );
    }
    let architecture =
        super::debian_architecture(target).context("unsupported Debian architecture")?;
    let package = root
        .join(DIST)
        .join(format!("youarel_{version}_{architecture}.deb"));
    for _ in 0..INSTALLATIONS {
        tool::succeed(
            Tool::Sudo
                .command(root)
                .arg(Tool::Dpkg.command(root).get_program())
                .arg("--install")
                .arg(&package),
            "install the Debian package on the fresh hosted runner",
        )?;
        versions(root, version, &programs)?;
    }
    tool::succeed(
        Tool::Sudo
            .command(root)
            .args(["dpkg", "--remove", "youarel"]),
        "uninstall the Debian package",
    )?;
    for path in programs {
        ensure!(
            !fs::is_file(&path)?,
            "uninstall retained {}",
            path.display()
        );
    }
    Ok(())
}

fn windows(root: &Path, target: &str, version: &str) -> Result<()> {
    let bytes = fs::read(
        &root
            .join(DIST)
            .join(format!("youarel-{version}-{target}.zip")),
    )?;
    let entries = package::read_zip(&bytes)?;
    let installation = tempfile::tempdir()?;
    desktop::write_entries(installation.path(), &entries)?;
    let folder = installation
        .path()
        .join(format!("youarel-{version}-{target}"));
    let programs = [
        folder.join(program_name(target)),
        folder.join(desktop::launcher_name(target)),
    ];
    let expected = super::required("WINDOWS_SIGNING_IDENTITY_SHA256")?.to_ascii_lowercase();
    ensure!(
        youarel_release::manifest::Digest::parse(&expected).is_some(),
        "the Windows publisher fingerprint is malformed"
    );
    for program in &programs {
        super::verify_windows_program(root, program, &expected)?;
    }
    versions(root, version, &programs)
}

pub(super) fn check(root: &Path, target: &str) -> Result<()> {
    desktop::supported(target)?;
    ensure!(
        permitted(
            variable("GITHUB_ACTIONS")?.as_deref(),
            variable("RUNNER_ENVIRONMENT")?.as_deref(),
            variable("GITHUB_WORKFLOW_REF")?.as_deref()
        ),
        "system installation checks require the fresh hosted release runner"
    );
    ensure!(
        super::host_target(root)? == target,
        "the installer target differs from the native runner"
    );
    let version = workspace_version(root)?.to_string();
    if target.contains("apple") {
        macos(root, target, &version)
    } else if target.contains("windows") {
        windows(root, target, &version)
    } else {
        linux(root, target, &version)
    }
}

#[cfg(test)]
mod tests {
    use super::{HOSTED, RELEASE_WORKFLOW, permitted};

    #[test]
    fn installation_refuses_local_self_hosted_and_unrelated_workflows() {
        assert!(permitted(
            Some("true"),
            Some(HOSTED),
            Some(RELEASE_WORKFLOW)
        ));
        assert!(!permitted(None, Some(HOSTED), Some(RELEASE_WORKFLOW)));
        assert!(!permitted(
            Some("true"),
            Some("self-hosted"),
            Some(RELEASE_WORKFLOW)
        ));
        assert!(!permitted(Some("true"), Some(HOSTED), Some("other.yml")));
    }
}
