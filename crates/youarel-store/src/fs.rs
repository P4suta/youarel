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

    pub(super) fn read(path: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    pub(super) fn create_dir_all(path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(path)
    }

    pub(super) fn create(path: &Path) -> std::io::Result<File> {
        File::create(path)
    }

    pub(super) fn create_new(path: &Path) -> std::io::Result<File> {
        OpenOptions::new().write(true).create_new(true).open(path)
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

    pub(super) fn remove_dir(path: &Path) -> std::io::Result<()> {
        std::fs::remove_dir(path)
    }

    pub(super) fn open(path: &Path) -> std::io::Result<File> {
        File::open(path)
    }

    pub(super) fn read_dir(path: &Path) -> std::io::Result<std::fs::ReadDir> {
        std::fs::read_dir(path)
    }

    pub(super) fn symlink_metadata(path: &Path) -> std::io::Result<std::fs::Metadata> {
        std::fs::symlink_metadata(path)
    }

    pub(super) fn canonicalize(path: &Path) -> std::io::Result<std::path::PathBuf> {
        std::fs::canonicalize(path)
    }

    /// A new file that its owner may run, and others may read and run.
    #[cfg(unix)]
    pub(super) fn create_executable(path: &Path) -> std::io::Result<File> {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(super::EXECUTABLE_MODE)
            .open(path)?;
        // The process's umask narrows the mode a file is created with, so the open file takes the mode again.
        file.set_permissions(std::fs::Permissions::from_mode(super::EXECUTABLE_MODE))?;
        Ok(file)
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

/// Removes an empty directory.
///
/// # Errors
///
/// Reports a failed directory removal, leaving a nonempty or absent directory alone.
pub fn remove_empty_dir(path: &Path) -> Result<(), FileError> {
    match raw::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(failed("remove the empty directory", path)(error)),
    }
}

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

/// The bytes of a file, or `None` when the file does not exist.
///
/// # Errors
///
/// Returns a [`FileError`] when the file exists but cannot be read.
pub fn read_bytes_if_present(path: &Path) -> Result<Option<Vec<u8>>, FileError> {
    match raw::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(failed("read", path)(error)),
    }
}

/// Gives a file another name in place of whatever that name held.
///
/// # Errors
///
/// Returns a [`FileError`] naming the file when it cannot be renamed.
pub fn rename(from: &Path, to: &Path) -> Result<(), FileError> {
    raw::rename(from, to).map_err(failed("rename", from))
}

/// Creates a directory and its parents.
///
/// # Errors
///
/// Returns a [`FileError`] when a directory cannot be created.
pub fn create_dir_all(path: &Path) -> Result<(), FileError> {
    raw::create_dir_all(path).map_err(failed("create", path))
}

/// Resolves a path to the spelling the operating system uses in file notices.
///
/// # Errors
///
/// Returns a file error when the path cannot be resolved.
pub fn canonicalize(path: &Path) -> Result<PathBuf, FileError> {
    raw::canonicalize(path).map_err(failed("resolve", path))
}

/// Replaces `path` with `bytes` whole: the bytes reach a sibling file and the disk before the rename, so a reader sees the old file or the new one.
///
/// # Errors
///
/// Returns a [`FileError`] naming the step that failed; the original file is then unchanged.
pub fn replace(path: &Path, bytes: &[u8]) -> Result<(), FileError> {
    replace_through(path, bytes, raw::create, settle)
}

/// Writes and syncs a new file, refusing to replace an existing file or symlink.
///
/// # Errors
///
/// Returns a file error naming the failed step; a failed write or sync may leave an incomplete new file.
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<(), FileError> {
    let mut file = raw::create_new(path).map_err(failed("create", path))?;
    file.write_all(bytes).map_err(failed("write", path))?;
    file.sync_all().map_err(failed("sync", path))
}

/// Moves a staged file over the file at `path`.
fn settle(staging: &Path, path: &Path) -> Result<(), FileError> {
    raw::rename(staging, path).map_err(failed("replace", path))
}

/// Replaces `path` with `bytes` whole through a sibling file that `create` makes and `place` moves over it.
fn replace_through(
    path: &Path,
    bytes: &[u8],
    create: fn(&Path) -> std::io::Result<File>,
    place: fn(&Path, &Path) -> Result<(), FileError>,
) -> Result<(), FileError> {
    let staging = path.with_extension("staging");
    let written = (|| {
        let mut file = create(&staging).map_err(failed("create", &staging))?;
        file.write_all(bytes).map_err(failed("write", &staging))?;
        file.sync_all().map_err(failed("sync", &staging))?;
        place(&staging, path)
    })();
    if written.is_err() {
        // The primary error names what failed, and a staging file left behind is replaced by the next write.
        drop(raw::remove_file(&staging));
    }
    written
}

/// The names of the files in a directory that are UTF-8, in ascending order.
///
/// # Errors
///
/// Returns a [`FileError`] when the directory cannot be read.
pub fn list(directory: &Path) -> Result<Vec<String>, FileError> {
    let mut names = Vec::new();
    for entry in raw::read_dir(directory).map_err(failed("list", directory))? {
        let entry = entry.map_err(failed("list", directory))?;
        if let Ok(name) = entry.file_name().into_string() {
            names.push(name);
        }
    }
    names.sort_unstable();
    Ok(names)
}

/// Lists at most `limit` names without allocating a directory's unbounded contents.
///
/// # Errors
///
/// Refuses an unreadable directory or one with more entries than the limit.
pub fn list_bounded(directory: &Path, limit: usize) -> Result<Vec<String>, FileError> {
    let mut names = Vec::new();
    for (index, entry) in raw::read_dir(directory)
        .map_err(failed("list", directory))?
        .enumerate()
    {
        if index >= limit {
            return Err(failed("list", directory)(std::io::Error::new(
                ErrorKind::InvalidData,
                "too many directory entries",
            )));
        }
        let entry = entry.map_err(failed("list", directory))?;
        if let Ok(name) = entry.file_name().into_string() {
            names.push(name);
        }
    }
    names.sort_unstable();
    Ok(names)
}

/// Whether a present path is a directory, with absence distinct from other failures.
///
/// # Errors
///
/// Returns a [`FileError`] when the path cannot be inspected.
pub fn is_directory(path: &Path) -> Result<bool, FileError> {
    match raw::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(failed("inspect", path)(error)),
    }
}

