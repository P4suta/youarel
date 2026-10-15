//! The Quint gate over the models of `specs/`.
//!
//! Each model typechecks, passes its tests, and holds its invariant in every reachable state, whose count the manifest records.
//! It also witnesses every outcome and writes its traces again, while its probe module breaks the invariant.
//! Quint runs in a pinned image with no network, limited memory, the checkout mounted read-only, and one writable output directory.

use crate::fs;
use crate::obligations::{self, Obligation, QuintModel};
use crate::tool::{self, Tool};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Output;

/// The Quint release the image's lockfile pins.
pub const QUINT_VERSION: &str = "0.33.0";

/// The directory that defines the image.
pub const IMAGE_DIRECTORY: &str = "xtask/proofs/quint";

/// The files whose bytes name the image, so a changed pin builds a new one.
const IMAGE_INPUTS: [&str; 3] = ["Dockerfile", "package.json", "package-lock.json"];

/// Where the container mounts the read-only checkout and the writable output directory.
pub const SOURCE: &str = "/source";
pub const OUTPUT: &str = "/out";

/// Apalache writes its run directory under the working directory, which the read-only checkout cannot hold.
const WORKDIR: &str = "/tmp";

/// The memory a run may use, with no swap beyond it, and the processors.
const MEMORY: &str = "6g";
const PROCESSORS: &str = "2";

/// How many hexadecimal digits of the inputs' digest the image name carries.
const DIGEST_DIGITS: usize = 16;

/// The runs that may search for the probe's violation, which the seed makes repeatable.
const PROBE_SAMPLES: &str = "1000";

/// One worker keeps the model checker's state count exact and the run repeatable.
const TLC_CONFIG: &str = r#"{"workers": 1}"#;

/// The verbosity at which Quint passes on the model checker's own report, which holds the state count.
const TLC_REPORT: &str = "3";

/// Where the committed traces live.
pub const TRACES: &str = "specs/traces";

/// The image name for the definition files, which changes with any of them.
///
/// # Errors
///
/// Returns an error when a definition file cannot be read.
pub fn image(root: &Path) -> Result<String> {
    pinned_image(root, IMAGE_DIRECTORY, &IMAGE_INPUTS, "youarel-quint")
}

/// The name `repository:digest` of the image that `directory` defines, whose digest covers the bytes of `inputs`.
///
/// # Errors
///
/// Returns an error when a definition file cannot be read.
pub fn pinned_image(
    root: &Path,
    directory: &str,
    inputs: &[&str],
    repository: &str,
) -> Result<String> {
    let mut hasher = Sha256::new();
    for input in inputs {
        hasher.update(fs::read(&root.join(directory).join(input))?);
    }
    let mut digest = String::new();
    for byte in hasher.finalize() {
        write!(digest, "{byte:02x}").context("format the digest")?;
    }
    Ok(format!(
        "{repository}:{}",
        digest.get(..DIGEST_DIGITS).unwrap_or(&digest)
    ))
}

/// Builds the pinned image unless it exists.
///
/// # Errors
///
/// Returns an error when Docker cannot build it.
pub fn build(root: &Path, image: &str) -> Result<()> {
    build_pinned(root, IMAGE_DIRECTORY, image)
}

/// Builds `image` from `directory` unless it exists.
///
/// # Errors
///
/// Returns an error when Docker cannot build it.
pub fn build_pinned(root: &Path, directory: &str, image: &str) -> Result<()> {
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
        .arg(root.join(directory));
    tool::succeed(&mut build, &format!("docker build of {image}"))
}

/// The `--mount` value that binds `source` at `target`.
#[must_use]
pub fn mount(source: &Path, target: &str, readonly: bool) -> OsString {
    // Docker reads `--mount` as one CSV record, so quoting keeps a comma in the path inside the field.
    let mut mount = OsString::from("type=bind,\"source=");
    mount.push(source);
    mount.push(format!("\",target={target}"));
    if readonly {
        mount.push(",readonly");
    }
    mount
}

/// Where one gate run happens: the checkout, the output directory, and the image.
#[derive(Clone, Copy, Debug)]
pub struct Session<'a> {
    pub root: &'a Path,
    pub output: &'a Path,
    pub image: &'a str,
}

