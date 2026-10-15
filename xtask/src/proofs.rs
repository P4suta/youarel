//! The proof gate: every verifier of the obligation manifest, each shown to refuse a false claim.

use crate::obligations::{self, Harness, Manifest, Obligation, Probe};
use crate::report::Report;
use crate::structure::{self, CORE, Harnesses};
use crate::tool::{self, FUZZ_VERSION, KANI_VERSION, MUTANTS_VERSION, Tool};
use crate::{fs, portable};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where each verifier writes the receipt of its last run.
pub const RECEIPTS: &str = "target/proofs/receipts";

/// A crate whose one harness asserts a false claim, which the Kani gate must refuse.
pub const KANI_FIXTURE: &str = "xtask/proofs/kani-negative";
pub const KANI_FIXTURE_HARNESS: &str = "the_gate_refuses_a_false_claim";
pub const KANI_FIXTURE_ASSERTION: &str = "NEGATIVE_FIXTURE_CONTROL";

/// A fuzz target whose committed corpus makes it panic, which the replay must refuse.
pub const FUZZ_FIXTURE: &str = "xtask/proofs/fuzz-fixture";
pub const FUZZ_FIXTURE_TARGET: &str = "crash";
/// The panic message of the fuzz fixture, which its refusal must show.
pub const FUZZ_FIXTURE_PANIC: &str = "the fixture input reached the target";

/// A crate with undefined behavior, which the Miri gate must refuse.
pub const MIRI_FIXTURE: &str = "xtask/proofs/miri-fixture";
/// What Miri reports for the fixture's read past the end, which its refusal must show.
pub const MIRI_FIXTURE_DIAGNOSTIC: &str = "Undefined Behavior";

/// A crate whose test asserts nothing, which the mutation gate must refuse.
pub const MUTANTS_FIXTURE: &str = "xtask/proofs/mutants-fixture";

/// The separate cargo-fuzz workspace of the product.
pub const FUZZ: &str = "fuzz";

/// The crates Miri interprets: the core, the adapter that runs the URL parser's unsafe code, and the HTML reader; the store's tests reach the file system, which Miri isolates.
pub const MIRI_CRATES: [&str; 3] = ["youarel-core", "youarel-link", "youarel-html"];

/// The budgets of the mutation campaign over the pure crates, which `assure.toml` declares.
pub const MISSED_CEILING: u64 = 0;
pub const TIMEOUT_CEILING: u64 = 0;

const HARNESS_TIMEOUT: &str = "300s";
const MUTANT_JOBS: &str = "2";
const MUTANT_TEST_THREADS: &str = "1";
const MAX_REPORT_BYTES: u64 = 16 * 1024 * 1024;
const MIRI_TARGET: &str = "target/miri";
/// The nextest profile Miri runs under, which bounds the time of each test.
const MIRI_PROFILE: &str = "miri";
const MUTANTS_OUTPUT: &str = "target/proofs/mutants";
/// The crates the mutation campaign runs over, each judged by its own tests alone.
const MUTANT_PACKAGES: [&str; 9] = [
    "youarel-core",
    "youarel-link",
    "youarel-html",
    "youarel-release",
    "youarel-store",
    "youarel-crypto",
    "youarel-protocol",
    "youarel-relay",
    "youarel-sync",
];
const CORE_PACKAGE: &str = "youarel-core";
const FIXTURE_OUTPUT: &str = "target/proofs/fixtures";
/// Where a fuzz campaign keeps the inputs it finds, outside the committed corpus.
const FUZZ_SCRATCH: &str = "target/fuzz-corpus";

/// One verifier the gate runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    Kani,
    Quint,
    Proverif,
    Miri,
    Fuzz,
    Mutants,
}

impl Step {
    pub const ALL: [Self; 6] = [
        Self::Kani,
        Self::Quint,
        Self::Proverif,
        Self::Miri,
        Self::Fuzz,
        Self::Mutants,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Kani => "kani",
            Self::Quint => "quint",
            Self::Proverif => "proverif",
            Self::Miri => "miri",
            Self::Fuzz => "fuzz",
            Self::Mutants => "mutants",
        }
    }

    /// Whether the step starts its own limited container, so a host runs it outside the proof container.
    #[must_use]
    pub const fn contained(self) -> bool {
        match self {
            Self::Quint | Self::Proverif => true,
            Self::Kani | Self::Miri | Self::Fuzz | Self::Mutants => false,
        }
    }

    #[must_use]
    pub fn named(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|step| step.name() == word)
    }
}

/// Which verifiers run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Options {
    /// A hosted run, which skips the mutation campaign that stays local.
    pub ci: bool,
    pub only: Option<Step>,
    /// A verifier to leave out.
    pub without: Option<Step>,
    /// Whether to leave out the steps that start their own containers, which the host runs outside the proof container.
    pub inside: bool,
}

