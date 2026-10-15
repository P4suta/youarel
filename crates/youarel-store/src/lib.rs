//! youarel's files on one device: the replicated state, the device's identity, and its own visits, read and written under one lock.
//!
//! Every read-modify-write runs in a [`Session`] under the lock, so a terminal interface and a command that run at once never lose each other's edits.

pub mod codec;
pub mod fs;

use crate::codec::DecodeError;
use crate::fs::{FileError, Lock};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use youarel_core::clock_rules::{self, DeviceId, Stamp};
use youarel_core::rank_rules::Visits;
use youarel_core::state::{Edit, EditError, ItemId, State};

/// The replicated state.
pub const STATE_FILE: &str = "state.json";
/// The device's identity, which stays on the device.
pub const DEVICE_FILE: &str = "device.json";
/// The device's visits, which stay on the device.
pub const VISITS_FILE: &str = "visits.json";
/// The file whose lock serializes the sessions of every process.
pub const LOCK_FILE: &str = "lock";

/// Why a session failed.
#[derive(Debug)]
pub enum StoreError {
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
    Encode(serde_json::Error),
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
            Self::Decode { path, .. } | Self::Local { path, .. } => {
                write!(formatter, "cannot read {}", path.display())
            }
            Self::Encode(_) => formatter.write_str("cannot write the state"),
            Self::Random(error) => write!(formatter, "no randomness for a new identifier: {error}"),
            Self::Edit(error) => write!(formatter, "{}", describe(*error)),
            Self::Clock => formatter.write_str("the clock has no stamp left"),
        }
    }
}

/// A sentence for an edit's refusal.
const fn describe(error: EditError) -> &'static str {
    match error {
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
            Self::Decode { error, .. } => Some(error),
            Self::Local { error, .. } | Self::Encode(error) => Some(error),
            Self::Random(_) | Self::Edit(_) | Self::Clock => None,
        }
    }
}

impl From<FileError> for StoreError {
    fn from(error: FileError) -> Self {
        Self::File(error)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DeviceDocument {
    device: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VisitDocument {
    id: String,
    count: u32,
    last: u64,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VisitsDocument {
    visits: Vec<VisitDocument>,
}

/// The directory that holds one device's files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Store {
    directory: PathBuf,
}

/// The state and the visits of one locked read-modify-write.
#[derive(Debug)]
pub struct Session {
    state: State,
    visits: BTreeMap<ItemId, Visits>,
    device: DeviceId,
    now: u64,
    changed: bool,
    visited: bool,
}

fn random<const N: usize>() -> Result<[u8; N], StoreError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(StoreError::Random)?;
    Ok(bytes)
}

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

    fn device(&self) -> Result<DeviceId, StoreError> {
        let path = self.directory.join(DEVICE_FILE);
        if let Some(text) = fs::read_if_present(&path)? {
            let document: DeviceDocument = local(&path, &text)?;
            let bytes = codec::unhex(&document.device).map_err(|error| StoreError::Decode {
                path: path.clone(),
                error,
            })?;
            return Ok(DeviceId(bytes));
        }
        let device = DeviceId(random()?);
        let document = DeviceDocument {
            device: codec::hex(&device.0),
        };
        fs::replace(&path, &json(&document)?)?;
        Ok(device)
    }

    fn state(&self) -> Result<State, StoreError> {
        let path = self.directory.join(STATE_FILE);
        fs::read_if_present(&path)?.map_or_else(
            || Ok(State::default()),
            |text| codec::decode(&text).map_err(|error| StoreError::Decode { path, error }),
        )
    }

    fn visits(&self) -> Result<BTreeMap<ItemId, Visits>, StoreError> {
        let path = self.directory.join(VISITS_FILE);
        let Some(text) = fs::read_if_present(&path)? else {
            return Ok(BTreeMap::new());
        };
        let document: VisitsDocument = local(&path, &text)?;
        let mut visits = BTreeMap::new();
        for visit in document.visits {
            let id = codec::unhex(&visit.id).map_err(|error| StoreError::Decode {
                path: path.clone(),
                error,
            })?;
            visits.insert(
                ItemId(id),
                Visits {
                    count: visit.count,
                    last: visit.last,
                },
            );
        }
        Ok(visits)
    }

    /// Runs `work` on the device's state at time `now`, under the lock, and writes back what it changed.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when a file cannot be read, decoded, or written, or when `work` fails; nothing is written then.
    pub fn session<R>(
        &self,
        now: u64,
        work: impl FnOnce(&mut Session) -> Result<R, StoreError>,
    ) -> Result<R, StoreError> {
        fs::create_dir_all(&self.directory)?;
        let _lock = Lock::acquire(&self.directory.join(LOCK_FILE))?;
        let mut session = Session {
            device: self.device()?,
            state: self.state()?,
            visits: self.visits()?,
            now,
            changed: false,
            visited: false,
        };
        let result = work(&mut session)?;
        if session.changed {
            // The device keeps only the images its live shortcuts show; another replica's state brings back any it shows again.
            session.state.prune_icons();
            let text = codec::encode(&session.state).map_err(StoreError::Encode)?;
            fs::replace(&self.directory.join(STATE_FILE), text.as_bytes())?;
        }
        if session.visited {
            let document = VisitsDocument {
                visits: session
                    .visits
                    .iter()
                    .map(|(id, visits)| VisitDocument {
                        id: codec::hex(&id.0),
                        count: visits.count,
                        last: visits.last,
                    })
                    .collect(),
            };
            fs::replace(&self.directory.join(VISITS_FILE), &json(&document)?)?;
        }
        Ok(result)
    }
}

impl Session {
    #[must_use]
    pub const fn state(&self) -> &State {
        &self.state
    }