/// The arguments that run `quint` with `arguments` in the image, with no network, limited memory, the checkout read-only, and the output writable.
#[must_use]
pub fn run_arguments(session: Session<'_>, arguments: &[&str]) -> Vec<OsString> {
    let mut docker: Vec<OsString> = [
        "run",
        "--rm",
        "--network",
        "none",
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
    docker.push(mount(session.root, SOURCE, true));
    docker.push("--mount".into());
    docker.push(mount(session.output, OUTPUT, false));
    docker.extend(["--workdir", WORKDIR, session.image, "quint"].map(OsString::from));
    docker.extend(arguments.iter().map(OsString::from));
    docker
}

/// The distinct states the model checker reports in `report`, or `None` when it reported none.
///
/// # Errors
///
/// Returns an error when the report names a count that is not a number.
pub fn distinct_states(report: &str) -> Result<Option<u64>> {
    let Some(count) = report.lines().find_map(|line| {
        let (_, rest) = line.split_once("states generated, ")?;
        rest.strip_suffix(" distinct states found, 0 states left on queue.")
    }) else {
        return Ok(None);
    };
    Ok(Some(
        count
            .parse()
            .context("the model checker's state count is a number")?,
    ))
}

/// How many traces reached the witness `name` in a `quint run` report, or `None` when the report never names it.
///
/// # Errors
///
/// Returns an error when the report names a count that is not a number.
pub fn witnessed(report: &str, name: &str) -> Result<Option<u64>> {
    let Some(count) = report.lines().find_map(|line| {
        let rest = line
            .strip_prefix(name)?
            .strip_prefix(" was witnessed in ")?;
        Some(rest.split_once(' ')?.0)
    }) else {
        return Ok(None);
    };
    Ok(Some(count.parse().with_context(|| {
        format!("the witness count of {name} is a number")
    })?))
}

/// The key under which a trace carries the run's own metadata.
const META: &str = "#meta";
/// The metadata fields that name the run rather than the model, which the committed traces leave out.
const VOLATILE: [&str; 2] = ["description", "timestamp"];

/// The states of a trace, which leave out the run's own metadata.
fn states(text: &str) -> Result<Value> {
    let document: Value = serde_json::from_str(text).context("parse a trace")?;
    Ok(document.get("states").cloned().unwrap_or(Value::Null))
}

/// The trace without the fields that name the run, so the file reads the same whenever the states do.
fn settled(text: &[u8]) -> Result<Vec<u8>> {
    let mut document: Value = serde_json::from_slice(text).context("parse a trace")?;
    if let Some(meta) = document.get_mut(META).and_then(Value::as_object_mut) {
        for field in VOLATILE {
            meta.remove(field);
        }
    }
    serde_json::to_vec(&document).context("write a trace")
}

/// The file name of the trace `index` of `main`, which the gate writes and the repository commits.
#[must_use]
pub fn trace_name(main: &str, index: usize) -> String {
    format!("{main}.{index}.itf.json")
}

/// The file name of the probe's violation trace.
#[must_use]
pub fn probe_trace_name(probe: &str) -> String {
    format!("{probe}.itf.json")
}

impl Session<'_> {
    fn quint(self, arguments: &[&str]) -> Result<Output> {
        let mut docker = Tool::Docker.command(self.root);
        docker.args(run_arguments(self, arguments));
        tool::output(&mut docker, "docker run of Quint")
    }

    /// Runs `quint` and requires success, returning its standard output.
    fn passing(self, what: &str, arguments: &[&str]) -> Result<String> {
        let ran = self.quint(arguments)?;
        let stdout = String::from_utf8_lossy(&ran.stdout).into_owned();
        ensure!(
            ran.status.success(),
            "{what} failed: {}\n{stdout}{}",
            ran.status,
            String::from_utf8_lossy(&ran.stderr)
        );
        Ok(stdout)
    }

    /// The model checker with one worker on `module`, which returns Quint's output.
    fn tlc(self, model: &str, module: &str, invariant: &str) -> Result<Output> {
        let config = format!("{OUTPUT}/tlc.json");
        self.quint(&[
            "verify",
            model,
            "--main",
            module,
            "--backend",
            "tlc",
            "--tlc-config",
            &config,
            "--invariant",
            invariant,
            "--verbosity",
            TLC_REPORT,
        ])
    }

    /// Typechecks the model, runs its tests and those of its probe, and checks the invariant in every reachable state.
    fn check(self, model: &str, quint: &QuintModel) -> Result<()> {
        self.passing("quint typecheck", &["typecheck", model])?;
        for module in [&quint.main, &quint.probe] {
            self.passing(
                &format!("the tests of {module}"),
                &["test", model, "--backend", "typescript", "--main", module],
            )?;
        }
        let checked = self.tlc(model, &quint.main, &quint.invariant)?;
        let report = String::from_utf8_lossy(&checked.stdout);
        let found = distinct_states(&report)?;
        ensure!(
            checked.status.success() && found == Some(quint.distinct_states),
            "the model checker found {found:?} distinct states of {} and ended with {}, while {} records {}; review the model change, then record the new count:\n{report}{}",
            quint.main,
            checked.status,
            obligations::MANIFEST,
            quint.distinct_states,
            String::from_utf8_lossy(&checked.stderr)
        );
        let probed = self.tlc(model, &quint.probe, &quint.invariant)?;
        ensure!(
            !probed.status.success()
                && String::from_utf8_lossy(&probed.stderr).contains("found a counterexample"),
            "the model checker found no violation of {} in {}, so nothing shows the check can fail",
            quint.invariant,
            quint.probe
        );
        Ok(())
    }

    /// Simulates the model at its seed and requires a trace for every witness.
    fn witness(self, model: &str, quint: &QuintModel) -> Result<()> {
        let samples = quint.samples.to_string();
        let steps = quint.steps.to_string();
        let mut simulate = vec![
            "run",
            model,
            "--backend",
            "typescript",
            "--main",
            &quint.main,
            "--seed",
            &quint.seed,
            "--max-samples",
            &samples,
            "--max-steps",
            &steps,
            "--invariant",
            &quint.invariant,
            "--witnesses",
        ];
        simulate.extend(quint.witnesses.iter().map(String::as_str));
        let report = self.passing(&format!("the simulation of {}", quint.main), &simulate)?;
        for name in &quint.witnesses {
            let count = witnessed(&report, name)?;
            ensure!(
                count.is_some_and(|count| count > 0),
                "no trace of {} at seed {} witnessed {name}: {count:?}",
                quint.main,
                quint.seed
            );
        }
        Ok(())
    }

    /// Writes the traces of the model and the violation of its probe under `OUTPUT/traces`, at the seed.
    fn write_traces(self, model: &str, quint: &QuintModel) -> Result<()> {
        fs::create_dir_all(&self.output.join("traces"))?;
        let steps = quint.steps.to_string();
        let count = quint.traces.to_string();
        let pattern = format!("{OUTPUT}/traces/{}.{{seq}}.itf.json", quint.main);
        let probe = format!("{OUTPUT}/traces/{}", probe_trace_name(&quint.probe));
        let base = [
            "run",
            model,
            "--backend",
            "typescript",
            "--seed",
            &quint.seed,
        ];
        let mut traced = base.to_vec();
        traced.extend([
            "--main",
            &quint.main,
            "--max-samples",
            &count,
            "--n-traces",
            &count,
            "--max-steps",
            &steps,
            "--mbt",
            "--out-itf",
            &pattern,
            "--verbosity",
            "0",
        ]);
        self.passing(&format!("the traces of {}", quint.main), &traced)?;
        let mut violated = base.to_vec();
        violated.extend([
            "--main",
            &quint.probe,
            "--max-samples",
            PROBE_SAMPLES,
            "--max-steps",
            &steps,
            "--invariant",
            &quint.invariant,
            "--mbt",
            "--out-itf",
            &probe,
            "--verbosity",
            "0",
        ]);
        // The probe's run fails on the violation it records.
        let probed = self.quint(&violated)?;
        ensure!(
            !probed.status.success(),
            "the probe {} broke no invariant in {PROBE_SAMPLES} runs at seed {}",
            quint.probe,
            quint.seed
        );
        Ok(())
    }

    /// Every trace file the model commits, by name.
    fn trace_names(quint: &QuintModel) -> Vec<String> {
        let mut names: Vec<String> = (0..quint.traces)
            .map(|index| trace_name(&quint.main, index))
            .collect();
        names.push(probe_trace_name(&quint.probe));
        names
    }

    /// Writes the traces again and requires the committed states.
    fn traces(self, model: &str, quint: &QuintModel) -> Result<()> {
        self.write_traces(model, quint)?;
        for name in Self::trace_names(quint) {
            let committed = self.root.join(TRACES).join(&name);
            let fresh = fs::read_to_string(&self.output.join("traces").join(&name))
                .with_context(|| format!("Quint wrote no trace {name}"))?;
            ensure!(
                fs::is_file(&committed)?,
                "the trace {TRACES}/{name} is not committed; run `cargo xtask gen` and review it"
            );
            ensure!(
                states(&fs::read_to_string(&committed)?)? == states(&fresh)?,
                "{TRACES}/{name} differs from the trace Quint writes at seed {}; run `cargo xtask gen` and review the change",
                quint.seed
            );
        }
        Ok(())
    }
}

