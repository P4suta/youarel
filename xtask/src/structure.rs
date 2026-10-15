//! The structural gates: the pure core keeps its boundary, and every crate keeps its place.

use crate::{fs, portable};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
use syn::visit::Visit;

/// The crate that holds every pure decision.
pub const CORE: &str = "crates/youarel-core";

/// The assurance manifest, whose protocol owners may define a step.
const ASSURE: &str = "assure.toml";
/// The shape of an owner that holds a protocol step.
const PROTOCOL: &str = "protocol";

/// The lints the core's root denies in its own `#![deny(...)]`, beyond the workspace policy.
pub const DENIED: [&str; 10] = [
    "clippy::unwrap_used",
    "clippy::expect_used",
    "clippy::panic",
    "clippy::todo",
    "clippy::unimplemented",
    "clippy::unreachable",
    "clippy::indexing_slicing",
    "clippy::string_slice",
    "clippy::exit",
    "clippy::mem_forget",
];

/// The workspace members and the members each may depend on.
pub const EDGES: [(&str, &[&str]); 12] = [
    ("youarel-core", &[]),
    ("youarel-crypto", &[]),
    ("youarel-html", &[]),
    ("youarel-icon", &["youarel-core"]),
    ("youarel-link", &["youarel-core"]),
    ("youarel-protocol", &["youarel-core", "youarel-crypto"]),
    (
        "youarel-relay",
        &["youarel-core", "youarel-crypto", "youarel-protocol"],
    ),
    ("youarel-release", &["youarel-core"]),
    ("youarel-store", &["youarel-core", "youarel-link"]),
    (
        "youarel-sync",
        &[
            "youarel-core",
            "youarel-crypto",
            "youarel-protocol",
            "youarel-store",
        ],
    ),
    (
        "youarel-cli",
        &[
            "youarel-core",
            "youarel-crypto",
            "youarel-html",
            "youarel-icon",
            "youarel-link",
            "youarel-protocol",
            "youarel-relay",
            "youarel-release",
            "youarel-store",
            "youarel-sync",
        ],
    ),
    ("xtask", &["youarel-core", "youarel-release"]),
];

const DEPENDENCY_TABLES: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

/// The harnesses a rule module holds.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Harnesses {
    /// The proofs, which must pass.
    pub proofs: Vec<String>,
    /// The rejecting probes, named `rejects_*`, which must fail on their assertion.
    pub probes: Vec<String>,
}

#[derive(Default)]
struct StdUse {
    lines: Vec<usize>,
}

impl Visit<'_> for StdUse {
    fn visit_item_extern_crate(&mut self, item: &syn::ItemExternCrate) {
        if item.ident == "std" {
            self.lines.push(item.ident.span().start().line);
        }
        syn::visit::visit_item_extern_crate(self, item);
    }

    fn visit_path(&mut self, path: &syn::Path) {
        if let Some(first) = path.segments.first()
            && first.ident == "std"
        {
            self.lines.push(first.ident.span().start().line);
        }
        syn::visit::visit_path(self, path);
    }
}

#[derive(Default)]
struct KaniHarnesses {
    found: Harnesses,
    /// The inline modules enclosing the current item, outermost first.
    modules: Vec<String>,
}

fn is_kani_proof(attribute: &syn::Attribute) -> bool {
    let segments: Vec<String> = attribute
        .path()
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    segments == ["kani", "proof"]
}

impl KaniHarnesses {
    fn qualified(&self, name: &str) -> String {
        let mut path = self.modules.clone();
        path.push(name.to_owned());
        path.join("::")
    }
}

impl Visit<'_> for KaniHarnesses {
    fn visit_item_mod(&mut self, item: &syn::ItemMod) {
        self.modules.push(item.ident.to_string());
        syn::visit::visit_item_mod(self, item);
        self.modules.pop();
    }

    fn visit_item_fn(&mut self, item: &syn::ItemFn) {
        let name = item.sig.ident.to_string();
        if item.attrs.iter().any(is_kani_proof) {
            let qualified = self.qualified(&name);
            if name.starts_with("rejects_") {
                self.found.probes.push(qualified);
            } else {
                self.found.proofs.push(qualified);
            }
        }
        syn::visit::visit_item_fn(self, item);
    }
}

