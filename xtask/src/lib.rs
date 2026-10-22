//! The repository's development tasks.
//!
//! Each task reads the checkout as values, decides, and runs the pinned tools through [`tool`].
//! Files are read and written only through [`fs`], and children start only through [`tool`].

pub mod adr;
pub mod app;
pub mod apple;
pub mod check;
pub mod ci;
pub mod ci_plan;
pub mod commit_msg;
pub mod container;
pub mod fs;
pub mod icons;
pub mod obligations;
pub mod package;
pub mod proofs;
pub mod proverif;
pub mod quint;
pub mod release;
pub mod report;
pub mod structure;
pub mod tool;
pub mod worker;

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

/// The lowercase hexadecimal digits, each at the place of its value.
const HEX: &[u8; 16] = b"0123456789abcdef";
/// The hexadecimal digits that write one byte.
const DIGITS_PER_BYTE: usize = 2;
/// The bits one hexadecimal digit writes.
const NIBBLE_BITS: u32 = 4;
/// The low half of a byte, which its second digit writes.
const LOW_NIBBLE: u8 = 0x0f;

/// Bytes as lowercase hexadecimal digits, two to a byte.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().saturating_mul(DIGITS_PER_BYTE));
    for byte in bytes {
        for nibble in [byte.wrapping_shr(NIBBLE_BITS), byte & LOW_NIBBLE] {
            if let Some(digit) = HEX.get(usize::from(nibble)) {
                text.push(char::from(*digit));
            }
        }
    }
    text
}
