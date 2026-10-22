//! The state as binary records: the devices that wrote it, then each folder and each shortcut with its registers.
//!
//! A stamp names its device by its place in a table at the head of the records, and a register that shares its item's base stamp, the stamp of the item's first register, spends two bits on it.
//! The reader refuses every form but the one the writer gives, so one state has one encoding, which a digest of the records stands for.
//! A record's link stays text here, and the adapters admit every link they read again.

use crate::clock_rules::{DeviceId, Stamp};
use crate::merge_rules::Register;
use crate::position_rules::Key;
use crate::state::{Folder, ItemId, Presence, Shortcut, State};
use crate::wire_rules::{Reader, WireError, Writer, ascending};
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

/// The magic that opens the records of a state.
pub const MAGIC: [u8; 4] = *b"YRR1";
/// The version of the records this module writes and reads.
pub const VERSION: u16 = 2;

#[cfg(test)]
mod device_tests;
/// The stamp of a register that no device wrote, which every write follows.
pub const NEVER: Stamp = Stamp::origin(DeviceId([0; 16]));

/// The flag of an item gone for good, which a record before the trash raised for a removed item.
const PURGED: u8 = 0b0_0001;
/// The flag of an item inside a folder: a shortcut's folder, or a folder's parent.
const PLACED: u8 = 0b0_0010;
/// The flag of a shortcut with an alias, or of a folder with a mark.
const NAMED: u8 = 0b0_0100;
/// The flag of a shortcut with a note.
const NOTED: u8 = 0b0_1000;
/// The flag of an item in the trash, which never stands beside the flag of an item gone for good.
const TRASHED: u8 = 0b1_0000;
/// The flags a shortcut's record may raise, all five; every other bit stays clear.
const SHORTCUT_FLAGS: u8 = 0b1_1111;
/// The flags a folder's record may raise, all but the note's; every other bit stays clear.
const FOLDER_FLAGS: u8 = 0b1_0111;

/// The flag of an item's presence: none while it is live.
const fn presence_flag(presence: Presence) -> u8 {
    match presence {
        Presence::Live => 0,
        Presence::Trashed => TRASHED,
        Presence::Purged => PURGED,
    }
}

/// The presence that a record's flags name, refusing flags that name it both in the trash and gone for good.
const fn read_presence(flags: u8) -> Result<Presence, WireError> {
    match (flags & PURGED != 0, flags & TRASHED != 0) {
        (false, false) => Ok(Presence::Live),
        (false, true) => Ok(Presence::Trashed),
        (true, false) => Ok(Presence::Purged),
        (true, true) => Err(WireError::Tag(flags)),
    }
}

/// The mode of a register that shares its item's base stamp.
const SAME: u8 = 0;
/// The mode of a register whose own stamp follows the modes.
const OWN: u8 = 1;
/// The mode of a register that no device wrote.
const UNWRITTEN: u8 = 2;
/// The two bits of one mode.
const MODE: u8 = 0b11;
/// Where each of the four modes of a byte sits in it, the first register's lowest.
const MODE_SHIFTS: [u32; 4] = [0, 2, 4, 6];
/// The registers of a shortcut after its link: title, alias, folder, position, presence, and note.
const SHORTCUT_REGISTERS: usize = 6;
/// The registers of a folder after its name: mark, parent, position, and presence.
const FOLDER_REGISTERS: usize = 4;
/// The shift that spreads a difference's sign across all its bits.
const SIGN_SHIFT: u32 = 63;

/// The devices whose stamps the records of a state write in full, in strictly ascending order, which each such stamp names by its place.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeviceTable {
    devices: Vec<DeviceId>,
}

impl DeviceTable {
    /// The table of devices that ascend strictly, or `None` for any other list.
    #[must_use]
    pub fn ascending(devices: Vec<DeviceId>) -> Option<Self> {
        ascending(&devices).then_some(Self { devices })
    }

    /// The table of devices in any order, each once, as a store keeps them so that every place it wrote stays, or `None` for a list that names a device twice.
    #[must_use]
    pub fn distinct(devices: Vec<DeviceId>) -> Option<Self> {
        let mut sorted = devices.clone();
        sorted.sort_unstable();
        ascending(&sorted).then_some(Self { devices })
    }

    /// The table of every device whose stamp a body's records write in full.
    #[must_use]
    pub fn of(body: &Body) -> Self {
        let devices: BTreeSet<DeviceId> = stamped(body).collect();
        Self {
            devices: devices.into_iter().collect(),
        }
    }

    /// The place of a device in the table, which an account's few devices let a walk find.
    #[must_use]
    pub fn place(&self, device: DeviceId) -> Option<usize> {
        self.devices.iter().position(|held| *held == device)
    }

    /// The devices in their order.
    #[must_use]
    pub fn devices(&self) -> &[DeviceId] {
        &self.devices
    }
}

/// The records of a state: its folders and its shortcuts, each list in strictly ascending order of the items' identifiers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Body {
    pub folders: Vec<(ItemId, Folder)>,
    pub shortcuts: Vec<(ItemId, Shortcut)>,
    pub devices: Vec<(crate::device_rules::MemberId, Register<String>)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Record {
    Folder(ItemId, Folder),
    Shortcut(ItemId, Shortcut),
    Device(crate::device_rules::MemberId, Register<String>),
}

impl Record {
    #[must_use]
    pub fn latest_of(&self, device: DeviceId) -> Option<Stamp> {
        match self {
            Self::Device(_, name) => (name.stamp.device == device).then_some(name.stamp),
            Self::Folder(_, folder) => folder
                .stamps()
                .into_iter()
                .filter(|stamp| stamp.device == device)
                .max(),
            Self::Shortcut(_, shortcut) => shortcut
                .stamps()
                .into_iter()
                .filter(|stamp| stamp.device == device)
                .max(),
        }
    }
}

/// The bytes surrounding a body's encoded records, including its device dictionary.
///
/// # Errors
///
/// Refuses counts whose framing cannot fit in memory.
pub fn body_framing(
    devices: usize,
    folders: usize,
    shortcuts: usize,
    names: usize,
) -> Result<usize, WireError> {
    const DEVICE_BYTES: usize = size_of::<DeviceId>();
    let mut writer = Writer::new();
    writer.raw(&MAGIC).short(VERSION);
    for count in [devices, folders, shortcuts, names] {
        write_count(&mut writer, count)?;
    }
    devices
        .checked_mul(DEVICE_BYTES)
        .and_then(|bytes| bytes.checked_add(writer.finish().len()))
        .ok_or(WireError::Length)
}

impl Body {
    /// The records of every folder and shortcut a state holds, removed ones included.
    #[must_use]
    pub fn of(state: &State) -> Self {
        Self {
            devices: state
                .devices
                .iter()
                .map(|(id, name)| (*id, name.clone()))
                .collect(),
            folders: state
                .folders
                .iter()
                .map(|(id, folder)| (*id, folder.clone()))
                .collect(),
            shortcuts: state
                .shortcuts
                .iter()
                .map(|(id, shortcut)| (*id, shortcut.clone()))
                .collect(),
        }
    }

    /// The state the records hold.
    #[must_use]
    pub fn into_state(self) -> State {
        State {
            devices: self.devices.into_iter().collect(),
            shortcuts: self.shortcuts.into_iter().collect(),
            folders: self.folders.into_iter().collect(),
        }
    }
}

/// A shortcut's base stamp, its link's, and the stamps of its other registers in their fixed order.
const fn shortcut_stamps(shortcut: &Shortcut) -> (Stamp, [Stamp; SHORTCUT_REGISTERS]) {
    (
        shortcut.link.stamp,
        [
            shortcut.title.stamp,
            shortcut.alias.stamp,
            shortcut.folder.stamp,
            shortcut.position.stamp,
            shortcut.presence.stamp,
            shortcut.note.stamp,
        ],
    )
}

/// A folder's base stamp, its name's, and the stamps of its other registers in their fixed order.
const fn folder_stamps(folder: &Folder) -> (Stamp, [Stamp; FOLDER_REGISTERS]) {
    (
        folder.name.stamp,
        [
            folder.mark.stamp,
            folder.parent.stamp,
            folder.position.stamp,
            folder.presence.stamp,
        ],
    )
}