/// The lines of `source` that name the standard library.
///
/// # Errors
///
/// Returns an error when the source fails to parse.
pub fn std_lines(source: &str) -> Result<Vec<usize>> {
    let file = syn::parse_file(source).context("parse the Rust source")?;
    let mut uses = StdUse::default();
    uses.visit_file(&file);
    Ok(uses.lines)
}

/// The Kani harnesses `source` defines, split into proofs and rejecting probes, each qualified by the inline modules that hold it.
///
/// # Errors
///
/// Returns an error when the source fails to parse.
pub fn harnesses(source: &str) -> Result<Harnesses> {
    let file = syn::parse_file(source).context("parse the Rust source")?;
    let mut visitor = KaniHarnesses::default();
    visitor.visit_file(&file);
    Ok(visitor.found)
}

/// Whether a qualified harness name is a rejecting probe.
#[must_use]
pub fn is_probe(qualified: &str) -> bool {
    qualified
        .rsplit("::")
        .next()
        .is_some_and(|name| name.starts_with("rejects_"))
}

/// The lints a crate root's `#![deny(...)]` attributes name from [`DENIED`].
#[must_use]
pub fn denied(root: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    for attribute in root.split("#![deny(").skip(1) {
        let list = attribute.split(")]").next().unwrap_or_default();
        for lint in list.split(',').map(str::trim) {
            if let Some(name) = DENIED.iter().find(|name| **name == lint) {
                found.push(*name);
            }
        }
    }
    found
}

/// The dependencies a manifest names in every dependency table, including the per-target ones.
///
/// # Errors
///
/// Returns an error when the manifest fails to parse as TOML.
pub fn dependencies(manifest: &str) -> Result<Vec<String>> {
    let manifest: toml::Table = toml::from_str(manifest).context("parse the manifest")?;
    let mut names = Vec::new();
    collect_dependencies(&manifest, &mut names);
    for target in manifest
        .get("target")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(toml::Table::values)
        .filter_map(toml::Value::as_table)
    {
        collect_dependencies(target, &mut names);
    }
    Ok(names)
}

fn collect_dependencies(table: &toml::Table, names: &mut Vec<String>) {
    for kind in DEPENDENCY_TABLES {
        if let Some(entries) = table.get(kind).and_then(toml::Value::as_table) {
            names.extend(entries.keys().cloned());
        }
    }
}

/// The name a crate manifest declares.
///
/// # Errors
///
/// Returns an error when the manifest fails to parse or names no package.
pub fn package_name(manifest: &str) -> Result<String> {
    let manifest: toml::Table = toml::from_str(manifest).context("parse the manifest")?;
    manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .context("the manifest names no package")
}

fn members(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for path in fs::entries(&root.join("crates"))? {
        if fs::is_directory(&path)? {
            found.push(path);
        }
    }
    found.push(root.join("xtask"));
    Ok(found)
}

fn core_findings(root: &Path, findings: &mut Vec<String>) -> Result<()> {
    let core = root.join(CORE);
    let manifest = fs::read_to_string(&core.join("Cargo.toml"))?;
    let names = dependencies(&manifest)?;
    if !names.is_empty() {
        findings.push(format!(
            "{CORE} depends on {}; the core reaches no crate at all, so move the need behind a value an adapter passes in",
            names.join(", ")
        ));
    }
    let library = fs::read_to_string(&core.join("src/lib.rs"))?;
    if !library.lines().any(|line| line.trim() == "#![no_std]") {
        findings.push(format!("{CORE} must remain unconditionally no_std"));
    }
    let found = denied(&library);
    for lint in DENIED {
        if !found.contains(&lint) {
            findings.push(format!("{CORE} no longer denies {lint} in its crate root"));
        }
    }
    for path in fs::files_with_suffix(&core.join("src"), ".rs")? {
        let source = fs::read_to_string(&path)?;
        for line in std_lines(&source)? {
            findings.push(format!(
                "{}:{line}: the core cannot name std",
                portable(&path)
            ));
        }
    }
    Ok(())
}

