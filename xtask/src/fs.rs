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

    pub(super) fn metadata(path: &Path) -> std::io::Result<std::fs::Metadata> {
        std::fs::metadata(path)
    }

    pub(super) fn symlink_metadata(path: &Path) -> std::io::Result<std::fs::Metadata> {
        std::fs::symlink_metadata(path)
    }

    #[cfg(unix)]
    pub(super) fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    pub(super) fn available_space(path: &Path) -> std::io::Result<u64> {
        fs4::available_space(path)
    }

    pub(super) fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        std::fs::write(path, bytes)
    }

    pub(super) fn create_dir_all(path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(path)
    }

    #[cfg(unix)]
    pub(super) fn create_private_directory(path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::DirBuilderExt as _;
        std::fs::DirBuilder::new()
            .mode(super::PRIVATE_DIRECTORY_MODE)
            .create(path)
    }

    #[cfg(all(unix, test))]
    pub(super) fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
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

    pub(super) fn open_lock(path: &Path) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .create(true)
            .read(true)
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

/// The mode of a secret's file: its owner alone may read and write it.
#[cfg(unix)]
const PRIVATE_MODE: u32 = 0o600;
#[cfg(unix)]
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
#[cfg(unix)]
const SHARED_ACCESS: u32 = 0o077;
#[cfg(unix)]
const SHARED_WRITE: u32 = 0o022;
#[cfg(unix)]
const STICKY: u32 = 0o1000;
#[cfg(unix)]
const ROOT_UID: u32 = 0;
/// The mode of a program's file: its owner may write it, and everyone may read and run it.
#[cfg(unix)]
const EXECUTABLE_MODE: u32 = 0o755;

/// # Errors
///
/// Reports unreadable metadata without following a symbolic link.
pub fn entry_metadata(path: &Path) -> Result<std::fs::Metadata> {
    raw::symlink_metadata(path).with_context(|| format!("inspect {}", path.display()))
}

/// # Errors
///
/// Reports a failed installer link creation or an unsupported host.
pub fn symlink(target: &Path, link: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        raw::symlink(target, link).with_context(|| format!("link {}", link.display()))
    }
    #[cfg(not(unix))]
    {
        anyhow::bail!(
            "installer link {} to {} requires Unix",
            link.display(),
            target.display()
        )
    }
}

#[derive(Debug)]
pub struct RunLock {
    _file: std::fs::File,
}

/// # Errors
///
/// Refuses another active run or an unreadable lock file.
pub fn lock_run(path: &Path) -> Result<RunLock> {
    let file = raw::open_lock(path).with_context(|| format!("open {}", path.display()))?;
    file.try_lock()
        .with_context(|| format!("another run holds {}", path.display()))?;
    Ok(RunLock { _file: file })
}

/// # Errors
///
/// Reports unreadable metadata.
/// An absent synthetic file has no bytes yet.
pub fn file_bytes_if_present(path: &Path) -> Result<u64> {
    match raw::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error).with_context(|| format!("measure {}", path.display())),
    }
}

/// # Errors
///
/// Reports a failed filesystem space query.
pub fn available_space(path: &Path) -> Result<u64> {
    raw::available_space(path)
        .with_context(|| format!("read available space at {}", path.display()))
}

/// # Errors
///
/// Reports a failed creation of an owned temporary workspace.
pub fn temporary_in(path: &Path) -> Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(path)
        .with_context(|| format!("create a temporary workspace at {}", path.display()))
}

/// Appends bytes to a file, creating an absent file.
///
/// # Errors
///
/// Reports file write failures with the affected path.
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
/// Reports existing destinations and file write failures with the affected path.
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
/// Reports existing destinations and file write failures with the affected path.
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
/// Reports file read and UTF-8 decoding failures with the affected path.
pub fn read_to_string(path: &Path) -> Result<String> {
    raw::read_to_string(path).with_context(|| format!("read {}", path.display()))
}