impl Options {
    /// Reads `--ci`, `--only STEP`, `--without STEP`, and `--inside`, refusing anything else.
    #[must_use]
    pub fn from_words(words: &[&str]) -> Option<Self> {
        let mut options = Self {
            ci: false,
            only: None,
            without: None,
            inside: false,
        };
        let mut words = words.iter();
        while let Some(word) = words.next() {
            match *word {
                "--ci" => options.ci = true,
                "--only" => options.only = Some(Step::named(words.next()?)?),
                "--without" => options.without = Some(Step::named(words.next()?)?),
                "--inside" => options.inside = true,
                _ => return None,
            }
        }
        Some(options)
    }

    #[must_use]
    pub fn selected(self) -> Vec<Step> {
        let chosen: Vec<Step> = self.only.map_or_else(
            || {
                Step::ALL
                    .into_iter()
                    .filter(|step| !(self.ci && *step == Step::Mutants))
                    .collect()
            },
            |step| vec![step],
        );
        chosen
            .into_iter()
            .filter(|step| Some(*step) != self.without && !(self.inside && step.contained()))
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct KaniReport {
    metadata: KaniMetadata,
    property_details: Vec<KaniProperties>,
    verification_results: KaniResults,
}

#[derive(Debug, Deserialize)]
struct KaniMetadata {
    kani_version: String,
}

#[derive(Debug, Deserialize)]
struct KaniProperties {
    harness_id: String,
    property_details: KaniCounts,
}

#[derive(Debug, Deserialize)]
struct KaniCounts {
    failed: u64,
    undetermined: u64,
    solver_error: u64,
    satisfied: u64,
    unsatisfiable: u64,
}

#[derive(Debug, Deserialize)]
struct KaniResults {
    summary: KaniSummary,
    results: Vec<KaniHarnessResult>,
}

#[derive(Debug, Deserialize)]
struct KaniSummary {
    total_harnesses: u64,
    executed: u64,
    status: String,
    successful: u64,
    failed: u64,
}

#[derive(Debug, Deserialize)]
struct KaniHarnessResult {
    harness_id: String,
    status: String,
    checks: Vec<KaniCheck>,
}

#[derive(Debug, Deserialize)]
struct KaniCheck {
    status: String,
    description: String,
    category: String,
}

fn read_report(path: &Path) -> Result<KaniReport> {
    let bytes = fs::read(path)?;
    ensure!(
        u64::try_from(bytes.len()).is_ok_and(|length| length <= MAX_REPORT_BYTES),
        "the Kani report at {} exceeds {MAX_REPORT_BYTES} bytes",
        path.display()
    );
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parse the Kani report at {}", path.display()))
}

fn check_positive(report: &KaniReport, harnesses: &[&Harness]) -> Result<()> {
    let summary = &report.verification_results.summary;
    let count = u64::try_from(harnesses.len()).context("count the harnesses")?;
    ensure!(
        count > 0
            && report.metadata.kani_version == KANI_VERSION
            && summary.status == "completed"
            && summary.total_harnesses == count
            && summary.executed == count
            && summary.successful == count
            && summary.failed == 0
            && report.property_details.len() == harnesses.len()
            && report.verification_results.results.len() == harnesses.len(),
        "the proof inventory requires every declared Kani harness to complete and pass"
    );
    for harness in harnesses {
        let name = &harness.name;
        let properties: Vec<_> = report
            .property_details
            .iter()
            .filter(|item| &item.harness_id == name)
            .collect();
        let results: Vec<_> = report
            .verification_results
            .results
            .iter()
            .filter(|item| &item.harness_id == name)
            .collect();
        let ([properties], [result]) = (properties.as_slice(), results.as_slice()) else {
            bail!("missing or duplicate proof: {name}");
        };
        let counts = &properties.property_details;
        ensure!(
            result.status == "Success"
                && result
                    .checks
                    .iter()
                    .any(|check| check.category == "assertion" && check.status == "Success")
                && counts.failed == 0
                && counts.undetermined == 0
                && counts.solver_error == 0
                && counts.unsatisfiable == 0
                && counts.satisfied == harness.covers
                && result.checks.iter().all(|check| {
                    matches!(
                        check.status.as_str(),
                        "Success" | "Satisfied" | "Unreachable"
                    )
                }),
            "the proof {name} has incomplete checks or reaches other covers than the {} it declares",
            harness.covers
        );
    }
    Ok(())
}

fn check_negative(report: &KaniReport, harness: &str, assertion: &str) -> Result<()> {
    let summary = &report.verification_results.summary;
    let [result] = report.verification_results.results.as_slice() else {
        bail!("a negative control executes exactly one harness");
    };
    let [properties] = report.property_details.as_slice() else {
        bail!("a negative control reports one complete property summary");
    };
    let counts = &properties.property_details;
    ensure!(
        report.metadata.kani_version == KANI_VERSION
            && summary.status == "completed"
            && summary.total_harnesses == 1
            && summary.executed == 1
            && summary.failed == 1
            && summary.successful == 0
            && result.harness_id == harness
            && result.status == "Failure"
            && properties.harness_id == harness
            && counts.failed == 1
            && counts.undetermined == 0
            && counts.solver_error == 0
            && result.checks.iter().any(|check| {
                check.status == "Failure"
                    && check.category == "assertion"
                    && check.description.contains(assertion)
            }),
        "the negative control {harness} needs the expected assertion counterexample `{assertion}`, not a tool error"
    );
    Ok(())
}

fn kani_command(root: &Path, manifest: Option<&Path>, target: &Path, report: &Path) -> Command {
    let mut command = Tool::Cargo.command(root);
    command.arg("kani");
    match manifest {
        Some(manifest) => {
            command.arg("--manifest-path").arg(manifest);
        }
        None => {
            command.args(["--package", CORE_PACKAGE]);
        }
    }
    command
        .args([
            "--exact",
            "--output-format=terse",
            "-Z",
            "unstable-options",
            &format!("--harness-timeout={HARNESS_TIMEOUT}"),
        ])
        .arg("--target-dir")
        .arg(target)
        .arg("--export-json")
        .arg(report);
    command
}

/// Every rule module of the core with the harnesses it defines, keyed by its portable path.
///
/// # Errors
///
/// Returns an error when a module cannot be read or parsed.
pub fn discover(root: &Path) -> Result<Vec<(String, Harnesses)>> {
    let mut found = Vec::new();
    for path in fs::files_with_suffix(&root.join(CORE).join("src"), "_rules.rs")? {
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("{} lies in the checkout", path.display()))?;
        found.push((
            portable(relative),
            structure::harnesses(&fs::read_to_string(&path)?)?,
        ));
    }
    Ok(found)
}

fn sorted(names: &[String]) -> Vec<String> {
    let mut names = names.to_vec();
    names.sort();
    names
}

/// Every way the manifest disagrees with the harnesses the rule modules define.
#[must_use]
pub fn inventory_findings(
    discovered: &[(String, Harnesses)],
    manifest: &Manifest,
    sources: &dyn Fn(&str) -> Result<String>,
) -> Vec<String> {
    let mut findings = Vec::new();
    for (source, found) in discovered {
        let Some(Obligation::Kani {
            harnesses, probes, ..
        }) = manifest.obligation.iter().find(
            |obligation| matches!(obligation, Obligation::Kani { source: declared, .. } if declared == source),
        )
        else {
            findings.push(format!(
                "{source} declares no Kani obligation in {}; add one naming its harnesses and probes",
                obligations::MANIFEST
            ));
            continue;
        };
        let declared: Vec<String> = harnesses
            .iter()
            .map(|harness| harness.name.clone())
            .collect();
        if sorted(&declared) != sorted(&found.proofs) {
            findings.push(format!(
                "{source} defines the proofs {} but the manifest declares {}",
                found.proofs.join(", "),
                declared.join(", ")
            ));
        }
        let declared_probes: Vec<String> =
            probes.iter().map(|probe| probe.harness.clone()).collect();
        if sorted(&declared_probes) != sorted(&found.probes) {
            findings.push(format!(
                "{source} defines the probes {} but the manifest declares {}",
                found.probes.join(", "),
                declared_probes.join(", ")
            ));
        }
        for probe in probes {
            match sources(source) {
                Ok(text) if text.contains(&probe.assertion) => {}
                Ok(_) => findings.push(format!(
                    "{source} never asserts `{}`, which the probe {} declares",
                    probe.assertion, probe.harness
                )),
                Err(error) => findings.push(format!("{source} cannot be read: {error:#}")),
            }
        }
    }
    for obligation in &manifest.obligation {
        let Obligation::Kani { source, .. } = obligation else {
            continue;
        };
        if !discovered.iter().any(|(found, _)| found == source) {
            findings.push(format!(
                "the manifest declares {source}, which is not a rule module of the core"
            ));
        }
    }
    findings
}

fn run_probe(root: &Path, scratch: &Path, manifest: Option<&Path>, probe: &Probe) -> Result<()> {
    let target = scratch.join(format!("probe-{}", probe.harness));
    let report = scratch.join(format!("probe-{}.json", probe.harness));
    let mut command = kani_command(root, manifest, &target, &report);
    command.args(["--harness", &probe.harness]);
    let passed = tool::status(&mut command, "cargo kani")?;
    ensure!(
        !passed,
        "the deliberately false assertion passed: {}",
        probe.harness
    );
    check_negative(&read_report(&report)?, &probe.harness, &probe.assertion)
}

/// The harness name Kani matches with `--exact`: the source file's module followed by the name inside it.
#[must_use]
pub fn qualify(source: &str, name: &str) -> String {
    let stem = Path::new(source)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    format!("{stem}::{name}")
}

fn verify_kani(root: &Path) -> Result<()> {
    tool::require_version(
        Tool::Cargo.command(root).arg("kani"),
        "cargo-kani",
        KANI_VERSION,
    )?;
    let discovered = discover(root)?;
    let manifest = obligations::load(root)?;
    let sources = |source: &str| fs::read_to_string(&root.join(source));
    let findings = inventory_findings(&discovered, &manifest, &sources);
    ensure!(
        findings.is_empty(),
        "the proof inventory is stale:\n{}",
        findings.join("\n")
    );
    let scratch = tempfile::tempdir().context("create the proof scratch directory")?;
    let qualified: Vec<Harness> = manifest
        .obligation
        .iter()
        .flat_map(|obligation| match obligation {
            Obligation::Kani {
                source, harnesses, ..
            } => harnesses
                .iter()
                .map(|harness| Harness {
                    name: qualify(source, &harness.name),
                    ..harness.clone()
                })
                .collect(),
            Obligation::Quint(_) | Obligation::Proverif(_) => Vec::new(),
        })
        .collect();
    ensure!(
        !qualified.is_empty(),
        "the manifest declares no Kani harness, so the gate would prove nothing"
    );
    let harnesses: Vec<&Harness> = qualified.iter().collect();
    let report = scratch.path().join("proofs.json");
    let mut command = kani_command(root, None, &scratch.path().join("proofs"), &report);
    for harness in &harnesses {
        command.args(["--harness", &harness.name]);
    }
    tool::succeed(&mut command, "cargo kani")?;
    check_positive(&read_report(&report)?, &harnesses)?;
    for obligation in &manifest.obligation {
        let Obligation::Kani { source, probes, .. } = obligation else {
            continue;
        };
        for probe in probes {
            let probe = Probe {
                harness: qualify(source, &probe.harness),
                assertion: probe.assertion.clone(),
            };
            run_probe(root, scratch.path(), None, &probe)?;
        }
    }
    let fixture = Probe {
        harness: KANI_FIXTURE_HARNESS.to_owned(),
        assertion: KANI_FIXTURE_ASSERTION.to_owned(),
    };
    run_probe(
        root,
        scratch.path(),
        Some(&root.join(KANI_FIXTURE).join("Cargo.toml")),
        &fixture,
    )
    .context("the Kani fixture must be refused")
}

/// Miri runs the tests through nextest, whose `miri` profile ends any test that outlives its budget and names it.
fn miri_command(root: &Path, manifest: Option<&Path>, target: &Path) -> Command {
    let mut command = Tool::CargoNightly.command(root);
    command.args(["miri", "nextest", "run", "--locked"]);
    match manifest {
        Some(manifest) => {
            command.arg("--manifest-path").arg(manifest);
        }
        // The workspace's own profile bounds each test; a fixture workspace runs under its defaults.
        None => {
            command.args(["--profile", MIRI_PROFILE]);
        }
    }
    command.arg("--target-dir").arg(target);
    command
}

fn verify_miri(root: &Path) -> Result<()> {
    tool::capture(
        Tool::CargoNightly
            .command(root)
            .arg("miri")
            .arg("--version"),
        "cargo miri",
    )?;
    let mut command = miri_command(root, None, &root.join(MIRI_TARGET));
    for krate in MIRI_CRATES {
        command.args(["--package", krate]);
    }
    tool::succeed(&mut command, "cargo miri test")?;
    let fixture = root.join(MIRI_FIXTURE);
    refused(
        &mut miri_command(
            root,
            Some(&fixture.join("Cargo.toml")),
            &root.join(FIXTURE_OUTPUT).join("miri"),
        ),
        "the Miri fixture with undefined behavior",
        MIRI_FIXTURE_DIAGNOSTIC,
    )
}

/// Runs a fixture that must fail and requires its output to name `expected`, so a tool error never passes for the refusal.
fn refused(command: &mut Command, what: &str, expected: &str) -> Result<()> {
    let output = tool::output(command, what)?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(
        !output.status.success(),
        "{what} passed; the gate cannot show it fails"
    );
    ensure!(
        text.contains(expected),
        "{what} failed without `{expected}`, so the failure is a tool error rather than the refusal:\n{text}"
    );
    Ok(())
}

/// One cargo-fuzz target: the directory of its fuzz workspace and its name.
#[derive(Clone, Copy, Debug)]
struct Target<'a> {
    directory: &'a str,
    name: &'a str,
}

