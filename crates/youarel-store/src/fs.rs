//! The one owner of file access in the store, which names the path in each error and replaces a file only whole.

use std::fs::File;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};

mod raw {
    #![expect(
        clippy::disallowed_methods,
        reason = "the store's one file owner forwards each call in a single statement"
    )]

    use std::fs::{File, OpenOptions};
    use std::path::Path;

    pub(super) fn read_to_string(path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }

    pub(super) fn create_dir_all(path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(path)
    }

    pub(super) fn create(path: &Path) -> std::io::Result<File> {
        File::create(path)
    }

    pub(super) fn open_lock(path: &Path) -> std::io::Result<File> {
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
    }

    pub(super) fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }

    pub(super) fn remove_file(path: &Path) -> std::io::Result<()> {
        std::fs::remove_file(path)
    }
}

/// A failed file operation and the path it concerned.
#[derive(Debug)]
pub struct FileError {
    pub action: &'static str,
    pub path: PathBuf,
    pub source: std::io::Error,
}

impl core::fmt::Display for FileError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "cannot {} {}: {}",
            self.action,
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for FileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

fn failed(action: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> FileError {
    let path = path.to_path_buf();
    move |source| FileError {
        action,
        path,
        source,
    }
}

/// The text of a file, or `None` when the file does not exist.
///
/// # Errors
///
/// Returns a [`FileError`] when the file exists but cannot be read as UTF-8 text.
pub fn read_if_present(path: &Path) -> Result<Option<String>, FileError> {
    match raw::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(failed("read", path)(error)),
    }
}

/// Creates a directory and its parents.
///
/// # Errors
///
/// Returns a [`FileError`] when a directory cannot be created.
pub fn create_dir_all(path: &Path) -> Result<(), FileError> {
    raw::create_dir_all(path).map_err(failed("create", path))
}

/// Replaces `path` with `bytes` whole: the bytes reach a sibling file and the disk before the rename, so a reader sees the old file or the new one.
///
/// # Errors
///
/// Returns a [`FileError`] naming the step that failed; the original file is then unchanged.
pub fn replace(path: &Path, bytes: &[u8]) -> Result<(), FileError> {
    let staging = path.with_extension("staging");
    let written = (|| {
        let mut file = raw::create(&staging).map_err(failed("create", &staging))?;
        file.write_all(bytes).map_err(failed("write", &staging))?;
        file.sync_all().map_err(failed("sync", &staging))?;
        raw::rename(&staging, path).map_err(failed("replace", path))
    })();
    if written.is_err() {
        // The primary error names what failed, and a staging file left behind is replaced by the next write.
        drop(raw::remove_file(&staging));
    }
    written
}

/// An exclusive lock on the store, which another process waits for and which the drop releases.
#[derive(Debug)]
pub struct Lock {
    file: File,
}

impl Lock {
    /// Waits for the exclusive lock on `path`, creating the file when it is absent.
    ///
    /// # Errors
    ///
    /// Returns a [`FileError`] when the file cannot be opened or locked.
    pub fn acquire(path: &Path) -> Result<Self, FileError> {
        let file = raw::open_lock(path).map_err(failed("open", path))?;
        file.lock().map_err(failed("lock", path))?;
        Ok(Self { file })
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // Closing the file releases the lock as well, so a failed unlock leaves nothing held.
        drop(self.file.unlock());
    }
}

#[cfg(test)]
mod tests {
    use super::{Lock, create_dir_all, read_if_present, replace};

    #[test]
    fn a_file_is_replaced_whole_and_read_back() {
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("a/b");
        create_dir_all(&nested).unwrap();
        let path = nested.join("state.json");
        assert_eq!(read_if_present(&path).unwrap(), None);
        replace(&path, b"first").unwrap();
        replace(&path, b"second").unwrap();
        assert_eq!(read_if_present(&path).unwrap().as_deref(), Some("second"));
        assert_eq!(
            read_if_present(&path.with_extension("staging")).unwrap(),
            None
        );
    }

    #[test]
    fn a_failed_replacement_names_the_path_and_keeps_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing/state.json");
        let error = replace(&path, b"x").unwrap_err();
        assert!(error.to_string().contains("missing"), "{error}");
        assert!(std::error::Error::source(&error).is_some());
        let unreadable = read_if_present(directory.path()).unwrap_err();
        assert_eq!(unreadable.action, "read");
    }

    #[test]
    fn the_lock_is_released_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        drop(Lock::acquire(&path).unwrap());
        drop(Lock::acquire(&path).unwrap());
    }
}
