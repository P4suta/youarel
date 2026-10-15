//! The verification obligation manifest, which names every property, its verifier, and its negative control.

use crate::fs;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// Where the manifest lives.
pub const MANIFEST: &str = "verification/obligations.toml";

/// Every obligation of the repository.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub obligation: Vec<Obligation>,
}

/// One obligation and the verifier that discharges it.
#[derive(Debug, Deserialize)]
#[serde(tag = "verifier", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Obligation {
    /// Every harness in `source` must pass with its covers reached, and every probe must fail on its assertion.
    Kani {
        source: String,
        harnesses: Vec<Harness>,
        probes: Vec<Probe>,
    },
}

/// A Kani harness, with the contract it states and the input domain it admits.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Harness {
    pub name: String,
    /// How many `kani::cover!` checks the harness reaches, which rules out contradictory assumptions.
    pub covers: u64,
    pub contract: String,
    pub input_domain: String,
    pub bound: String,
}

/// A rejecting probe: a harness that asserts a false claim and must fail on the named assertion.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub harness: String,
    pub assertion: String,
}

/// The manifest of a checkout, or an empty one when the file is absent.
///
/// # Errors
///
/// Returns an error when the manifest is present but malformed.
pub fn load(root: &Path) -> Result<Manifest> {
    let path = root.join(MANIFEST);
    if !fs::is_file(&path)? {
        return Ok(Manifest::default());
    }
    let text = fs::read_to_string(&path)?;
    toml::from_str(&text).with_context(|| format!("parse {MANIFEST}"))
}

#[cfg(test)]
mod tests {
    use super::{Manifest, Obligation};

    #[test]
    fn a_kani_obligation_names_its_harnesses_and_probes() {
        let manifest: Manifest = toml::from_str(
            r#"
[[obligation]]
verifier = "kani"
source = "crates/youarel-core/src/link_rules.rs"
harnesses = [{ name = "admits", covers = 4, contract = "c", input_domain = "i", bound = "b" }]
probes = [{ harness = "rejects_x", assertion = "NEGATIVE_LINK_CONTROL" }]
"#,
        )
        .unwrap();
        let [
            Obligation::Kani {
                source,
                harnesses,
                probes,
            },
        ] = manifest.obligation.as_slice()
        else {
            panic!("one Kani obligation");
        };
        assert_eq!(source, "crates/youarel-core/src/link_rules.rs");
        assert_eq!(harnesses[0].covers, 4);
        assert_eq!(probes[0].assertion, "NEGATIVE_LINK_CONTROL");
        toml::from_str::<Manifest>("[[obligation]]\nverifier = \"lean\"\n").unwrap_err();
        assert!(
            toml::from_str::<Manifest>("")
                .unwrap()
                .obligation
                .is_empty()
        );
    }
}
