//! A session's other queries: a shortcut, the entries of a folder, the counts, the holder of a link, the shortcuts a name names, the newest write of a device, and every record, each read through the records and the index rows.

use super::{Session, decode_shortcut};
use crate::StoreError;
use crate::database::{SHORTCUTS, count, failed, index, open};
use redb::{ReadableTable as _, ReadableTableMetadata as _};
use std::collections::BTreeSet;
use std::ops::Bound;
use youarel_core::clock_rules::{DeviceId, Stamp};
use youarel_core::index_rules::{self, SORT_PREFIX, Standing, Table};
use youarel_core::library_rules::{self, Counts};
use youarel_core::record_rules::Body;
use youarel_core::search_rules::{self, Bounds};
use youarel_core::state::{Entry, Folder, ItemId, Presence, Shortcut};

/// Shortcuts that show, each with its record, in the order of their identifiers.
pub type Named = Vec<(ItemId, Shortcut)>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IconBatch {
    pub after: Option<ItemId>,
    pub links: Vec<youarel_link::Link>,
    pub more: bool,
}

/// The identifier at the end of a key.
fn id(key: &[u8]) -> Result<ItemId, StoreError> {
    index_rules::identified(key).ok_or(StoreError::Missing("an identifier"))
}

impl Session {
    /// Reads a bounded identifier range and returns one link per missing live host.
    ///
    /// # Errors
    ///
    /// Refuses unreadable records or an invalid stored link.
    pub fn icon_batch(&self, after: Option<ItemId>) -> Result<IconBatch, StoreError> {
        use youarel_core::icon_rules::{HOSTS_PER_TICK, RECORDS_PER_TICK};
        use youarel_link::Link;
        let shortcuts = open(&self.transaction, SHORTCUTS)?;
        let start = after
            .as_ref()
            .map_or(Bound::Unbounded, |id| Bound::Excluded(id.0.as_slice()));
        let mut rows = shortcuts
            .range::<&[u8]>((start, Bound::Unbounded))
            .map_err(failed)?;
        let mut batch = IconBatch::default();
        let mut hosts = BTreeSet::new();
        for row in rows.by_ref().take(RECORDS_PER_TICK) {
            let (key, value) = row.map_err(failed)?;
            batch.after = Some(ItemId(super::key_id(key.value())?));
            let shortcut = decode_shortcut(value.value(), &self.table)?;
            if self.folders.standing(&shortcut) != Standing::Shown {
                continue;
            }
            let link = Link::parse(&shortcut.link.value)
                .map_err(|_invalid| StoreError::Missing("an admitted link"))?;
            if hosts.insert(link.host().to_owned()) && self.icon(link.host())?.is_none() {
                batch.links.push(link);
                if batch.links.len() >= HOSTS_PER_TICK {
                    break;
                }
            }
        }
        batch.more = rows.next().transpose().map_err(failed)?.is_some();
        Ok(batch)
    }

    /// The keys of an index table from `start` on, before `end` when there is one.
    fn keys(
        &self,
        table: Table,
        start: &[u8],
        end: Option<&[u8]>,
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        let rows = open(&self.transaction, index(table))?;
        let finish = end.map_or(Bound::Unbounded, Bound::Excluded);
        let mut keys = Vec::new();
        for entry in rows
            .range::<&[u8]>((Bound::Included(start), finish))
            .map_err(failed)?
        {
            keys.push(entry.map_err(failed)?.0.value().to_vec());
        }
        Ok(keys)
    }

    /// The keys of an index table that start with `prefix`.
    fn under(&self, table: Table, prefix: &[u8]) -> Result<Vec<Vec<u8>>, StoreError> {
        self.keys(table, prefix, super::prefix_end(prefix).as_deref())
    }

