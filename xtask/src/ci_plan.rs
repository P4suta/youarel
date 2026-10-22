//! Source-bound fast checks and the full inventory of a release candidate.

use crate::{fs, proofs, tool};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;
use youarel_core::verification_rules::{self, Confidence, Job, Selection, Stage};

const SCHEMA: u8 = 1;

/// The workflow's stable rendering of a verification job.
#[must_use]
pub const fn job_name(job: Job) -> &'static str {
    match job {
        Job::Check => "check",
        Job::Kani => "kani",
        Job::Quint => "quint",
        Job::Proverif => "proverif",
        Job::Miri => "miri",
        Job::Fuzz => "fuzz",
        Job::Mutants => "mutants",
        Job::App => "app",
        Job::Macos => "macos",
        Job::Windows => "windows",
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub requested_base: String,
    pub base: Option<String>,
    pub merge_base: Option<String>,
    pub head: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    Affected,
    Manual,
    MissingBase,
    EmptyRange,
    UnknownInput,
    DeletedInput,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema: u8,
    pub campaign: bool,
    pub source: Source,
    pub reason: Reason,
    pub changed: Vec<String>,
    pub proofs: proofs::Scope,
    pub app: bool,
    pub platforms: bool,
}

impl Plan {
    #[must_use]
    pub fn jobs(&self) -> Selection {
        let mut affected = Selection::default();
        for (job, present) in [
            (Job::Kani, !self.proofs.kani.is_empty()),
            (Job::Quint, !self.proofs.quint.is_empty()),
            (Job::Proverif, !self.proofs.proverif.is_empty()),
            (Job::Miri, !self.proofs.miri.is_empty()),
            (Job::Fuzz, !self.proofs.fuzz.is_empty()),
            (Job::Mutants, !self.proofs.mutants.is_empty()),
            (Job::App, self.app),
            (Job::Macos, self.platforms),
            (Job::Windows, self.platforms),
        ] {
            if present {
                affected = affected.with(job);
            }
        }
        verification_rules::select(
            affected,
            if self.reason == Reason::Affected {
                Confidence::Scoped
            } else {
                Confidence::Conservative
            },
            if self.campaign {
                Stage::Candidate
            } else {
                Stage::Development
            },
        )
    }
}

fn git(root: &Path, words: &[&str]) -> Result<String> {
    tool::capture(tool::repository_git(root).args(words), "git source range")
}

fn commit(root: &Path, reference: &str) -> Result<Option<String>> {
    if reference.is_empty() {
        return Ok(None);
    }
    let result = tool::output(
        tool::repository_git(root).args([
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{reference}^{{commit}}"),
        ]),
        "git resolve commit",
    )?;
    if !result.status.success() {
        return Ok(None);
    }
    let object = String::from_utf8(result.stdout)?.trim().to_owned();
    ensure!(
        !object.is_empty() && object.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Git returned an invalid commit identity"
    );
    Ok(Some(object))
}

fn source(root: &Path, requested_base: &str) -> Result<(Source, Vec<String>)> {
    let head = commit(root, "HEAD")?.context("the checkout has no HEAD commit")?;
    let base = commit(root, requested_base)?;
    let merge_base = match base.as_deref() {
        Some(base) => {
            let result = tool::output(
                tool::repository_git(root).args(["merge-base", base, &head]),
                "git merge-base",
            )?;
            if result.status.success() {
                Some(String::from_utf8(result.stdout)?.trim().to_owned())
            } else {
                None
            }
        }
        None => None,
    };
    let changed = if let Some(ancestor) = &merge_base {
        let paths = git(
            root,
            &[
                "diff",
                "--no-renames",
                "--name-only",
                "-z",
                ancestor,
                &head,
                "--",
            ],
        )?;
        paths.split_terminator('\0').map(str::to_owned).collect()
    } else {
        Vec::new()
    };
    Ok((
        Source {
            requested_base: requested_base.to_owned(),
            base,
            merge_base,
            head,
        },
        changed,
    ))
}

