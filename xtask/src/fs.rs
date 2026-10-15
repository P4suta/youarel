//! The one owner of file reads and writes in the repository tasks.
//!
//! Every other module reads and writes through these functions, which name the path in each error.

use anyhow::{Context, Result};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

mod raw {
    #![expect(
        clippy::disallowed_methods,
        reason = "the repository tasks' one file owner forwards each call in a single statement"
    )]

    use std::path::Path;

    pub(super) fn read_to_string(path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }

    pub(super) fn read(path: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    pub(super) fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        std::fs::write(path, bytes)
    }

    pub(super) fn create_dir_all(path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(path)
    }

    pub(super) fn remove_dir_all(path: &Path) -> std::io::Result<()> {
        std::fs::remove_dir_all(path)
    }

    pub(super) fn append(path: &Path) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    }

    /// A new file that only its owner may read or write.
    #[cfg(unix)]
    pub(super) fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
        use std::os::unix::fs::OpenOptionsExt as _;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(super::PRIVATE_MODE)
            .open(path)
    }

    /// A new file, which a fresh private directory keeps from other users.
    #[cfg(not(unix))]
    pub(super) fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
    }

    /// A new file that its owner may write and everyone may read and run.
    #[cfg(unix)]
    pub(super) fn create_executable(path: &Path) -> std::io::Result<std::fs::File> {
        use std::os::unix::fs::OpenOptionsExt as _;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(super::EXECUTABLE_MODE)
            .open(path)
    }

    /// A new file, which Windows runs by its name.
    #[cfg(not(unix))]
    pub(super) fn create_executable(path: &Path) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
    }
}

/// The mode of a secret's file: its owner may read and write it, and nobody else may touch it.
#[cfg(unix)]
const PRIVATE_MODE: u32 = 0o600;
/// The mode of a program's file: its owner may write it, and everyone may read and run it.
#[cfg(unix)]
const EXECUTABLE_MODE: u32 = 0o755;

/// Appends bytes to a file, creating it when it is absent.
///
/// # Errors
///
/// Returns an error naming the path when the file cannot be written.
pub fn append(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let mut file = raw::append(path).with_context(|| format!("open {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("append to {}", path.display()))
}

/// Writes a secret to a new file that only its owner may read.
///
/// # Errors
///
/// Returns an error naming the path when the file exists or cannot be written.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let mut file =
        raw::create_private(path).with_context(|| format!("create {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("write {}", path.display()))
}

/// Writes a program to a new file that may run, without the extended attributes a copy would carry along.
///
/// # Errors
///
/// Returns an error naming the path when the file exists or cannot be written.
pub fn write_executable(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let mut file =
        raw::create_executable(path).with_context(|| format!("create {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("write {}", path.display()))
}

/// What a path names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    File,
    Directory,
    Other,
}

/// The text of a file.
///
/// # Errors
///
/// Returns an error naming the path when it cannot be read as UTF-8 text.
pub fn read_to_string(path: &Path) -> Result<String> {
    raw::read_to_string(path).with_context(|| format!("read {}", path.display()))
}

/// The bytes of a file.
///
/// # Errors
///
/// Returns an error naming the path when it cannot be read.
pub fn read(path: &Path) -> Result<Vec<u8>> {
    raw::read(path).with_context(|| format!("read {}", path.display()))
}

/// Writes a file, creating its parent directories.
///
/// # Errors
///
/// Returns an error naming the path when a directory or the file cannot be written.
pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    raw::write(path, bytes).with_context(|| format!("write {}", path.display()))
}

/// Creates a directory and its parents.
///
/// # Errors
///
/// Returns an error naming the path when a directory cannot be created.
pub fn create_dir_all(path: &Path) -> Result<()> {
    raw::create_dir_all(path).with_context(|| format!("create {}", path.display()))
}

/// Removes a directory tree that may be absent.
///
/// # Errors
///
/// Returns an error naming the path when a present tree cannot be removed.
pub fn remove_dir_all(path: &Path) -> Result<()> {
    match raw::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("remove {}", path.display())),
    }
}

/// What `path` names, or `None` when nothing is there.
///
/// # Errors
///
/// Returns an error naming the path when it is present but unreadable.
pub fn kind(path: &Path) -> Result<Option<Kind>> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(Kind::File)),
        Ok(metadata) if metadata.is_dir() => Ok(Some(Kind::Directory)),
        Ok(_) => Ok(Some(Kind::Other)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("inspect {}", path.display())),
    }
}

/// Whether `path` names a directory.
///
/// # Errors
///
/// Returns an error naming the path when it is present but unreadable.
pub fn is_directory(path: &Path) -> Result<bool> {
    Ok(kind(path)? == Some(Kind::Directory))
}

/// Whether `path` names a file.
///
/// # Errors
///
/// Returns an error naming the path when it is present but unreadable.
pub fn is_file(path: &Path) -> Result<bool> {
    Ok(kind(path)? == Some(Kind::File))
}

/// The entries of a directory in name order, or none when the directory is absent.
///
/// # Errors
///
/// Returns an error naming the path when a present directory cannot be listed.
pub fn entries(directory: &Path) -> Result<Vec<PathBuf>> {
    let listing = match std::fs::read_dir(directory) {
        Ok(listing) => listing,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("list {}", directory.display())),
    };
    let mut paths = Vec::new();
    for entry in listing {
        let entry = entry.with_context(|| format!("list {}", directory.display()))?;
        paths.push(entry.path());
    }
    paths.sort();
    Ok(paths)
}

/// Every file below `directory` whose name ends with `suffix`, in path order.
///
/// # Errors
///
/// Returns an error naming the directory that cannot be listed.
pub fn files_with_suffix(directory: &Path, suffix: &str) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    collect(directory, suffix, &mut found)?;
    found.sort();
    Ok(found)
}

fn collect(directory: &Path, suffix: &str, found: &mut Vec<PathBuf>) -> Result<()> {
    for path in entries(directory)? {
        match kind(&path)? {
            Some(Kind::Directory) => collect(&path, suffix, found)?,
            Some(Kind::File) => {
                if path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(suffix))
                {
                    found.push(path);
                }
            }
            Some(Kind::Other) | None => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Kind, entries, files_with_suffix, kind, read_to_string, remove_dir_all, write};

    #[test]
    fn files_are_written_read_listed_and_removed() {
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("a/b/c_rules.rs");
        write(&nested, b"text").unwrap();
        assert_eq!(read_to_string(&nested).unwrap(), "text");
        assert_eq!(kind(&nested).unwrap(), Some(Kind::File));
        assert_eq!(
            kind(&directory.path().join("a")).unwrap(),
            Some(Kind::Directory)
        );
        assert_eq!(kind(&directory.path().join("missing")).unwrap(), None);
        write(&directory.path().join("a/other.rs"), b"").unwrap();
        let rules = files_with_suffix(directory.path(), "_rules.rs").unwrap();
        assert_eq!(rules, std::slice::from_ref(&nested));
        assert_eq!(
            entries(&directory.path().join("absent")).unwrap(),
            Vec::<std::path::PathBuf>::new()
        );
        remove_dir_all(&directory.path().join("a")).unwrap();
        assert_eq!(kind(&nested).unwrap(), None);
        remove_dir_all(&directory.path().join("a")).unwrap();
    }
}
