//! youarel's files on one device: the replicated state, the device's identity, its own visits, and the icons it gathered, read and written under one lock.
//!
//! Every read-modify-write runs in a [`Session`] under the lock, so a terminal interface and a command that run at once never lose each other's edits.

pub mod codec;
pub mod fs;
pub mod icons;
pub mod records;
pub mod settings;

use crate::codec::{DecodeError, Shown};
use crate::fs::{FileError, Lock};
use crate::icons::{Icon, IconId, LONGEST_IMAGE};
use crate::settings::Settings;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, btree_map};
use std::path::{Path, PathBuf};
use youarel_core::clock_rules::{self, DeviceId, Stamp};
use youarel_core::rank_rules::Visits;
use youarel_core::state::{Edit, EditError, ItemId, State};
use youarel_core::sync_rules::Carried;
use youarel_core::wire_rules::WireError;
use youarel_link::Link;

/// The replicated state, as its binary records.
pub const STATE_FILE: &str = "state.v2";
/// The replicated state in the JSON form of earlier builds, which a store reads once and then keeps as [`MIGRATED_STATE_FILE`].
pub const LEGACY_STATE_FILE: &str = "state.json";
/// The JSON form of the state, kept once the store wrote the state's records.
pub const MIGRATED_STATE_FILE: &str = "state.json.migrated";
/// The device's identity, which stays on the device.
pub const DEVICE_FILE: &str = "device.json";
/// The device's visits, which stay on the device.
pub const VISITS_FILE: &str = "visits.json";
/// The file whose lock serializes the sessions of every process.
pub const LOCK_FILE: &str = "lock";
/// What the device's last publish carried and the target it reached, which stay on the device.
pub const PUBLISHED_FILE: &str = "published.json";
/// The file whose lock lets one process at a time publish the device's bundle and record what it carried.
pub const PUBLISH_LOCK_FILE: &str = "publish.lock";

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
    /// The window's settings hold what this youarel would not have written.
    Settings {
        path: PathBuf,
        error: settings::DecodeError,
    },
    /// The index of the icons holds what this youarel would not have written.
    Icons {
        path: PathBuf,
        error: icons::DecodeError,
    },
    /// An icon's image holds more bytes than the device keeps, which counts them.
    Image(usize),
    Encode(serde_json::Error),
    /// The state does not write as records.
    Records(WireError),
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
            Self::Decode { path, .. }
            | Self::Local { path, .. }
            | Self::Settings { path, .. }
            | Self::Icons { path, .. } => {
                write!(formatter, "cannot read {}", path.display())
            }
            Self::Image(length) => write!(
                formatter,
                "an icon's image of {length} bytes passes the {LONGEST_IMAGE} bytes the device keeps"
            ),
            Self::Encode(_) | Self::Records(_) => formatter.write_str("cannot write the state"),
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
            Self::Settings { error, .. } => Some(error),
            Self::Icons { error, .. } => Some(error),
            Self::Records(error) => Some(error),
            Self::Local { error, .. } | Self::Encode(error) => Some(error),
            Self::Image(_) | Self::Random(_) | Self::Edit(_) | Self::Clock => None,
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