    /// A shortcut that shows: live, at the top or in a folder that shows.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn item(&self, id: ItemId) -> Result<Option<Shortcut>, StoreError> {
        Ok(self
            .record(id)?
            .filter(|shortcut| self.folders.standing(shortcut) == Standing::Shown))
    }

    /// A shortcut in the trash itself.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn trashed(&self, id: ItemId) -> Result<Option<Shortcut>, StoreError> {
        Ok(self
            .record(id)?
            .filter(|shortcut| shortcut.presence.value == Presence::Trashed))
    }

    /// The presence of the item an entry names, or `None` when the store holds no record of it.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn presence_of(&self, entry: Entry) -> Result<Option<Presence>, StoreError> {
        Ok(match entry {
            Entry::Folder(id) => self
                .folders
                .records()
                .get(&id)
                .map(|folder| folder.presence.value),
            Entry::Shortcut(id) => self.record(id)?.map(|shortcut| shortcut.presence.value),
        })
    }

    /// Every record of a shortcut, those in the trash and those gone for good included, in the order of their identifiers.
    fn all_shortcuts(&self) -> Result<Vec<(ItemId, Shortcut)>, StoreError> {
        let mut found = Vec::new();
        for entry in open(&self.transaction, SHORTCUTS)?.iter().map_err(failed)? {
            let (key, value) = entry.map_err(failed)?;
            let id = ItemId(super::key_id(key.value())?);
            found.push((id, decode_shortcut(value.value(), &self.table)?));
        }
        Ok(found)
    }

    /// Every shortcut that shows, in the order of their identifiers, read from every record the store holds.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub(crate) fn shortcuts(&self) -> Result<Named, StoreError> {
        let mut shown = self.all_shortcuts()?;
        shown.retain(|(_, shortcut)| self.folders.standing(shortcut) == Standing::Shown);
        Ok(shown)
    }

    /// The entries that show in a folder, or in the top, in their order.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn entries(&self, folder: Option<ItemId>) -> Result<Vec<Entry>, StoreError> {
        self.entries_page(folder, 0, usize::MAX)
    }

    /// How many shortcuts that show each scope holds, and how many items wait in the trash.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn counts(&self) -> Result<Counts, StoreError> {
        let mut counts = Counts::default();
        let top = index_rules::home_key(None);
        for entry in open(&self.transaction, count(Table::HomeCount))?
            .iter()
            .map_err(failed)?
        {
            let (key, held) = entry.map_err(failed)?;
            let held =
                usize::try_from(held.value()).map_err(|_huge| StoreError::Missing("a count"))?;
            counts.all = counts.all.saturating_add(held);
            let key = key.value();
            if key == top {
                counts.unsorted = held;
                continue;
            }
            let folder = ItemId(super::key_id(key.get(1..).unwrap_or_default())?);
            for inside in self.folders.tree().lineage(Some(folder)) {
                let total = counts.collections.entry(inside).or_default();
                *total = total.saturating_add(held);
            }
        }
        counts.recent = self.added_since(library_rules::recent_since(self.now))?;
        counts.trash = self.trash_count()?;
        Ok(counts)
    }

    /// How many items the trash lists: the folders and the shortcuts in it themselves.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn trash_count(&self) -> Result<usize, StoreError> {
        Ok(self
            .trashed_count()?
            .saturating_add(self.folders.trashed().len()))
    }

    /// How many shortcuts wait in the trash themselves.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn trashed_count(&self) -> Result<usize, StoreError> {
        let shortcuts = open(&self.transaction, index(Table::Trash))?
            .len()
            .map_err(failed)?;
        usize::try_from(shortcuts).map_err(|_huge| StoreError::Missing("a count that fits"))
    }

    /// How many shortcuts the trash hides inside a folder and every folder inside it, which come back with the folder.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn hidden_count(&self, folder: ItemId) -> Result<usize, StoreError> {
        let tree = self.folders.tree();
        let mut total = 0_usize;
        for (id, _) in tree.folders() {
            if tree.within(id, folder) {
                let held = self
                    .under(Table::Hidden, &index_rules::home_key(Some(id)))?
                    .len();
                total = total.saturating_add(held);
            }
        }
        Ok(total)
    }

    /// The first shortcut that shows, by its identifier, whose link is `link` among those that a hash prefix names.
    fn first_holding(&self, prefix: &[u8], link: &str) -> Result<Option<ItemId>, StoreError> {
        for key in self.under(Table::LinkHash, prefix)? {
            let found = id(&key)?;
            if self
                .item(found)?
                .is_some_and(|shortcut| shortcut.link.value == link)
            {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    /// The shortcut that shows with `link` in `folder`, or the top for `None`, the first by its identifier.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn holder(&self, folder: Option<ItemId>, link: &str) -> Result<Option<ItemId>, StoreError> {
        self.first_holding(&index_rules::link_hash_home(link, folder), link)
    }

    /// Whether any shortcut that shows holds `link`.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn holds_link(&self, link: &str) -> Result<bool, StoreError> {
        Ok(self
            .first_holding(&index_rules::link_hash_prefix(link), link)?
            .is_some())
    }

    /// The shortcuts that show among the identifiers that end the rows of an index table within bounds, each once.
    fn live_within(&self, table: Table, bounds: &Bounds) -> Result<Named, StoreError> {
        let ids = self
            .keys(table, &bounds.start, bounds.end.as_deref())?
            .iter()
            .map(|key| id(key))
            .collect::<Result<BTreeSet<ItemId>, StoreError>>()?;
        let mut found = Vec::new();
        for id in ids {
            if let Some(shortcut) = self.item(id)? {
                found.push((id, shortcut));
            }
        }
        Ok(found)
    }

    /// The shortcuts that show whose alias is `alias`, and none for an empty one, which names no alias.
    ///
    /// The index of words holds every word of an alias, so the shortcuts with the longest word of `alias` hold every match.
    /// An alias of separators alone files no word, and only a read of every shortcut finds it.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn aliased(&self, alias: &str) -> Result<Named, StoreError> {
        if alias.is_empty() {
            return Ok(Vec::new());
        }
        let candidates = match search_rules::longest(alias) {
            Some(word) => {
                self.live_within(Table::Token, &search_rules::equal(word.as_str().as_bytes()))?
            }
            None => self.shortcuts()?,
        };
        Ok(candidates
            .into_iter()
            .filter(|(_, shortcut)| shortcut.alias.value == alias)
            .collect())
    }

    /// The shortcuts that show whose title is `title`, read through the index of titles under the start of its folded form.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn titled(&self, title: &str) -> Result<Named, StoreError> {
        let folded = title.to_lowercase();
        let kept = index_rules::prefix::<SORT_PREFIX>(folded.as_bytes());
        Ok(self
            .live_within(Table::Title, &search_rules::equal(kept))?
            .into_iter()
            .filter(|(_, shortcut)| shortcut.title.value == title)
            .collect())
    }

    /// The shortcuts that show with `link`, in every folder, read through the index of links' hashes.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn holders(&self, link: &str) -> Result<Named, StoreError> {
        let prefix = index_rules::link_hash_prefix(link);
        Ok(self
            .live_within(Table::LinkHash, &search_rules::starting(&prefix))?
            .into_iter()
            .filter(|(_, shortcut)| shortcut.link.value == link)
            .collect())
    }

    /// The shortcuts that show whose identifier's lowercase hexadecimal spelling starts with `spelled`, read from the records within the bounds it spells.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn spelled(&self, spelled: &str) -> Result<Named, StoreError> {
        let Some(bounds) = search_rules::identifiers(spelled.as_bytes()) else {
            return Ok(Vec::new());
        };
        let shortcuts = open(&self.transaction, SHORTCUTS)?;
        let finish = bounds
            .end
            .as_deref()
            .map_or(Bound::Unbounded, Bound::Excluded);
        let mut found = Vec::new();
        for entry in shortcuts
            .range::<&[u8]>((Bound::Included(bounds.start.as_slice()), finish))
            .map_err(failed)?
        {
            let (key, value) = entry.map_err(failed)?;
            let shortcut = decode_shortcut(value.value(), &self.table)?;
            if self.folders.standing(&shortcut) == Standing::Shown {
                found.push((ItemId(super::key_id(key.value())?), shortcut));
            }
        }
        Ok(found)
    }

    /// The latest stamp of a write by `device` that the store still holds.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn latest_of(&self, device: DeviceId) -> Result<Option<Stamp>, StoreError> {
        let folders = self.folders.records().values().flat_map(Folder::stamps);
        let shortcuts: Vec<Stamp> = self
            .all_shortcuts()?
            .iter()
            .flat_map(|(_, shortcut)| shortcut.stamps())
            .collect();
        Ok(folders
            .chain(shortcuts)
            .chain(self.device_names()?.values().map(|name| name.stamp))
            .filter(|stamp| stamp.device == device)
            .max())
    }

    /// Every record the store holds, those in the trash and those gone for good included, which a bundle carries.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the store cannot be read.
    pub fn records(&self) -> Result<Body, StoreError> {
        Ok(Body {
            devices: self.device_names()?.into_iter().collect(),
            folders: self
                .folders
                .records()
                .iter()
                .map(|(id, folder)| (*id, folder.clone()))
                .collect(),
            shortcuts: self.all_shortcuts()?,
        })
    }
}