fn rule_findings(root: &Path, findings: &mut Vec<String>) -> Result<()> {
    let core_source = root.join(CORE).join("src");
    for path in fs::files_with_suffix(&core_source, "_rules.rs")? {
        let found = harnesses(&fs::read_to_string(&path)?)?;
        if found.proofs.is_empty() {
            findings.push(format!(
                "{}: a rule module proves its decision with a `#[kani::proof]` harness in the same file",
                portable(&path)
            ));
        }
        if found.probes.is_empty() {
            findings.push(format!(
                "{}: a rule module shows its proof is not vacuous with a `rejects_*` harness that fails on a named assertion",
                portable(&path)
            ));
        }
    }
    for member in members(root)? {
        let source = member.join("src");
        if source.starts_with(&core_source) {
            continue;
        }
        for path in fs::files_with_suffix(&source, "_rules.rs")? {
            findings.push(format!(
                "{}: a rule module lives in {CORE}, where its Kani harnesses own it",
                portable(&path)
            ));
        }
    }
    Ok(())
}

/// The modules that may define a protocol step: those `assure.toml` names as protocol owners, each with a Quint model and a trace replay.
fn protocol_owners(root: &Path) -> Result<Vec<PathBuf>> {
    let manifest = root.join(ASSURE);
    if !fs::is_file(&manifest)? {
        return Ok(Vec::new());
    }
    let table: toml::Table = fs::read_to_string(&manifest)?
        .parse()
        .with_context(|| format!("parse {ASSURE}"))?;
    Ok(table
        .get("owner")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|owner| owner.get("shape").and_then(toml::Value::as_str) == Some(PROTOCOL))
        .filter_map(|owner| owner.get("path").and_then(toml::Value::as_str))
        .map(|path| root.join(path))
        .collect())
}

fn step_findings(root: &Path, findings: &mut Vec<String>) -> Result<()> {
    let owners = protocol_owners(root)?;
    for member in members(root)? {
        for path in fs::files_with_suffix(&member.join("src"), ".rs")? {
            if owners.contains(&path) {
                continue;
            }
            let source = fs::read_to_string(&path)?;
            for (index, line) in source.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("pub fn step(") || trimmed.starts_with("pub const fn step(")
                {
                    findings.push(format!(
                        "{}:{}: a protocol step lives only in a module {ASSURE} names as a protocol owner, whose Quint model and trace replay own it",
                        portable(&path),
                        index.saturating_add(1)
                    ));
                }
            }
        }
    }
    Ok(())
}

fn edge_findings(root: &Path, findings: &mut Vec<String>) -> Result<()> {
    let member_names: Vec<&str> = EDGES.iter().map(|(name, _)| *name).collect();
    for member in members(root)? {
        let manifest = fs::read_to_string(&member.join("Cargo.toml"))?;
        let name = package_name(&manifest)?;
        let Some((_, allowed)) = EDGES.iter().find(|(candidate, _)| *candidate == name) else {
            findings.push(format!(
                "{}: the crate {name} is not in the dependency map of xtask/src/structure.rs",
                portable(&member)
            ));
            continue;
        };
        for dependency in dependencies(&manifest)? {
            if member_names.contains(&dependency.as_str())
                && !allowed.contains(&dependency.as_str())
            {
                findings.push(format!(
                    "{name} depends on {dependency}, which the dependency direction forbids"
                ));
            }
        }
    }
    Ok(())
}

/// Every way the checkout breaks its structure.
///
/// # Errors
///
/// Returns an error when a manifest or source cannot be read or parsed.
pub fn findings(root: &Path) -> Result<Vec<String>> {
    let mut findings = Vec::new();
    core_findings(root, &mut findings)?;
    rule_findings(root, &mut findings)?;
    step_findings(root, &mut findings)?;
    edge_findings(root, &mut findings)?;
    Ok(findings)
}