/// How a register's stamp stands beside its item's base stamp.
fn mode(base: Stamp, stamp: Stamp) -> u8 {
    if stamp == base {
        SAME
    } else if stamp == NEVER {
        UNWRITTEN
    } else {
        OWN
    }
}

/// The devices whose stamps an item's record writes in full: its base's, and each of a register with a stamp of its own.
fn written<const N: usize>((base, stamps): (Stamp, [Stamp; N])) -> impl Iterator<Item = DeviceId> {
    core::iter::once(base.device).chain(
        stamps
            .into_iter()
            .filter(move |stamp| mode(base, *stamp) == OWN)
            .map(|stamp| stamp.device),
    )
}

/// The devices whose stamps a body's records write in full, one for each such stamp.
fn stamped(body: &Body) -> impl Iterator<Item = DeviceId> + '_ {
    let folders = body
        .folders
        .iter()
        .flat_map(|(_, folder)| written(folder_stamps(folder)));
    let shortcuts = body
        .shortcuts
        .iter()
        .flat_map(|(_, shortcut)| written(shortcut_stamps(shortcut)));
    folders
        .chain(shortcuts)
        .chain(body.devices.iter().map(|(_, name)| name.stamp.device))
}

/// The zigzag form of a wrapping difference, which keeps a small step either way small.
const fn zigzag(difference: u64) -> u64 {
    let signed = difference.cast_signed();
    signed.wrapping_shl(1).cast_unsigned() ^ signed.wrapping_shr(SIGN_SHIFT).cast_unsigned()
}

/// The wrapping difference a zigzag form stands for.
const fn unzigzag(form: u64) -> u64 {
    form.wrapping_shr(1) ^ (form & 1).wrapping_neg()
}

/// Writes a count or a length as a varint.
fn write_count(writer: &mut Writer, count: usize) -> Result<(), WireError> {
    writer.varint(u64::try_from(count).map_err(|_huge| WireError::Length)?);
    Ok(())
}

/// A count or a length that a varint holds.
fn read_count(reader: &mut Reader<'_>) -> Result<usize, WireError> {
    usize::try_from(reader.varint()?).map_err(|_huge| WireError::Length)
}

/// Writes a device's place in the table.
fn write_place(
    writer: &mut Writer,
    devices: &DeviceTable,
    device: DeviceId,
) -> Result<(), WireError> {
    write_count(writer, devices.place(device).ok_or(WireError::Value)?)
}

/// The device at the place a varint names.
fn read_device(reader: &mut Reader<'_>, devices: &DeviceTable) -> Result<DeviceId, WireError> {
    let place = usize::try_from(reader.varint()?).map_err(|_huge| WireError::Value)?;
    devices.devices.get(place).copied().ok_or(WireError::Value)
}

/// A stamp's counter, which a varint beyond thirty-two bits cannot be.
fn read_counter(reader: &mut Reader<'_>) -> Result<u32, WireError> {
    u32::try_from(reader.varint()?).map_err(|_wide| WireError::Value)
}

/// The byte that holds the modes of up to four registers, the first in its lowest bits.
fn pack_modes(modes: impl IntoIterator<Item = u8>) -> u8 {
    let mut byte = 0;
    for (mode, shift) in modes.into_iter().zip(MODE_SHIFTS) {
        byte |= mode.wrapping_shl(shift);
    }
    byte
}

/// The modes that a byte holds for as many registers as `modes` has places, up to four, refusing a set bit past the last register.
fn unpack_modes(byte: u8, modes: &mut [u8]) -> Result<(), WireError> {
    let mut spare = byte;
    for (slot, shift) in modes.iter_mut().zip(MODE_SHIFTS) {
        *slot = byte.wrapping_shr(shift) & MODE;
        spare &= !MODE.wrapping_shl(shift);
    }
    if spare != 0 {
        return Err(WireError::Tag(byte));
    }
    Ok(())
}

/// Writes an item's stamps: its base in full, the mode of each other register four to a byte, then each stamp of a register's own with its wall time as the zigzag difference from the base's.
fn write_stamps<const N: usize>(
    writer: &mut Writer,
    devices: &DeviceTable,
    (base, stamps): (Stamp, [Stamp; N]),
) -> Result<(), WireError> {
    writer.varint(base.wall).varint(u64::from(base.counter));
    write_place(writer, devices, base.device)?;
    for chunk in stamps.chunks(MODE_SHIFTS.len()) {
        writer.byte(pack_modes(chunk.iter().map(|stamp| mode(base, *stamp))));
    }
    for stamp in stamps {
        if mode(base, stamp) == OWN {
            writer
                .varint(zigzag(stamp.wall.wrapping_sub(base.wall)))
                .varint(u64::from(stamp.counter));
            write_place(writer, devices, stamp.device)?;
        }
    }
    Ok(())
}

/// The stamp of its own that a register writes beside its item's base: the base's wall time moved by the difference a zigzag form names, a counter, and a device, which can be neither the base nor the stamp that no device wrote.
fn own_stamp(base: Stamp, form: u64, counter: u32, device: DeviceId) -> Result<Stamp, WireError> {
    let stamp = Stamp {
        wall: base.wall.wrapping_add(unzigzag(form)),
        counter,
        device,
    };
    if stamp == base || stamp == NEVER {
        return Err(WireError::Canonical);
    }
    Ok(stamp)
}

/// A register's own stamp beside its item's base, as [`write_stamps`] writes it.
fn read_own(
    reader: &mut Reader<'_>,
    devices: &DeviceTable,
    base: Stamp,
) -> Result<Stamp, WireError> {
    let form = reader.varint()?;
    let counter = read_counter(reader)?;
    let device = read_device(reader, devices)?;
    own_stamp(base, form, counter, device)
}

/// An item's stamps, as [`write_stamps`] writes them.
fn read_stamps<const N: usize>(
    reader: &mut Reader<'_>,
    devices: &DeviceTable,
) -> Result<(Stamp, [Stamp; N]), WireError> {
    let base = Stamp {
        wall: reader.varint()?,
        counter: read_counter(reader)?,
        device: read_device(reader, devices)?,
    };
    let mut modes = [SAME; N];
    for chunk in modes.chunks_mut(MODE_SHIFTS.len()) {
        unpack_modes(reader.byte()?, chunk)?;
    }
    let mut stamps = [base; N];
    for (stamp, mode) in stamps.iter_mut().zip(modes) {
        *stamp = match mode {
            SAME => base,
            OWN => read_own(reader, devices, base)?,
            // The stamp no device wrote is the base itself when the base is that stamp, which only the first mode names.
            UNWRITTEN if base == NEVER => return Err(WireError::Canonical),
            UNWRITTEN => NEVER,
            reserved => return Err(WireError::Tag(reserved)),
        };
    }
    Ok((base, stamps))
}

/// Writes text as its length in a varint and its UTF-8 bytes.
fn write_text(writer: &mut Writer, text: &str) -> Result<(), WireError> {
    write_count(writer, text.len())?;
    writer.raw(text.as_bytes());
    Ok(())
}

/// Text as [`write_text`] writes it.
fn read_text(reader: &mut Reader<'_>) -> Result<String, WireError> {
    let length = read_count(reader)?;
    let bytes = reader.take(length)?;
    core::str::from_utf8(bytes)
        .map(String::from)
        .map_err(|_unreadable| WireError::Value)
}

/// Text that a flag says the record holds, which is never empty then, or the empty text that the flag leaves out.
fn read_flagged(reader: &mut Reader<'_>, flagged: bool) -> Result<String, WireError> {
    if !flagged {
        return Ok(String::new());
    }
    let text = read_text(reader)?;
    if text.is_empty() {
        return Err(WireError::Canonical);
    }
    Ok(text)
}

/// A position, which only a key can be.
fn read_key(reader: &mut Reader<'_>) -> Result<Key, WireError> {
    Key::parse(&read_text(reader)?).ok_or(WireError::Value)
}

/// Writes a shortcut's record: its identifier, then its fields as [`encode_shortcut_fields`] writes them.
///
/// # Errors
///
/// Returns [`WireError::Value`] when a stamp names a device that the table does not hold.
pub fn encode_shortcut(
    writer: &mut Writer,
    devices: &DeviceTable,
    (id, shortcut): (ItemId, &Shortcut),
) -> Result<(), WireError> {
    writer.raw(&id.0);
    encode_shortcut_fields(writer, devices, shortcut)
}

