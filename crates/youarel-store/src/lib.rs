//! youarel's files on one device: the bookmarks with their indexes, the local writer's identifier, its own visits, and the icons it gathered, kept in one database under one lock.
//!
//! Every read-modify-write runs in a [`Session`] under the lock, so a terminal interface and a command that run at once never lose each other's edits.

pub mod codec;
pub mod database;
pub mod folders;
pub mod fs;
pub mod icons;
pub mod records;
pub mod session;

use crate::fs::{FileError, Lock};
use crate::icons::{IconId, LONGEST_IMAGE};
use redb::ReadableTable as _;
use std::path::{Path, PathBuf};
use youarel_core::state::EditError;
use youarel_core::wire_rules::WireError;

pub use database::{CACHE_BYTES, DATABASE_FILE, FORMAT};
pub use session::{
    BATCH, Group, Grouped, Listing, Named, ON_DEMAND_LIMIT, PATH_SEPARATOR, Part, Parts, Record,
    Session, Span, Within,
};

/// The file whose lock serializes the sessions of every process.
pub const LOCK_FILE: &str = "lock";
/// Why a session failed.
#[derive(Debug)]
pub enum StoreError {
    File(FileError),
    /// An icon's image holds more bytes than the device keeps, which counts them.
    Image(usize),
    /// The state does not write as records.
    Records(WireError),
    /// The store's database failed.
    Database(redb::Error),
    /// The store's database holds a value this youarel would not have written.
    Damaged(WireError),
    /// The store's database lacks what every store of this youarel holds, which it names.
    Missing(&'static str),
    /// The store's database names another format, or none.
    Format(Option<u16>),
    /// The operating system gave no randomness for a new identifier.
    Random(getrandom::Error),
    Edit(EditError),
    /// The clock has spent every stamp.
    Clock,
}

/// The failure in a sentence; an underlying error is the source, which a chain of errors prints next.
impl core::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::File(error) => write!(formatter, "{error}"),
            Self::Image(length) => write!(
                formatter,
                "an icon's image of {length} bytes passes the {LONGEST_IMAGE} bytes the device keeps"
            ),
            Self::Records(_) => formatter.write_str("cannot write the state"),
            Self::Database(_) => formatter.write_str("the store's database failed"),
            Self::Damaged(_) => formatter.write_str(
                "the store's database holds a value this youarel would not have written",
            ),
            Self::Missing(what) => write!(formatter, "the store's database lacks {what}"),
            Self::Format(Some(format)) => write!(
                formatter,
                "the store's database has format {format}, but this youarel reads format {FORMAT}"
            ),
            Self::Format(None) => formatter.write_str("the store's database names no format"),
            Self::Random(error) => write!(formatter, "no randomness for a new identifier: {error}"),
            Self::Edit(error) => write!(formatter, "{}", describe(*error)),
            Self::Clock => formatter.write_str("the clock has no stamp left"),
        }
    }
}

/// A sentence for an edit's refusal.
const fn describe(error: EditError) -> &'static str {
    match error {
        EditError::DeviceName => {
            "a device name must fit within 128 bytes without padding or control characters"
        }
        EditError::Unknown(_) => "no such shortcut or folder",
        EditError::Exists(_) => "an entry with that identifier already exists",
        EditError::IntoItself(_) => "a folder cannot move into itself",
        EditError::Position(_) => "no position fits there",
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            // The file error is this error's own sentence, so its reason comes next.
            Self::File(error) => std::error::Error::source(error),
            Self::Records(error) | Self::Damaged(error) => Some(error),
            Self::Database(error) => Some(error),
            Self::Image(_)
            | Self::Missing(_)
            | Self::Format(_)
            | Self::Random(_)
            | Self::Edit(_)
            | Self::Clock => None,
        }
    }
}

impl From<FileError> for StoreError {
    fn from(error: FileError) -> Self {
        Self::File(error)
    }
}

/// The directory that holds one device's files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Store {
    directory: PathBuf,
}

/// Random bytes from the operating system.
pub(crate) fn random<const N: usize>() -> Result<[u8; N], StoreError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(StoreError::Random)?;
    Ok(bytes)
}

impl Store {
    #[must_use]
    pub const fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The store's database, made empty the first time, under the lock the caller holds.
    fn database(&self) -> Result<redb::Database, StoreError> {
        let path = self.directory.join(DATABASE_FILE);
        if !fs::exists(&path)? {
            database::create(&path)?;
        }
        Ok(fs::open_database(&path, CACHE_BYTES)?)
    }

    /// The images of icons that their identifiers name, read under the lock, each `None` when the device keeps no such image.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the lock cannot be taken or the database cannot be read.
    pub fn images(&self, wanted: &[IconId]) -> Result<Vec<Option<Vec<u8>>>, StoreError> {
        fs::create_dir_all(&self.directory)?;
        let _lock = Lock::acquire(&self.directory.join(LOCK_FILE))?;
        let path = self.directory.join(DATABASE_FILE);
        // A store whose database no session made yet keeps no image.
        if !fs::exists(&path)? {
            return Ok(vec![None; wanted.len()]);
        }
        let database = fs::open_database(&path, CACHE_BYTES)?;
        let transaction = database.begin_write().map_err(database::failed)?;
        let found = {
            let images = database::open(&transaction, database::IMAGES)?;
            wanted
                .iter()
                .map(|image| {
                    Ok(images
                        .get(image.0.as_slice())
                        .map_err(database::failed)?
                        .map(|held| held.value().to_vec()))
                })
                .collect::<Result<Vec<_>, StoreError>>()?
        };
        transaction.abort().map_err(database::failed)?;
        Ok(found)
    }

    /// The image an icon's identifier names, read under the lock, or `None` when the device keeps no such image.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the lock cannot be taken or the database cannot be read.
    pub fn image(&self, image: IconId) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.images(&[image])?.into_iter().flatten().next())
    }

    /// Runs `work` on the device's state at time `now`, under the lock, and keeps what it changed.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the database cannot be opened, read, or written, or when `work` fails; nothing is written then.
    pub fn session<R>(
        &self,
        now: u64,
        work: impl FnOnce(&mut Session) -> Result<R, StoreError>,
    ) -> Result<R, StoreError> {
        fs::create_dir_all(&self.directory)?;
        let _lock = Lock::acquire(&self.directory.join(LOCK_FILE))?;
        let database = self.database()?;
        let mut transaction = database.begin_write().map_err(database::failed)?;
        transaction.set_quick_repair(true);
        let mut session = Session::open(transaction, now)?;
        let result = work(&mut session)?;
        session.finish()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
