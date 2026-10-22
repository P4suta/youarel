//! The tables of the store's database, the small values they keep beside the records, and the making of a new database.
//!
//! A record keeps its fields as `record_rules` writes them, with each stamp's device named by its place in the store's own table of devices, which only grows, so a place once written never moves.
//! Every index table keeps the rows that `index_rules` names, as keys with no value, and every count table keeps a count under each key it counts.

use crate::{StoreError, fs, random};
use redb::{ReadableTable as _, Table, TableDefinition, WriteTransaction};
use std::path::{Path, PathBuf};
use youarel_core::clock_rules::{DeviceId, Stamp};
use youarel_core::index_rules::Table as Index;
use youarel_core::rank_rules::Visits;
use youarel_core::wire_rules::{Reader, Writer};

/// The file of the store's database, beside the device's other files.
pub const DATABASE_FILE: &str = "library.redb";
/// The store format that tracks dirty records and pending segments, which earlier writers cannot maintain.
pub const FORMAT: u16 = 4;
/// The most bytes of the database's pages that a session keeps in memory.
pub const CACHE_BYTES: usize = 256 * 1024 * 1024;

/// A table of bytes by bytes.
type Bytes = TableDefinition<'static, &'static [u8], &'static [u8]>;

/// The store's facts about itself: its format, its device, and the latest stamp of any write it holds.
pub(crate) const META: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("meta");
/// The devices whose stamps the records name, by the place each took when the store first met it.
pub(crate) const DEVICES: TableDefinition<'static, u64, &'static [u8]> =
    TableDefinition::new("devices");
/// Every shortcut's record, those in the trash and those gone for good included, by its identifier.
pub(crate) const SHORTCUTS: Bytes = TableDefinition::new("shortcuts");
/// Every folder's record, those in the trash and those gone for good included, by its identifier.
pub(crate) const FOLDERS: Bytes = TableDefinition::new("folders");
pub(crate) const DEVICE_NAMES: Bytes = TableDefinition::new("device_names");
pub(crate) const DIRTY: Bytes = TableDefinition::new("dirty");
pub(crate) const PENDING_BODIES: TableDefinition<'static, u64, &'static [u8]> =
    TableDefinition::new("pending_bodies");
pub(crate) const PENDING_DIGESTS: TableDefinition<'static, u64, &'static [u8]> =
    TableDefinition::new("pending_digests");
pub(crate) const PENDING_SEALED: TableDefinition<'static, u64, &'static [u8]> =
    TableDefinition::new("pending_sealed");
pub(crate) const PEER_CURSORS: Bytes = TableDefinition::new("peer_cursors");
/// How often and how lately this device opened each shortcut, by its identifier.
pub(crate) const VISITS: Bytes = TableDefinition::new("visits");
/// Each visited shortcut by the time this device opened it last, the latest first.
pub(crate) const BY_OPENED: TableDefinition<'static, &'static [u8], ()> =
    TableDefinition::new("by_opened");
/// The image each host shows on this device and when the device fetched it, by the host.
pub(crate) const ICONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("icons");
/// The images of the icons, by their identifiers.
pub(crate) const IMAGES: Bytes = TableDefinition::new("images");

/// The key of the store's format in the facts about itself.
pub(crate) const FORMAT_KEY: &str = "format";
/// The key of the store's device.
pub(crate) const DEVICE_KEY: &str = "device";
/// The key of the latest stamp of any write the store holds.
pub(crate) const LATEST_KEY: &str = "latest";
/// The key of how many sessions changed the store, which a reader that keeps what it read compares to know whether it still holds.
pub(crate) const GENERATION_KEY: &str = "generation";
pub(crate) const PUBLISH_WATERMARK_KEY: &str = "publish_watermark";

/// The bytes of an identifier of a device or an item.
const ID: usize = 16;
/// The bytes of an icon's identifier.
const DIGEST: usize = 32;
/// The ending of the database's name while a new store makes it.
const STAGING: &str = ".staging";

/// The name of the table of the rows of a kind.
pub(crate) const fn name(table: Index) -> &'static str {
    match table {
        Index::Placement => "placement",
        Index::Added => "by_added",
        Index::FolderAdded => "by_folder_added",
        Index::Title => "by_title",
        Index::FolderTitle => "by_folder_title",
        Index::Link => "by_link",
        Index::FolderLink => "by_folder_link",
        Index::LinkHash => "by_link_hash",
        Index::Token => "by_token",
        Index::Site => "by_site",
        Index::Orphan => "orphans",
        Index::Hidden => "hidden",
        Index::Trash => "trash",
        Index::HomeCount => "home_counts",
        Index::SiteCount => "site_counts",
        Index::DayCount => "added_per_day",
    }
}