    #[must_use]
    pub const fn visits(&self) -> &BTreeMap<ItemId, Visits> {
        &self.visits
    }

    /// The time the session runs at, in milliseconds since the Unix epoch.
    #[must_use]
    pub const fn now(&self) -> u64 {
        self.now
    }

    #[must_use]
    pub const fn device(&self) -> DeviceId {
        self.device
    }

    /// The stamp of the next write, after every write the state holds.
    fn stamp(&self) -> Result<Stamp, StoreError> {
        let origin = Stamp::origin(self.device);
        let last = self
            .state
            .latest()
            .map_or(origin, |latest| clock_rules::observe(origin, latest));
        clock_rules::tick(last, self.now).ok_or(StoreError::Clock)
    }

    /// Applies one edit at the next stamp.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Edit`] when the state refuses the edit, and [`StoreError::Clock`] when no stamp is left.
    pub fn edit(&mut self, edit: Edit) -> Result<(), StoreError> {
        let stamp = self.stamp()?;
        self.state.apply(edit, stamp).map_err(StoreError::Edit)?;
        self.changed = true;
        Ok(())
    }

    /// A new identifier: the time and random bits, as a version 7 UUID.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Random`] when the operating system gives no randomness.
    pub fn fresh_id(&self) -> Result<ItemId, StoreError> {
        Ok(ItemId::v7(self.now, random()?))
    }

    /// Counts one visit of a shortcut now.
    pub fn visit(&mut self, id: ItemId) {
        let visits = self.visits.entry(id).or_default();
        visits.count = visits.count.saturating_add(1);
        visits.last = self.now;
        self.visited = true;
    }