/// The models the manifest declares, each with the path Quint reads it from inside the container.
fn models(manifest: &obligations::Manifest) -> Vec<(String, &QuintModel)> {
    manifest
        .obligation
        .iter()
        .filter_map(|obligation| match obligation {
            Obligation::Quint(quint) => Some((format!("{SOURCE}/{}", quint.spec), quint)),
            Obligation::Kani { .. } | Obligation::Proverif(_) => None,
        })
        .collect()
}

/// A session over a fresh output directory with the model checker's configuration written.
fn start(root: &Path, output: &Path) -> Result<String> {
    let image = image(root)?;
    build(root, &image)?;
    fs::write(&output.join("tlc.json"), TLC_CONFIG.as_bytes())?;
    Ok(image)
}

/// Discharges every Quint obligation of the manifest in the pinned image, offline.
///
/// # Errors
///
/// Returns an error when Docker cannot build or run the image, or any check of an obligation fails.
pub fn verify(root: &Path) -> Result<()> {
    let root = std::fs::canonicalize(root).context("resolve the checkout")?;
    let manifest = obligations::load(&root)?;
    let output = tempfile::tempdir().context("create the Quint output directory")?;
    let image = start(&root, output.path())?;
    let session = Session {
        root: &root,
        output: output.path(),
        image: &image,
    };
    let found = models(&manifest);
    ensure!(
        !found.is_empty(),
        "the manifest declares no Quint obligation"
    );
    for (model, quint) in &found {
        session.check(model, quint)?;
        session.witness(model, quint)?;
        session.traces(model, quint)?;
    }
    eprintln!(
        "quint: {} models typecheck, pass their tests, hold their invariant in every reachable state, witness every outcome, and reproduce their traces, and each probe breaks the invariant",
        found.len()
    );
    Ok(())
}

