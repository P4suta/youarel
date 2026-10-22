//! A locked session over the store's database: the records it reads and writes, the index rows each write files, and the merges, visits, and icons of a device.
//!
//! A session runs in one write transaction, which commits only when the session changed something and its work succeeded, so a failed session writes nothing.

mod edit;
mod listing;
mod names;
mod publication;
mod query;

use crate::StoreError;
use crate::database::{
    BY_OPENED, DEVICE_KEY, DEVICES, FOLDERS, FORMAT, GENERATION_KEY, ICONS, IMAGES, LATEST_KEY,
    META, SHORTCUTS, VISITS, count, failed, format, generation_bytes, icon_bytes, index, open,
    read_device, read_generation, read_icon, read_stamp, read_visits, stamp_bytes, visits_bytes,
};
use crate::folders::Folders;
use crate::icons::{Icon, IconId, LONGEST_IMAGE};
use redb::{ReadableTable as _, ReadableTableMetadata as _, WriteTransaction};
use std::collections::BTreeMap;
use youarel_core::clock_rules::{self, DeviceId, Stamp};
use youarel_core::host_rules::{self, HostParts};
use youarel_core::index_rules::{self, Delta, Indexed, Row, Standing, Table};
use youarel_core::merge_rules::Register;
use youarel_core::rank_rules::Visits;
use youarel_core::record_rules::{self, DeviceTable};
use youarel_core::search_rules;
use youarel_core::state::{Folder, ItemId, Shortcut, State};
use youarel_core::wire_rules::{Reader, Writer};
use youarel_link::Link;

pub use listing::{
    Group, Grouped, Listing, ON_DEMAND_LIMIT, PATH_SEPARATOR, Part, Parts, Span, Within,
};
pub use publication::{
    PendingPublish, PublicationError, PublicationState, PublishRequest, ReceivedSegment, RecordSet,
    StagedPart,
};
pub use query::{IconBatch, Named};
pub use youarel_core::record_rules::Record;

/// How many rows of one folder a session reads at a time while it moves or removes what the folder holds, which bounds what it keeps in memory.
pub const BATCH: usize = 10_000;

/// The state, the visits, and the icons of one locked read-modify-write, which a session reads through its queries and writes through its edits and merges.
pub struct Session {
    transaction: WriteTransaction,
    device: DeviceId,
    /// The devices by the place each took when the store first met it.
    devices: Vec<DeviceId>,
    table: DeviceTable,
    folders: Folders,
    latest: Option<Stamp>,
    /// How many sessions changed the store before this one.
    generation: u64,
    now: u64,
    changed: bool,
}

impl core::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("device", &self.device)
            .field("devices", &self.devices)
            .field("latest", &self.latest)
            .field("generation", &self.generation)
            .field("now", &self.now)
            .field("changed", &self.changed)
            .finish_non_exhaustive()
    }
}

/// The site of a link: its registrable domain, or the link whole when it names no host, as the window groups it.
pub(crate) fn site(link: &str) -> String {
    Link::parse(link).map_or_else(
        |_unparsed| link.to_owned(),
        |parsed| {
            let HostParts { name, suffix, .. } = host_rules::split(parsed.host());
            format!("{name}{suffix}")
        },
    )
}

/// Overwrites a register with a write at `stamp`, as the merge with that write would.
fn write<T: Ord + Clone>(target: &mut Register<T>, value: T, stamp: Stamp) {
    *target = target.merge(&Register { value, stamp });
}

/// The bytes just after every key that starts with `prefix`, or `None` when no key follows them all.
fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
    search_rules::starting(prefix).end
}

/// Whether a key starts with a prefix.
fn starts(key: &[u8], prefix: &[u8]) -> bool {
    key.starts_with(prefix)
}