/// A run over the committed corpus of `target`, after `scratch` when one is given.
///
/// libFuzzer writes every new input into its first corpus directory, so a campaign names a scratch directory first to keep the committed seeds curated.
fn fuzz_run(
    root: &Path,
    target: Target<'_>,
    scratch: Option<&Path>,
    arguments: &[&str],
) -> Command {
    let mut command = Tool::CargoNightly.command(root);
    command.args(["fuzz", "run", "--fuzz-dir", target.directory, target.name]);
    if let Some(scratch) = scratch {
        command.arg(scratch);
    }
    command
        .arg(Path::new(target.directory).join("corpus").join(target.name))
        .arg("--")
        .args(arguments);
    command
}

fn fuzz_targets(root: &Path, directory: &str) -> Result<Vec<String>> {
    let mut targets = Vec::new();
    for path in fs::files_with_suffix(&root.join(directory).join("fuzz_targets"), ".rs")? {
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .context("a fuzz target has a UTF-8 name")?;
        targets.push(stem.to_owned());
    }
    Ok(targets)
}

fn verify_fuzz(root: &Path) -> Result<()> {
    tool::require_version(
        Tool::Cargo.command(root).arg("fuzz"),
        "cargo-fuzz",
        FUZZ_VERSION,
    )?;
    let targets = fuzz_targets(root, FUZZ)?;
    ensure!(!targets.is_empty(), "the fuzz workspace defines no target");
    for target in targets {
        let corpus = root.join(FUZZ).join("corpus").join(&target);
        ensure!(
            !fs::entries(&corpus)?.is_empty(),
            "the fuzz target {target} has no committed corpus under {}",
            portable(&corpus)
        );
        let product = Target {
            directory: FUZZ,
            name: &target,
        };
        tool::succeed(
            &mut fuzz_run(root, product, None, &["-runs=0"]),
            "cargo fuzz run",
        )?;
    }
    let fixture = Target {
        directory: FUZZ_FIXTURE,
        name: FUZZ_FIXTURE_TARGET,
    };
    refused(
        &mut fuzz_run(root, fixture, None, &["-runs=0"]),
        "the fuzz fixture whose corpus panics",
        FUZZ_FIXTURE_PANIC,
    )
}