/// Writes a shortcut's fields: its flags, its stamps, then its link, title, alias, folder, position, and note, each optional field only when a flag says it is there.
///
/// A store keeps the fields under the identifier as its key.
///
/// # Errors
///
/// Returns [`WireError::Value`] when a stamp names a device that the table does not hold.
pub fn encode_shortcut_fields(
    writer: &mut Writer,
    devices: &DeviceTable,
    shortcut: &Shortcut,
) -> Result<(), WireError> {
    let mut flags = presence_flag(shortcut.presence.value);
    if shortcut.folder.value.is_some() {
        flags |= PLACED;
    }
    if !shortcut.alias.value.is_empty() {
        flags |= NAMED;
    }
    if !shortcut.note.value.is_empty() {
        flags |= NOTED;
    }
    writer.byte(flags);
    write_stamps(writer, devices, shortcut_stamps(shortcut))?;
    write_text(writer, &shortcut.link.value)?;
    write_text(writer, &shortcut.title.value)?;
    if flags & NAMED != 0 {
        write_text(writer, &shortcut.alias.value)?;
    }
    if let Some(folder) = shortcut.folder.value {
        writer.raw(&folder.0);
    }
    write_text(writer, shortcut.position.value.as_str())?;
    if flags & NOTED != 0 {
        write_text(writer, &shortcut.note.value)?;
    }
    Ok(())
}

/// A shortcut's record, as [`encode_shortcut`] writes it.
///
/// # Errors
///
/// Returns a [`WireError`] for bytes that [`encode_shortcut`] would not have written.
pub fn decode_shortcut(
    reader: &mut Reader<'_>,
    devices: &DeviceTable,
) -> Result<(ItemId, Shortcut), WireError> {
    let id = ItemId(reader.array()?);
    Ok((id, decode_shortcut_fields(reader, devices)?))
}

/// A shortcut's fields, as [`encode_shortcut_fields`] writes them.
///
/// # Errors
///
/// Returns a [`WireError`] for bytes that [`encode_shortcut_fields`] would not have written.
pub fn decode_shortcut_fields(
    reader: &mut Reader<'_>,
    devices: &DeviceTable,
) -> Result<Shortcut, WireError> {
    let flags = reader.byte()?;
    if flags & !SHORTCUT_FLAGS != 0 {
        return Err(WireError::Tag(flags));
    }
    let present = read_presence(flags)?;
    let (base, [title, alias, folder, position, presence, note]) = read_stamps(reader, devices)?;
    let link = read_text(reader)?;
    let titled = read_text(reader)?;
    let aliased = read_flagged(reader, flags & NAMED != 0)?;
    let placed = if flags & PLACED == 0 {
        None
    } else {
        Some(ItemId(reader.array()?))
    };
    let key = read_key(reader)?;
    let noted = read_flagged(reader, flags & NOTED != 0)?;
    let shortcut = Shortcut {
        link: Register {
            value: link,
            stamp: base,
        },
        title: Register {
            value: titled,
            stamp: title,
        },
        alias: Register {
            value: aliased,
            stamp: alias,
        },
        folder: Register {
            value: placed,
            stamp: folder,
        },
        position: Register {
            value: key,
            stamp: position,
        },
        presence: Register {
            value: present,
            stamp: presence,
        },
        note: Register {
            value: noted,
            stamp: note,
        },
    };
    Ok(shortcut)
}

/// Writes a folder's record: its identifier, then its fields as [`encode_folder_fields`] writes them.
///
/// # Errors
///
/// Returns [`WireError::Value`] when a stamp names a device that the table does not hold.
pub fn encode_folder(
    writer: &mut Writer,
    devices: &DeviceTable,
    (id, folder): (ItemId, &Folder),
) -> Result<(), WireError> {
    writer.raw(&id.0);
    encode_folder_fields(writer, devices, folder)
}

/// Writes a folder's fields: its flags, its stamps, then its name, mark, parent, and position, each optional field only when a flag says it is there.
///
/// # Errors
///
/// Returns [`WireError::Value`] when a stamp names a device that the table does not hold.
pub fn encode_folder_fields(
    writer: &mut Writer,
    devices: &DeviceTable,
    folder: &Folder,
) -> Result<(), WireError> {
    let mut flags = presence_flag(folder.presence.value);
    if folder.parent.value.is_some() {
        flags |= PLACED;
    }
    if !folder.mark.value.is_empty() {
        flags |= NAMED;
    }
    writer.byte(flags);
    write_stamps(writer, devices, folder_stamps(folder))?;
    write_text(writer, &folder.name.value)?;
    if flags & NAMED != 0 {
        write_text(writer, &folder.mark.value)?;
    }
    if let Some(parent) = folder.parent.value {
        writer.raw(&parent.0);
    }
    write_text(writer, folder.position.value.as_str())
}

/// A folder's record, as [`encode_folder`] writes it.
///
/// # Errors
///
/// Returns a [`WireError`] for bytes that [`encode_folder`] would not have written.
pub fn decode_folder(
    reader: &mut Reader<'_>,
    devices: &DeviceTable,
) -> Result<(ItemId, Folder), WireError> {
    let id = ItemId(reader.array()?);
    Ok((id, decode_folder_fields(reader, devices)?))
}

/// A folder's fields, as [`encode_folder_fields`] writes them.
///
/// # Errors
///
/// Returns a [`WireError`] for bytes that [`encode_folder_fields`] would not have written.
pub fn decode_folder_fields(
    reader: &mut Reader<'_>,
    devices: &DeviceTable,
) -> Result<Folder, WireError> {
    let flags = reader.byte()?;
    if flags & !FOLDER_FLAGS != 0 {
        return Err(WireError::Tag(flags));
    }
    let present = read_presence(flags)?;
    let (base, [mark, parent, position, presence]) = read_stamps(reader, devices)?;
    let name = read_text(reader)?;
    let marked = read_flagged(reader, flags & NAMED != 0)?;
    let placed = if flags & PLACED == 0 {
        None
    } else {
        Some(ItemId(reader.array()?))
    };
    let key = read_key(reader)?;
    let folder = Folder {
        name: Register {
            value: name,
            stamp: base,
        },
        mark: Register {
            value: marked,
            stamp: mark,
        },
        parent: Register {
            value: placed,
            stamp: parent,
        },
        position: Register {
            value: key,
            stamp: position,
        },
        presence: Register {
            value: present,
            stamp: presence,
        },
    };
    Ok(folder)
}

/// The records as bytes: the magic and the version, the table of devices, then the folders and the shortcuts, each list counted by a varint.
///
/// # Errors
///
/// Returns [`WireError::Order`] when the folders or the shortcuts are out of ascending order.
pub fn encode_body(body: &Body) -> Result<Vec<u8>, WireError> {
    if !ascending(body.folders.iter().map(|(id, _)| id))
        || !ascending(body.shortcuts.iter().map(|(id, _)| id))
        || !ascending(body.devices.iter().map(|(id, _)| id))
    {
        return Err(WireError::Order);
    }
    let devices = DeviceTable::of(body);
    let mut writer = Writer::new();
    writer.raw(&MAGIC).short(VERSION);
    write_count(&mut writer, devices.devices.len())?;
    for device in &devices.devices {
        writer.raw(&device.0);
    }
    write_count(&mut writer, body.folders.len())?;
    for (id, folder) in &body.folders {
        encode_folder(&mut writer, &devices, (*id, folder))?;
    }
    write_count(&mut writer, body.shortcuts.len())?;
    for (id, shortcut) in &body.shortcuts {
        encode_shortcut(&mut writer, &devices, (*id, shortcut))?;
    }
    write_count(&mut writer, body.devices.len())?;
    for (id, name) in &body.devices {
        encode_device(&mut writer, &devices, (*id, name))?;
    }
    Ok(writer.finish())
}