fn prose(path: &str) -> bool {
    matches!(
        path,
        "README.md" | "LICENSE" | "LICENSE-APACHE" | "LICENSE-MIT"
    ) || path.starts_with("docs/adr/")
}

fn classify(root: &Path, paths: &[String]) -> Result<Reason> {
    if paths.is_empty() {
        return Ok(Reason::EmptyRange);
    }
    for path in paths {
        if path.starts_with('/') || path.split('/').any(|part| matches!(part, "" | "." | "..")) {
            return Ok(Reason::UnknownInput);
        }
        if !fs::is_file(&root.join(path))? {
            return Ok(Reason::DeletedInput);
        }
        if prose(path)
            || path.starts_with("app/")
            || path.starts_with("crates/")
            || path.starts_with("server/src/")
        {
            continue;
        }
        return Ok(Reason::UnknownInput);
    }
    Ok(Reason::Affected)
}

/// Exposes required work without executing it; candidate mode keeps the full inventory.
///
/// # Errors
///
/// Returns an error for an absent inventory or unreadable source range.
pub fn build(root: &Path, base: &str, full: bool) -> Result<Plan> {
    let proofs = proofs::Scope::full(root)?;
    let (source, changed) = source(root, base)?;
    let reason = if full {
        Reason::Manual
    } else if source.merge_base.is_none() {
        Reason::MissingBase
    } else {
        classify(root, &changed)?
    };
    let app = reason != Reason::Affected || changed.iter().any(|path| !prose(path));
    Ok(Plan {
        schema: SCHEMA,
        campaign: full,
        source,
        reason,
        changed,
        proofs,
        app,
        platforms: full,
    })
}