/// The counts a mutation campaign reports, and the mutants behind the counts that matter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Outcomes {
    pub total_mutants: u64,
    pub missed: u64,
    pub caught: u64,
    pub timeout: u64,
    pub unviable: u64,
    /// Every run of the campaign, including the baseline, under the key cargo-mutants writes.
    #[serde(default, rename = "outcomes")]
    pub runs: Vec<Run>,
}

/// One run of the campaign and how it ended.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Run {
    pub scenario: Scenario,
    pub summary: String,
}

/// What a campaign ran: the unmutated baseline or one mutant.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub enum Scenario {
    Baseline,
    Mutant(Mutant),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Mutant {
    pub name: String,
}

const TIMEOUT_SUMMARY: &str = "Timeout";
const MISSED_SUMMARY: &str = "MissedMutant";

impl Outcomes {
    /// A campaign that found nothing to mutate, which writes no outcomes.
    pub const NONE: Self = Self {
        total_mutants: 0,
        missed: 0,
        caught: 0,
        timeout: 0,
        unviable: 0,
        runs: Vec::new(),
    };

    /// The names of the mutants whose outcome is `summary`.
    #[must_use]
    pub fn named(&self, summary: &str) -> Vec<String> {
        self.runs
            .iter()
            .filter(|run| run.summary == summary)
            .filter_map(|run| match &run.scenario {
                Scenario::Mutant(mutant) => Some(mutant.name.clone()),
                Scenario::Baseline => None,
            })
            .collect()
    }
}

