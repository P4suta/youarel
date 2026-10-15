//! The one owner of file access in the store, which names the path in each error and replaces a file only whole.

use std::fs::File;
use std::io::{ErrorKind, Read as _, Write as _};
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

    pub(super) fn open(path: &Path) -> std::io::Result<File> {
        File::open(path)
    }

    /// A new file that its owner may run, and others may read and run.
    #[cfg(unix)]
    pub(super) fn create_executable(path: &Path) -> std::io::Result<File> {
        use std::os::unix::fs::OpenOptionsExt as _;
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(super::EXECUTABLE_MODE)
            .open(path)
    }
}

#[cfg(windows)]
mod windows;

#[cfg(unix)]
use raw::create_executable;
#[cfg(windows)]
use windows::{create_executable, install};

/// The mode of a replaced program: its owner may write it, and everyone may read and run it.
#[cfg(unix)]
const EXECUTABLE_MODE: u32 = 0o755;

/// A failed file operation and the path it concerned.
#[derive(Debug)]
pub struct FileError {
    pub action: &'static str,
    pub path: PathBuf,
    pub source: std::io::Error,
}

/// The failed step and its path; the operating system's reason is the error's source, which a chain of errors prints next.
impl core::fmt::Display for FileError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "cannot {} {}", self.action, self.path.display())
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

/// The bytes of a file, no more than `limit` of them, or `None` when the file does not exist.
///
/// # Errors
///
/// Returns a [`FileError`] when the file exists but cannot be read, or holds more than `limit` bytes.
pub fn read_bounded_if_present(path: &Path, limit: u64) -> Result<Option<Vec<u8>>, FileError> {
    let file = match raw::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failed("open", path)(error)),
    };
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(failed("read", path))?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > limit) {
        return Err(failed("read", path)(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("the file is larger than {limit} bytes"),
        )));
    }
    Ok(Some(bytes))
}

/// The bytes of a file, no more than `limit` of them.
///
/// # Errors
///
/// Returns a [`FileError`] when the file is absent or unreadable, or holds more than `limit` bytes.
pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, FileError> {
    read_bounded_if_present(path, limit)?
        .ok_or_else(|| failed("read", path)(std::io::Error::from(ErrorKind::NotFound)))
}

/// Replaces the program at `path` with `bytes` whole, ready to run.
///
/// The bytes reach a sibling file and the disk before the rename, so the path names the old program or the new one.
/// Windows keeps a running program's file, so the old one moves aside to `.old` first, and the next replacement removes it.
///
/// # Errors
///
/// Returns a [`FileError`] naming the step that failed; the program is then unchanged.
pub fn replace_executable(path: &Path, bytes: &[u8]) -> Result<(), FileError> {
    let staging = path.with_extension("staging");
    let written = (|| {
        let mut file = create_executable(&staging).map_err(failed("create", &staging))?;
        file.write_all(bytes).map_err(failed("write", &staging))?;
        file.sync_all().map_err(failed("sync", &staging))?;
        install(&staging, path)
    })();
    if written.is_err() {
        // The primary error names what failed, and a staging file left behind is replaced by the next update.
        drop(raw::remove_file(&staging));
    }
    written
}

/// Moves the staged program over the old one.
#[cfg(unix)]
fn install(staging: &Path, path: &Path) -> Result<(), FileError> {
    raw::rename(staging, path).map_err(failed("replace", path))
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
    use super::{
        Lock, create_dir_all, read_bounded, read_bounded_if_present, read_if_present, replace,
        replace_executable,
    };

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
    fn bounded_reads_refuse_a_longer_file_and_tell_absence_apart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("asset");
        assert_eq!(read_bounded_if_present(&path, 4).unwrap(), None);
        assert_eq!(read_bounded(&path, 4).unwrap_err().action, "read");
        replace(&path, b"four").unwrap();
        assert_eq!(read_bounded(&path, 4).unwrap(), b"four");
        let longer = read_bounded(&path, 3).unwrap_err();
        let reason = std::error::Error::source(&longer).unwrap().to_string();
        assert!(reason.contains("larger than 3 bytes"), "{reason}");
    }

    #[test]
    fn a_program_is_replaced_whole_and_runnable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("youarel");
        replace(&path, b"old").unwrap();
        replace_executable(&path, b"new").unwrap();
        replace_executable(&path, b"newer").unwrap();
        assert_eq!(read_bounded(&path, 16).unwrap(), b"newer");
        assert_eq!(
            read_bounded_if_present(&path.with_extension("staging"), 16).unwrap(),
            None
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, super::EXECUTABLE_MODE);
        }
        let missing = directory.path().join("missing/youarel");
        assert_eq!(
            replace_executable(&missing, b"x").unwrap_err().action,
            "create"
        );
    }

    #[test]
    fn the_lock_is_released_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        drop(Lock::acquire(&path).unwrap());
        drop(Lock::acquire(&path).unwrap());
    }
}