/// Writes the traces of every Quint obligation under `specs/traces`, for the repository to commit.
///
/// # Errors
///
/// Returns an error when Docker cannot build or run the image, or Quint writes no trace.
pub fn generate(root: &Path) -> Result<()> {
    let root = std::fs::canonicalize(root).context("resolve the checkout")?;
    let manifest = obligations::load(&root)?;
    let output = tempfile::tempdir().context("create the Quint output directory")?;
    let image = start(&root, output.path())?;
    let session = Session {
        root: &root,
        output: output.path(),
        image: &image,
    };
    fs::create_dir_all(&root.join(TRACES))?;
    for (model, quint) in models(&manifest) {
        session.write_traces(&model, quint)?;
        for name in Session::trace_names(quint) {
            let fresh = fs::read(&output.path().join("traces").join(&name))
                .with_context(|| format!("Quint wrote no trace {name}"))?;
            fs::write(&root.join(TRACES).join(&name), &settled(&fresh)?)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Session, distinct_states, probe_trace_name, run_arguments, settled, states, trace_name,
        witnessed,
    };
    use std::path::Path;

    #[test]
    fn a_settled_trace_keeps_its_states_and_forgets_the_run() {
        let text = br##"{"#meta":{"format":"ITF","description":"Created by Quint on Tue","timestamp":1,"source":"x"},"vars":["a"],"states":[{"a":{"#bigint":"1"}}]}"##;
        let once = settled(text).unwrap();
        let written = String::from_utf8(once.clone()).unwrap();
        assert!(!written.contains("timestamp"), "{written}");
        assert!(!written.contains("Created by"), "{written}");
        assert!(written.contains(r#""source":"x""#), "{written}");
        assert_eq!(
            states(&written).unwrap(),
            states(std::str::from_utf8(text).unwrap()).unwrap()
        );
        assert_eq!(settled(&once).unwrap(), once);
    }

    #[test]
    fn the_reports_yield_their_counts_and_the_run_mounts_the_checkout_read_only() {
        let report =
            "...\n12345 states generated, 971 distinct states found, 0 states left on queue.\n";
        assert_eq!(distinct_states(report).unwrap(), Some(971));
        assert_eq!(distinct_states("nothing").unwrap(), None);
        distinct_states("1 states generated, x distinct states found, 0 states left on queue.")
            .unwrap_err();
        let witnesses =
            "[ok] All invariants hold\nopened was witnessed in 42 trace(s) out of 100 explored\n";
        assert_eq!(witnessed(witnesses, "opened").unwrap(), Some(42));
        assert_eq!(witnessed(witnesses, "other").unwrap(), None);
        assert_eq!(trace_name("ui", 3), "ui.3.itf.json");
        assert_eq!(probe_trace_name("ui_probe"), "ui_probe.itf.json");
        let session = Session {
            root: Path::new("/checkout"),
            output: Path::new("/scratch"),
            image: "youarel-quint:abc",
        };
        let text: Vec<String> = run_arguments(session, &["typecheck", "/source/specs/ui.qnt"])
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(text[0..4], ["run", "--rm", "--network", "none"]);
        assert!(text.windows(2).any(|pair| pair == ["--memory", "6g"]));
        assert!(text.contains(&String::from(
            "type=bind,\"source=/checkout\",target=/source,readonly"
        )));
        assert!(text.contains(&String::from("quint")));
        assert_eq!(
            text.last().map(String::as_str),
            Some("/source/specs/ui.qnt")
        );
    }
}
