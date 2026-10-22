use super::report::Build;
use crate::{
    fs,
    tool::{self, Tool},
};
use anyhow::{Context as _, Result};
use sha2::{Digest as _, Sha512_256};
use std::path::Path;

pub(super) fn identify(root: &Path) -> Result<Build> {
    let commit = tool::capture(
        Tool::Git.command(root).args(["rev-parse", "HEAD"]),
        "read the measured commit",
    )?;
    let changes = tool::capture(
        Tool::Git.command(root).args(["status", "--porcelain"]),
        "read the measured worktree status",
    )?;
    let rustc = tool::capture(
        Tool::Rustc.command(root).args(["--version", "--verbose"]),
        "identify the measured compiler",
    )?;
    let executable = std::env::current_exe().context("find the measured executable")?;
    Ok(Build {
        commit: commit.trim().to_owned(),
        dirty: !changes.trim().is_empty(),
        rustc,
        lock_digest: crate::hex(&Sha512_256::digest(fs::read(&root.join("Cargo.lock"))?)),
        executable_digest: crate::hex(&Sha512_256::digest(fs::read(&executable)?)),
    })
}