impl Session {
    /// The session of a transaction at `now`, with the store's device, its devices, its folders, and its latest stamp read.
    pub(crate) fn open(transaction: WriteTransaction, now: u64) -> Result<Self, StoreError> {
        let format = format(&transaction)?;
        if format != Some(FORMAT) {
            return Err(StoreError::Format(format));
        }
        let (device, latest, generation) = {
            let meta = open(&transaction, META)?;
            let device = meta
                .get(DEVICE_KEY)
                .map_err(failed)?
                .ok_or(StoreError::Missing(DEVICE_KEY))
                .and_then(|held| read_device(held.value()))?;
            let latest = match meta.get(LATEST_KEY).map_err(failed)? {
                Some(held) => Some(read_stamp(held.value())?),
                None => None,
            };
            let generation = match meta.get(GENERATION_KEY).map_err(failed)? {
                Some(held) => read_generation(held.value())?,
                None => 0,
            };
            (device, latest, generation)
        };
        let devices = Self::read_devices(&transaction)?;
        let table = DeviceTable::distinct(devices.clone()).ok_or(StoreError::Missing("devices"))?;
        let mut records = BTreeMap::new();
        for entry in open(&transaction, FOLDERS)?.iter().map_err(failed)? {
            let (key, value) = entry.map_err(failed)?;
            let id = ItemId(key_id(key.value())?);
            records.insert(id, decode_folder(value.value(), &table)?);
        }
        Ok(Self {
            transaction,
            device,
            devices,
            table,
            folders: Folders::new(records),
            latest,
            generation,
            now,
            changed: false,
        })
    }

    /// The devices of the store's table, each at its place, which must run from zero without a gap.
    fn read_devices(transaction: &WriteTransaction) -> Result<Vec<DeviceId>, StoreError> {
        let mut devices = Vec::new();
        for entry in open(transaction, DEVICES)?.iter().map_err(failed)? {
            let (place, device) = entry.map_err(failed)?;
            if !usize::try_from(place.value()).is_ok_and(|place| place == devices.len()) {
                return Err(StoreError::Missing("a place of the devices"));
            }
            devices.push(read_device(device.value())?);
        }
        Ok(devices)
    }

    /// Commits what the session changed, with the latest stamp it reached and one more session that changed the store, or leaves the store as it was when it changed nothing.
    pub(crate) fn finish(self) -> Result<(), StoreError> {
        if !self.changed {
            return self.transaction.abort().map_err(failed);
        }
        {
            let mut meta = open(&self.transaction, META)?;
            if let Some(latest) = self.latest {
                meta.insert(LATEST_KEY, stamp_bytes(latest).as_slice())
                    .map_err(failed)?;
            }
            meta.insert(
                GENERATION_KEY,
                generation_bytes(self.generation()).as_slice(),
            )
            .map_err(failed)?;
        }
        self.transaction.commit().map_err(failed)
    }