/// Refuses a checkout that breaks its structure, naming each breach.
///
/// # Errors
///
/// Returns an error listing every finding, or when the checkout cannot be read.
pub fn check(root: &Path) -> Result<()> {
    let found = findings(root)?;
    ensure!(
        found.is_empty(),
        "the checkout breaks its structure:\n{}",
        found.join("\n")
    );
    eprintln!("structure: the core keeps its boundary and every crate keeps its place");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CORE, DENIED, denied, dependencies, findings, harnesses, std_lines};
    use crate::fs;
    use std::path::Path;

    fn core_root() -> String {
        format!("#![no_std]\n#![deny(\n    {}\n)]\n", DENIED.join(",\n    "))
    }

    fn write_crate(root: &Path, name: &str, dependencies: &str, lib: &str) {
        let directory = if name == "xtask" {
            root.join("xtask")
        } else {
            root.join("crates").join(name)
        };
        fs::write(
            &directory.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\n[dependencies]\n{dependencies}").as_bytes(),
        )
        .unwrap();
        fs::write(&directory.join("src/lib.rs"), lib.as_bytes()).unwrap();
    }

    fn clean_checkout(root: &Path) {
        write_crate(root, "youarel-core", "", &core_root());
        write_crate(
            root,
            "youarel-link",
            "youarel-core = { path = \"../youarel-core\" }\n",
            "",
        );
        write_crate(root, "xtask", "", "");
    }

    #[test]
    fn the_deny_list_is_read_lint_by_lint() {
        assert_eq!(denied(&core_root()), DENIED.to_vec());
        let fewer = core_root().replace("clippy::indexing_slicing,", "");
        assert_eq!(denied(&fewer).len(), DENIED.len() - 1);
        assert_eq!(denied("#![allow(clippy::panic)]"), Vec::<&str>::new());
    }

    #[test]
    fn every_dependency_table_counts() {
        assert_eq!(
            dependencies("[package]\nname = \"youarel-core\"\n").unwrap(),
            Vec::<String>::new()
        );
        let names = dependencies(
            "[dependencies]\nserde = \"1\"\n[dev-dependencies]\nx = \"1\"\n[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n",
        )
        .unwrap();
        assert_eq!(names, ["serde", "x", "libc"]);
    }

    #[test]
    fn std_paths_and_harnesses_are_found_by_syntax() {
        assert_eq!(
            std_lines("extern crate alloc;\nfn f() { std::fs::read(\"x\"); }").unwrap(),
            [2]
        );
        assert_eq!(std_lines("\nextern crate std;").unwrap(), [2]);
        std_lines("fn broken(").unwrap_err();
        let found = harnesses(
            "fn plain() {}\n#[cfg(kani)]\nmod proofs {\n#[kani::proof]\nfn admits() {}\n#[kani::proof]\nfn rejects_a_false_claim() {}\n#[test]\nfn other() {}\n}",
        )
        .unwrap();
        assert_eq!(found.proofs, ["proofs::admits"]);
        assert_eq!(found.probes, ["proofs::rejects_a_false_claim"]);
        assert!(super::is_probe("proofs::rejects_a_false_claim"));
        assert!(!super::is_probe("proofs::admits"));
    }

    #[test]
    fn a_clean_checkout_has_no_findings() {
        let root = tempfile::tempdir().unwrap();
        clean_checkout(root.path());
        assert_eq!(findings(root.path()).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn each_breach_is_named() {
        let root = tempfile::tempdir().unwrap();
        clean_checkout(root.path());
        let core = root.path().join(CORE);
        fs::write(
            &core.join("Cargo.toml"),
            b"[package]\nname = \"youarel-core\"\n[dev-dependencies]\nserde = \"1\"\n",
        )
        .unwrap();
        fs::write(
            &core.join("src/lib.rs"),
            b"#![deny(clippy::unwrap_used)]\nfn f() { std::process::exit(1) }\n",
        )
        .unwrap();
        fs::write(&core.join("src/link_rules.rs"), b"pub fn admit() {}\n").unwrap();
        fs::write(&root.path().join("crates/youarel-link/src/x_rules.rs"), b"").unwrap();
        fs::write(
            &root.path().join("crates/youarel-link/src/proto.rs"),
            b"pub fn step() {}\n",
        )
        .unwrap();
        fs::write(
            &root.path().join("xtask/Cargo.toml"),
            b"[package]\nname = \"xtask\"\n[dependencies]\nyouarel-link = { path = \"../crates/youarel-link\" }\n",
        )
        .unwrap();
        let text = findings(root.path()).unwrap().join("\n");
        assert!(text.contains("depends on serde"), "{text}");
        assert!(
            text.contains("must remain unconditionally no_std"),
            "{text}"
        );
        assert!(text.contains("no longer denies clippy::panic"), "{text}");
        assert!(
            text.contains("lib.rs:2: the core cannot name std"),
            "{text}"
        );
        assert!(
            text.contains("link_rules.rs: a rule module proves"),
            "{text}"
        );
        assert!(
            text.contains("link_rules.rs: a rule module shows"),
            "{text}"
        );
        assert!(
            text.contains("x_rules.rs: a rule module lives in"),
            "{text}"
        );
        assert!(
            text.contains("proto.rs:1: a protocol step lives only in"),
            "{text}"
        );
        assert!(text.contains("xtask depends on youarel-link"), "{text}");
    }
}
