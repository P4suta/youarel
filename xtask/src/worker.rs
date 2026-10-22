//! The relay's Worker test: worker-build builds the Worker, `wrangler dev --local` serves it in a pinned container, and the relay's and the recovery's end-to-end tests run against it there.
//!
//! The container reaches the network to fetch crates and the bundler that worker-build uses, and nothing in it deploys the Worker.

use crate::tool::{self, Tool};
use crate::{container, quint};
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// The directory that defines the image, the files whose bytes name it, and its repository.
const IMAGE_DIRECTORY: &str = "xtask/proofs/worker";
const IMAGE_INPUTS: [&str; 3] = ["Dockerfile", "package.json", "package-lock.json"];
const REPOSITORY: &str = "youarel-worker";

/// The memory the run may use, with no swap beyond it, and the processors.
const MEMORY: &str = "8g";
const PROCESSORS: &str = "4";

/// The volumes that keep the build and the crates between runs.
const TARGET_VOLUME: &str = "youarel-worker-target";
const REGISTRY_VOLUME: &str = "youarel-worker-registry";

/// Lint, build, and serve the Worker, wait until it answers, and run the relay's test against it.
const TASK: &str = r#"cd /work/server
CARGO_TARGET_DIR=/work/target/worker cargo clippy --locked --target wasm32-unknown-unknown -- -D warnings
CARGO_TARGET_DIR=/work/target/worker worker-build --release
wrangler dev --local --ip 127.0.0.1 --port 8787 --show-interactive-dev-session=false >/tmp/wrangler.log 2>&1 &
cd /work
cargo build --locked --package youarel-cli
ready=no
for attempt in $(seq 120); do
  if curl --silent --output /dev/null --request POST http://127.0.0.1:8787/v1/mail; then ready=yes; break; fi
  sleep 1
done
if [ "$ready" != yes ]; then cat /tmp/wrangler.log; exit 1; fi
YOUAREL_RELAY_URL=http://127.0.0.1:8787 cargo nextest run --locked --package youarel-cli --test relay_e2e --test recovery_e2e
requests=$(grep -c 'POST /v1/requests' /tmp/wrangler.log || true)
mail=$(grep -c 'POST /v1/mail' /tmp/wrangler.log || true)
located=$(grep -c 'POST /v1/locate' /tmp/wrangler.log || true)
echo "the Worker served $requests requests, $mail pieces of mail, and $located questions of location"
if [ "$requests" -eq 0 ] || [ "$mail" -eq 0 ] || [ "$located" -eq 0 ]; then cat /tmp/wrangler.log; exit 1; fi
"#;

/// Builds the Worker, serves it locally, and runs the relay's and the recovery's end-to-end tests against it.
///
/// # Errors
///
/// Returns an error when Docker cannot build or run the image, or the Worker or the test fails.
pub fn run(root: &Path) -> Result<()> {
    let root = std::fs::canonicalize(root).context("resolve the checkout")?;
    let image = quint::pinned_image(&root, IMAGE_DIRECTORY, &IMAGE_INPUTS, REPOSITORY)?;
    quint::build_pinned(&root, IMAGE_DIRECTORY, &image)?;
    tool::succeed(&mut command(&root, &image)?, "the Worker's end-to-end test")
}

fn command(root: &Path, image: &str) -> Result<Command> {
    let target_volume = container::compiler_volume(root, image, TARGET_VOLUME)?;
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
        .args([
            "--mount",
            &format!("type=volume,source={target_volume},target=/work/target"),
            "--mount",
            &format!("type=volume,source={REGISTRY_VOLUME},target=/usr/local/cargo/registry"),
            "--workdir",
            "/work",
            image,
            "sh",
            "-c",
            &format!("{}\n{TASK}", container::copy_script()),
        ]);
    Ok(docker)
}

#[cfg(test)]
mod tests {
    use super::command;
    use std::path::Path;

    #[test]
    fn compiler_outputs_belong_to_the_checkout_and_image() {
        let volume = |root: &Path, image: &str| {
            command(root, image)
                .unwrap()
                .get_args()
                .find(|argument| argument.to_string_lossy().ends_with(",target=/work/target"))
                .unwrap()
                .to_os_string()
        };
        let first = volume(Path::new("/first"), "image:a");
        assert_ne!(first, volume(Path::new("/second"), "image:a"));
        assert_ne!(first, volume(Path::new("/first"), "image:b"));
    }
}
