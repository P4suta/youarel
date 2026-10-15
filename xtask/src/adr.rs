//! The decision record gate: every record is named by its decision and dated, and holds its sections.

use crate::{fs, portable};
use anyhow::{Result, ensure};
use std::path::Path;

/// Where the decision records live.
pub const DIRECTORY: &str = "docs/adr";

/// The closed status vocabulary of a record.
pub const STATUSES: [&str; 5] = [
    "proposed",
    "accepted",
    "amended",
    "superseded",
    "deprecated",
];

/// The sections every record holds, in order.
pub const SECTIONS: [&str; 3] = ["## Context", "## Decision", "## Consequences"];

const DATE_LENGTH: usize = 10;
const DATE_HYPHENS: [usize; 2] = [4, 7];

/// The front matter of a record: the `key: value` lines between the first two `---` lines.
#[must_use]
pub fn front_matter(text: &str) -> Vec<(&str, &str)> {
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return Vec::new();
    }
    lines
        .take_while(|line| *line != "---")
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect()
}

fn is_date(value: &str) -> bool {
    value.len() == DATE_LENGTH
        && value.char_indices().all(|(index, character)| {
            if DATE_HYPHENS.contains(&index) {
                character == '-'
            } else {
                character.is_ascii_digit()
            }
        })
}

fn is_decision_name(stem: &str) -> bool {
    let valid_characters = stem.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });
    let numbered = stem.split('-').next().is_some_and(|head| {
        !head.is_empty() && head.chars().all(|character| character.is_ascii_digit())
    });
    valid_characters && !numbered && !stem.is_empty()
}

/// Every way one record's text breaks the record format.
#[must_use]
pub fn text_findings(stem: &str, text: &str) -> Vec<String> {
    let mut findings = Vec::new();
    if !is_decision_name(stem) {
        findings.push(format!(
            "{stem}.md: name a record by its decision in lowercase words joined by hyphens, with no number"
        ));
    }
    let matter = front_matter(text);
    match matter.iter().find(|(key, _)| *key == "status") {
        Some((_, status)) if STATUSES.contains(status) => {}
        Some((_, status)) => findings.push(format!(
            "{stem}.md: the status `{status}` is not one of {}",
            STATUSES.join(", ")
        )),
        None => findings.push(format!("{stem}.md: the front matter names no `status`")),
    }
    match matter.iter().find(|(key, _)| *key == "date") {
        Some((_, date)) if is_date(date) => {}
        Some((_, date)) => findings.push(format!("{stem}.md: the date `{date}` is not YYYY-MM-DD")),
        None => findings.push(format!("{stem}.md: the front matter names no `date`")),
    }
    if !text.lines().any(|line| line.starts_with("# ")) {
        findings.push(format!("{stem}.md: the record has no title"));
    }
    for section in SECTIONS {
        if !text.lines().any(|line| line == section) {
            findings.push(format!("{stem}.md: the record has no `{section}` section"));
        }
    }
    findings
}

/// Every way the record collection breaks the format.
///
/// # Errors
///
/// Returns an error when a record cannot be read.
pub fn findings(root: &Path) -> Result<Vec<String>> {
    let mut findings = Vec::new();
    for path in fs::files_with_suffix(&root.join(DIRECTORY), ".md")? {
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        if stem == "README" {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        findings.extend(
            text_findings(stem, &text)
                .into_iter()
                .map(|finding| format!("{}: {finding}", portable(&path))),
        );
    }
    Ok(findings)
}

/// Refuses a record collection that breaks the format, naming each breach.
///
/// # Errors
///
/// Returns an error listing every finding, or when a record cannot be read.
pub fn check(root: &Path) -> Result<()> {
    let found = findings(root)?;
    ensure!(
        found.is_empty(),
        "the decision records break their format:\n{}",
        found.join("\n")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{front_matter, text_findings};

    const RECORD: &str = "---\nstatus: accepted\ndate: 2026-10-15\n---\n# Keep the core pure\n\n## Context\n\nx.\n\n## Decision\n\ny.\n\n## Consequences\n\nz.\n";

    #[test]
    fn a_complete_record_has_no_findings() {
        assert_eq!(
            front_matter(RECORD),
            [("status", "accepted"), ("date", "2026-10-15")]
        );
        assert_eq!(
            text_findings("keep-the-core-pure", RECORD),
            Vec::<String>::new()
        );
    }

    #[test]
    fn each_breach_is_named() {
        let numbered = text_findings("0001-numbered", "# Title\n\n## Context\n").join("\n");
        assert!(numbered.contains("no number"), "{numbered}");
        assert!(numbered.contains("names no `status`"), "{numbered}");
        assert!(numbered.contains("names no `date`"), "{numbered}");
        assert!(numbered.contains("no `## Decision` section"), "{numbered}");
        let malformed = text_findings("x", "---\nstatus: done\ndate: 2026-1-3\n---\n").join("\n");
        assert!(malformed.contains("`done` is not one of"), "{malformed}");
        assert!(
            malformed.contains("`2026-1-3` is not YYYY-MM-DD"),
            "{malformed}"
        );
        assert!(malformed.contains("has no title"), "{malformed}");
    }
}
