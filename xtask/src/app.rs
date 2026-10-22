//! The window launcher's Linux check, in a pinned container with `WebKitGTK`.
//!
//! The container builds, lints, and tests the launcher, and types into its window on a virtual display, so a Linux host needs none of the webview's development packages.
//! It reaches the network to fetch crates, and nothing in it installs on the host.

use crate::tool::{self, Tool};
use crate::{container, quint};
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

mod cache;
use cache::CompilerCache;

/// The directory that defines the image, the files whose bytes name it, and its repository.
const IMAGE_DIRECTORY: &str = "xtask/proofs/app-linux";
const IMAGE_INPUTS: [&str; 3] = ["Dockerfile", "package.json", "package-lock.json"];
const REPOSITORY: &str = "youarel-app-linux";

/// The memory the run may use, with no swap beyond it, and the processors.
const MEMORY: &str = "8g";
const PROCESSORS: &str = "4";

/// The volumes that keep the build and the crates between runs.
const TARGET_VOLUME: &str = "youarel-app-linux-target";
const REGISTRY_VOLUME: &str = "youarel-app-linux-registry";

/// Compile the script, lint and test the launcher, then type into its window on a virtual display.
const TASK: &str = r"cd /work
tsc --project app/tsconfig.json
export CARGO_TARGET_DIR=/work/target/app
cargo clippy --locked --all-targets --manifest-path app/Cargo.toml -- -D warnings
cargo nextest run --locked --no-tests=fail --manifest-path app/Cargo.toml
Xvfb :99 -screen 0 1280x800x24 >/tmp/xvfb.log 2>&1 &
export DISPLAY=:99
dbus-run-session -- cargo nextest run --locked --no-tests=fail --manifest-path app/Cargo.toml --run-ignored only --test window
";

/// Lints and tests the window launcher on Linux in the pinned container.
///
/// # Errors
///
/// Returns an error when Docker cannot build or run the image, or the launcher fails a check.
pub fn linux(root: &Path) -> Result<()> {
    let root = std::fs::canonicalize(root).context("resolve the checkout")?;
    let image = quint::pinned_image(&root, IMAGE_DIRECTORY, &IMAGE_INPUTS, REPOSITORY)?;
    let cache = CompilerCache::from_environment(&root, &image)?;
    cache.prepare()?;
    quint::build_pinned(&root, IMAGE_DIRECTORY, &image)?;
    tool::succeed(
        &mut command(&root, &image, &cache),
        "the window launcher's Linux check",
    )
}

pub(crate) fn prepare_cache(root: &Path) -> Result<()> {
    let image = quint::pinned_image(root, IMAGE_DIRECTORY, &IMAGE_INPUTS, REPOSITORY)?;
    cache::publish(root, &image)
}

fn command(root: &Path, image: &str, cache: &CompilerCache) -> Command {
    let mut docker = Tool::Docker.command(root);
    docker
        .args([
            "run",
            "--rm",
            "--init",
            "--memory",
            MEMORY,
            "--memory-swap",
            MEMORY,
            "--cpus",
            PROCESSORS,
            "--mount",
        ])
        .arg(quint::mount(root, "/checkout", true))
        .args(["--mount"])
        .arg(cache.mount())
        .args([
            "--mount",
            &format!("type=volume,source={REGISTRY_VOLUME},target=/usr/local/cargo/registry"),
            "--workdir",
            "/work",
            image,
            "sh",
            "-c",
            &format!("{}\n{TASK}", container::copy_script()),
        ]);
    docker
}

#[cfg(test)]
mod tests {
    use super::{CompilerCache, command};
    use std::path::Path;

    #[test]
    fn compiler_outputs_belong_to_the_checkout_and_image() {
        let volume = |root: &Path, image: &str| {
            let cache = CompilerCache::from_facts(root, image, None).unwrap();
            command(root, image, &cache)
                .get_args()
                .find(|argument| argument.to_string_lossy().ends_with(",target=/work/target"))
                .unwrap()
                .to_os_string()
        };
        let first_checkout = tempfile::tempdir().unwrap();
        let second_checkout = tempfile::tempdir().unwrap();
        let first = volume(first_checkout.path(), "image:a");
        assert_ne!(first, volume(second_checkout.path(), "image:a"));
        assert_ne!(first, volume(first_checkout.path(), "image:b"));
    }
}