fn campaign(
    root: &Path,
    manifest: Option<&Path>,
    package: Option<&str>,
    output: &Path,
) -> Result<Outcomes> {
    fs::remove_dir_all(output)?;
    fs::create_dir_all(output)?;
    let mut command = Tool::Cargo.command(root);
    command.env("RUST_TEST_THREADS", MUTANT_TEST_THREADS);
    command.args(["mutants", "--no-shuffle", "--jobs", MUTANT_JOBS]);
    if let Some(manifest) = manifest {
        command.arg("--manifest-path").arg(manifest);
    }
    if let Some(package) = package {
        // The crate's own tests decide every outcome, so a mutant survives whatever another crate happens to exercise.
        command.args(["--package", package, "--test-package", package]);
    }
    command.arg("--output").arg(output);
    tool::status(&mut command, "cargo mutants")?;
    let path = output.join("mutants.out/outcomes.json");
    if !fs::is_file(&path)? {
        return Ok(Outcomes::NONE);
    }
    let bytes = fs::read(&path)?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}

/// Whether a count stays within its ceiling.
#[must_use]
pub const fn within(count: u64, ceiling: u64) -> bool {
    count <= ceiling
}

/// What a campaign over `package` leaves to fix, with its counts when anything does.
fn package_findings(package: &str, outcomes: &Outcomes) -> Vec<String> {
    let mut findings = Vec::new();
    if !within(outcomes.missed, MISSED_CEILING) {
        findings.extend(
            outcomes.named(MISSED_SUMMARY).iter().map(|mutant| {
                format!("{package}: `{mutant}` survived; add the test that detects it")
            }),
        );
    }
    if !within(outcomes.timeout, TIMEOUT_CEILING) {
        findings.extend(outcomes.named(TIMEOUT_SUMMARY).iter().map(|mutant| {
            format!(
                "{package}: `{mutant}` ran past the timeout; make the loop it changes end by construction"
            )
        }));
    }
    if !findings.is_empty() {
        findings.push(format!(
            "{package}: {} missed, {} timed out, {} of {} caught",
            outcomes.missed, outcomes.timeout, outcomes.caught, outcomes.total_mutants
        ));
    }
    findings
}

