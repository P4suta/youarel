//! youarel's files on one device: the replicated state with its indexes, the device's identity, its own visits, and the icons it gathered, kept in one database under one lock.
//!
//! Every read-modify-write runs in a [`Session`] under the lock, so a terminal interface and a command that run at once never lose each other's edits.

pub mod codec;
pub mod database;
pub mod folders;
pub mod fs;
pub mod icons;
pub mod records;
pub mod session;
pub mod settings;

use crate::codec::DecodeError;
use crate::fs::{FileError, Lock};
use crate::icons::{IconId, LONGEST_IMAGE};
use crate::settings::Settings;
use redb::ReadableTable as _;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use youarel_core::clock_rules::{DeviceId, Stamp};
use youarel_core::state::EditError;
use youarel_core::sync_rules::Carried;
use youarel_core::wire_rules::WireError;

pub use database::{CACHE_BYTES, DATABASE_FILE, FORMAT};
pub use session::{
    BATCH, Group, Grouped, IconBatch, Listing, Named, ON_DEMAND_LIMIT, PATH_SEPARATOR, Part, Parts,
    PendingPublish, PublicationError, PublicationState, PublishRequest, ReceivedSegment, Record,
    RecordSet, Session, Span, StagedPart, Within,
};

/// The file whose lock serializes the sessions of every process.
pub const LOCK_FILE: &str = "lock";
/// What the device's last publish carried and the target it reached, which stay on the device.
pub const PUBLISHED_FILE: &str = "published.json";
/// The file whose lock lets one process at a time publish the device's bundle and record what it carried.
pub const PUBLISH_LOCK_FILE: &str = "publish.lock";

/// Why a session failed.
#[derive(Debug)]
pub enum StoreError {
    Publication(PublicationError),
    Received(DecodeError),
    File(FileError),
    /// A file holds what this youarel would not have written.
    Decode {
        path: PathBuf,
        error: DecodeError,
    },
    /// A local file is not the JSON shape this youarel writes.
    Local {
        path: PathBuf,
        error: serde_json::Error,
    },
    /// The window's settings hold what this youarel would not have written.
    Settings {
        path: PathBuf,
        error: settings::DecodeError,
    },
    /// An icon's image holds more bytes than the device keeps, which counts them.
    Image(usize),
    Encode(serde_json::Error),
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
            Self::Publication(error) => write!(formatter, "{error}"),
            Self::Received(_) => {
                formatter.write_str("the received segment's records are malformed")
            }
            Self::File(error) => write!(formatter, "{error}"),
            Self::Decode { path, .. } | Self::Local { path, .. } | Self::Settings { path, .. } => {
                write!(formatter, "cannot read {}", path.display())
            }
            Self::Image(length) => write!(
                formatter,
                "an icon's image of {length} bytes passes the {LONGEST_IMAGE} bytes the device keeps"
            ),
            Self::Encode(_) | Self::Records(_) => formatter.write_str("cannot write the state"),
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
            Self::Publication(error) => Some(error),
            // The file error is this error's own sentence, so its reason comes next.
            Self::File(error) => std::error::Error::source(error),
            Self::Decode { error, .. } | Self::Received(error) => Some(error),
            Self::Settings { error, .. } => Some(error),
            Self::Records(error) | Self::Damaged(error) => Some(error),
            Self::Local { error, .. } | Self::Encode(error) => Some(error),
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

impl From<PublicationError> for StoreError {
    fn from(error: PublicationError) -> Self {
        Self::Publication(error)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StampDocument {
    wall: u64,
    counter: u32,
    device: String,
}

/// What the last publish carried, and the target it reached.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PublishedDocument {
    target: String,
    latest: Option<StampDocument>,
    operations: usize,
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

/// A local document as JSON.
fn local<T: for<'de> Deserialize<'de>>(path: &Path, text: &str) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|error| StoreError::Local {
        path: path.to_path_buf(),
        error,
    })
}

fn json<T: Serialize>(document: &T) -> Result<Vec<u8>, StoreError> {
    let mut text = serde_json::to_string_pretty(document).map_err(StoreError::Encode)?;
    text.push('\n');
    Ok(text.into_bytes())
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

    /// What the last publish carried, when it reached `target`, or `None` when the last publish reached another target or none happened.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the lock cannot be taken or the record cannot be read or holds what this youarel would not have written.
    pub fn published(&self, target: &str) -> Result<Option<Carried>, StoreError> {
        fs::create_dir_all(&self.directory)?;
        let _lock = Lock::acquire(&self.directory.join(LOCK_FILE))?;
        let path = self.directory.join(PUBLISHED_FILE);
        let Some(text) = fs::read_if_present(&path)? else {
            return Ok(None);
        };
        let document: PublishedDocument = local(&path, &text)?;
        if document.target != target {
            return Ok(None);
        }
        let latest = match document.latest {
            Some(stamp) => Some(Stamp {
                wall: stamp.wall,
                counter: stamp.counter,
                device: DeviceId(
                    codec::unhex(&stamp.device)
                        .map_err(|error| StoreError::Decode { path, error })?,
                ),
            }),
            None => None,
        };
        Ok(Some(Carried {
            latest,
            operations: document.operations,
        }))
    }

    /// Records that a publish to `target` carried `carried`, in place of the record of the publish before it.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the lock cannot be taken or the record cannot be written.
    pub fn record_published(&self, target: &str, carried: Carried) -> Result<(), StoreError> {
        fs::create_dir_all(&self.directory)?;
        let _lock = Lock::acquire(&self.directory.join(LOCK_FILE))?;
        let document = PublishedDocument {
            target: target.to_owned(),
            latest: carried.latest.map(|stamp| StampDocument {
                wall: stamp.wall,
                counter: stamp.counter,
                device: codec::hex(&stamp.device.0),
            }),
            operations: carried.operations,
        };
        fs::replace(&self.directory.join(PUBLISHED_FILE), &json(&document)?)?;
        Ok(())
    }

    /// Holds the device's publishing until the hold drops, so a second process waits until the first wrote its bundle and recorded what it carried.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the directory or the lock cannot be made.
    pub fn publishing(&self) -> Result<Lock, StoreError> {
        fs::create_dir_all(&self.directory)?;
        Ok(Lock::acquire(&self.directory.join(PUBLISH_LOCK_FILE))?)
    }

    /// The window's settings, or the defaults when it never saved any.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the file cannot be read or holds what the window would not have written.
    pub fn settings(&self) -> Result<Settings, StoreError> {
        let path = self.directory.join(settings::SETTINGS_FILE);
        fs::read_if_present(&path)?.map_or_else(
            || Ok(Settings::default()),
            |text| settings::decode(&text).map_err(|error| StoreError::Settings { path, error }),
        )
    }

    /// Saves the window's settings in place of the ones it saved before.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the directory or the file cannot be written.
    pub fn save_settings(&self, saved: &Settings) -> Result<(), StoreError> {
        fs::create_dir_all(&self.directory)?;
        let text = settings::encode(saved).map_err(StoreError::Encode)?;
        fs::replace(
            &self.directory.join(settings::SETTINGS_FILE),
            text.as_bytes(),
        )?;
        Ok(())
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
