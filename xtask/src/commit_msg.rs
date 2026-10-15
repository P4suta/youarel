//! The commit message gate: a Conventional Commit type, and a fix that stages the test it needs.

use crate::fs;
use crate::tool::{Tool, capture};
use anyhow::{Result, bail, ensure};
use std::path::Path;

/// The commit types `committed.toml` allows.
pub const TYPES: [&str; 11] = [
    "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert",
];

/// The Conventional Commit type of a subject line, if it has one.
#[must_use]
pub fn kind(subject: &str) -> Option<&str> {
    let (head, _) = subject.split_once(':')?;
    let name = head.split(['(', '!']).next()?.trim();
    TYPES.contains(&name).then_some(name)
}

/// Whether the staged change carries a test: a file under a test directory or an added test, harness, or fuzz target.
#[must_use]
pub fn carries_test(paths: &[&str], diff: &str) -> bool {
    let test_path = paths.iter().any(|path| {
        path.starts_with("tests/")
            || path.contains("/tests/")
            || path.starts_with("fuzz/")
            || path.ends_with("_test.rs")
    });
    let added_test = diff.lines().any(|line| {
        line.starts_with('+')
            && (line.contains("#[test]")
                || line.contains("#[kani::proof]")
                || line.contains("fuzz_target!"))
    });
    test_path || added_test
}

fn subject(message: &str) -> Option<&str> {
    message
        .lines()
        .map(str::trim_end)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
}

/// Refuses a commit message without a known type, or a fix without a staged test.
///
/// # Errors
///
/// Returns an error naming the breach, or when the message or the staged change cannot be read.
pub fn check(root: &Path, message: &Path) -> Result<()> {
    let text = fs::read_to_string(message)?;
    let Some(subject) = subject(&text) else {
        bail!("the commit message is empty");
    };
    if subject.starts_with("Merge ") {
        return Ok(());
    }
    let Some(kind) = kind(subject) else {
        bail!(
            "`{subject}` names no Conventional Commit type; start with one of {}",
            TYPES.join(", ")
        );
    };
    if kind != "fix" {
        return Ok(());
    }
    let staged = capture(
        Tool::Git
            .command(root)
            .args(["diff", "--cached", "--name-only", "--diff-filter=ACMR"]),
        "git diff --cached --name-only",
    )?;
    let diff = capture(
        Tool::Git.command(root).args(["diff", "--cached", "-U0"]),
        "git diff --cached",
    )?;
    let paths: Vec<&str> = staged.lines().collect();
    ensure!(
        carries_test(&paths, &diff),
        "a fix lands with a test that fails without it; stage the test with the fix"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{carries_test, kind};

    #[test]
    fn the_type_is_read_before_scope_bang_and_colon() {
        assert_eq!(kind("feat(core)!: add a rule"), Some("feat"));
        assert_eq!(kind("fix: keep the primary error"), Some("fix"));
        assert_eq!(kind("wip: nothing"), None);
        assert_eq!(kind("no colon here"), None);
    }

    #[test]
    fn a_fix_needs_a_staged_test() {
        assert!(carries_test(&["crates/youarel-link/tests/parse.rs"], ""));
        assert!(carries_test(&["fuzz/fuzz_targets/link_parse.rs"], ""));
        assert!(carries_test(
            &["crates/youarel-core/src/link_rules.rs"],
            "+    #[test]\n"
        ));
        assert!(carries_test(&["x.rs"], "+    #[kani::proof]\n"));
        assert!(!carries_test(
            &["crates/youarel-core/src/link_rules.rs"],
            "-    #[test]\n+ fn x() {}\n"
        ));
    }
}