    /// Merges another replica's state into this one.
    pub fn merge(&mut self, other: &State) {
        let merged = self.state.merge(other);
        if merged != self.state {
            self.state = merged;
            self.changed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DEVICE_FILE, STATE_FILE, Store, StoreError, VISITS_FILE};
    use youarel_core::state::{Edit, Entry, IconId};

    const NOW: u64 = 1_700_000_000_000;

    fn add(store: &Store, link: &str) -> youarel_core::state::ItemId {
        store
            .session(NOW, |session| {
                let id = session.fresh_id()?;
                session.edit(Edit::AddShortcut {
                    id,
                    link: link.to_owned(),
                    title: link.to_owned(),
                    folder: None,
                })?;
                Ok(id)
            })
            .unwrap()
    }

    #[test]
    fn a_merged_replica_persists_and_the_session_runs_at_its_time() {
        let directory = tempfile::tempdir().unwrap();
        let mine = Store::new(directory.path().join("mine"));
        let theirs = Store::new(directory.path().join("theirs"));
        let shared = add(&theirs, "https://example.net/");
        let other = theirs
            .session(NOW, |session| Ok(session.state().clone()))
            .unwrap();
        let now = mine
            .session(NOW + 7, |session| {
                session.merge(&other);
                Ok(session.now())
            })
            .unwrap();
        assert_eq!(now, NOW + 7);
        let held = mine
            .session(NOW + 8, |session| {
                Ok(session.state().shortcut(shared).is_some())
            })
            .unwrap();
        assert!(held, "the merged shortcut was written");
    }

    #[test]
    fn edits_and_visits_persist_across_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().join("youarel"));
        let first = add(&store, "https://example.com/");
        let second = add(&store, "https://example.org/");
        store
            .session(NOW, |session| {
                session.visit(second);
                Ok(())
            })
            .unwrap();
        let (entries, visits, device) = store
            .session(NOW + 1, |session| {
                let tree = session.state().tree();
                Ok((
                    session.state().entries(&tree, None),
                    session.visits().get(&second).copied(),
                    session.device(),
                ))
            })
            .unwrap();
        assert_eq!(entries, [Entry::Shortcut(first), Entry::Shortcut(second)]);
        assert_eq!(visits.map(|visits| visits.count), Some(1));
        let again = store.session(NOW, |session| Ok(session.device())).unwrap();
        assert_eq!(device, again);
        for file in [STATE_FILE, DEVICE_FILE, VISITS_FILE] {
            let text = super::fs::read_if_present(&store.directory().join(file)).unwrap();
            assert!(text.is_some(), "{file}");
        }
    }

    #[test]
    fn a_failed_session_writes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().to_path_buf());
        let refused = store.session(NOW, |session| {
            let id = session.fresh_id()?;
            session.edit(Edit::AddShortcut {
                id,
                link: "https://example.com/".to_owned(),
                title: String::new(),
                folder: None,
            })?;
            session.edit(Edit::Retitle {
                id: youarel_core::state::ItemId([9; 16]),
                title: String::new(),
            })
        });
        let error = refused.unwrap_err();
        assert!(matches!(error, StoreError::Edit(_)));
        assert_eq!(error.to_string(), "no such shortcut or folder");
        assert_eq!(
            super::fs::read_if_present(&store.directory().join(STATE_FILE)).unwrap(),
            None
        );
    }

    #[test]
    fn a_damaged_state_names_its_file() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().to_path_buf());
        add(&store, "https://example.com/");
        super::fs::replace(&store.directory().join(STATE_FILE), b"{}").unwrap();
        let error = store.session(NOW, |_| Ok(())).unwrap_err();
        assert!(error.to_string().contains(STATE_FILE), "{error}");
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn later_edits_order_after_earlier_ones_even_when_the_clock_moves_back() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().to_path_buf());
        let id = add(&store, "https://example.com/");
        store
            .session(NOW - 10_000, |session| {
                session.edit(Edit::Retitle {
                    id,
                    title: "renamed".to_owned(),
                })
            })
            .unwrap();
        let title = store
            .session(NOW, |session| {
                Ok(session
                    .state()
                    .shortcut(id)
                    .map(|shortcut| shortcut.title.value.clone()))
            })
            .unwrap();
        assert_eq!(title.as_deref(), Some("renamed"));
    }

    #[test]
    fn a_saved_state_keeps_only_the_icons_its_shortcuts_show() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().to_path_buf());
        let id = add(&store, "https://example.com/");
        let icon = |byte| Some((IconId([byte; 32]), image(byte)));
        for byte in [1, 2] {
            store
                .session(NOW, |session| {
                    session.edit(Edit::SetIcon {
                        id,
                        icon: icon(byte),
                    })
                })
                .unwrap();
        }
        let (kept, shown) = store
            .session(NOW, |session| {
                Ok((
                    session.state().icons.keys().copied().collect::<Vec<_>>(),
                    session.state().icon(id).map(<[u8]>::to_vec),
                ))
            })
            .unwrap();
        assert_eq!(kept, [IconId([2; 32])]);
        assert_eq!(shown, Some(image(2)));
    }

    fn image(byte: u8) -> Vec<u8> {
        vec![byte; 4]
    }
}