    /// How many sessions changed the store, this one included once it changed something, which a reader that keeps what it read compares to know whether it still holds.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        if self.changed {
            self.generation.wrapping_add(1)
        } else {
            self.generation
        }
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

    /// The folders, with the tree of the live ones.
    #[must_use]
    pub const fn folders(&self) -> &Folders {
        &self.folders
    }

    /// The latest stamp of any write the store holds.
    #[must_use]
    pub const fn latest(&self) -> Option<Stamp> {
        self.latest
    }

    /// The stamp of the next write, after every write the store holds.
    fn stamp(&self) -> Result<Stamp, StoreError> {
        let origin = Stamp::origin(self.device);
        let last = self
            .latest
            .map_or(origin, |latest| clock_rules::observe(origin, latest));
        clock_rules::tick(last, self.now).ok_or(StoreError::Clock)
    }

    /// Takes the stamps of a written record into the latest stamp.
    fn observe(&mut self, stamps: impl IntoIterator<Item = Stamp>) {
        self.latest = self.latest.into_iter().chain(stamps).max();
    }

    /// A new identifier: the time and random bits, as a version 7 UUID.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Random`] when the operating system gives no randomness.
    pub fn fresh_id(&self) -> Result<ItemId, StoreError> {
        Ok(ItemId::v7(self.now, crate::random()?))
    }

    /// A shortcut's record, whatever its presence.
    fn record(&self, id: ItemId) -> Result<Option<Shortcut>, StoreError> {
        let shortcuts = open(&self.transaction, SHORTCUTS)?;
        let Some(held) = shortcuts.get(id.0.as_slice()).map_err(failed)? else {
            return Ok(None);
        };
        decode_shortcut(held.value(), &self.table).map(Some)
    }

    /// Adds every device that a record's stamps name to the store's table, each at the next place.
    fn meet(&mut self, stamps: impl IntoIterator<Item = Stamp>) -> Result<(), StoreError> {
        for stamp in stamps {
            if self.table.place(stamp.device).is_some() {
                continue;
            }
            let place = u64::try_from(self.devices.len()).map_err(|_huge| StoreError::Clock)?;
            open(&self.transaction, DEVICES)?
                .insert(place, stamp.device.0.as_slice())
                .map_err(failed)?;
            self.devices.push(stamp.device);
            self.table = DeviceTable::distinct(self.devices.clone())
                .ok_or(StoreError::Missing("devices"))?;
        }
        Ok(())
    }

    /// Writes a shortcut's record in place of the one the store held.
    fn put_shortcut(&mut self, id: ItemId, shortcut: &Shortcut) -> Result<(), StoreError> {
        self.meet(shortcut.stamps())?;
        let mut writer = Writer::new();
        record_rules::encode_shortcut_fields(&mut writer, &self.table, shortcut)
            .map_err(StoreError::Records)?;
        open(&self.transaction, SHORTCUTS)?
            .insert(id.0.as_slice(), writer.finish().as_slice())
            .map_err(failed)?;
        self.observe(shortcut.stamps());
        self.record_dirty(youarel_core::state::Entry::Shortcut(id), shortcut.stamps())?;
        self.changed = true;
        Ok(())
    }

    /// Writes a folder's record in place of the one the store held, and refiles the shortcuts that declare each folder whose standing the record moves: the folder itself, and every folder inside it when it goes into the trash, comes back, moves, or goes for good.
    fn put_folder(&mut self, id: ItemId, folder: Folder) -> Result<(), StoreError> {
        self.meet(folder.stamps())?;
        let mut writer = Writer::new();
        record_rules::encode_folder_fields(&mut writer, &self.table, &folder)
            .map_err(StoreError::Records)?;
        open(&self.transaction, FOLDERS)?
            .insert(id.0.as_slice(), writer.finish().as_slice())
            .map_err(failed)?;
        self.observe(folder.stamps());
        self.record_dirty(youarel_core::state::Entry::Folder(id), folder.stamps())?;
        self.changed = true;
        let before = self.folders.standings();
        self.folders.set(id, folder);
        let after = self.folders.standings();
        for (moved, now) in &after {
            let was = before.get(moved).copied().unwrap_or(Standing::Gone);
            if was != *now {
                self.rehome(*moved, was, *now)?;
            }
        }
        Ok(())
    }

    /// The rows a shortcut files while the folder it declares stands as `folder`.
    fn rows(id: ItemId, shortcut: &Shortcut, folder: Standing) -> Vec<Row> {
        let site = site(&shortcut.link.value);
        index_rules::rows(&Indexed {
            id,
            shortcut,
            folder,
            site: &site,
        })
    }

    /// Takes away the rows a shortcut filed and files the rows it files now, each table opened once.
    fn refile(&self, before: &[Row], after: &[Row]) -> Result<(), StoreError> {
        let Delta { removed, inserted } = index_rules::delta(before, after);
        for (rows, adding) in [(removed, false), (inserted, true)] {
            for group in rows.chunk_by(|left, right| left.table == right.table) {
                let Some(first) = group.first() else {
                    continue;
                };
                if first.table.counts() {
                    self.recount(first.table, group, adding)?;
                } else {
                    let mut table = open(&self.transaction, index(first.table))?;
                    for row in group {
                        if adding {
                            table.insert(row.key.as_slice(), ()).map_err(failed)?;
                        } else {
                            table.remove(row.key.as_slice()).map_err(failed)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Adds one under each row's key of a count table, or takes one away, keeping no key whose count reaches zero.
    fn recount(&self, table: Table, rows: &[Row], adding: bool) -> Result<(), StoreError> {
        let mut counts = open(&self.transaction, count(table))?;
        for row in rows {
            let key = row.key.as_slice();
            let held = counts
                .get(key)
                .map_err(failed)?
                .map_or(0, |held| held.value());
            let next = if adding {
                held.saturating_add(1)
            } else {
                held.saturating_sub(1)
            };
            if next == 0 {
                counts.remove(key).map_err(failed)?;
            } else {
                counts.insert(key, next).map_err(failed)?;
            }
        }
        Ok(())
    }

    /// Rewrites a shortcut's record and refiles its rows, from `before` to `after`, with the standing of the folder each declares.
    fn rewrite(
        &mut self,
        id: ItemId,
        before: Option<&Shortcut>,
        after: &Shortcut,
    ) -> Result<(), StoreError> {
        let old = before.map_or_else(Vec::new, |shortcut| {
            Self::rows(id, shortcut, self.folders.declared(shortcut))
        });
        let new = Self::rows(id, after, self.folders.declared(after));
        self.put_shortcut(id, after)?;
        self.refile(&old, &new)
    }

    /// The identifiers of the rows of an index table under `prefix`, read in batches of [`BATCH`] strictly after the last one read, each batch handed to `each`.
    ///
    /// The reads end once a batch comes back empty, and after as many rounds as the table holds batches of rows at most.
    fn in_batches(
        &mut self,
        table: Table,
        prefix: &[u8],
        mut each: impl FnMut(&mut Self, Vec<ItemId>) -> Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        let end = prefix_end(prefix);
        let held = open(&self.transaction, index(table))?
            .len()
            .map_err(failed)?;
        let rounds = usize::try_from(held)
            .map_err(|_huge| StoreError::Clock)?
            .div_ceil(BATCH);
        let mut after: Vec<u8> = prefix.to_vec();
        let mut first = true;
        for _ in 0..=rounds {
            let batch = {
                let rows = open(&self.transaction, index(table))?;
                let start = if first {
                    std::ops::Bound::Included(after.as_slice())
                } else {
                    std::ops::Bound::Excluded(after.as_slice())
                };
                let finish = end
                    .as_deref()
                    .map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
                let mut keys = Vec::new();
                for entry in rows.range::<&[u8]>((start, finish)).map_err(failed)? {
                    if keys.len() >= BATCH {
                        break;
                    }
                    keys.push(entry.map_err(failed)?.0.value().to_vec());
                }
                keys
            };
            let Some(last) = batch.last() else {
                return Ok(());
            };
            after.clone_from(last);
            first = false;
            let ids = batch
                .iter()
                .filter(|key| starts(key, prefix))
                .map(|key| index_rules::identified(key).ok_or(StoreError::Missing("an identifier")))
                .collect::<Result<Vec<ItemId>, StoreError>>()?;
            each(self, ids)?;
        }
        Ok(())
    }

    /// Refiles every shortcut not gone for good that declares `folder`, whose standing just moved from `was` to `now`: those that sat in it move to the top when it goes, those that waited for it at the top move in when it comes, and those in it hide or show with it.
    ///
    /// A folder the tree held keeps its shortcuts under its home in the rows of the ones that show and of the ones the trash hides, and a folder it did not hold keeps them as orphans.
    /// The rows of the table that the shortcuts move into are read first, so a shortcut that one batch moves never meets the batches after it.
    fn rehome(&mut self, folder: ItemId, was: Standing, now: Standing) -> Result<(), StoreError> {
        let home = index_rules::home_key(Some(folder)).to_vec();
        let sources = match (was, now) {
            (Standing::Gone, _) => vec![(Table::Orphan, folder.0.to_vec())],
            (Standing::Shown | Standing::Hidden, Standing::Hidden) => {
                vec![(Table::Hidden, home.clone()), (Table::Placement, home)]
            }
            (Standing::Shown | Standing::Hidden, Standing::Shown | Standing::Gone) => {
                vec![(Table::Placement, home.clone()), (Table::Hidden, home)]
            }
        };
        for (table, prefix) in sources {
            self.in_batches(table, &prefix, |session, ids| {
                for id in ids {
                    let Some(shortcut) = session.record(id)? else {
                        continue;
                    };
                    let old = Self::rows(id, &shortcut, was);
                    let new = Self::rows(id, &shortcut, now);
                    session.refile(&old, &new)?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    /// Merges one record of another replica into the store, and answers whether the store changed.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read or written.
    pub fn merge_record(&mut self, record: Record) -> Result<bool, StoreError> {
        match record {
            Record::Device(id, theirs) => {
                if !youarel_core::device_rules::admitted_name(&theirs.value) {
                    return Err(StoreError::Records(
                        youarel_core::wire_rules::WireError::Value,
                    ));
                }
                let before = self.device_name(id)?;
                let merged = before
                    .as_ref()
                    .map_or_else(|| theirs.clone(), |mine| mine.merge(&theirs));
                if before.as_ref() == Some(&merged) {
                    return Ok(false);
                }
                self.put_device_name(id, &merged)?;
            }
            Record::Shortcut(id, theirs) => {
                let before = self.record(id)?;
                let merged = before
                    .as_ref()
                    .map_or_else(|| theirs.clone(), |mine| mine.merge(&theirs));
                if before.as_ref() == Some(&merged) {
                    return Ok(false);
                }
                self.rewrite(id, before.as_ref(), &merged)?;
            }
            Record::Folder(id, theirs) => {
                let before = self.folders.records().get(&id).cloned();
                let merged = before
                    .as_ref()
                    .map_or_else(|| theirs.clone(), |mine| mine.merge(&theirs));
                if before.as_ref() == Some(&merged) {
                    return Ok(false);
                }
                self.put_folder(id, merged)?;
            }
        }
        Ok(true)
    }

    /// Merges every record of another replica's state into the store, its folders first, and answers whether the store changed.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read or written.
    pub fn merge(&mut self, other: &State) -> Result<bool, StoreError> {
        let mut changed = false;
        for (id, name) in &other.devices {
            changed |= self.merge_record(Record::Device(*id, name.clone()))?;
        }
        for (id, folder) in &other.folders {
            changed |= self.merge_record(Record::Folder(*id, folder.clone()))?;
        }
        for (id, shortcut) in &other.shortcuts {
            changed |= self.merge_record(Record::Shortcut(*id, shortcut.clone()))?;
        }
        Ok(changed)
    }

    /// Counts one visit of a shortcut now.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read or written.
    pub fn visit(&mut self, id: ItemId) -> Result<(), StoreError> {
        let count = self
            .visits_of(id)?
            .map_or(1, |held| held.count.saturating_add(1));
        self.put_visits(
            id,
            Visits {
                count,
                last: self.now,
            },
        )
    }

    /// Keeps a shortcut's visits as they stand, in place of the ones the store held, with its row by the time it was opened last.
    fn put_visits(&mut self, id: ItemId, visits: Visits) -> Result<(), StoreError> {
        let before = self.visits_of(id)?;
        open(&self.transaction, VISITS)?
            .insert(id.0.as_slice(), visits_bytes(visits).as_slice())
            .map_err(failed)?;
        let mut opened = open(&self.transaction, BY_OPENED)?;
        if let Some(held) = before {
            opened
                .remove(index_rules::added_key(!held.last, id).as_slice())
                .map_err(failed)?;
        }
        opened
            .insert(index_rules::added_key(!visits.last, id).as_slice(), ())
            .map_err(failed)?;
        self.changed = true;
        Ok(())
    }

    /// How often and how lately this device opened a shortcut, or `None` when it never did.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn visits_of(&self, id: ItemId) -> Result<Option<Visits>, StoreError> {
        let visits = open(&self.transaction, VISITS)?;
        visits
            .get(id.0.as_slice())
            .map_err(failed)?
            .map_or(Ok(None), |held| read_visits(held.value()).map(Some))
    }

    /// The icon this device keeps for a host, or `None` when it keeps none.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn icon(&self, host: &str) -> Result<Option<Icon>, StoreError> {
        let icons = open(&self.transaction, ICONS)?;
        match icons.get(host).map_err(failed)? {
            Some(held) => {
                let (image, fetched) = read_icon(held.value())?;
                Ok(Some(Icon {
                    image: IconId(image),
                    fetched,
                }))
            }
            None => Ok(None),
        }
    }

    /// The icon this device keeps for each host.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn icons(&self) -> Result<BTreeMap<String, Icon>, StoreError> {
        let mut found = BTreeMap::new();
        for entry in open(&self.transaction, ICONS)?.iter().map_err(failed)? {
            let (host, held) = entry.map_err(failed)?;
            let (image, fetched) = read_icon(held.value())?;
            found.insert(
                host.value().to_owned(),
                Icon {
                    image: IconId(image),
                    fetched,
                },
            );
        }
        Ok(found)
    }

    /// Keeps an image as the icon of `host`, fetched now, in place of the one the host showed, and lets go of an image that no host shows any more.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Image`] for an image of more than [`LONGEST_IMAGE`] bytes, and another [`StoreError`] when the store cannot be written.
    pub fn keep_icon(&mut self, host: &str, image: IconId, bytes: &[u8]) -> Result<(), StoreError> {
        if u64::try_from(bytes.len()).map_or(true, |length| length > LONGEST_IMAGE) {
            return Err(StoreError::Image(bytes.len()));
        }
        let before = self.icon(host)?;
        let icon = Icon {
            image,
            fetched: self.now,
        };
        self.put_icon(host, icon, bytes)?;
        if let Some(Icon { image: old, .. }) = before
            && old != image
            && !self.icons()?.values().any(|kept| kept.image == old)
        {
            open(&self.transaction, IMAGES)?
                .remove(old.0.as_slice())
                .map_err(failed)?;
        }
        Ok(())
    }

    /// Keeps an icon of a host with its image, in place of the one the host showed.
    fn put_icon(&mut self, host: &str, icon: Icon, bytes: &[u8]) -> Result<(), StoreError> {
        open(&self.transaction, IMAGES)?
            .insert(icon.image.0.as_slice(), bytes)
            .map_err(failed)?;
        open(&self.transaction, ICONS)?
            .insert(host, icon_bytes(&icon.image.0, icon.fetched).as_slice())
            .map_err(failed)?;
        self.changed = true;
        Ok(())
    }
}

/// The identifier a record's key holds.
fn key_id(key: &[u8]) -> Result<[u8; 16], StoreError> {
    key.try_into()
        .map_err(|_unsized| StoreError::Missing("an identifier"))
}

/// A shortcut's record as the store keeps it.
fn decode_shortcut(bytes: &[u8], table: &DeviceTable) -> Result<Shortcut, StoreError> {
    let mut reader = Reader::new(bytes);
    let shortcut =
        record_rules::decode_shortcut_fields(&mut reader, table).map_err(StoreError::Damaged)?;
    reader.finish().map_err(StoreError::Damaged)?;
    Ok(shortcut)
}

/// A folder's record as the store keeps it.
fn decode_folder(bytes: &[u8], table: &DeviceTable) -> Result<Folder, StoreError> {
    let mut reader = Reader::new(bytes);
    let folder =
        record_rules::decode_folder_fields(&mut reader, table).map_err(StoreError::Damaged)?;
    reader.finish().map_err(StoreError::Damaged)?;
    Ok(folder)
}

/// Every table of index rows and of counts.
#[cfg(test)]
const TABLES: [Table; 16] = [
    Table::Placement,
    Table::Added,
    Table::FolderAdded,
    Table::Title,
    Table::FolderTitle,
    Table::Link,
    Table::FolderLink,
    Table::LinkHash,
    Table::Token,
    Table::Site,
    Table::Orphan,
    Table::Hidden,
    Table::Trash,
    Table::HomeCount,
    Table::SiteCount,
    Table::DayCount,
];

#[cfg(test)]
impl Session {
    /// Every key of a table with the count it keeps, or one for a table of index rows.
    fn kept(&self, table: Table) -> BTreeMap<Vec<u8>, u64> {
        let mut kept = BTreeMap::new();
        if table.counts() {
            for entry in open(&self.transaction, count(table))
                .unwrap()
                .iter()
                .unwrap()
            {
                let (key, held) = entry.unwrap();
                kept.insert(key.value().to_vec(), held.value());
            }
        } else {
            for entry in open(&self.transaction, index(table))
                .unwrap()
                .iter()
                .unwrap()
            {
                kept.insert(entry.unwrap().0.value().to_vec(), 1);
            }
        }
        kept
    }

    /// Requires every table of index rows and counts to hold exactly the rows that the records file now, and the row of each visit by its time.
    pub(crate) fn audit(&self) {
        let mut filed: BTreeMap<Table, BTreeMap<Vec<u8>, u64>> = BTreeMap::new();
        for (id, shortcut) in self.records().unwrap().shortcuts {
            for row in Self::rows(id, &shortcut, self.folders.declared(&shortcut)) {
                let held = filed
                    .entry(row.table)
                    .or_default()
                    .entry(row.key)
                    .or_default();
                *held = held.saturating_add(1);
            }
        }
        for table in TABLES {
            let wanted = filed.remove(&table).unwrap_or_default();
            assert_eq!(self.kept(table), wanted, "{table:?}");
        }
        let mut opened = BTreeMap::new();
        for entry in open(&self.transaction, VISITS).unwrap().iter().unwrap() {
            let (key, held) = entry.unwrap();
            let visits = read_visits(held.value()).unwrap();
            let key = index_rules::added_key(!visits.last, ItemId(key_id(key.value()).unwrap()));
            opened.insert(key.to_vec(), 1_u64);
        }
        let mut kept = BTreeMap::new();
        for entry in open(&self.transaction, BY_OPENED).unwrap().iter().unwrap() {
            kept.insert(entry.unwrap().0.value().to_vec(), 1_u64);
        }
        assert_eq!(kept, opened, "the visits by the time of the last");
    }
}