/// Writes a membership identity and its sealed name register.
///
/// # Errors
///
/// Refuses names outside the device-name contract or a stamp outside the dictionary.
pub fn encode_device(
    writer: &mut Writer,
    devices: &DeviceTable,
    (id, name): (crate::device_rules::MemberId, &Register<String>),
) -> Result<(), WireError> {
    if !crate::device_rules::admitted_name(&name.value) {
        return Err(WireError::Value);
    }
    writer.raw(&id.0);
    write_stamps(writer, devices, (name.stamp, []))?;
    write_text(writer, &name.value)
}

/// # Errors
///
/// Refuses a malformed stamp, noncanonical text, or an invalid name.
pub fn decode_device(
    reader: &mut Reader<'_>,
    devices: &DeviceTable,
) -> Result<(crate::device_rules::MemberId, Register<String>), WireError> {
    let id = crate::device_rules::MemberId(reader.array()?);
    let (stamp, []) = read_stamps::<0>(reader, devices)?;
    let value = read_text(reader)?;
    if !crate::device_rules::admitted_name(&value) {
        return Err(WireError::Value);
    }
    Ok((id, Register { value, stamp }))
}

fn read_list<K: Copy + Ord, T>(
    reader: &mut Reader<'_>,
    read: impl Fn(&mut Reader<'_>) -> Result<(K, T), WireError>,
) -> Result<Vec<(K, T)>, WireError> {
    let count = read_count(reader)?;
    let mut items: Vec<(K, T)> = Vec::new();
    for _ in 0..count {
        let item = read(reader)?;
        if items.last().is_some_and(|(last, _)| *last >= item.0) {
            return Err(WireError::Order);
        }
        items.push(item);
    }
    Ok(items)
}

/// Whether a body's records write a stamp of every device that its table lists.
fn names_every(devices: &DeviceTable, body: &Body) -> bool {
    let mut named = alloc::vec![false; devices.devices.len()];
    for device in stamped(body) {
        if let Some(slot) = devices.place(device).and_then(|place| named.get_mut(place)) {
            *slot = true;
        }
    }
    named.into_iter().all(|named| named)
}

/// The records that bytes hold, as [`encode_body`] writes them.
///
/// # Errors
///
/// Returns a [`WireError`] for bytes that [`encode_body`] would not have written.
pub fn decode_body(bytes: &[u8]) -> Result<Body, WireError> {
    let mut reader = Reader::new(bytes);
    if reader.array::<{ MAGIC.len() }>()? != MAGIC {
        return Err(WireError::Magic);
    }
    let version = reader.short()?;
    if version != VERSION {
        return Err(WireError::Version(version));
    }
    let count = read_count(&mut reader)?;
    let mut listed = Vec::new();
    for _ in 0..count {
        listed.push(DeviceId(reader.array()?));
    }
    let devices = DeviceTable::ascending(listed).ok_or(WireError::Order)?;
    let folders = read_list(&mut reader, |reader| decode_folder(reader, &devices))?;
    let shortcuts = read_list(&mut reader, |reader| decode_shortcut(reader, &devices))?;
    let names = read_list(&mut reader, |reader| decode_device(reader, &devices))?;
    reader.finish()?;
    let body = Body {
        folders,
        shortcuts,
        devices: names,
    };
    if !names_every(&devices, &body) {
        return Err(WireError::Canonical);
    }
    Ok(body)
}

/// The properties Kani proves of the records' parts: a stamp beside its base, a byte of modes, and the order of a body's lists.
///
/// A whole record decodes texts into strings, which took the solver past the proof gate's memory, so the tests sweep every flag and every mode of whole records instead, and the fuzz target `record_decode` requires the one form of every body it reads.
#[cfg(kani)]
mod proofs {
    use super::{
        DeviceTable, MODE, NEVER, decode_shortcut, encode_shortcut, pack_modes, read_list,
        unpack_modes,
    };
    use crate::clock_rules::{DeviceId, Stamp};
    use crate::merge_rules::Register;
    use crate::position_rules::Key;
    use crate::state::{ItemId, Presence, Shortcut};
    use crate::wire_rules::{Reader, WireError, Writer};
    use alloc::string::String;
    use alloc::vec;

    /// The bytes of an identifier.
    const ID: usize = 16;
    /// The most registers one byte of modes holds.
    const MODES_PER_BYTE: usize = 4;

    /// Any stamp of any device.
    fn any_stamp() -> Stamp {
        Stamp {
            wall: kani::any(),
            counter: kani::any(),
            device: DeviceId(kani::any()),
        }
    }

    fn register<T>(value: T, stamp: Stamp) -> Register<T> {
        Register { value, stamp }
    }

    fn count_width(mut value: usize) -> u128 {
        const BITS: u32 = 7;
        const BASE: usize = 128;
        let mut bytes: u128 = 1;
        while value >= BASE {
            value >>= BITS;
            bytes += 1;
        }
        bytes
    }

    #[kani::proof]
    #[kani::unwind(65)]
    fn body_framing_reports_its_exact_size_without_overflow() {
        let devices: usize = kani::any();
        let folders: usize = kani::any();
        let shortcuts: usize = kani::any();
        let names: usize = kani::any();
        let fixed = (super::MAGIC.len() + size_of::<u16>()) as u128;
        let expected = fixed
            + count_width(devices)
            + count_width(folders)
            + count_width(shortcuts)
            + count_width(names)
            + (devices as u128) * (size_of::<DeviceId>() as u128);
        match super::body_framing(devices, folders, shortcuts, names) {
            Ok(bytes) => kani::assert(
                bytes as u128 == expected,
                "framing is the four counts, magic, version, and dictionary",
            ),
            Err(error) => kani::assert(
                error == super::WireError::Length && expected > usize::MAX as u128,
                "only framing beyond memory refuses",
            ),
        }
        kani::cover!(expected <= usize::MAX as u128, "framing fits");
        kani::cover!(
            expected > usize::MAX as u128,
            "dictionary bytes would overflow"
        );
    }

    #[kani::proof]
    #[kani::unwind(33)]
    fn every_record_reports_its_latest_stamp_of_the_requested_device() {
        let stamps: [Stamp; 7] = core::array::from_fn(|_| any_stamp());
        let device = DeviceId(kani::any());
        let folder: bool = kani::any();
        let Some(position) = Key::parse("a1") else {
            kani::assert(false, "the fixed position is admitted");
            return;
        };
        let record = if folder {
            super::Record::Folder(
                ItemId([1; ID]),
                super::Folder {
                    name: register(String::new(), stamps[0]),
                    mark: register(String::new(), stamps[1]),
                    parent: register(None, stamps[2]),
                    position: register(position, stamps[3]),
                    presence: register(super::Presence::Live, stamps[4]),
                },
            )
        } else {
            super::Record::Shortcut(
                ItemId([1; ID]),
                Shortcut {
                    link: register(String::new(), stamps[0]),
                    title: register(String::new(), stamps[1]),
                    alias: register(String::new(), stamps[2]),
                    folder: register(None, stamps[3]),
                    position: register(position, stamps[4]),
                    presence: register(super::Presence::Live, stamps[5]),
                    note: register(String::new(), stamps[6]),
                },
            )
        };
        let fields = if folder { &stamps[..5] } else { &stamps[..] };
        let latest = record.latest_of(device);
        if let Some(latest) = latest {
            kani::assert(
                latest.device == device && fields.contains(&latest),
                "the latest stamp is a field of this device",
            );
            kani::assert(
                fields
                    .iter()
                    .filter(|stamp| stamp.device == device)
                    .all(|stamp| *stamp <= latest),
                "no field of this device follows the result",
            );
        } else {
            kani::assert(
                fields.iter().all(|stamp| stamp.device != device),
                "none means this device wrote no field",
            );
        }
        kani::cover!(latest.is_some() && folder, "a folder holds an own write");
        kani::cover!(latest.is_some() && !folder, "a shortcut holds an own write");
        kani::cover!(latest.is_none(), "this device wrote no field");
    }

    /// Every stamp beside every base takes the one mode its relation to the base names, and a stamp of its own reads back from its zigzag difference, its counter, and its device, while the base and the stamp no device wrote never read as a stamp of their own.
    ///
    /// The zigzag form is a bijection, so the difference that a stamp of its own writes is its only form, and the varints that carry each part read back by `every_varint_reads_back_canonically`.
    #[kani::proof]
    fn a_stamp_reads_back_beside_its_base() {
        let base = any_stamp();
        let stamp = any_stamp();
        let mode = super::mode(base, stamp);
        kani::assert(
            (mode == super::SAME) == (stamp == base)
                && (mode == super::UNWRITTEN) == (stamp == NEVER && stamp != base)
                && (mode == super::OWN) == (stamp != base && stamp != NEVER),
            "a stamp takes the mode of its relation to the base",
        );
        let form = super::zigzag(stamp.wall.wrapping_sub(base.wall));
        let read = super::own_stamp(base, form, stamp.counter, stamp.device);
        kani::assert(
            if mode == super::OWN {
                read == Ok(stamp)
            } else {
                read == Err(WireError::Canonical)
            },
            "a stamp of its own reads back, and no other reads as one",
        );
        let any_form: u64 = kani::any();
        kani::assert(
            super::zigzag(super::unzigzag(any_form)) == any_form,
            "each difference has one zigzag form",
        );
        kani::cover!(
            mode == super::OWN && stamp.wall < base.wall,
            "a stamp of its own before its base"
        );
        kani::cover!(
            base == NEVER && stamp == NEVER,
            "a base that no device wrote"
        );
    }

    /// Every byte of modes for one to four registers reads as the modes that write it again, and every modes of those registers write the one byte that reads back as them, so two records of one item hold the same modes.
    #[kani::proof]
    #[kani::unwind(5)]
    fn a_byte_of_modes_has_one_form() {
        let byte: u8 = kani::any();
        let count: usize = kani::any();
        kani::assume((1..=MODES_PER_BYTE).contains(&count));
        let mut read = [0_u8; MODES_PER_BYTE];
        let unpacked = unpack_modes(byte, &mut read[..count]);
        let spare = (count < MODES_PER_BYTE) && byte.wrapping_shr(2 * count as u32) != 0;
        kani::assert(
            unpacked.is_err() == spare && (!spare || unpacked == Err(WireError::Tag(byte))),
            "a byte of modes is refused exactly when a bit past the last register is set",
        );
        if unpacked.is_ok() {
            kani::assert(
                pack_modes(read[..count].iter().copied()) == byte,
                "the modes a byte reads as write it again",
            );
        }
        let modes: [u8; MODES_PER_BYTE] = kani::any();
        kani::assume(modes.iter().all(|mode| *mode <= MODE));
        let packed = pack_modes(modes[..count].iter().copied());
        let mut back = [0_u8; MODES_PER_BYTE];
        kani::assert(
            unpack_modes(packed, &mut back[..count]).is_ok() && back[..count] == modes[..count],
            "modes read back from the byte they write",
        );
        kani::cover!(spare, "a byte with a bit past its last register");
        kani::cover!(count == MODES_PER_BYTE && byte == u8::MAX, "a full byte");
    }

    /// A body's list reads two items exactly when their identifiers ascend strictly, refusing them as out of order otherwise, and its table takes two devices exactly when they ascend strictly.
    ///
    /// `decode_body` reads its folders and its shortcuts through `read_list` and its devices through `DeviceTable::ascending`, which compare items only by their identifiers, so items of an identifier alone stand for records.
    #[kani::proof]
    #[kani::unwind(17)]
    fn a_body_refuses_items_out_of_order() {
        let ids: [[u8; ID]; 2] = kani::any();
        let mut bytes = [2_u8; 1 + 2 * ID];
        bytes[1..=ID].copy_from_slice(&ids[0]);
        bytes[1 + ID..].copy_from_slice(&ids[1]);
        let mut reader = Reader::new(&bytes);
        let read = read_list(&mut reader, |reader| Ok((ItemId(reader.array()?), ())));
        let ordered = ids[0] < ids[1];
        kani::assert(
            read.is_ok() == ordered && (ordered || read == Err(WireError::Order)),
            "a list reads exactly when its items ascend, and is refused as out of order otherwise",
        );
        if let Ok(items) = read {
            kani::assert(
                items == [(ItemId(ids[0]), ()), (ItemId(ids[1]), ())] && reader.remaining() == 0,
                "a list holds its items in their order",
            );
        }
        let devices: [[u8; ID]; 2] = kani::any();
        let table = DeviceTable::ascending(vec![DeviceId(devices[0]), DeviceId(devices[1])]);
        kani::assert(
            table.is_some() == (devices[0] < devices[1]),
            "a table takes its devices exactly when they ascend",
        );
        kani::cover!(ordered, "two items in order");
        kani::cover!(ids[0] == ids[1], "one identifier twice");
        kani::cover!(devices[0] > devices[1], "two devices out of order");
    }

    /// A record that raises a flag no item raises would give one shortcut a second form; this probe claims such a record reads and must fail.
    #[kani::proof]
    #[kani::unwind(17)]
    fn rejects_a_record_with_reserved_flag_bits() {
        let base = Stamp {
            wall: 5,
            counter: 0,
            device: DeviceId([1; ID]),
        };
        let table = DeviceTable::ascending(vec![base.device]).unwrap();
        let shortcut = Shortcut {
            link: register(String::from("a"), base),
            title: register(String::new(), base),
            alias: register(String::new(), base),
            folder: register(None, base),
            position: register(Key::parse("V").unwrap(), base),
            presence: register(Presence::Live, base),
            note: register(String::new(), base),
        };
        let mut writer = Writer::new();
        encode_shortcut(&mut writer, &table, (ItemId([0; ID]), &shortcut)).unwrap();
        let mut bytes = writer.finish();
        bytes[ID] |= 0b10_0000;
        kani::assert(
            decode_shortcut(&mut Reader::new(&bytes), &table).is_ok(),
            "NEGATIVE_RECORD_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Body, DeviceTable, FOLDER_FLAGS, MAGIC, NAMED, NEVER, NOTED, PLACED, PURGED,
        SHORTCUT_FLAGS, TRASHED, VERSION, decode_body, decode_folder, decode_folder_fields,
        decode_shortcut, decode_shortcut_fields, encode_body, encode_folder, encode_folder_fields,
        encode_shortcut, encode_shortcut_fields, read_presence, unzigzag, zigzag,
    };
    use crate::clock_rules::{DeviceId, Stamp};
    use crate::merge_rules::Register;
    use crate::position_rules::Key;
    use crate::state::{Edit, Entry, Folder, ItemId, Presence, Shortcut, State};
    use crate::wire_rules::{Reader, WireError, Writer};
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    const ZERO: DeviceId = DeviceId([0; 16]);
    const FIRST: DeviceId = DeviceId([1; 16]);
    const SECOND: DeviceId = DeviceId([2; 16]);
    const ID: ItemId = ItemId([7; 16]);
    const BASE: u64 = 1_700_000_000_000;

    const fn at(wall: u64, counter: u32, device: DeviceId) -> Stamp {
        Stamp {
            wall,
            counter,
            device,
        }
    }

    fn register<T>(value: T, stamp: Stamp) -> Register<T> {
        Register { value, stamp }
    }

    fn table() -> DeviceTable {
        DeviceTable::ascending(vec![FIRST, SECOND]).unwrap()
    }

    fn key(text: &str) -> Key {
        Key::parse(text).unwrap()
    }

    /// A shortcut that raises every flag and holds a register of each mode.
    fn full() -> Shortcut {
        let base = at(BASE, 2, FIRST);
        Shortcut {
            link: register(String::from("https://example.com/"), base),
            title: register(String::from("Example"), base),
            alias: register(String::from("ex"), NEVER),
            folder: register(Some(ItemId([9; 16])), at(BASE + 5, 0, SECOND)),
            position: register(key("1z"), at(BASE - 10, 7, FIRST)),
            presence: register(Presence::Purged, at(u64::MAX, u32::MAX, SECOND)),
            note: register(String::from("é note"), base),
        }
    }

    /// A shortcut whose registers all share its base stamp.
    fn plain(stamp: Stamp) -> Shortcut {
        Shortcut {
            link: register(String::from("a"), stamp),
            title: register(String::new(), stamp),
            alias: register(String::new(), stamp),
            folder: register(None, stamp),
            position: register(key("V"), stamp),
            presence: register(Presence::Live, stamp),
            note: register(String::new(), stamp),
        }
    }

    /// A folder that raises every flag and holds a register of each mode.
    fn marked() -> Folder {
        let base = at(BASE, 0, SECOND);
        Folder {
            name: register(String::from("仕事"), base),
            mark: register(String::from("📚"), at(BASE + 1, 1, FIRST)),
            parent: register(Some(ItemId([3; 16])), base),
            position: register(key("V"), NEVER),
            presence: register(Presence::Purged, at(0, 0, SECOND)),
        }
    }

    fn written(shortcut: &Shortcut) -> Vec<u8> {
        let mut writer = Writer::new();
        encode_shortcut(&mut writer, &table(), (ID, shortcut)).unwrap();
        writer.finish()
    }

    fn read(bytes: &[u8], devices: &DeviceTable) -> Result<(ItemId, Shortcut), WireError> {
        let mut reader = Reader::new(bytes);
        let read = decode_shortcut(&mut reader, devices)?;
        reader.finish()?;
        Ok(read)
    }

    fn read_folder(bytes: &[u8]) -> Result<(ItemId, Folder), WireError> {
        let mut reader = Reader::new(bytes);
        let read = decode_folder(&mut reader, &table())?;
        reader.finish()?;
        Ok(read)
    }

    /// The start of a shortcut's record: its identifier, its flags, and its base stamp at a wall time with a counter of zero on the device at `place`.
    fn head(flags: u8, wall: u64, place: u64) -> Writer {
        let mut writer = Writer::new();
        writer
            .raw(&ID.0)
            .byte(flags)
            .varint(wall)
            .varint(0)
            .varint(place);
        writer
    }

    /// The fields after a plain shortcut's stamps: the link "a", an empty title, and the position "V".
    fn plain_fields(writer: &mut Writer) -> &mut Writer {
        writer.varint(1).raw(b"a").varint(0).varint(1).raw(b"V")
    }

    #[test]
    fn the_flags_of_each_record_are_the_ones_their_names_say() {
        assert_eq!(SHORTCUT_FLAGS, PURGED | PLACED | NAMED | NOTED | TRASHED);
        assert_eq!(FOLDER_FLAGS, PURGED | PLACED | NAMED | TRASHED);
        assert_eq!(
            [PURGED, PLACED, NAMED, NOTED, TRASHED],
            [0b0_0001, 0b0_0010, 0b0_0100, 0b0_1000, 0b1_0000]
        );
        assert_eq!(read_presence(0), Ok(Presence::Live));
        assert_eq!(read_presence(PURGED | PLACED), Ok(Presence::Purged));
        assert_eq!(read_presence(TRASHED | NAMED), Ok(Presence::Trashed));
        assert_eq!(
            read_presence(PURGED | TRASHED),
            Err(WireError::Tag(0b1_0001)),
            "an item is never both in the trash and gone for good"
        );
    }

    /// The flags a sweep raises over a record that may raise `allowed`: every combination but the one that names both presences.
    fn swept_flags(allowed: u8) -> impl Iterator<Item = u8> {
        (0..=allowed).filter(move |flags| {
            flags & !allowed == 0 && flags & (PURGED | TRASHED) != PURGED | TRASHED
        })
    }

    #[test]
    fn a_shortcut_s_record_is_its_fields_after_its_stamps_and_reads_back() {
        let mut expected = Writer::new();
        expected
            .raw(&ID.0)
            .byte(0x0f)
            .varint(BASE)
            .varint(2)
            .varint(0)
            .byte(0b0101_1000)
            .byte(0b0000_0001)
            .varint(10)
            .varint(0)
            .varint(1)
            .varint(19)
            .varint(7)
            .varint(0)
            .varint(2 * (BASE + 1) - 1)
            .varint(u64::from(u32::MAX))
            .varint(1)
            .varint(20)
            .raw(b"https://example.com/")
            .varint(7)
            .raw(b"Example")
            .varint(2)
            .raw(b"ex")
            .raw(&[9; 16])
            .varint(2)
            .raw(b"1z")
            .varint(7)
            .raw("é note".as_bytes());
        let bytes = written(&full());
        assert_eq!(bytes, expected.finish());
        assert_eq!(read(&bytes, &table()), Ok((ID, full())));
        let plain = plain(at(5, 0, SECOND));
        let mut short = head(0, 5, 1);
        short.byte(0).byte(0);
        plain_fields(&mut short);
        assert_eq!(written(&plain), short.finish());
        assert_eq!(read(&written(&plain), &table()), Ok((ID, plain)));
    }

    #[test]
    fn a_store_s_table_keeps_its_devices_in_any_order_each_once_and_fields_read_back_alone() {
        let kept = DeviceTable::distinct(vec![SECOND, FIRST]).unwrap();
        assert_eq!(kept.devices(), [SECOND, FIRST]);
        assert_eq!((kept.place(SECOND), kept.place(FIRST)), (Some(0), Some(1)));
        assert_eq!(DeviceTable::distinct(vec![FIRST, SECOND, FIRST]), None);
        assert_eq!(
            DeviceTable::distinct(Vec::new()),
            Some(DeviceTable::default())
        );
        let mut shortcut_writer = Writer::new();
        encode_shortcut_fields(&mut shortcut_writer, &table(), &full()).unwrap();
        let shortcut_fields = shortcut_writer.finish();
        assert_eq!(shortcut_fields, written(&full())[ID.0.len()..]);
        let mut shortcut_reader = Reader::new(&shortcut_fields);
        assert_eq!(
            decode_shortcut_fields(&mut shortcut_reader, &table()),
            Ok(full())
        );
        assert_eq!(shortcut_reader.finish(), Ok(()));
        let mut folder_writer = Writer::new();
        encode_folder_fields(&mut folder_writer, &kept, &marked()).unwrap();
        let folder_fields = folder_writer.finish();
        let mut folder_reader = Reader::new(&folder_fields);
        assert_eq!(
            decode_folder_fields(&mut folder_reader, &kept),
            Ok(marked())
        );
        assert_eq!(folder_reader.finish(), Ok(()));
        assert_ne!(
            decode_folder_fields(&mut Reader::new(&folder_fields), &table()),
            Ok(marked()),
            "a place names the device of its own table"
        );
    }

    #[test]
    fn a_folder_s_record_is_its_fields_after_its_stamps_and_reads_back() {
        let mut writer = Writer::new();
        encode_folder(&mut writer, &table(), (ID, &marked())).unwrap();
        let bytes = writer.finish();
        let mut expected = Writer::new();
        expected
            .raw(&ID.0)
            .byte(0b0111)
            .varint(BASE)
            .varint(0)
            .varint(1)
            .byte(0b0110_0001)
            .varint(2)
            .varint(1)
            .varint(0)
            .varint(2 * BASE - 1)
            .varint(0)
            .varint(1)
            .varint(6)
            .raw("仕事".as_bytes())
            .varint(4)
            .raw("📚".as_bytes())
            .raw(&[3; 16])
            .varint(1)
            .raw(b"V");
        assert_eq!(bytes, expected.finish());
        assert_eq!(read_folder(&bytes), Ok((ID, marked())));
    }

    #[test]
    fn a_stamp_beside_the_stamp_no_device_wrote_takes_the_base_s_mode() {
        let devices = DeviceTable::ascending(vec![ZERO, FIRST]).unwrap();
        let shortcut = plain(NEVER);
        let mut writer = Writer::new();
        encode_shortcut(&mut writer, &devices, (ID, &shortcut)).unwrap();
        let bytes = writer.finish();
        let mut expected = head(0, 0, 0);
        expected.byte(0).byte(0);
        plain_fields(&mut expected);
        assert_eq!(
            bytes,
            expected.finish(),
            "a base of wall time zero on the zero device, which every register shares"
        );
        assert_eq!(read(&bytes, &devices), Ok((ID, shortcut)));
    }

    /// The stamps a register takes in the sweeps: the base, the stamp no device wrote, and stamps of their own after and before it on either device.
    fn sweep_stamps(base: Stamp) -> [Stamp; 4] {
        [
            base,
            NEVER,
            at(base.wall.wrapping_add(1), 3, SECOND),
            at(base.wall.wrapping_sub(70), u32::MAX, FIRST),
        ]
    }

    /// The stamp that a code picks from the sweep's four for the register at `place`, two bits a register.
    fn swept(stamps: &[Stamp; 4], code: usize, place: u32) -> Stamp {
        stamps[code.wrapping_shr(place.wrapping_mul(2)) & 0b11]
    }

    #[test]
    #[cfg_attr(miri, ignore = "an exhaustive sweep that the native run covers")]
    fn every_flag_and_every_mode_of_every_register_reads_back_as_written() {
        let devices = table();
        for base in [at(BASE, 0, FIRST), at(0, 9, SECOND)] {
            let stamps = sweep_stamps(base);
            for code in 0..4096 {
                for flags in swept_flags(SHORTCUT_FLAGS) {
                    let text = |flag: u8, text: &str| {
                        if flags & flag == 0 {
                            String::new()
                        } else {
                            String::from(text)
                        }
                    };
                    let shortcut = Shortcut {
                        link: register(String::from("https://example.com/"), base),
                        title: register(String::from("Example"), swept(&stamps, code, 0)),
                        alias: register(text(NAMED, "ex"), swept(&stamps, code, 1)),
                        folder: register(
                            (flags & PLACED != 0).then_some(ItemId([9; 16])),
                            swept(&stamps, code, 2),
                        ),
                        position: register(key("1z"), swept(&stamps, code, 3)),
                        presence: register(read_presence(flags).unwrap(), swept(&stamps, code, 4)),
                        note: register(text(NOTED, "é"), swept(&stamps, code, 5)),
                    };
                    let bytes = written(&shortcut);
                    assert_eq!(bytes.get(ID.0.len()), Some(&flags));
                    assert_eq!(read(&bytes, &devices), Ok((ID, shortcut)), "{code} {flags}");
                }
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "an exhaustive sweep that the native run covers")]
    fn every_flag_and_every_mode_of_every_folder_register_reads_back_as_written() {
        let devices = table();
        let base = at(BASE, 0, SECOND);
        let stamps = sweep_stamps(base);
        for code in 0..256 {
            for flags in swept_flags(FOLDER_FLAGS) {
                let folder = Folder {
                    name: register(String::from("仕事"), base),
                    mark: register(
                        if flags & NAMED == 0 {
                            String::new()
                        } else {
                            String::from("📚")
                        },
                        swept(&stamps, code, 0),
                    ),
                    parent: register(
                        (flags & PLACED != 0).then_some(ItemId([3; 16])),
                        swept(&stamps, code, 1),
                    ),
                    position: register(key("V"), swept(&stamps, code, 2)),
                    presence: register(read_presence(flags).unwrap(), swept(&stamps, code, 3)),
                };
                let mut writer = Writer::new();
                encode_folder(&mut writer, &devices, (ID, &folder)).unwrap();
                let bytes = writer.finish();
                assert_eq!(bytes.get(ID.0.len()), Some(&flags));
                assert_eq!(read_folder(&bytes), Ok((ID, folder)), "{code} {flags}");
            }
        }
    }

    #[test]
    fn zigzag_keeps_small_steps_either_way_small_and_reads_back() {
        let steps: [(i64, u64); 6] = [(0, 0), (-1, 1), (1, 2), (-2, 3), (2, 4), (-10, 19)];
        for (step, form) in steps {
            assert_eq!(zigzag(step.cast_unsigned()), form, "{step}");
            assert_eq!(unzigzag(form), step.cast_unsigned(), "{step}");
        }
        for difference in [u64::MAX, 1 << 63, (1 << 63) - 1, 0x5555_5555_5555_5555] {
            assert_eq!(unzigzag(zigzag(difference)), difference, "{difference}");
        }
        assert_eq!(zigzag(i64::MIN.cast_unsigned()), u64::MAX);
        assert_eq!(zigzag(i64::MAX.cast_unsigned()), u64::MAX - 1);
    }

    #[test]
    fn a_record_the_writer_would_not_write_is_refused() {
        let devices = table();
        let modes = |first: u8, second: u8| {
            let mut writer = head(0, 5, 0);
            writer.byte(first).byte(second);
            writer
        };
        let mut reserved = head(0b10_0000, 5, 0);
        reserved.byte(0).byte(0);
        let mut both = head(PURGED | TRASHED, 5, 0);
        both.byte(0).byte(0);
        plain_fields(&mut both);
        let mut reserved_mode = modes(0b11, 0);
        plain_fields(&mut reserved_mode);
        let mut spare = modes(0, 0b1_0000);
        plain_fields(&mut spare);
        let mut named = head(NAMED, 5, 0);
        named
            .byte(0)
            .byte(0)
            .varint(1)
            .raw(b"a")
            .varint(0)
            .varint(0)
            .varint(1)
            .raw(b"V");
        let mut noted = head(NOTED, 5, 0);
        noted.byte(0).byte(0);
        plain_fields(&mut noted).varint(0);
        let mut beyond = head(0, 5, 2);
        beyond.byte(0).byte(0);
        plain_fields(&mut beyond);
        let mut wide = Writer::new();
        wide.raw(&ID.0).byte(0).varint(5).varint(1 << 32).varint(0);
        let mut unreadable = modes(0, 0);
        unreadable
            .varint(1)
            .raw(&[0xff])
            .varint(0)
            .varint(1)
            .raw(b"V");
        let mut unkeyed = modes(0, 0);
        unkeyed.varint(1).raw(b"a").varint(0).varint(2).raw(b"V0");
        let refusals = [
            (reserved, WireError::Tag(0b10_0000)),
            (both, WireError::Tag(0b1_0001)),
            (reserved_mode, WireError::Tag(0b11)),
            (spare, WireError::Tag(0b1_0000)),
            (named, WireError::Canonical),
            (noted, WireError::Canonical),
            (beyond, WireError::Value),
            (wide, WireError::Value),
            (unreadable, WireError::Value),
            (unkeyed, WireError::Value),
        ];
        for (writer, refusal) in refusals {
            let bytes = writer.finish();
            assert_eq!(read(&bytes, &devices), Err(refusal), "{bytes:?}");
        }
        let full = written(&full());
        for cut in 0..full.len() {
            assert_eq!(
                read(full.get(..cut).unwrap(), &devices),
                Err(WireError::Truncated),
                "{cut}"
            );
        }
    }

    #[test]
    fn a_stamp_in_another_mode_than_its_own_is_refused() {
        let devices = DeviceTable::ascending(vec![ZERO, FIRST]).unwrap();
        let mut unwritten_beside_never = head(0, 0, 0);
        unwritten_beside_never.byte(0b10).byte(0);
        plain_fields(&mut unwritten_beside_never);
        let mut own_never = head(0, 5, 1);
        own_never.byte(0b01).byte(0).varint(9).varint(0).varint(0);
        plain_fields(&mut own_never);
        let mut own_base = head(0, 5, 1);
        own_base.byte(0b01).byte(0).varint(0).varint(0).varint(1);
        plain_fields(&mut own_base);
        for writer in [unwritten_beside_never, own_never, own_base] {
            let bytes = writer.finish();
            assert_eq!(
                read(&bytes, &devices),
                Err(WireError::Canonical),
                "{bytes:?}"
            );
        }
        let mut own = head(0, 5, 1);
        own.byte(0b01).byte(0).varint(1).varint(0).varint(0);
        plain_fields(&mut own);
        assert_eq!(
            read(&own.finish(), &devices).map(|(_, shortcut)| shortcut.title.stamp),
            Ok(at(4, 0, ZERO)),
            "a stamp of its own one step before the base, on another device"
        );
    }

    #[test]
    fn a_folder_record_the_writer_would_not_write_is_refused() {
        let mut writer = Writer::new();
        encode_folder(&mut writer, &table(), (ID, &marked())).unwrap();
        let mut noted = writer.finish();
        noted[ID.0.len()] |= NOTED;
        assert_eq!(read_folder(&noted), Err(WireError::Tag(0b1111)));
        let mut empty = Writer::new();
        empty
            .raw(&ID.0)
            .byte(NAMED)
            .varint(5)
            .varint(0)
            .varint(0)
            .byte(0)
            .varint(1)
            .raw(b"f")
            .varint(0)
            .varint(1)
            .raw(b"V");
        assert_eq!(read_folder(&empty.finish()), Err(WireError::Canonical));
        let mut missing = Writer::new();
        let unknown = Folder {
            mark: register(String::new(), at(1, 0, ZERO)),
            ..marked()
        };
        assert_eq!(
            encode_folder(&mut missing, &table(), (ID, &unknown)),
            Err(WireError::Value)
        );
        let mut unplaced = Writer::new();
        assert_eq!(
            encode_shortcut(&mut unplaced, &table(), (ID, &plain(at(1, 0, ZERO)))),
            Err(WireError::Value)
        );
    }

    /// A state of two devices' edits, with a folder inside another, notes, marks, aliases, a removal for good, and a folder in the trash.
    fn sample() -> State {
        let mut state = State::default();
        let folder = |byte: u8, parent| Edit::AddFolder {
            id: ItemId([byte; 16]),
            name: alloc::format!("folder {byte}"),
            parent,
        };
        let shortcut = |byte: u8, inside| Edit::AddShortcut {
            id: ItemId([byte; 16]),
            link: alloc::format!("https://example.com/{byte}"),
            title: alloc::format!("page {byte}"),
            folder: inside,
        };
        let edits = [
            (folder(1, None), at(1, 0, FIRST)),
            (folder(2, Some(ItemId([1; 16]))), at(2, 0, SECOND)),
            (shortcut(3, Some(ItemId([2; 16]))), at(3, 0, FIRST)),
            (shortcut(4, None), at(4, 1, SECOND)),
            (shortcut(5, None), at(5, 0, FIRST)),
            (
                Edit::SetNote {
                    id: ItemId([4; 16]),
                    note: String::from("read first"),
                },
                at(6, 0, FIRST),
            ),
            (
                Edit::Realias {
                    id: ItemId([3; 16]),
                    alias: String::from("three"),
                },
                at(7, 0, SECOND),
            ),
            (
                Edit::MarkFolder {
                    id: ItemId([1; 16]),
                    mark: String::from("📚"),
                },
                at(8, 0, SECOND),
            ),
            (
                Edit::Remove {
                    entry: Entry::Shortcut(ItemId([5; 16])),
                },
                at(9, 0, SECOND),
            ),
            (
                Edit::Trash {
                    entry: Entry::Folder(ItemId([2; 16])),
                },
                at(10, 0, FIRST),
            ),
        ];
        for (edit, stamp) in edits {
            state.apply(edit, stamp).unwrap();
        }
        state
    }

    #[test]
    fn a_state_reads_back_from_its_body() {
        let state = sample();
        let body = Body::of(&state);
        assert_eq!(DeviceTable::of(&body).devices(), [FIRST, SECOND]);
        let bytes = encode_body(&body).unwrap();
        assert_eq!(decode_body(&bytes).unwrap().into_state(), state);
        assert_eq!(decode_body(&bytes), Ok(body));
        assert_eq!(
            encode_body(&Body::default()).unwrap(),
            [&MAGIC[..], &VERSION.to_be_bytes(), &[0, 0, 0, 0]].concat()
        );
        assert_eq!(
            decode_body(&encode_body(&Body::default()).unwrap()),
            Ok(Body::default())
        );
        let mut empty = State::default();
        empty
            .apply(
                Edit::AddShortcut {
                    id: ItemId([1; 16]),
                    link: String::from("https://example.com/"),
                    title: String::new(),
                    folder: None,
                },
                NEVER,
            )
            .unwrap();
        let never = Body::of(&empty);
        assert_eq!(DeviceTable::of(&never).devices(), [ZERO]);
        assert_eq!(decode_body(&encode_body(&never).unwrap()), Ok(never));
        empty
            .apply(
                Edit::Retitle {
                    id: ItemId([1; 16]),
                    title: String::from("retitled"),
                },
                at(1, 0, SECOND),
            )
            .unwrap();
        let retitled = Body::of(&empty);
        assert_eq!(
            DeviceTable::of(&retitled).devices(),
            [ZERO, SECOND],
            "a device that only a register's own stamp names"
        );
        assert_eq!(decode_body(&encode_body(&retitled).unwrap()), Ok(retitled));
    }

    /// The bytes of a body: its devices, then its folders' and its shortcuts' records, written with the devices of `table`.
    fn body_bytes(
        listed: &[DeviceId],
        folders: &[(ItemId, Folder)],
        shortcuts: &[(ItemId, Shortcut)],
        table: &DeviceTable,
    ) -> Vec<u8> {
        let mut writer = Writer::new();
        writer
            .raw(&MAGIC)
            .short(VERSION)
            .varint(u64::try_from(listed.len()).unwrap());
        for device in listed {
            writer.raw(&device.0);
        }
        writer.varint(u64::try_from(folders.len()).unwrap());
        for (id, folder) in folders {
            encode_folder(&mut writer, table, (*id, folder)).unwrap();
        }
        writer.varint(u64::try_from(shortcuts.len()).unwrap());
        for (id, shortcut) in shortcuts {
            encode_shortcut(&mut writer, table, (*id, shortcut)).unwrap();
        }
        writer.varint(0);
        writer.finish()
    }

    /// Two folders, and two shortcuts written by each device of the table, in ascending order of their identifiers.
    fn items() -> Body {
        Body {
            devices: Vec::new(),
            folders: vec![(ItemId([3; 16]), marked()), (ItemId([4; 16]), marked())],
            shortcuts: vec![
                (ItemId([1; 16]), plain(at(1, 0, FIRST))),
                (ItemId([2; 16]), plain(at(1, 0, SECOND))),
            ],
        }
    }

    #[test]
    fn a_body_whose_head_or_table_the_writer_would_not_write_is_refused() {
        let devices = table();
        let Body {
            folders, shortcuts, ..
        } = items();
        let good = body_bytes(&[FIRST, SECOND], &folders, &shortcuts, &devices);
        assert_eq!(decode_body(&good).map(|body| body.shortcuts.len()), Ok(2));
        let mut magic = good.clone();
        magic[0] = b'X';
        let mut version = good.clone();
        version[5] = 3;
        let mut trailing = good.clone();
        trailing.push(0);
        let first = core::slice::from_ref(&shortcuts[0]);
        let refusals = [
            (magic, WireError::Magic),
            (version, WireError::Version(3)),
            (trailing, WireError::Trailing),
            (
                body_bytes(&[SECOND, FIRST], &[], &shortcuts, &devices),
                WireError::Order,
            ),
            (
                body_bytes(&[FIRST, FIRST], &[], first, &devices),
                WireError::Order,
            ),
            (
                body_bytes(&[FIRST, SECOND], &[], first, &devices),
                WireError::Canonical,
            ),
        ];
        for (bytes, refusal) in refusals {
            assert_eq!(decode_body(&bytes), Err(refusal), "{bytes:?}");
        }
        assert_eq!(
            decode_body(&good[..good.len() - 1]),
            Err(WireError::Truncated)
        );
    }

    #[test]
    fn a_body_whose_items_stand_out_of_order_is_refused_and_does_not_write() {
        let devices = table();
        let Body {
            folders, shortcuts, ..
        } = items();
        let (one, two) = (shortcuts[0].clone(), shortcuts[1].clone());
        let (low, high) = (folders[0].clone(), folders[1].clone());
        let refusals = [
            body_bytes(
                &devices.devices,
                &[high.clone(), low.clone()],
                &shortcuts,
                &devices,
            ),
            body_bytes(
                &devices.devices,
                &[low.clone(), low.clone()],
                &shortcuts,
                &devices,
            ),
            body_bytes(
                &devices.devices,
                &folders,
                &[two.clone(), one.clone()],
                &devices,
            ),
            body_bytes(
                &devices.devices,
                &folders,
                &[one.clone(), one.clone(), two.clone()],
                &devices,
            ),
        ];
        for bytes in refusals {
            assert_eq!(decode_body(&bytes), Err(WireError::Order), "{bytes:?}");
        }
        let unsorted = |placed: Vec<(ItemId, Folder)>, listed: Vec<(ItemId, Shortcut)>| {
            encode_body(&Body {
                devices: Vec::new(),
                folders: placed,
                shortcuts: listed,
            })
        };
        assert_eq!(
            unsorted(vec![high.clone(), low.clone()], Vec::new()),
            Err(WireError::Order)
        );
        assert_eq!(
            unsorted(vec![low, high], vec![two, one]),
            Err(WireError::Order)
        );
    }
}
