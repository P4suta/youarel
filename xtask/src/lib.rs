//! The repository's development tasks.
//!
//! Each task reads the checkout as values, decides, and runs the pinned tools through [`tool`].
//! Files are read and written only through [`fs`], and children start only through [`tool`].

pub mod adr;
pub mod check;
pub mod ci;
pub mod commit_msg;
pub mod container;
pub mod fs;
pub mod obligations;
pub mod proofs;
pub mod quint;
pub mod report;
pub mod structure;
pub mod tool;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// The checkout that holds this task crate.
///
/// # Errors
///
/// Returns an error when the parent of the task crate cannot be resolved.
pub fn root() -> Result<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let parent = manifest
        .parent()
        .context("the task crate sits at the checkout root")?;
    std::fs::canonicalize(parent).with_context(|| format!("resolve {}", parent.display()))
}

/// A path shown with forward slashes on every host.
#[must_use]
pub fn portable(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
