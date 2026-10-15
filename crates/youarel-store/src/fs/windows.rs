//! The Windows half of replacing a program: Windows keeps the file of a running program, so the old one moves aside instead of being overwritten.

use super::{FileError, failed, raw};
use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;

/// A new file, which Windows runs by its name.
pub(super) fn create_executable(path: &Path) -> std::io::Result<File> {
    raw::create(path)
}

/// Moves the old program aside and the staged one into its place, and puts the old one back when the second move fails.
pub(super) fn install(staging: &Path, path: &Path) -> Result<(), FileError> {
    let old = path.with_extension("old");
    match raw::remove_file(&old) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(failed("remove", &old)(error)),
    }
    raw::rename(path, &old).map_err(failed("move aside", path))?;
    raw::rename(staging, path).map_err(|error| {
        // The old program returns to its place, and the primary error is the failed move.
        drop(raw::rename(&old, path));
        failed("replace", path)(error)
    })
}
