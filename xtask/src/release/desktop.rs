use super::built_program;
use crate::fs;
use crate::package::{EXECUTABLE, Entry, REGULAR};
use crate::tool::{self, Tool};
use anyhow::{Context, Result, ensure};
use std::path::{Component, Path, PathBuf};

pub(super) const LAUNCHER: &str = "youarel-launcher";
pub(super) const APP: &str = "youarel.app";
pub(super) const PRODUCTION_FEATURES: &str = "tauri/custom-protocol";
pub(super) const TARGETS: [&str; 6] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-pc-windows-msvc",
    "x86_64-pc-windows-msvc",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
];
pub(super) const LINUX_DEPENDS: &str =
    "libc6 (>= 2.39), libwebkit2gtk-4.1-0, libayatana-appindicator3-1, libxdo3";
const DESKTOP_ENTRY: &str = "[Desktop Entry]\nType=Application\nName=youarel\nComment=Open saved bookmarks\nExec=youarel-launcher\nIcon=io.github.p4suta.youarel\nTerminal=false\nCategories=Network;\n";
const ICON: &str = "app/icons/128x128.png";
const NATIVE_MAC_ICON: &str = "Contents/Resources/Assets.car";
const FALLBACK_MAC_ICON: &str = "Contents/Resources/icon.icns";

pub(super) fn supported(target: &str) -> Result<()> {
    ensure!(
        TARGETS.contains(&target),
        "unsupported release target `{target}`"
    );
    Ok(())
}

pub(super) fn launcher_name(target: &str) -> String {
    if target.contains("windows") {
        format!("{LAUNCHER}.exe")
    } else {
        LAUNCHER.to_owned()
    }
}

pub(super) fn launcher(root: &Path, target: &str) -> PathBuf {
    root.join("app/target")
        .join(target)
        .join("release")
        .join(launcher_name(target))
}

pub(super) fn application(root: &Path, target: &str) -> PathBuf {
    root.join("app/target")
        .join(target)
        .join("release/bundle/macos")
        .join(APP)
}

pub(super) fn build(root: &Path, target: &str) -> Result<()> {
    supported(target)?;
    let sidecars = root.join("app/binaries");
    fs::create_dir_all(&sidecars)?;
    let suffix = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    fs::write(
        &sidecars.join(format!("youarel-{target}{suffix}")),
        &fs::read(&built_program(root, target))?,
    )?;
    tool::succeed(
        Tool::Tsc
            .command(root)
            .args(["--project", "app/tsconfig.json"]),
        "compile the desktop interface",
    )?;
    tool::succeed(
        Tool::Tauri.command(&root.join("app")).args([
            "build",
            "--ci",
            "--no-bundle",
            "--features",
            PRODUCTION_FEATURES,
            "--config",
            "tauri.release.conf.json",
            "--target",
            target,
            "--",
            "--locked",
        ]),
        "build the desktop launcher",
    )?;
    if target.contains("apple") {
        tool::succeed(
            Tool::Tauri.command(&root.join("app")).args([
                "bundle",
                "--bundles",
                "app",
                "--config",
                "tauri.release.conf.json",
                "--target",
                target,
                "--no-sign",
                "--no-binary-patching",
            ]),
            "bundle the unsigned macOS application",
        )?;
    }
    check_programs(root, target, &super::workspace_version(root)?.to_string())
}

fn executable(path: &Path) -> bool {
    path.components().any(|part| part.as_os_str() == "MacOS")
}

fn read_tree(base: &Path, directory: &Path, entries: &mut Vec<Entry>) -> Result<()> {
    for path in fs::entries(directory)? {
        let metadata = fs::entry_metadata(&path)?;
        ensure!(
            !metadata.is_symlink(),
            "release payload contains a symbolic link: {}",
            path.display()
        );
        if metadata.is_dir() {
            read_tree(base, &path, entries)?;
        } else {
            ensure!(
                metadata.is_file(),
                "release payload contains a special file: {}",
                path.display()
            );
            entries.push(Entry {
                path: crate::portable(path.strip_prefix(base)?),
                mode: if executable(&path) {
                    EXECUTABLE
                } else {
                    REGULAR
                },
                bytes: fs::read(&path)?,
            });
        }
    }
    Ok(())
}

pub(super) fn app_entries(root: &Path, target: &str) -> Result<Vec<Entry>> {
    let app = application(root, target);
    let base = app.parent().context("the application has no parent")?;
    let mut entries = Vec::new();
    read_tree(base, &app, &mut entries)?;
    for required in [
        format!("{APP}/Contents/Info.plist"),
        format!("{APP}/Contents/MacOS/{LAUNCHER}"),
        format!("{APP}/Contents/MacOS/youarel"),
        format!("{APP}/{NATIVE_MAC_ICON}"),
        format!("{APP}/{FALLBACK_MAC_ICON}"),
    ] {
        ensure!(
            entries.iter().any(|entry| entry.path == required),
            "the macOS application lacks {required}"
        );
    }
    Ok(entries)
}

