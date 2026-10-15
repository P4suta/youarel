//! The proof gate inside a container with fixed memory and processor limits, so a solver that runs away stops there instead of exhausting the workstation.
//!
//! The container copies the checkout from a read-only mount, leaving out what Git ignores, so uncommitted edits run and nothing the container writes reaches the checkout.
//! Named volumes keep the build outputs and the downloaded crates between runs.

use crate::tool::{self, Tool};
use crate::{fs, proofs};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;

/// The directory that defines the image.
pub const IMAGE_DIRECTORY: &str = "xtask/proofs/image";
/// The memory a run may use, with no swap beyond it.
pub const MEMORY: &str = "8g";
/// The processors a run may use, which leaves the rest of the workstation responsive.
pub const PROCESSORS: &str = "4";

/// Where the container mounts the checkout, read-only.
const CHECKOUT: &str = "/checkout";
/// Where the container works on its copy.
const WORK: &str = "/work";
/// How many hexadecimal digits of the definition's digest the image name carries.
const DIGEST_DIGITS: usize = 16;

/// The named volumes the runs share, and where each mounts.
const VOLUMES: [(&str, &str); 4] = [
    ("youarel-proofs-target", "/work/target"),
    ("youarel-proofs-fuzz-target", "/work/fuzz/target"),
    (
        "youarel-proofs-fixture-target",
        "/work/xtask/proofs/fuzz-fixture/target",
    ),
    ("youarel-proofs-registry", "/usr/local/cargo/registry"),
];

/// The image name for the definition, which changes with it.
///
/// # Errors
///
/// Returns an error when the definition cannot be read.
pub fn image(root: &Path) -> Result<String> {
    let definition = fs::read(&root.join(IMAGE_DIRECTORY).join("Dockerfile"))?;
    let mut digest = String::new();
    for byte in Sha256::digest(&definition) {
        write!(digest, "{byte:02x}").context("format the digest")?;
    }
    Ok(format!(
        "youarel-proofs:{}",
        digest.get(..DIGEST_DIGITS).unwrap_or(&digest)
    ))
}

/// Builds the image unless it exists.
fn build(root: &Path, image: &str) -> Result<()> {
    let mut inspect = Tool::Docker.command(root);
    inspect.args(["image", "inspect", image]);
    if tool::output(&mut inspect, "docker image inspect")?
        .status
        .success()
    {
        return Ok(());
    }
    let mut build = Tool::Docker.command(root);
    build
        .args(["build", "--tag", image])
        .arg(root.join(IMAGE_DIRECTORY));
    tool::succeed(&mut build, "docker build of the proof image")
}

/// A `--mount` value, quoted so a comma in the path stays inside its field.
fn bind(source: &Path) -> OsString {
    let mut mount = OsString::from("type=bind,\"source=");
    mount.push(source);
    mount.push(format!("\",target={CHECKOUT},readonly"));
    mount
}

/// The shell command the container runs: copy the checkout without what Git ignores, then run the task.
fn script(words: &[&str]) -> String {
    let mut task = String::from("cargo xtask");
    for word in words {
        task.push(' ');
        task.push_str(&shell_quote(word));
    }
    format!(
        "set -eu; rsync --archive --delete --exclude=.git --exclude-from={CHECKOUT}/.gitignore --exclude=target {CHECKOUT}/ {WORK}/; cd {WORK}; {task}"
    )
}

/// One word for a POSIX shell, in single quotes.
fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// The arguments of `docker run` for a task in the image.
#[must_use]
pub fn arguments(root: &Path, image: &str, words: &[&str]) -> Vec<OsString> {
    let mut docker: Vec<OsString> = [
        "run",
        "--rm",
        "--init",
        "--memory",
        MEMORY,
        "--memory-swap",
        MEMORY,
        "--cpus",
        PROCESSORS,
    ]
    .map(OsString::from)
    .into();
    docker.push("--mount".into());
    docker.push(bind(root));
    for (volume, target) in VOLUMES {
        docker.push("--mount".into());
        docker.push(format!("type=volume,source={volume},target={target}").into());
    }
    docker.extend(["--workdir", WORK, image, "sh", "-c"].map(OsString::from));
    docker.push(script(words).into());
    docker
}

/// Runs `cargo xtask` with `words` in the proof image.
///
/// The Quint and ProVerif steps already run in their own limited containers, so a `proofs` task runs them from the host and everything else inside.
///
/// # Errors
///
/// Returns an error when Docker cannot build or run the image, or when the task fails.
pub fn run(root: &Path, words: &[&str]) -> Result<()> {
    let mut inner: Vec<&str> = words.to_vec();
    if let Some((&"proofs", rest)) = words.split_first() {
        let options = proofs::Options::from_words(rest)
            .with_context(|| format!("unknown proof options {rest:?}"))?;
        let selected = options.selected();
        for step in selected.iter().filter(|step| step.contained()) {
            let hosted = proofs::Options {
                only: Some(*step),
                ..options
            };
            proofs::run(root, hosted)?;
        }
        if selected.iter().all(|step| step.contained()) {
            return Ok(());
        }
        inner.push("--inside");
    }
    let image = image(root)?;
    build(root, &image)?;
    let mut docker = Tool::Docker.command(root);
    docker.args(arguments(root, &image, &inner));
    tool::succeed(&mut docker, "the proof container")
}

#[cfg(test)]
mod tests {
    use super::{MEMORY, arguments, script, shell_quote};
    use std::path::Path;

    #[test]
    fn the_run_limits_memory_and_mounts_the_checkout_read_only() {
        let text: Vec<String> = arguments(
            Path::new("/checkout dir"),
            "youarel-proofs:abc",
            &["proofs", "--only", "kani"],
        )
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
        assert_eq!(text[0..3], ["run", "--rm", "--init"]);
        assert!(text.windows(2).any(|pair| pair == ["--memory", MEMORY]));
        assert!(
            text.windows(2)
                .any(|pair| pair == ["--memory-swap", MEMORY])
        );
        assert!(text.contains(&String::from(
            "type=bind,\"source=/checkout dir\",target=/checkout,readonly"
        )));
        assert!(
            text.iter()
                .any(|argument| argument.contains("youarel-proofs-target"))
        );
        assert!(
            text.last()
                .is_some_and(|last| last.ends_with("cargo xtask 'proofs' '--only' 'kani'"))
        );
    }

    #[test]
    fn words_reach_the_shell_quoted() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert!(script(&["fuzz", "a b"]).ends_with("cargo xtask 'fuzz' 'a b'"));
        assert!(script(&[]).contains("--exclude-from=/checkout/.gitignore"));
    }
}