/// The bytes of a file.
///
/// # Errors
///
/// Reports file read failures with the affected path.
pub fn read(path: &Path) -> Result<Vec<u8>> {
    raw::read(path).with_context(|| format!("read {}", path.display()))
}

/// Writes a file, creating its parent directories.
///
/// # Errors
///
/// Reports directory creation and file write failures with the affected path.
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
/// Reports directory creation failures with the affected path.
pub fn create_dir_all(path: &Path) -> Result<()> {
    raw::create_dir_all(path).with_context(|| format!("create {}", path.display()))
}

/// Creates a private directory under an existing private parent and returns its canonical path.
///
/// # Errors
///
/// Refuses existing destinations, untrusted ancestors, shared parents, and hosts without Unix permissions.
pub fn create_private_directory(path: &Path) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;

        let name = path.file_name().context("name a fresh private directory")?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = std::fs::canonicalize(parent)
            .with_context(|| format!("resolve the existing parent of {}", path.display()))?;
        let uid = tempfile::tempfile()
            .context("create an owned file to identify the effective user")?
            .metadata()?
            .uid();
        let ancestors: Vec<_> = parent.ancestors().collect();
        for ancestor in ancestors.into_iter().rev() {
            let metadata = entry_metadata(ancestor)?;
            anyhow::ensure!(
                metadata.is_dir() && (metadata.uid() == uid || metadata.uid() == ROOT_UID),
                "{} is not a directory owned by the current user or root",
                ancestor.display()
            );
            anyhow::ensure!(
                metadata.mode() & SHARED_WRITE == 0 || metadata.mode() & STICKY != 0,
                "{} permits other users to replace directory entries",
                ancestor.display()
            );
            #[cfg(target_os = "macos")]
            refuse_granting_acl(ancestor)?;
        }
        let metadata = entry_metadata(&parent)?;
        anyhow::ensure!(
            metadata.uid() == uid && metadata.mode() & SHARED_ACCESS == 0,
            "{} must be a private directory owned by the current user",
            parent.display()
        );
        let destination = parent.join(name);
        raw::create_private_directory(&destination)
            .with_context(|| format!("create a fresh private directory at {}", path.display()))?;
        Ok(destination)
    }
    #[cfg(not(unix))]
    {
        anyhow::bail!(
            "private directory creation at {} requires Unix",
            path.display()
        )
    }
}

#[cfg(target_os = "macos")]
fn refuse_granting_acl(path: &Path) -> Result<()> {
    const NATIVE_LISTING: &str = "/bin/ls";
    let listing = crate::tool::capture(
        crate::tool::built(Path::new("/"), Path::new(NATIVE_LISTING))
            .arg("-lde")
            .arg(path)
            .env("LC_ALL", "C"),
        "inspect native directory access controls",
    )?;
    anyhow::ensure!(
        !listing
            .lines()
            .skip(1)
            .any(|line| { line.split_ascii_whitespace().any(|word| word == "allow") }),
        "{} has a granting access control entry",
        path.display()
    );
    Ok(())
}

/// Removes a directory tree that may be absent.
///
/// # Errors
///
/// Reports directory removal failures with the affected path.
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
/// Reports metadata failures with the affected path.
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
/// Reports metadata failures with the affected path.
pub fn is_directory(path: &Path) -> Result<bool> {
    Ok(kind(path)? == Some(Kind::Directory))
}

/// Whether `path` names a file.
///
/// # Errors
///
/// Reports metadata failures with the affected path.
pub fn is_file(path: &Path) -> Result<bool> {
    Ok(kind(path)? == Some(Kind::File))
}