fn verify_mutants(root: &Path) -> Result<()> {
    tool::require_version(
        Tool::Cargo.command(root).arg("mutants"),
        "cargo-mutants",
        MUTANTS_VERSION,
    )?;
    let mut findings = Vec::new();
    for package in MUTANT_PACKAGES {
        let outcomes = campaign(
            root,
            None,
            Some(package),
            &root.join(MUTANTS_OUTPUT).join(package),
        )?;
        ensure!(
            outcomes.total_mutants > 0,
            "the mutation campaign over {package} found nothing to mutate"
        );
        findings.extend(package_findings(package, &outcomes));
    }
    ensure!(
        findings.is_empty(),
        "the mutation campaign is not clean; see {MUTANTS_OUTPUT}:\n{}",
        findings.join("\n")
    );
    let fixture = campaign(
        root,
        Some(&root.join(MUTANTS_FIXTURE).join("Cargo.toml")),
        None,
        &root.join(FIXTURE_OUTPUT).join("mutants"),
    )?;
    ensure!(
        fixture.missed >= 1,
        "the mutation fixture whose test asserts nothing caught every mutant; the gate cannot show it fails"
    );
    Ok(())
}

/// The digest of every input a proof result depends on: the sources of the verified crates, the fuzz targets, and the manifest.
fn digest(root: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut paths = Vec::new();
    for directory in ["crates", FUZZ] {
        paths.extend(fs::files_with_suffix(&root.join(directory), ".rs")?);
    }
    paths.push(root.join(obligations::MANIFEST));
    paths.push(root.join("Cargo.lock"));
    for path in paths {
        if fs::is_file(&path)? {
            let relative = path
                .strip_prefix(root)
                .with_context(|| format!("{} lies in the checkout", path.display()))?;
            hasher.update(portable(relative).as_bytes());
            hasher.update(fs::read(&path)?);
        }
    }
    let mut text = String::new();
    for byte in hasher.finalize() {
        write!(text, "{byte:02x}").context("format the digest")?;
    }
    Ok(text)
}

fn receipt(root: &Path, step: Step) -> Result<()> {
    let version = match step {
        Step::Kani => KANI_VERSION,
        Step::Quint => crate::quint::QUINT_VERSION,
        Step::Proverif => crate::proverif::PROVERIF_VERSION,
        Step::Miri => tool::NIGHTLY,
        Step::Fuzz => FUZZ_VERSION,
        Step::Mutants => MUTANTS_VERSION,
    };
    let receipt = serde_json::json!({
        "step": step.name(),
        "version": version,
        "inputs": digest(root)?,
        "result": "pass",
    });
    let path = root.join(RECEIPTS).join(format!("{}.json", step.name()));
    fs::write(&path, serde_json::to_string_pretty(&receipt)?.as_bytes())
}

fn verify(root: &Path, step: Step) -> Result<()> {
    match step {
        Step::Kani => verify_kani(root)?,
        Step::Quint => crate::quint::verify(root)?,
        Step::Proverif => crate::proverif::verify(root)?,
        Step::Miri => verify_miri(root)?,
        Step::Fuzz => verify_fuzz(root)?,
        Step::Mutants => verify_mutants(root)?,
    }
    receipt(root, step)
}

/// Runs the selected verifiers and reports every failure together.
///
/// # Errors
///
/// Returns an error naming each failed verifier.
pub fn run(root: &Path, options: Options) -> Result<()> {
    let mut report = Report::default();
    for step in options.selected() {
        report.stage(step.name(), || verify(root, step));
    }
    report.finish("proofs")
}

/// Fuzzes one product target beyond its corpus for `runs` runs with a fixed seed.
///
/// # Errors
///
/// Returns an error when the target is unknown, the count is not a number, or the run finds a crash.
pub fn fuzz(root: &Path, target: &str, runs: &str) -> Result<()> {
    let runs: u64 = runs.parse().context("the run count is a whole number")?;
    ensure!(
        fuzz_targets(root, FUZZ)?
            .iter()
            .any(|known| known == target),
        "{target} is not a target of the fuzz workspace"
    );
    let scratch = root.join(FUZZ_SCRATCH).join(target);
    fs::create_dir_all(&scratch)?;
    let product = Target {
        directory: FUZZ,
        name: target,
    };
    tool::succeed(
        &mut fuzz_run(
            root,
            product,
            Some(&scratch),
            &[&format!("-runs={runs}"), "-seed=1"],
        ),
        "cargo fuzz run",
    )
}