pub(super) fn entries(root: &Path, target: &str) -> Result<Vec<Entry>> {
    let mut entries = vec![Entry {
        path: launcher_name(target),
        mode: EXECUTABLE,
        bytes: fs::read(&launcher(root, target))?,
    }];
    if target.contains("apple") {
        entries.extend(app_entries(root, target)?);
    }
    Ok(entries)
}

pub(super) fn linux_entries(root: &Path, launcher: &[u8]) -> Result<Vec<Entry>> {
    Ok(vec![
        Entry {
            path: format!("usr/bin/{LAUNCHER}"),
            mode: EXECUTABLE,
            bytes: launcher.to_vec(),
        },
        Entry {
            path: "usr/share/applications/io.github.p4suta.youarel.desktop".to_owned(),
            mode: REGULAR,
            bytes: DESKTOP_ENTRY.as_bytes().to_vec(),
        },
        Entry {
            path: "usr/share/icons/hicolor/128x128/apps/io.github.p4suta.youarel.png".to_owned(),
            mode: REGULAR,
            bytes: fs::read(&root.join(ICON))?,
        },
    ])
}

pub(super) fn safe_path(text: &str) -> bool {
    !text.is_empty()
        && !text.contains(['\\', ':'])
        && Path::new(text)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

pub(super) fn write_entries(base: &Path, entries: &[Entry]) -> Result<()> {
    for entry in entries {
        ensure!(
            safe_path(&entry.path),
            "unsafe payload path `{}`",
            entry.path
        );
        let path = base.join(&entry.path);
        fs::create_dir_all(path.parent().context("payload file has no parent")?)?;
        if entry.mode == EXECUTABLE {
            fs::write_executable(&path, &entry.bytes)?;
        } else {
            fs::write(&path, &entry.bytes)?;
        }
    }
    Ok(())
}

pub(super) fn check_programs(root: &Path, target: &str, version: &str) -> Result<()> {
    let home = tempfile::tempdir()?;
    for path in [built_program(root, target), launcher(root, target)] {
        check_version(root, &path, version, home.path())?;
    }
    ensure!(
        fs::entries(home.path())?.is_empty(),
        "version checks changed the bookmark library"
    );
    Ok(())
}

pub(super) fn check_version(root: &Path, path: &Path, version: &str, home: &Path) -> Result<()> {
    let output = tool::capture(
        tool::built(root, path)
            .arg("--version")
            .env("YOUAREL_HOME", home),
        "check packaged program version",
    )?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("program name is not UTF-8")?;
    let stem = name.trim_end_matches(".exe");
    ensure!(
        output.trim() == format!("{stem} {version}"),
        "{} reports another version: {output}",
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{safe_path, supported};

    #[test]
    fn macos_payload_requires_native_and_fallback_icons() {
        const TARGET: &str = "aarch64-apple-darwin";
        const FILES: [&str; 5] = [
            "Contents/Info.plist",
            "Contents/MacOS/youarel-launcher",
            "Contents/MacOS/youarel",
            "Contents/Resources/Assets.car",
            "Contents/Resources/icon.icns",
        ];
        for missing in [
            "Contents/Resources/Assets.car",
            "Contents/Resources/icon.icns",
        ] {
            let root = tempfile::tempdir().unwrap();
            let app = super::application(root.path(), TARGET);
            for file in FILES.into_iter().filter(|file| *file != missing) {
                crate::fs::write(&app.join(file), b"fixture").unwrap();
            }
            assert!(
                super::app_entries(root.path(), TARGET).is_err(),
                "{missing}"
            );
            crate::fs::write(&app.join(missing), b"fixture").unwrap();
            assert_eq!(
                super::app_entries(root.path(), TARGET).unwrap().len(),
                FILES.len()
            );
        }
    }

    #[test]
    fn release_paths_and_targets_reject_uncontrolled_inputs() {
        for path in [
            "../outside",
            "/tmp/outside",
            "a/../../outside",
            "C:/outside",
            "a\\outside",
            "",
        ] {
            assert!(!safe_path(path), "{path}");
        }
        assert!(safe_path("youarel.app/Contents/MacOS/youarel"));
        supported("x86_64-unknown-linux-gnu").unwrap();
        assert!(supported("../../outside").is_err());
        assert!(supported("x86_64-unknown-linux-musl").is_err());
    }
}