/// Refuses a supplied plan unless the clean source and current event reproduce it.
///
/// # Errors
///
/// Returns an error for a changed source, event, inventory, or selection.
pub fn validate(root: &Path, base: &str, full: bool, supplied: &str) -> Result<Plan> {
    let plan: Plan = serde_json::from_str(supplied).context("parse the selected CI plan")?;
    ensure!(
        git(root, &["status", "--porcelain", "--untracked-files=normal"])?
            .trim()
            .is_empty(),
        "CI evidence requires a clean checkout"
    );
    let expected = build(root, base, full)?;
    ensure!(
        plan == expected,
        "the selected plan does not match the current source and event"
    );
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obligations;

    fn snapshot(root: &Path) -> String {
        git(root, &["add", "--all"]).unwrap();
        let tree = git(root, &["write-tree"]).unwrap();
        let parent = commit(root, "HEAD").unwrap();
        let mut command = tool::repository_git(root);
        command.args([
            "-c",
            "commit.gpgsign=false",
            "commit-tree",
            tree.trim(),
            "-m",
            "fixture",
        ]);
        if let Some(parent) = parent {
            command.args(["-p", &parent]);
        }
        for variable in ["GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME"] {
            command.env(variable, "Verification fixture");
        }
        for variable in ["GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL"] {
            command.env(variable, "fixture@example.invalid");
        }
        let head = tool::capture(&mut command, "fixture commit").unwrap();
        git(root, &["update-ref", "HEAD", head.trim()]).unwrap();
        head.trim().to_owned()
    }

    fn repository() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "--quiet"]).unwrap();
        fs::write(&root.join("app/ui/main.ts"), b"const first: number = 1;\n").unwrap();
        fs::write(&root.join("README.md"), b"Fixture.\n").unwrap();
        fs::write(&root.join("fuzz/fuzz_targets/parse.rs"), b"fn main() {}\n").unwrap();
        fs::write(&root.join(obligations::MANIFEST), b"[[obligation]]\nverifier = 'kani'\nsource = 'crates/youarel-core/src/rules.rs'\nharnesses = []\nprobes = []\n[[obligation]]\nverifier = 'quint'\nspec = 'specs/test.qnt'\nmain = 'test'\nprobe = 'probe'\ninvariant = 'safe'\nwitnesses = []\nseed = '0x2a'\nsamples = 1\nsteps = 1\ndistinct_states = 1\ntraces = 1\n[[obligation]]\nverifier = 'proverif'\nmodel = 'specs/test.pv'\nlibrary = 'specs/primitives.pvl'\nqueries = 1\nprobe = 'specs/probe.pv'\n").unwrap();
        snapshot(root);
        directory
    }

    #[test]
    fn development_keeps_fast_checks_and_candidates_keep_every_contract() {
        let repository = repository();
        let root = repository.path();
        let base = commit(root, "HEAD").unwrap().unwrap();
        fs::write(&root.join("app/ui/main.ts"), b"const second: number = 2;\n").unwrap();
        snapshot(root);
        let development = build(root, &base, false).unwrap();
        assert!(development.jobs().includes(Job::Check));
        assert!(development.jobs().includes(Job::App));
        assert!(!development.jobs().includes(Job::Mutants));
        assert!(!development.jobs().includes(Job::Proverif));
        let candidate = build(root, &base, true).unwrap();
        assert_eq!(candidate.jobs(), Selection::all());
        assert_eq!(
            candidate.proofs.mutants.len(),
            proofs::MUTANT_PACKAGES.len()
        );
    }

    #[test]
    fn git_ranges_bind_the_plan_and_reject_forgery_and_dirty_inputs() {
        let repository = repository();
        let root = repository.path();
        let base = commit(root, "HEAD").unwrap().unwrap();
        fs::write(&root.join("app/ui/main.ts"), b"const second: number = 2;\n").unwrap();
        let head = snapshot(root);
        let plan = build(root, &base, false).unwrap();
        assert_eq!(plan.reason, Reason::Affected);
        assert_eq!(plan.source.head, head);
        assert_eq!(plan.changed, ["app/ui/main.ts"]);
        let text = serde_json::to_string(&plan).unwrap();
        assert_eq!(validate(root, &base, false, &text).unwrap(), plan);
        let mut forged = plan;
        forged.app = false;
        validate(root, &base, false, &serde_json::to_string(&forged).unwrap()).unwrap_err();
        validate(root, &base, true, &text).unwrap_err();
        validate(root, &base, false, "{}").unwrap_err();
        validate(
            root,
            &base,
            false,
            &text.replacen("\"schema\":1", "\"schema\":1,\"extra\":true", 1),
        )
        .unwrap_err();
        fs::write(&root.join("app/ui/main.ts"), b"const dirty: number = 3;\n").unwrap();
        validate(root, &base, false, &text).unwrap_err();
        snapshot(root);
        validate(root, &base, false, &text).unwrap_err();
    }

    #[test]
    fn renames_keep_both_paths_and_missing_bases_preserve_the_inventory() {
        let repository = repository();
        let root = repository.path();
        let base = commit(root, "HEAD").unwrap().unwrap();
        git(root, &["mv", "app/ui/main.ts", "app/ui/renamed.ts"]).unwrap();
        let head = snapshot(root);
        let renamed = build(root, &base, false).unwrap();
        assert_eq!(renamed.changed, ["app/ui/main.ts", "app/ui/renamed.ts"]);
        assert_eq!(renamed.reason, Reason::DeletedInput);
        for other_base in ["missing-base", head.as_str(), ""] {
            let plan = build(root, other_base, false).unwrap();
            assert!(plan.jobs().includes(Job::Check));
            assert!(plan.jobs().includes(Job::App));
            assert!(!plan.jobs().includes(Job::Mutants));
            assert_eq!(plan.proofs.mutants.len(), proofs::MUTANT_PACKAGES.len());
        }
    }

    #[test]
    fn prose_only_changes_still_require_the_fast_gate() {
        let repository = repository();
        let root = repository.path();
        let base = commit(root, "HEAD").unwrap().unwrap();
        fs::write(&root.join("README.md"), b"Updated fixture.\n").unwrap();
        snapshot(root);
        let plan = build(root, &base, false).unwrap();
        assert!(plan.jobs().includes(Job::Check));
        assert!(!plan.jobs().includes(Job::App));
    }
}