/// The entries of a directory in name order, or none when the directory is absent.
///
/// # Errors
///
/// Reports directory listing failures with the affected path.
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
/// Reports directory listing failures with the affected path.
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

    #[cfg(unix)]
    #[test]
    fn private_directory_creation_is_atomic_and_owner_only() {
        use std::os::unix::fs::MetadataExt as _;
        let parent = private_parent();
        let path = parent.path().join("keys");
        let results = std::thread::scope(|scope| {
            let first = scope.spawn(|| super::create_private_directory(&path));
            let second = scope.spawn(|| super::create_private_directory(&path));
            [first.join().unwrap(), second.join().unwrap()]
        });
        let mut successes = 0;
        for result in results {
            match result {
                Ok(created) => {
                    successes += 1;
                    assert_eq!(
                        super::entry_metadata(&created).unwrap().mode() & 0o777,
                        super::PRIVATE_DIRECTORY_MODE
                    );
                }
                Err(error) => assert_eq!(
                    error.downcast_ref::<std::io::Error>().unwrap().kind(),
                    std::io::ErrorKind::AlreadyExists
                ),
            }
        }
        assert_eq!(successes, 1);
    }

    #[cfg(unix)]
    #[test]
    fn private_directories_use_resolved_parents_and_refuse_existing_links() {
        let parent = private_parent();
        let alias = parent.path().join("alias");
        super::symlink(parent.path(), &alias).unwrap();
        let path = alias.join("keys");
        let created = super::create_private_directory(&path).unwrap();
        assert_eq!(
            created,
            std::fs::canonicalize(parent.path()).unwrap().join("keys")
        );
        let link = parent.path().join("existing");
        super::symlink(&created, &link).unwrap();
        let error = super::create_private_directory(&link).unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::AlreadyExists
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_directories_refuse_shared_parents_and_replaceable_ancestors() {
        let workspace = private_parent();
        let parent = workspace.path().join("parent");
        super::raw::create_private_directory(&parent).unwrap();
        super::raw::set_mode(&parent, 0o755).unwrap();
        let destination = parent.join("keys");
        let shared_parent = super::create_private_directory(&destination).unwrap_err();
        assert!(
            shared_parent
                .to_string()
                .contains("must be a private directory")
        );
        assert_eq!(kind(&destination).unwrap(), None);
        super::raw::set_mode(&parent, super::PRIVATE_DIRECTORY_MODE).unwrap();
        super::raw::set_mode(workspace.path(), 0o777).unwrap();
        let replaceable_ancestor = super::create_private_directory(&destination).unwrap_err();
        assert!(
            replaceable_ancestor
                .to_string()
                .contains("replace directory entries")
        );
        assert_eq!(kind(&destination).unwrap(), None);
    }

    #[cfg(unix)]
    fn private_parent() -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt as _;
        tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(
                super::PRIVATE_DIRECTORY_MODE,
            ))
            .tempdir()
            .unwrap()
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn private_directories_refuse_access_controls_that_override_owner_only_modes() {
        let workspace = private_parent();
        crate::tool::succeed(
            crate::tool::built(std::path::Path::new("/"), std::path::Path::new("/bin/chmod"))
                .args(["+a", "everyone allow list,search,add_file,add_subdirectory,directory_inherit,file_inherit"])
                .arg(workspace.path()),
            "prepare an access control regression fixture",
        ).unwrap();
        let destination = workspace.path().join("keys");
        let error = super::create_private_directory(&destination).unwrap_err();
        assert!(error.to_string().contains("granting access control entry"));
        assert_eq!(kind(&destination).unwrap(), None);
    }

    #[test]
    fn a_campaign_lock_refuses_overlap_and_releases_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("run.lock");
        let first = super::lock_run(&path).unwrap();
        super::lock_run(&path).unwrap_err();
        drop(first);
        let _second = super::lock_run(&path).unwrap();
    }

    #[test]
    fn a_campaign_lock_preserves_existing_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("run.lock");
        write(&path, b"state").unwrap();
        let guard = super::lock_run(&path).unwrap();
        drop(guard);
        assert_eq!(read_to_string(&path).unwrap(), "state");
    }

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
