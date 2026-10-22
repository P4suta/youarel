use super::{
    Session,
    publication::{DirtyRecord, dirty_key},
};
use crate::{
    StoreError,
    database::{DEVICE_NAMES, failed, open},
};
use redb::ReadableTable as _;
use std::collections::BTreeMap;
use youarel_core::{
    device_rules::MemberId,
    merge_rules::Register,
    record_rules,
    wire_rules::{Reader, WireError, Writer},
};

fn read_name(
    bytes: &[u8],
    table: &record_rules::DeviceTable,
    id: MemberId,
) -> Result<Register<String>, StoreError> {
    let mut reader = Reader::new(bytes);
    let (named, name) =
        record_rules::decode_device(&mut reader, table).map_err(StoreError::Damaged)?;
    reader.finish().map_err(StoreError::Damaged)?;
    if named != id {
        return Err(StoreError::Damaged(WireError::Value));
    }
    Ok(name)
}

impl Session {
    /// # Errors
    ///
    /// Refuses a damaged name record or an unreadable database.
    pub fn device_name(&self, id: MemberId) -> Result<Option<Register<String>>, StoreError> {
        open(&self.transaction, DEVICE_NAMES)?
            .get(id.0.as_slice())
            .map_err(failed)?
            .map_or_else(
                || Ok(None),
                |held| read_name(held.value(), &self.table, id).map(Some),
            )
    }

    /// # Errors
    ///
    /// Refuses a damaged name record or an unreadable database.
    pub fn device_names(&self) -> Result<BTreeMap<MemberId, Register<String>>, StoreError> {
        let mut names = BTreeMap::new();
        for entry in open(&self.transaction, DEVICE_NAMES)?
            .iter()
            .map_err(failed)?
        {
            let (key, value) = entry.map_err(failed)?;
            let id = MemberId(
                key.value()
                    .try_into()
                    .map_err(|_length| StoreError::Damaged(WireError::Value))?,
            );
            let name = read_name(value.value(), &self.table, id)?;
            names.insert(id, name);
        }
        Ok(names)
    }

    pub(super) fn put_device_name(
        &mut self,
        id: MemberId,
        name: &Register<String>,
    ) -> Result<(), StoreError> {
        self.meet([name.stamp])?;
        let mut writer = Writer::new();
        record_rules::encode_device(&mut writer, &self.table, (id, name))
            .map_err(StoreError::Records)?;
        open(&self.transaction, DEVICE_NAMES)?
            .insert(id.0.as_slice(), writer.finish().as_slice())
            .map_err(failed)?;
        self.observe([name.stamp]);
        self.record_dirty_key(&dirty_key(DirtyRecord::Device(id)), [name.stamp])?;
        self.changed = true;
        Ok(())
    }
}