/// Where a scratch path for proof outputs lives, for tests and callers that keep diagnostics.
#[must_use]
pub fn scratch(root: &Path) -> PathBuf {
    root.join(FIXTURE_OUTPUT)
}

#[cfg(test)]
mod tests {
    use super::{
        Harness, Harnesses, KaniReport, MISSED_SUMMARY, Manifest, Options, Outcomes, Step,
        TIMEOUT_SUMMARY, check_negative, check_positive, inventory_findings, package_findings,
        qualify,
    };

    #[test]
    fn the_outcomes_name_the_mutants_behind_each_count() {
        let outcomes: Outcomes = serde_json::from_str(
            r#"{"total_mutants":3,"missed":1,"caught":1,"timeout":1,"unviable":0,"outcomes":[
                {"scenario":"Baseline","summary":"Success"},
                {"scenario":{"Mutant":{"name":"a.rs:1:1: replace == with != in admit","file":"a.rs"}},"summary":"MissedMutant"},
                {"scenario":{"Mutant":{"name":"a.rs:2:1: replace && with || in walk","file":"a.rs"}},"summary":"Timeout"},
                {"scenario":{"Mutant":{"name":"a.rs:3:1: delete ! in admit","file":"a.rs"}},"summary":"CaughtMutant"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            outcomes.named(MISSED_SUMMARY),
            vec!["a.rs:1:1: replace == with != in admit".to_owned()]
        );
        assert_eq!(
            outcomes.named(TIMEOUT_SUMMARY),
            vec!["a.rs:2:1: replace && with || in walk".to_owned()]
        );
        assert_eq!(Outcomes::NONE.named(MISSED_SUMMARY), Vec::<String>::new());
        let findings = package_findings("youarel-core", &outcomes).join("\n");
        assert!(findings.contains("survived"), "{findings}");
        assert!(findings.contains("ran past the timeout"), "{findings}");
        assert!(
            findings.contains("1 missed, 1 timed out, 1 of 3 caught"),
            "{findings}"
        );
        assert_eq!(
            package_findings("youarel-core", &Outcomes::NONE),
            Vec::<String>::new()
        );
    }

    fn harness(name: &str) -> Harness {
        Harness {
            name: name.to_owned(),
            covers: 1,
            contract: "fixture".to_owned(),
            input_domain: "fixture".to_owned(),
            bound: "fixture".to_owned(),
        }
    }

    fn positive() -> KaniReport {
        serde_json::from_str(r#"{"metadata":{"kani_version":"0.68.0"},"property_details":[{"harness_id":"contract","property_details":{"failed":0,"undetermined":0,"solver_error":0,"satisfied":1,"unsatisfiable":0}}],"verification_results":{"summary":{"total_harnesses":1,"executed":1,"status":"completed","successful":1,"failed":0},"results":[{"harness_id":"contract","status":"Success","checks":[{"status":"Success","description":"contract assertion","category":"assertion"}]}]}}"#).unwrap()
    }

    fn negative() -> KaniReport {
        serde_json::from_str(r#"{"metadata":{"kani_version":"0.68.0"},"property_details":[{"harness_id":"rejects_x","property_details":{"failed":1,"undetermined":0,"solver_error":0,"satisfied":0,"unsatisfiable":0}}],"verification_results":{"summary":{"total_harnesses":1,"executed":1,"status":"completed","successful":0,"failed":1},"results":[{"harness_id":"rejects_x","status":"Failure","checks":[{"status":"Failure","description":"NEGATIVE_X_CONTROL","category":"assertion"}]}]}}"#).unwrap()
    }

    const fn options(ci: bool, only: Option<Step>, without: Option<Step>) -> Options {
        Options {
            ci,
            only,
            without,
            inside: false,
        }
    }

    #[test]
    fn options_read_ci_only_and_without() {
        assert_eq!(Options::from_words(&[]), Some(options(false, None, None)));
        assert_eq!(
            Options::from_words(&["--ci", "--only", "miri"]),
            Some(options(true, Some(Step::Miri), None))
        );
        assert_eq!(
            Options::from_words(&["--without", "quint"]),
            Some(options(false, None, Some(Step::Quint)))
        );
        assert_eq!(Options::from_words(&["--only"]), None);
        assert_eq!(Options::from_words(&["--only", "lean"]), None);
        assert_eq!(Options::from_words(&["--verbose"]), None);
        assert_eq!(
            options(true, None, None).selected(),
            [
                Step::Kani,
                Step::Quint,
                Step::Proverif,
                Step::Miri,
                Step::Fuzz
            ]
        );
        assert_eq!(options(false, None, None).selected(), Step::ALL);
        assert_eq!(
            options(true, Some(Step::Mutants), None).selected(),
            [Step::Mutants]
        );
        assert_eq!(
            options(false, None, Some(Step::Quint)).selected(),
            [
                Step::Kani,
                Step::Proverif,
                Step::Miri,
                Step::Fuzz,
                Step::Mutants
            ]
        );
        let inside = Options::from_words(&["--inside"]).unwrap();
        assert_eq!(
            inside.selected(),
            [Step::Kani, Step::Miri, Step::Fuzz, Step::Mutants]
        );
        assert_eq!(
            options(false, Some(Step::Quint), Some(Step::Quint)).selected(),
            []
        );
    }

    #[test]
    fn a_positive_report_needs_the_exact_harnesses_and_covers() {
        let contract = harness("contract");
        check_positive(&positive(), &[&contract]).unwrap();
        check_positive(&positive(), &[]).unwrap_err();
        let mut uncovered = positive();
        uncovered.property_details[0].property_details.satisfied = 0;
        check_positive(&uncovered, &[&contract]).unwrap_err();
        let mut stale = positive();
        stale.verification_results.results[0].harness_id = "stale".to_owned();
        check_positive(&stale, &[&contract]).unwrap_err();
        let mut skipped = positive();
        skipped.verification_results.summary.executed = 0;
        check_positive(&skipped, &[&contract]).unwrap_err();
        let mut cover_only = positive();
        cover_only.verification_results.results[0].checks[0].category = "cover".to_owned();
        check_positive(&cover_only, &[&contract]).unwrap_err();
        let mut other_version = positive();
        other_version.metadata.kani_version = "0.67.0".to_owned();
        check_positive(&other_version, &[&contract]).unwrap_err();
    }

    #[test]
    fn a_negative_report_needs_the_named_assertion_and_no_tool_error() {
        check_negative(&negative(), "rejects_x", "NEGATIVE_X_CONTROL").unwrap();
        check_negative(&negative(), "rejects_x", "NEGATIVE_OTHER").unwrap_err();
        check_negative(&negative(), "rejects_y", "NEGATIVE_X_CONTROL").unwrap_err();
        let mut solver = negative();
        solver.property_details[0].property_details.solver_error = 1;
        check_negative(&solver, "rejects_x", "NEGATIVE_X_CONTROL").unwrap_err();
        check_negative(&positive(), "contract", "anything").unwrap_err();
    }

    fn discovered() -> Vec<(String, Harnesses)> {
        vec![(
            "crates/youarel-core/src/link_rules.rs".to_owned(),
            Harnesses {
                proofs: vec!["admits".to_owned()],
                probes: vec!["rejects_x".to_owned()],
            },
        )]
    }

    #[test]
    fn the_inventory_must_match_the_rule_modules() {
        let discovered = discovered();
        let manifest: Manifest = toml::from_str(
            r#"
[[obligation]]
verifier = "kani"
source = "crates/youarel-core/src/link_rules.rs"
harnesses = [{ name = "admits", covers = 1, contract = "c", input_domain = "i", bound = "b" }]
probes = [{ harness = "rejects_x", assertion = "NEGATIVE_LINK_CONTROL" }]
"#,
        )
        .unwrap();
        let sources = |_: &str| Ok("assert!(x, \"NEGATIVE_LINK_CONTROL\")".to_owned());
        assert_eq!(
            inventory_findings(&discovered, &manifest, &sources),
            Vec::<String>::new()
        );
        let missing = |_: &str| Ok(String::new());
        let unasserted = inventory_findings(&discovered, &manifest, &missing);
        assert!(
            unasserted
                .iter()
                .any(|finding| finding.contains("never asserts")),
            "{unasserted:?}"
        );
        let unreadable = |_: &str| Err(anyhow::anyhow!("gone"));
        let lost = inventory_findings(&discovered, &manifest, &unreadable);
        assert!(
            lost.iter()
                .any(|finding| finding.contains("cannot be read")),
            "{lost:?}"
        );
        let undeclared = inventory_findings(&discovered, &Manifest::default(), &sources);
        assert!(
            undeclared
                .iter()
                .any(|finding| finding.contains("declares no Kani obligation"))
        );
        let stale = inventory_findings(&[], &manifest, &sources);
        assert!(
            stale
                .iter()
                .any(|finding| finding.contains("not a rule module"))
        );
        let renamed = vec![(
            discovered[0].0.clone(),
            Harnesses {
                proofs: vec!["other".to_owned()],
                probes: vec!["rejects_y".to_owned()],
            },
        )];
        let drifted = inventory_findings(&renamed, &manifest, &sources).join("\n");
        assert!(drifted.contains("defines the proofs other"), "{drifted}");
        assert!(
            drifted.contains("defines the probes rejects_y"),
            "{drifted}"
        );
    }

    #[test]
    fn a_harness_is_qualified_by_its_module() {
        assert_eq!(
            qualify("crates/youarel-core/src/link_rules.rs", "proofs::admits"),
            "link_rules::proofs::admits"
        );
    }
}