/// The definition of an index table, which keeps keys alone.
pub(crate) const fn index(table: Index) -> TableDefinition<'static, &'static [u8], ()> {
    TableDefinition::new(name(table))
}

/// The definition of a count table, which keeps a count under each key.
pub(crate) const fn count(table: Index) -> TableDefinition<'static, &'static [u8], u64> {
    TableDefinition::new(name(table))
}

/// A database error as the store reports it.
pub(crate) fn failed(error: impl Into<redb::Error>) -> StoreError {
    StoreError::Database(error.into())
}

/// A table of a transaction, opened for reading and writing.
pub(crate) fn open<'txn, K: redb::Key + 'static, V: redb::Value + 'static>(
    transaction: &'txn WriteTransaction,
    definition: TableDefinition<'_, K, V>,
) -> Result<Table<'txn, K, V>, StoreError> {
    transaction.open_table(definition).map_err(failed)
}

/// A stamp as the store keeps it: the wall time, the counter, and the device.
pub(crate) fn stamp_bytes(stamp: Stamp) -> Vec<u8> {
    let mut writer = Writer::new();
    writer
        .long(stamp.wall)
        .word(stamp.counter)
        .raw(&stamp.device.0);
    writer.finish()
}

/// A stamp as [`stamp_bytes`] keeps it.
pub(crate) fn read_stamp(bytes: &[u8]) -> Result<Stamp, StoreError> {
    let mut reader = Reader::new(bytes);
    let stamp = Stamp {
        wall: reader.long().map_err(StoreError::Damaged)?,
        counter: reader.word().map_err(StoreError::Damaged)?,
        device: DeviceId(reader.array::<ID>().map_err(StoreError::Damaged)?),
    };
    reader.finish().map_err(StoreError::Damaged)?;
    Ok(stamp)
}

/// A count of sessions as the store keeps it.
pub(crate) fn generation_bytes(generation: u64) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.long(generation);
    writer.finish()
}

/// A count of sessions as [`generation_bytes`] keeps it.
pub(crate) fn read_generation(bytes: &[u8]) -> Result<u64, StoreError> {
    let mut reader = Reader::new(bytes);
    let generation = reader.long().map_err(StoreError::Damaged)?;
    reader.finish().map_err(StoreError::Damaged)?;
    Ok(generation)
}

/// Visits as the store keeps them: the count and the time of the latest.
pub(crate) fn visits_bytes(visits: Visits) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.word(visits.count).long(visits.last);
    writer.finish()
}

/// Visits as [`visits_bytes`] keeps them.
pub(crate) fn read_visits(bytes: &[u8]) -> Result<Visits, StoreError> {
    let mut reader = Reader::new(bytes);
    let visits = Visits {
        count: reader.word().map_err(StoreError::Damaged)?,
        last: reader.long().map_err(StoreError::Damaged)?,
    };
    reader.finish().map_err(StoreError::Damaged)?;
    Ok(visits)
}

/// The icon a host shows as the store keeps it: the image's identifier and when the device fetched it.
pub(crate) fn icon_bytes(image: &[u8; DIGEST], fetched: u64) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.raw(image).long(fetched);
    writer.finish()
}

/// An icon as [`icon_bytes`] keeps it.
pub(crate) fn read_icon(bytes: &[u8]) -> Result<([u8; DIGEST], u64), StoreError> {
    let mut reader = Reader::new(bytes);
    let image = reader.array::<DIGEST>().map_err(StoreError::Damaged)?;
    let fetched = reader.long().map_err(StoreError::Damaged)?;
    reader.finish().map_err(StoreError::Damaged)?;
    Ok((image, fetched))
}

/// The device a place of the store's table names.
pub(crate) fn read_device(bytes: &[u8]) -> Result<DeviceId, StoreError> {
    let mut reader = Reader::new(bytes);
    let device = DeviceId(reader.array::<ID>().map_err(StoreError::Damaged)?);
    reader.finish().map_err(StoreError::Damaged)?;
    Ok(device)
}