/// The state, the visits, and the icons of one locked read-modify-write.
#[derive(Debug)]
pub struct Session {
    state: State,
    visits: BTreeMap<ItemId, Visits>,
    icons: BTreeMap<String, Icon>,
    /// The images the session kept, which reach the disk before the index names them.
    images: Vec<(IconId, Vec<u8>)>,
    device: DeviceId,
    now: u64,
    changed: bool,
    visited: bool,
    iconed: bool,
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

/// Gives each host that shows no icon yet the icon its live shortcuts of an older document chose last, and answers with the images to write.
fn adopt<'a>(shown: &'a [Shown], icons: &mut BTreeMap<String, Icon>) -> Vec<(IconId, &'a [u8])> {
    let mut latest: BTreeMap<String, &Shown> = BTreeMap::new();
    for each in shown {
        // The document admitted every link, so a link that does not parse again names no host to keep an icon for.
        let Ok(link) = Link::parse(&each.link) else {
            continue;
        };
        let host = link.host().to_owned();
        if latest
            .get(&host)
            .is_none_or(|chosen| chosen.chosen < each.chosen)
        {
            latest.insert(host, each);
        }
    }
    let mut images = Vec::new();
    for (host, each) in latest {
        if let btree_map::Entry::Vacant(vacant) = icons.entry(host) {
            vacant.insert(Icon {
                image: each.image,
                fetched: each.chosen.wall,
            });
            images.push((each.image, each.bytes.as_slice()));
        }
    }
    images
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

    /// The state, and the icon this device keeps for each host.
    ///
    /// A store that holds only the JSON form of earlier builds moves the icons it names into the device's own, writes the state's records, and keeps the JSON form under another name, once.
    /// The move writes the images and the index before the records, so a move cut short before the records starts again with the next session.
    fn load(&self) -> Result<(State, BTreeMap<String, Icon>), StoreError> {
        let mut icons = self.icons()?;
        let path = self.directory.join(STATE_FILE);
        if let Some(bytes) = fs::read_bytes_if_present(&path)? {
            let state =
                records::decode(&bytes).map_err(|error| StoreError::Decode { path, error })?;
            return Ok((state, icons));
        }
        let legacy = self.directory.join(LEGACY_STATE_FILE);
        let Some(text) = fs::read_if_present(&legacy)? else {
            return Ok((State::default(), icons));
        };
        let decoded = codec::decode(&text).map_err(|error| StoreError::Decode {
            path: legacy.clone(),
            error,
        })?;
        let adopted = adopt(&decoded.icons, &mut icons);
        fs::create_dir_all(&self.directory.join(icons::IMAGES_DIRECTORY))?;
        for (image, bytes) in adopted {
            fs::replace(&self.image_path(image), bytes)?;
        }
        let index = icons::encode(&icons).map_err(StoreError::Encode)?;
        fs::replace(&self.directory.join(icons::INDEX_FILE), index.as_bytes())?;
        let written = records::encode(&decoded.state).map_err(StoreError::Records)?;
        fs::replace(&path, &written)?;
        fs::rename(&legacy, &self.directory.join(MIGRATED_STATE_FILE))?;
        Ok((decoded.state, icons))
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

    fn icons(&self) -> Result<BTreeMap<String, Icon>, StoreError> {
        let path = self.directory.join(icons::INDEX_FILE);
        fs::read_if_present(&path)?.map_or_else(
            || Ok(BTreeMap::new()),
            |text| icons::decode(&text).map_err(|error| StoreError::Icons { path, error }),
        )
    }

    fn image_path(&self, image: IconId) -> PathBuf {
        self.directory
            .join(icons::IMAGES_DIRECTORY)
            .join(icons::file_name(image))
    }

    /// The image an icon's identifier names, read under the lock, or `None` when the device keeps no such image.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the lock cannot be taken or the image cannot be read, or holds more than [`LONGEST_IMAGE`] bytes.
    pub fn image(&self, image: IconId) -> Result<Option<Vec<u8>>, StoreError> {
        fs::create_dir_all(&self.directory)?;
        let _lock = Lock::acquire(&self.directory.join(LOCK_FILE))?;
        Ok(fs::read_bounded_if_present(
            &self.image_path(image),
            LONGEST_IMAGE,
        )?)
    }

    /// Writes the images a session kept that its index names, then the index, then removes each image that no host names any more.
    fn save_icons(&self, held: &BTreeSet<IconId>, session: &Session) -> Result<(), StoreError> {
        let shown: BTreeSet<IconId> = session.icons.values().map(|icon| icon.image).collect();
        fs::create_dir_all(&self.directory.join(icons::IMAGES_DIRECTORY))?;
        for (image, bytes) in &session.images {
            if shown.contains(image) {
                fs::replace(&self.image_path(*image), bytes)?;
            }
        }
        let text = icons::encode(&session.icons).map_err(StoreError::Encode)?;
        fs::replace(&self.directory.join(icons::INDEX_FILE), text.as_bytes())?;
        for gone in held.difference(&shown) {
            fs::remove_if_present(&self.image_path(*gone))?;
        }
        Ok(())
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
        let (state, icons) = self.load()?;
        let held: BTreeSet<IconId> = icons.values().map(|icon| icon.image).collect();
        let mut session = Session {
            device: self.device()?,
            state,
            visits: self.visits()?,
            icons,
            images: Vec::new(),
            now,
            changed: false,
            visited: false,
            iconed: false,
        };
        let result = work(&mut session)?;
        if session.iconed {
            self.save_icons(&held, &session)?;
        }
        if session.changed {
            let written = records::encode(&session.state).map_err(StoreError::Records)?;
            fs::replace(&self.directory.join(STATE_FILE), &written)?;
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

    /// The icon this device keeps for each host.
    #[must_use]
    pub const fn icons(&self) -> &BTreeMap<String, Icon> {
        &self.icons
    }

    /// Keeps an image as the icon of `host`, fetched now, in place of the one the host showed.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Image`] for an image of more than [`LONGEST_IMAGE`] bytes.
    pub fn keep_icon(
        &mut self,
        host: &str,
        image: IconId,
        bytes: Vec<u8>,
    ) -> Result<(), StoreError> {
        if u64::try_from(bytes.len()).map_or(true, |length| length > LONGEST_IMAGE) {
            return Err(StoreError::Image(bytes.len()));
        }
        self.icons.insert(
            host.to_owned(),
            Icon {
                image,
                fetched: self.now,
            },
        );
        self.images.push((image, bytes));
        self.iconed = true;
        Ok(())
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
    use super::{
        DEVICE_FILE, LEGACY_STATE_FILE, MIGRATED_STATE_FILE, PUBLISH_LOCK_FILE, PUBLISHED_FILE,
        STATE_FILE, Store, StoreError, VISITS_FILE, icons,
    };
    use youarel_core::state::{Edit, Entry, ItemId, State};

    const NOW: u64 = 1_700_000_000_000;

    fn add(store: &Store, link: &str) -> ItemId {
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
            let bytes = super::fs::read_bytes_if_present(&store.directory().join(file)).unwrap();
            assert!(bytes.is_some(), "{file}");
        }
        let records = super::fs::read_bytes_if_present(&store.directory().join(STATE_FILE));
        let state = super::records::decode(&records.unwrap().unwrap()).unwrap();
        assert_eq!(state.shortcuts.len(), 2, "the state is kept as its records");
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
                id: ItemId([9; 16]),
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
        super::fs::remove_if_present(&store.directory().join(STATE_FILE)).unwrap();
        super::fs::replace(&store.directory().join(LEGACY_STATE_FILE), b"{").unwrap();
        let legacy = store.session(NOW, |_| Ok(())).unwrap_err();
        assert!(legacy.to_string().contains(LEGACY_STATE_FILE), "{legacy}");
        let file = |name: &str| super::fs::read_if_present(&store.directory().join(name));
        assert_eq!(
            (
                file(STATE_FILE).unwrap(),
                file(MIGRATED_STATE_FILE).unwrap()
            ),
            (None, None),
            "a damaged JSON form moves nowhere"
        );
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
    fn the_window_s_settings_default_save_read_back_and_name_a_damaged_file() {
        use super::settings::{SETTINGS_FILE, Settings};
        use youarel_core::library_rules::{Layout, Sort};
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().join("youarel"));
        assert_eq!(store.settings().unwrap(), Settings::default());
        let chosen = Settings {
            layout: Layout::List,
            sort: Sort::Title,
            ..Settings::default()
        };
        store.save_settings(&chosen).unwrap();
        assert_eq!(store.settings().unwrap(), chosen);
        super::fs::replace(&store.directory().join(SETTINGS_FILE), b"{}").unwrap();
        let error = store.settings().unwrap_err();
        assert!(error.to_string().contains(SETTINGS_FILE), "{error}");
        assert!(std::error::Error::source(&error).is_some());
    }

    /// Keeps an icon for a host in one session at `now`, its image and its identifier spelled by one byte.
    fn keep(store: &Store, now: u64, host: &str, image: u8) {
        store
            .session(now, |session| {
                session.keep_icon(host, icons::IconId([image; 32]), picture(image))
            })
            .unwrap();
    }

    fn picture(image: u8) -> Vec<u8> {
        vec![image; 8]
    }

    /// The image each host shows, and when the device fetched it.
    fn shown(store: &Store) -> Vec<(String, u8, u64)> {
        store
            .session(NOW, |session| {
                Ok(session
                    .icons()
                    .iter()
                    .map(|(host, icon)| (host.clone(), icon.image.0[0], icon.fetched))
                    .collect())
            })
            .unwrap()
    }

    #[test]
    fn an_icon_kept_for_a_host_shows_in_the_next_session_and_its_image_reads_back() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().join("youarel"));
        assert_eq!(shown(&store), []);
        keep(&store, NOW + 5, "example.com", 1);
        assert_eq!(shown(&store), [("example.com".to_owned(), 1, NOW + 5)]);
        assert_eq!(
            store.image(icons::IconId([1; 32])).unwrap(),
            Some(picture(1))
        );
        assert_eq!(store.image(icons::IconId([2; 32])).unwrap(), None);
        let index = super::fs::read_if_present(&store.directory().join(icons::INDEX_FILE));
        assert!(
            index
                .unwrap()
                .is_some_and(|text| text.contains("example.com"))
        );
    }

    #[test]
    fn a_new_icon_for_a_host_takes_its_place_and_an_image_no_host_shows_goes() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().to_path_buf());
        let held = |image: u8| store.image(icons::IconId([image; 32])).unwrap().is_some();
        keep(&store, NOW, "a.example", 1);
        keep(&store, NOW, "b.example", 1);
        keep(&store, NOW, "a.example", 2);
        assert!(
            held(1) && held(2),
            "another host still shows the first image"
        );
        keep(&store, NOW, "b.example", 3);
        assert!(
            !held(1) && held(2) && held(3),
            "no host shows the first image"
        );
        store
            .session(NOW, |session| {
                session.keep_icon("c.example", icons::IconId([4; 32]), picture(4))?;
                session.keep_icon("c.example", icons::IconId([5; 32]), picture(5))
            })
            .unwrap();
        assert!(
            !held(4) && held(5),
            "an image replaced in its session never lands"
        );
        assert_eq!(
            shown(&store)
                .into_iter()
                .map(|(host, image, _)| (host, image))
                .collect::<Vec<_>>(),
            [
                ("a.example".to_owned(), 2),
                ("b.example".to_owned(), 3),
                ("c.example".to_owned(), 5)
            ]
        );
    }

    #[test]
    fn an_image_beyond_the_limit_is_refused_and_a_failed_session_keeps_no_icon() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().to_path_buf());
        let longest = usize::try_from(icons::LONGEST_IMAGE).unwrap();
        let refused = store
            .session(NOW, |session| {
                session.keep_icon("a.example", icons::IconId([1; 32]), vec![0; longest + 1])
            })
            .unwrap_err();
        assert!(matches!(refused, StoreError::Image(length) if length == longest + 1));
        assert!(refused.to_string().contains("passes"), "{refused}");
        assert!(std::error::Error::source(&refused).is_none());
        let failed = store.session(NOW, |session| {
            session.keep_icon("a.example", icons::IconId([2; 32]), vec![0; longest])?;
            Err::<(), _>(StoreError::Clock)
        });
        assert!(failed.is_err());
        assert_eq!(shown(&store), []);
        assert_eq!(store.image(icons::IconId([2; 32])).unwrap(), None);
        store
            .session(NOW, |session| {
                session.keep_icon("a.example", icons::IconId([2; 32]), vec![0; longest])
            })
            .unwrap();
        assert_eq!(
            store
                .image(icons::IconId([2; 32]))
                .unwrap()
                .map(|image| image.len()),
            Some(longest)
        );
    }

    /// Rewrites the store's state as a build from before icons left it wrote it, in the JSON form with no records beside it, and returns the state and the document.
    ///
    /// Each listed shortcut chose the icon its byte spells at its wall time, and the document holds those icons' images.
    fn as_before_icons_left(store: &Store, chosen: &[(ItemId, u8, u64)]) -> (State, String) {
        use base64::Engine as _;
        let state = store
            .session(NOW, |session| Ok(session.state().clone()))
            .unwrap();
        let text = super::codec::encode(&state).unwrap();
        let mut document: serde_json::Value = serde_json::from_str(&text).unwrap();
        for shortcut in document["shortcuts"].as_array_mut().unwrap() {
            let id = shortcut["id"].as_str().unwrap().to_owned();
            let Some((_, image, wall)) = chosen
                .iter()
                .find(|(item, _, _)| super::codec::hex(&item.0) == id)
            else {
                continue;
            };
            let register = serde_json::json!({
                "value": icons::IconId([*image; 32]).name(),
                "stamp": { "wall": wall, "counter": 0, "device": "07".repeat(16) },
            });
            let fields = shortcut.as_object_mut().unwrap();
            fields.insert("icon".to_owned(), register);
        }
        let images: Vec<serde_json::Value> = chosen
            .iter()
            .map(|(_, image, _)| {
                serde_json::json!({
                    "id": icons::IconId([*image; 32]).name(),
                    "image": base64::engine::general_purpose::STANDARD.encode(picture(*image)),
                })
            })
            .collect();
        let fields = document.as_object_mut().unwrap();
        fields.insert("icons".to_owned(), serde_json::Value::from(images));
        let older = serde_json::to_string_pretty(&document).unwrap();
        super::fs::remove_if_present(&store.directory().join(STATE_FILE)).unwrap();
        super::fs::replace(&store.directory().join(LEGACY_STATE_FILE), older.as_bytes()).unwrap();
        (state, older)
    }

    /// Fills a store with eight shortcuts, one of them removed, and an icon of the device's own for one host, then rewrites it as a build from before icons left the state wrote it.
    fn older_store(store: &Store) -> (State, String) {
        let links = [
            "https://a.example/one",
            "https://a.example/two",
            "https://b.example/",
            "https://c.example/",
            "https://d.example/later",
            "https://d.example/earlier",
            "https://e.example/first",
            "https://e.example/second",
        ];
        let ids: Vec<ItemId> = (1..=8).map(|byte| ItemId([byte; 16])).collect();
        store
            .session(NOW, |session| {
                for (id, link) in ids.iter().zip(links) {
                    session.edit(Edit::AddShortcut {
                        id: *id,
                        link: link.to_owned(),
                        title: link.to_owned(),
                        folder: None,
                    })?;
                }
                session.edit(Edit::Remove {
                    entry: Entry::Shortcut(ids[3]),
                })
            })
            .unwrap();
        keep(store, NOW, "b.example", 9);
        let chosen: Vec<(ItemId, u8, u64)> = ids
            .iter()
            .zip([
                (1, 10),
                (2, 20),
                (3, 30),
                (4, 40),
                (5, 50),
                (6, 40),
                (7, 60),
                (8, 60),
            ])
            .map(|(id, (image, wall))| (*id, image, wall))
            .collect();
        as_before_icons_left(store, &chosen)
    }

    #[test]
    fn a_state_from_before_icons_left_it_hands_its_icons_to_the_device_once() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().to_path_buf());
        older_store(&store);
        assert_eq!(
            shown(&store),
            [
                ("a.example".to_owned(), 2, 20),
                ("b.example".to_owned(), 9, NOW),
                ("d.example".to_owned(), 5, 50),
                ("e.example".to_owned(), 7, 60)
            ],
            "the latest choice of a host's live shortcuts, the first of a tie, and no host that shows an icon already"
        );
        let held = |image: u8| store.image(icons::IconId([image; 32])).unwrap();
        assert_eq!(
            (held(2), held(5), held(7)),
            (Some(picture(2)), Some(picture(5)), Some(picture(7)))
        );
        assert_eq!(
            [1, 3, 4, 6, 8].map(held),
            [None, None, None, None, None],
            "no image that no host shows lands"
        );
    }

    #[test]
    fn a_state_in_the_json_form_moves_into_its_records_once() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().to_path_buf());
        let (state, older) = older_store(&store);
        store.session(NOW, |_| Ok(())).unwrap();
        let file =
            |name: &str| super::fs::read_bytes_if_present(&store.directory().join(name)).unwrap();
        let records = file(STATE_FILE).unwrap();
        assert_eq!(super::records::decode(&records).unwrap(), state);
        assert_eq!(
            (file(LEGACY_STATE_FILE), file(MIGRATED_STATE_FILE)),
            (None, Some(older.clone().into_bytes())),
            "the JSON form stays as it was under another name"
        );
        super::fs::replace(&store.directory().join(LEGACY_STATE_FILE), b"{").unwrap();
        store.session(NOW, |_| Ok(())).unwrap();
        assert_eq!(
            (
                file(STATE_FILE),
                file(LEGACY_STATE_FILE),
                file(MIGRATED_STATE_FILE)
            ),
            (Some(records), Some(b"{".to_vec()), Some(older.into_bytes())),
            "the move happens once, and the records outrank a JSON form an older build wrote again"
        );
    }

    #[test]
    fn the_record_of_a_publish_reads_back_for_its_target_alone() {
        use youarel_core::clock_rules::{DeviceId, Stamp};
        use youarel_core::sync_rules::Carried;
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().join("youarel"));
        assert_eq!(store.published("/shared").unwrap(), None, "no publish yet");
        let carried = Carried {
            latest: Some(Stamp {
                wall: NOW,
                counter: 2,
                device: DeviceId([7; 16]),
            }),
            operations: 4,
        };
        store.record_published("/shared", carried).unwrap();
        assert_eq!(store.published("/shared").unwrap(), Some(carried));
        assert_eq!(
            store.published("/shared/").unwrap(),
            None,
            "a record names its target exactly"
        );
        let empty = Carried {
            latest: None,
            operations: 0,
        };
        store
            .record_published("https://relay.example", empty)
            .unwrap();
        assert_eq!(
            store.published("https://relay.example").unwrap(),
            Some(empty)
        );
        assert_eq!(
            store.published("/shared").unwrap(),
            None,
            "the record of the publish before is gone"
        );
        // Windows refuses to read a file another handle holds locked, so the test looks for the lock's name alone.
        let held = store.publishing().unwrap();
        let names = super::fs::list(store.directory()).unwrap();
        assert!(
            names.iter().any(|name| name == PUBLISH_LOCK_FILE),
            "the hold names its file: {names:?}"
        );
        drop(held);
        drop(store.publishing().unwrap());
    }

    #[test]
    fn a_damaged_record_of_a_publish_names_its_file() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().to_path_buf());
        let path = store.directory().join(PUBLISHED_FILE);
        super::fs::replace(&path, b"{}").unwrap();
        let unshaped = store.published("/shared").unwrap_err();
        assert!(matches!(unshaped, StoreError::Local { .. }));
        assert!(unshaped.to_string().contains(PUBLISHED_FILE), "{unshaped}");
        let misnamed =
            br#"{"target":"/shared","latest":{"wall":1,"counter":0,"device":"zz"},"operations":1}"#;
        super::fs::replace(&path, misnamed).unwrap();
        let undecoded = store.published("/shared").unwrap_err();
        assert!(matches!(undecoded, StoreError::Decode { .. }));
        assert!(
            undecoded.to_string().contains(PUBLISHED_FILE),
            "{undecoded}"
        );
    }

    #[test]
    fn a_damaged_index_of_icons_names_its_file() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().to_path_buf());
        keep(&store, NOW, "a.example", 1);
        super::fs::replace(&store.directory().join(icons::INDEX_FILE), b"{}").unwrap();
        let error = store.session(NOW, |_| Ok(())).unwrap_err();
        assert!(matches!(error, StoreError::Icons { .. }));
        assert!(error.to_string().contains(icons::INDEX_FILE), "{error}");
        assert!(std::error::Error::source(&error).is_some());
    }

    /// Writes indexes of icons into the seed corpus of their fuzz target, which `cargo nextest run --run-ignored only seeds` regenerates.
    #[test]
    #[ignore = "writes the committed seeds of the fuzz corpus; run it to regenerate them"]
    fn seeds() {
        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/corpus/icon_index_decode");
        super::fs::create_dir_all(&corpus).unwrap();
        let one = std::collections::BTreeMap::from([(
            "example.com".to_owned(),
            icons::Icon {
                image: icons::IconId([0xab; 32]),
                fetched: NOW,
            },
        )]);
        let mut two = one.clone();
        two.insert(
            "[::1]".to_owned(),
            icons::Icon {
                image: icons::IconId([0x01; 32]),
                fetched: 0,
            },
        );
        let seeds = [
            ("empty", std::collections::BTreeMap::new()),
            ("one-host", one),
            ("two-hosts", two),
        ];
        for (name, index) in seeds {
            let text = icons::encode(&index).unwrap();
            assert_eq!(icons::decode(&text).unwrap(), index, "{name}");
            super::fs::replace(&corpus.join(name), text.as_bytes()).unwrap();
        }
    }
}
