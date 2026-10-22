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

const CACHE_DOMAIN: &[u8] = b"youarel-compiler-cache-v1\0";
const CACHE_SEPARATOR: &[u8] = b"\0";
const REGISTRY_VOLUME: &str = "youarel-proofs-registry";
const REGISTRY_TARGET: &str = "/usr/local/cargo/registry";

const VOLUMES: [(&str, &str); 3] = [
    ("youarel-proofs-target", "/work/target"),
    ("youarel-proofs-fuzz-target", "/work/fuzz/target"),
    (
        "youarel-proofs-fixture-target",
        "/work/xtask/proofs/fuzz-fixture/target",
    ),
];

pub(crate) fn compiler_volume(root: &Path, image: &str, namespace: &str) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(CACHE_DOMAIN);
    digest.update(root.as_os_str().as_encoded_bytes());
    digest.update(CACHE_SEPARATOR);
    digest.update(image.as_bytes());
    let mut volume = format!("{namespace}-");
    for byte in digest.finalize() {
        write!(volume, "{byte:02x}").context("format the compiler cache digest")?;
    }
    Ok(volume)
}

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
    format!("{}; cd {WORK}; {task}", copy_script())
}

fn copy_arguments(source: &str, target: &str) -> Vec<String> {
    let mut arguments: Vec<String> = ["--archive", "--delete", "--exclude=.git"]
        .map(String::from)
        .into();
    arguments.extend([
        "--include=/.mise/".to_owned(),
        "--include=/.mise/locks/***".to_owned(),
        format!("--exclude-from={source}/.gitignore"),
        "--exclude=target".to_owned(),
        format!("{source}/"),
        format!("{target}/"),
    ]);
    arguments
}

pub(crate) fn copy_script() -> String {
    let mut script = String::from("set -eu; rsync");
    for argument in copy_arguments(CHECKOUT, WORK) {
        script.push(' ');
        script.push_str(&shell_quote(&argument));
    }
    script
}

/// One word for a POSIX shell, in single quotes.
fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// The arguments of `docker run` for a task in the image.
///
/// # Errors
///
/// Returns an error when a compiler cache digest cannot be formatted.
pub fn arguments(root: &Path, image: &str, words: &[&str]) -> Result<Vec<OsString>> {
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
        let volume = compiler_volume(root, image, volume)?;
        docker.push("--mount".into());
        docker.push(format!("type=volume,source={volume},target={target}").into());
    }
    docker.push("--mount".into());
    docker.push(format!("type=volume,source={REGISTRY_VOLUME},target={REGISTRY_TARGET}").into());
    docker.extend(["--workdir", WORK, image, "sh", "-c"].map(OsString::from));
    docker.push(script(words).into());
    Ok(docker)
}

/// Runs `cargo xtask` with `words` in the proof image.
///
/// The Quint and ProVerif steps already run in their own limited containers, so a `proofs` task runs them from the host and everything else inside.
///
/// # Errors
///
/// Returns an error when Docker cannot build or run the image, or when the task fails.
pub fn run(root: &Path, words: &[&str]) -> Result<()> {
    let root = std::fs::canonicalize(root).context("resolve the checkout")?;
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
            proofs::run(&root, hosted)?;
        }
        if selected.iter().all(|step| step.contained()) {
            return Ok(());
        }
        inner.push("--inside");
    }
    let image = image(&root)?;
    build(&root, &image)?;
    let mut docker = Tool::Docker.command(&root);
    docker.args(arguments(&root, &image, &inner)?);
    tool::succeed(&mut docker, "the proof container")
}

#[cfg(test)]
mod tests {
    use super::{MEMORY, arguments, script, shell_quote};
    use std::path::Path;

    #[test]
    fn compiler_outputs_belong_to_the_checkout_and_image() {
        let mounts = |root: &Path, image: &str| -> Vec<String> {
            arguments(root, image, &[])
                .unwrap()
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .filter(|argument| argument.starts_with("type=volume,"))
                .collect()
        };
        let first = mounts(Path::new("/first"), "image:a");
        let other_checkout = mounts(Path::new("/second"), "image:a");
        let other_image = mounts(Path::new("/first"), "image:b");
        for target in [
            "/work/target",
            "/work/fuzz/target",
            "/work/xtask/proofs/fuzz-fixture/target",
        ] {
            let at = |values: &[String]| {
                values
                    .iter()
                    .find(|value| value.ends_with(&format!(",target={target}")))
                    .unwrap()
                    .clone()
            };
            assert_ne!(at(&first), at(&other_checkout), "{target}");
            assert_ne!(at(&first), at(&other_image), "{target}");
        }
        let registry = |values: &[String]| {
            values
                .iter()
                .find(|value| value.ends_with(",target=/usr/local/cargo/registry"))
                .unwrap()
                .clone()
        };
        assert_eq!(registry(&first), registry(&other_checkout));
        assert_eq!(registry(&first), registry(&other_image));
    }

    #[test]
    #[cfg(unix)]
    fn the_copy_keeps_native_dependency_locks_and_excludes_private_state() {
        use super::copy_arguments;
        use crate::fs;
        use crate::tool::{self, Tool};
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("source dir's");
        let target = fixture.path().join("target");
        fs::write(&source.join(".gitignore"), b"/.mise/*\n!/.mise/locks/\n").unwrap();
        fs::write(&source.join(".mise/locks/native/lock"), b"locked").unwrap();
        fs::write(&source.join(".mise/cache/private"), b"private").unwrap();
        fs::write(&source.join(".git/private"), b"private").unwrap();
        tool::succeed(
            Tool::Rsync.command(fixture.path()).args(copy_arguments(
                source.to_str().unwrap(),
                target.to_str().unwrap(),
            )),
            "the source copy fixture",
        )
        .unwrap();
        assert_eq!(
            fs::read(&target.join(".mise/locks/native/lock")).unwrap(),
            b"locked"
        );
        assert_eq!(fs::kind(&target.join(".mise/cache/private")).unwrap(), None);
        assert_eq!(fs::kind(&target.join(".git/private")).unwrap(), None);
    }

    #[test]
    fn the_run_limits_memory_and_mounts_the_checkout_read_only() {
        let text: Vec<String> = arguments(
            Path::new("/checkout dir"),
            "youarel-proofs:abc",
            &["proofs", "--only", "kani"],
        )
        .unwrap()
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