/// The store's format, as its facts about itself name it, or `None` for a database that names none.
pub(crate) fn format(transaction: &WriteTransaction) -> Result<Option<u16>, StoreError> {
    let meta = open(transaction, META)?;
    let Some(held) = meta.get(FORMAT_KEY).map_err(failed)? else {
        return Ok(None);
    };
    let mut reader = Reader::new(held.value());
    let format = reader.short().map_err(StoreError::Damaged)?;
    reader.finish().map_err(StoreError::Damaged)?;
    Ok(Some(format))
}

/// Names the store's format in its facts about itself.
pub(crate) fn set_format(transaction: &WriteTransaction) -> Result<(), StoreError> {
    let mut meta = open(transaction, META)?;
    meta.insert(FORMAT_KEY, FORMAT.to_be_bytes().as_slice())
        .map_err(failed)?;
    Ok(())
}

/// Makes an empty database at `path` that names a new device and the store's format.
///
/// The database takes its name only once it names both, so a making cut short leaves a staging file, which the next making replaces.
pub(crate) fn create(path: &Path) -> Result<(), StoreError> {
    let mut staged = path.as_os_str().to_owned();
    staged.push(STAGING);
    let staging = PathBuf::from(staged);
    fs::remove_if_present(&staging)?;
    {
        let database = fs::open_database(&staging, CACHE_BYTES)?;
        let transaction = database.begin_write().map_err(failed)?;
        open(&transaction, META)?
            .insert(DEVICE_KEY, random::<ID>()?.as_slice())
            .map_err(failed)?;
        set_format(&transaction)?;
        transaction.commit().map_err(failed)?;
    }
    fs::rename(&staging, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CACHE_BYTES, FORMAT, count, generation_bytes, icon_bytes, index, name, read_device,
        read_generation, read_icon, read_stamp, read_visits, stamp_bytes, visits_bytes,
    };
    use crate::StoreError;
    use redb::TableHandle as _;
    use youarel_core::clock_rules::{DeviceId, Stamp};
    use youarel_core::index_rules::Table;
    use youarel_core::rank_rules::Visits;
    use youarel_core::wire_rules::WireError;

    #[test]
    fn small_values_read_back_as_kept_and_refuse_anything_longer_or_shorter() {
        let stamp = Stamp {
            wall: 1_700_000_000_000,
            counter: 3,
            device: DeviceId([7; 16]),
        };
        let kept = stamp_bytes(stamp);
        assert_eq!(kept.len(), 28);
        assert_eq!(read_stamp(&kept).unwrap(), stamp);
        let visits = Visits { count: 2, last: 9 };
        assert_eq!(read_visits(&visits_bytes(visits)).unwrap(), visits);
        let icon = icon_bytes(&[5; 32], 11);
        assert_eq!(read_icon(&icon).unwrap(), ([5; 32], 11));
        assert_eq!(read_device(&[4; 16]).unwrap(), DeviceId([4; 16]));
        let generation = generation_bytes(u64::MAX - 3);
        assert_eq!(generation.len(), 8);
        assert_eq!(read_generation(&generation).unwrap(), u64::MAX - 3);
        for damaged in [
            read_generation(&generation[1..]).map(|_| ()),
            read_generation(&[generation.as_slice(), &[0]].concat()).map(|_| ()),
            read_stamp(&kept[1..]).map(|_| ()),
            read_stamp(&[kept.as_slice(), &[0]].concat()).map(|_| ()),
            read_visits(&[0; 11]).map(|_| ()),
            read_visits(&[0; 13]).map(|_| ()),
            read_icon(&icon[1..]).map(|_| ()),
            read_icon(&[icon.as_slice(), &[0]].concat()).map(|_| ()),
            read_device(&[4; 15]).map(|_| ()),
            read_device(&[4; 17]).map(|_| ()),
        ] {
            assert!(matches!(
                damaged,
                Err(StoreError::Damaged(
                    WireError::Truncated | WireError::Trailing
                ))
            ));
        }
    }

    #[test]
    fn every_table_has_a_name_of_its_own() {
        let tables = [
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
        let mut names: Vec<&str> = tables.iter().map(|table| name(*table)).collect();
        assert_eq!(names[0], "placement");
        assert_eq!(index(Table::Placement).name(), "placement");
        assert_eq!(count(Table::DayCount).name(), "added_per_day");
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), tables.len());
        assert_eq!((FORMAT, CACHE_BYTES), (4, 256 * 1024 * 1024));
    }
}