/// Whether a file or a directory stands at `path`.
///
/// # Errors
///
/// Returns a [`FileError`] when the path cannot be looked at for another reason than its absence.
pub fn exists(path: &Path) -> Result<bool, FileError> {
    match raw::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(failed("look at", path)(error)),
    }
}

/// The store's database at `path`, made when the file does not exist, which a process may hold open only once at a time, with `cache` bytes of its pages kept in memory.
///
/// # Errors
///
/// Returns a [`FileError`] naming the file when it cannot be opened, holds no database, or another handle holds it.
pub fn open_database(path: &Path, cache: usize) -> Result<redb::Database, FileError> {
    redb::Builder::new()
        .set_cache_size(cache)
        .create(path)
        .map_err(|error| failed("open", path)(std::io::Error::other(error)))
}

/// Removes a file that may be absent.
///
/// # Errors
///
/// Returns a [`FileError`] when a present file cannot be removed.
pub fn remove_if_present(path: &Path) -> Result<(), FileError> {
    match raw::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(failed("remove", path)(error)),
    }
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
    replace_through(path, bytes, create_executable, install)
}

/// Moves the staged program over the old one.
#[cfg(unix)]
fn install(staging: &Path, path: &Path) -> Result<(), FileError> {
    settle(staging, path)
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
    #[test]
    fn a_new_file_never_replaces_an_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("export.html");
        super::write_new(&path, b"original").unwrap();
        let refused = super::write_new(&path, b"replacement").unwrap_err();
        assert_eq!(refused.source.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(read_bytes_if_present(&path).unwrap().unwrap(), b"original");
    }
    use super::{
        Lock, create_dir_all, list, read_bounded, read_bounded_if_present, read_bytes_if_present,
        read_if_present, remove_if_present, rename, replace, replace_executable,
    };

    #[test]
    fn a_file_is_replaced_whole_and_read_back() {
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("a/b");
        create_dir_all(&nested).unwrap();
        let path = nested.join("identity");
        let staging = path.with_extension("staging");
        assert_eq!(read_if_present(&path).unwrap(), None);
        replace(&nested.join("left"), b"left behind by a failed write").unwrap();
        rename(&nested.join("left"), &staging).unwrap();
        replace(&path, b"first").unwrap();
        replace(&path, b"second").unwrap();
        assert_eq!(read_if_present(&path).unwrap().as_deref(), Some("second"));
        assert_eq!(read_if_present(&staging).unwrap(), None);
    }

    #[test]
    fn a_failed_replacement_names_the_path_and_keeps_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing/published.json");
        let error = replace(&path, b"x").unwrap_err();
        assert!(error.to_string().contains("missing"), "{error}");
        assert!(std::error::Error::source(&error).is_some());
        let unreadable = read_if_present(directory.path()).unwrap_err();
        assert_eq!(unreadable.action, "read");
    }

    #[test]
    fn bytes_read_back_whole_and_a_file_takes_another_name() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("epochs");
        assert_eq!(read_bytes_if_present(&path).unwrap(), None);
        replace(&path, &[0xff, 0x00, b'Y']).unwrap();
        assert_eq!(
            read_bytes_if_present(&path).unwrap(),
            Some(vec![0xff, 0x00, b'Y'])
        );
        assert_eq!(
            read_bytes_if_present(directory.path()).unwrap_err().action,
            "read"
        );
        let moved = directory.path().join("epochs.moved");
        replace(&moved, b"older").unwrap();
        rename(&path, &moved).unwrap();
        assert_eq!(read_bytes_if_present(&path).unwrap(), None);
        assert_eq!(
            read_bytes_if_present(&moved).unwrap(),
            Some(vec![0xff, 0x00, b'Y'])
        );
        let error = rename(&path, &moved).unwrap_err();
        assert_eq!(error.action, "rename");
        assert!(error.to_string().contains("epochs"), "{error}");
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
    fn a_directory_lists_its_names_in_order_and_an_absent_file_counts_as_removed() {
        let directory = tempfile::tempdir().unwrap();
        replace(&directory.path().join("b"), b"").unwrap();
        replace(&directory.path().join("a"), b"").unwrap();
        assert_eq!(list(directory.path()).unwrap(), ["a", "b"]);
        remove_if_present(&directory.path().join("a")).unwrap();
        remove_if_present(&directory.path().join("a")).unwrap();
        assert_eq!(list(directory.path()).unwrap(), ["b"]);
        assert_eq!(
            remove_if_present(directory.path()).unwrap_err().action,
            "remove"
        );
        assert_eq!(
            list(&directory.path().join("missing")).unwrap_err().action,
            "list"
        );
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
