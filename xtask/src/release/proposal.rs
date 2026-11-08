use super::workspace_version;
use crate::fs;
use crate::tool::{self, Tool};
use anyhow::{Context, Result, ensure};
use std::path::Path;
use youarel_core::update_rules::Version;

const VERSION_FILE: &str = "version.txt";
const APP_MANIFEST: &str = "app/Cargo.toml";
const APP_CONFIG: &str = "app/tauri.conf.json";
const RELEASE_MANIFEST: &str = ".release-please-manifest.json";
const LOCKFILES: [&str; 3] = ["Cargo.lock", "app/Cargo.lock", "fuzz/Cargo.lock"];
const CHECK_DIRECTORY: &str = "xtask/release-please";
const TYPES: &str = "xtask/release-please/tsconfig.json";
const TESTS: &str = "xtask/release-please/version-updates.test.ts";

fn product_package(name: &str) -> bool {
    (name.starts_with("youarel-") && name != "youarel-fuzz") || name == "xtask"
}

fn matches(expected: Version, actual: Option<&str>, source: &str) -> Result<()> {
    let actual = actual.with_context(|| format!("{source} names no version"))?;
    ensure!(
        actual == expected.to_string(),
        "{source} names {actual}, expected {expected}"
    );
    Ok(())
}

pub(super) fn versions(root: &Path) -> Result<Version> {
    let expected = workspace_version(root)?;
    matches(
        expected,
        Some(fs::read_to_string(&root.join(VERSION_FILE))?.trim()),
        VERSION_FILE,
    )?;
    let app: toml::Table = toml::from_str(&fs::read_to_string(&root.join(APP_MANIFEST))?)?;
    matches(
        expected,
        app.get("package")
            .and_then(|package| package.get("version"))
            .and_then(toml::Value::as_str),
        APP_MANIFEST,
    )?;
    let config: serde_json::Value = serde_json::from_slice(&fs::read(&root.join(APP_CONFIG))?)?;
    matches(
        expected,
        config.get("version").and_then(serde_json::Value::as_str),
        APP_CONFIG,
    )?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&root.join(RELEASE_MANIFEST))?)?;
    matches(
        expected,
        manifest.get(".").and_then(serde_json::Value::as_str),
        RELEASE_MANIFEST,
    )?;
    for source in LOCKFILES {
        let locked: toml::Table = toml::from_str(&fs::read_to_string(&root.join(source))?)?;
        let packages = locked
            .get("package")
            .and_then(toml::Value::as_array)
            .with_context(|| format!("{source} contains no packages"))?;
        let mut found = false;
        for package in packages {
            let name = package
                .get("name")
                .and_then(toml::Value::as_str)
                .context("a locked package has no name")?;
            if product_package(name) {
                found = true;
                matches(
                    expected,
                    package.get("version").and_then(toml::Value::as_str),
                    &format!("{source}: {name}"),
                )?;
            }
        }
        ensure!(found, "{source} contains no product packages");
    }
    Ok(expected)
}

pub(super) fn check(root: &Path) -> Result<()> {
    versions(root)?;
    tool::succeed(
        Tool::Npm.command(root).args([
            "ci",
            "--prefix",
            CHECK_DIRECTORY,
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ]),
        "install the locked proposal checks",
    )?;
    tool::succeed(
        Tool::Tsc.command(root).args(["--project", TYPES]),
        "check proposal test types",
    )?;
    tool::succeed(
        Tool::Node.command(root).args(["--test", TESTS]),
        "exercise the pinned release-please updaters",
    )?;
    tool::succeed(
        Tool::Npm
            .command(root)
            .args(["audit", "--prefix", CHECK_DIRECTORY, "--audit-level=low"]),
        "audit proposal check dependencies",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir_all(&root.join("app")).unwrap();
        fs::create_dir_all(&root.join("fuzz")).unwrap();
        for (path, content) in [
            ("Cargo.toml", "[workspace.package]\nversion = '0.4.0'\n"),
            (VERSION_FILE, "0.4.0\n"),
            (APP_MANIFEST, "[package]\nversion = '0.4.0'\n"),
            (APP_CONFIG, "{\"version\":\"0.4.0\"}"),
            (RELEASE_MANIFEST, "{\".\":\"0.4.0\"}"),
        ] {
            fs::write(&root.join(path), content.as_bytes()).unwrap();
        }
        for path in LOCKFILES {
            fs::write(&root.join(path), b"[[package]]\nname = 'youarel-core'\nversion = '0.4.0'\n[[package]]\nname = 'independent'\nversion = '9.8.7'\n").unwrap();
        }
        directory
    }

    #[test]
    fn product_versions_match_while_independent_dependencies_keep_their_versions() {
        let directory = fixture();
        assert_eq!(versions(directory.path()).unwrap().to_string(), "0.4.0");
    }

    #[test]
    fn version_drift_in_any_product_input_is_rejected() {
        for path in [
            "Cargo.toml",
            VERSION_FILE,
            APP_MANIFEST,
            APP_CONFIG,
            RELEASE_MANIFEST,
            "Cargo.lock",
            "app/Cargo.lock",
            "fuzz/Cargo.lock",
        ] {
            let directory = fixture();
            let file = directory.path().join(path);
            let changed = fs::read_to_string(&file).unwrap().replace("0.4.0", "0.5.0");
            fs::write(&file, changed.as_bytes()).unwrap();
            assert!(versions(directory.path()).is_err(), "{path}");
        }
    }
}
