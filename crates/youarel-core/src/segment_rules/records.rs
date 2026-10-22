use crate::record_rules::{self, Body, DeviceTable, Record};
use crate::wire_rules::{WireError, Writer};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

/// A bounded snapshot part whose next record stays whole, with exact canonical dictionary sizes.
#[derive(Debug)]
pub struct Parts {
    body: Body,
    table: DeviceTable,
    record_bytes: usize,
    limit: usize,
}

fn records_size(body: &Body, table: &DeviceTable) -> Result<usize, WireError> {
    let mut writer = Writer::new();
    for (id, folder) in &body.folders {
        record_rules::encode_folder(&mut writer, table, (*id, folder))?;
    }
    for (id, shortcut) in &body.shortcuts {
        record_rules::encode_shortcut(&mut writer, table, (*id, shortcut))?;
    }
    for (id, name) in &body.devices {
        record_rules::encode_device(&mut writer, table, (*id, name))?;
    }
    Ok(writer.finish().len())
}

fn single(record: Record) -> Body {
    let mut body = Body::default();
    match record {
        Record::Folder(id, folder) => body.folders.push((id, folder)),
        Record::Shortcut(id, shortcut) => body.shortcuts.push((id, shortcut)),
        Record::Device(id, name) => body.devices.push((id, name)),
    }
    body
}

fn in_order(before: &Body, one: &Body) -> bool {
    let folders = before
        .folders
        .last()
        .zip(one.folders.first())
        .is_none_or(|(last, next)| last.0 < next.0);
    let shortcuts = before
        .shortcuts
        .last()
        .zip(one.shortcuts.first())
        .is_none_or(|(last, next)| last.0 < next.0);
    let names = before
        .devices
        .last()
        .zip(one.devices.first())
        .is_none_or(|(last, next)| last.0 < next.0);
    folders && shortcuts && names
}

impl Parts {
    /// Starts an empty part under a limit no larger than the production body bound.
    ///
    /// # Errors
    ///
    /// Refuses a limit too small for an empty body or larger than the segment body's bound.
    pub fn new(limit: usize) -> Result<Self, WireError> {
        if limit < record_rules::body_framing(0, 0, 0, 0)? || limit > super::LONGEST_BODY {
            return Err(WireError::Length);
        }
        Ok(Self {
            body: Body::default(),
            table: DeviceTable::default(),
            record_bytes: 0,
            limit,
        })
    }

    fn expanded_table(&self, one: &DeviceTable) -> Result<Option<DeviceTable>, WireError> {
        if one
            .devices()
            .iter()
            .all(|device| self.table.place(*device).is_some())
        {
            return Ok(None);
        }
        let devices: BTreeSet<_> = self
            .table
            .devices()
            .iter()
            .chain(one.devices())
            .copied()
            .collect();
        DeviceTable::ascending(devices.into_iter().collect())
            .map(Some)
            .ok_or(WireError::Order)
    }

    fn framing(&self, one: &Body, devices: usize) -> Result<usize, WireError> {
        let folders = self
            .body
            .folders
            .len()
            .checked_add(one.folders.len())
            .ok_or(WireError::Length)?;
        let shortcuts = self
            .body
            .shortcuts
            .len()
            .checked_add(one.shortcuts.len())
            .ok_or(WireError::Length)?;
        let names = self
            .body
            .devices
            .len()
            .checked_add(one.devices.len())
            .ok_or(WireError::Length)?;
        record_rules::body_framing(devices, folders, shortcuts, names)
    }

    /// Keeps a record whole and returns the preceding completed part when this record starts another.
    ///
    /// # Errors
    ///
    /// Refuses an oversized or unordered record without changing the records already held.
    pub fn push(&mut self, record: Record) -> Result<Option<Vec<u8>>, WireError> {
        let mut one = single(record);
        if !in_order(&self.body, &one) {
            return Err(WireError::Order);
        }
        let own_table = DeviceTable::of(&one);
        let expanded = self.expanded_table(&own_table)?;
        let table = expanded.as_ref().unwrap_or(&self.table);
        let before_bytes = if expanded.is_some() {
            records_size(&self.body, table)?
        } else {
            self.record_bytes
        };
        let bytes = before_bytes
            .checked_add(records_size(&one, table)?)
            .ok_or(WireError::Length)?;
        let framing = self.framing(&one, table.devices().len())?;
        if bytes
            .checked_add(framing)
            .is_some_and(|length| length <= self.limit)
        {
            self.body.folders.append(&mut one.folders);
            self.body.shortcuts.append(&mut one.shortcuts);
            self.body.devices.append(&mut one.devices);
            self.record_bytes = bytes;
            if let Some(expanded_table) = expanded {
                self.table = expanded_table;
            }
            return Ok(None);
        }
        let incoming = record_rules::encode_body(&one)?;
        if incoming.len() > self.limit {
            return Err(WireError::Length);
        }
        let completed = record_rules::encode_body(&self.body)?;
        if completed.len() > self.limit {
            return Err(WireError::Length);
        }
        let record_bytes = records_size(&one, &own_table)?;
        self.body = one;
        self.table = own_table;
        self.record_bytes = record_bytes;
        Ok(Some(completed))
    }

    /// Finishes the last part, including an empty snapshot's one part.
    ///
    /// # Errors
    ///
    /// Refuses a malformed body or a body past its byte limit.
    pub fn finish(self) -> Result<Vec<u8>, WireError> {
        let bytes = record_rules::encode_body(&self.body)?;
        if bytes.len() > self.limit {
            return Err(WireError::Length);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::Parts;
    use crate::clock_rules::{DeviceId, Stamp};
    use crate::record_rules::{Body, Record, decode_body};
    use crate::state::{Edit, ItemId, State};
    use crate::wire_rules::WireError;
    use alloc::format;

    fn state() -> State {
        state_of(8)
    }

    fn state_of(count: u8) -> State {
        let mut state = State::default();
        for number in 1..=count {
            state
                .apply(
                    Edit::AddShortcut {
                        id: ItemId([number; 16]),
                        link: format!("https://example.com/{number}"),
                        title: "名".repeat(20),
                        folder: None,
                    },
                    Stamp {
                        wall: u64::from(number),
                        counter: 0,
                        device: DeviceId([number; 16]),
                    },
                )
                .unwrap();
        }
        state
    }

    #[test]
    fn a_growing_dictionary_and_record_counts_cross_the_varint_boundary() {
        let expected = state_of(140);
        let mut parts = Parts::new(50_000).unwrap();
        for (id, shortcut) in &expected.shortcuts {
            assert_eq!(
                parts.push(Record::Shortcut(*id, shortcut.clone())).unwrap(),
                None
            );
        }
        let bytes = parts.finish().unwrap();
        assert_eq!(decode_body(&bytes).unwrap().into_state(), expected);
    }

    #[test]
    fn canonical_parts_keep_every_record_under_the_byte_limit_as_the_device_table_grows() {
        let expected = state();
        let body = Body::of(&expected);
        for limit in [200, 250, 400, 1000] {
            let mut writer = Parts::new(limit).unwrap();
            let mut parts = alloc::vec::Vec::new();
            for (id, shortcut) in &body.shortcuts {
                if let Some(part) = writer
                    .push(Record::Shortcut(*id, shortcut.clone()))
                    .unwrap()
                {
                    parts.push(part);
                }
            }
            parts.push(writer.finish().unwrap());
            let mut received = State::default();
            for part in &parts {
                assert!(part.len() <= limit);
                let decoded = decode_body(part).unwrap();
                received = received.merge(&decoded.into_state());
            }
            assert_eq!(received, expected);
            if limit == 200 {
                assert!(parts.len() > 1);
            }
        }
    }

    #[test]
    fn an_oversized_record_refuses_without_discarding_the_records_already_packed() {
        let mut writer = Parts::new(200).unwrap();
        let body = Body::of(&state());
        let (id, first) = body.shortcuts.first().unwrap();
        writer.push(Record::Shortcut(*id, first.clone())).unwrap();
        assert_eq!(
            writer.push(Record::Shortcut(*id, first.clone())),
            Err(WireError::Order)
        );
        let mut large = first.clone();
        large.note.value = "x".repeat(1000);
        assert_eq!(
            writer.push(Record::Shortcut(ItemId([99; 16]), large)),
            Err(WireError::Length)
        );
        assert_eq!(
            decode_body(&writer.finish().unwrap()).unwrap().shortcuts,
            [(*id, first.clone())]
        );
        assert_eq!(Parts::new(0).unwrap_err(), WireError::Length);
    }
}

#[cfg(kani)]
pub(super) fn proof_body(record: Record) -> Body {
    single(record)
}

#[cfg(kani)]
pub(super) fn proof_record(id: u8, writer: u8, kind: u8) -> Option<Record> {
    use crate::clock_rules::{DeviceId, Stamp};
    use crate::merge_rules::Register;
    use crate::position_rules::Key;
    use crate::state::{Folder, ItemId, Presence, Shortcut};
    use alloc::string::String;
    let stamp = Stamp {
        wall: 1,
        counter: 0,
        device: DeviceId([writer; 16]),
    };
    let position = Key::parse("a1")?;
    if kind == 2 {
        Some(Record::Device(
            crate::device_rules::MemberId([id; 32]),
            Register {
                value: String::from("x"),
                stamp,
            },
        ))
    } else if kind == 1 {
        Some(Record::Folder(
            ItemId([id; 16]),
            Folder {
                name: Register {
                    value: String::from("x"),
                    stamp,
                },
                mark: Register {
                    value: String::new(),
                    stamp,
                },
                parent: Register { value: None, stamp },
                position: Register {
                    value: position,
                    stamp,
                },
                presence: Register {
                    value: Presence::Live,
                    stamp,
                },
            },
        ))
    } else {
        Some(Record::Shortcut(
            ItemId([id; 16]),
            Shortcut {
                link: Register {
                    value: String::from("https://x.test/"),
                    stamp,
                },
                title: Register {
                    value: String::from("x"),
                    stamp,
                },
                alias: Register {
                    value: String::new(),
                    stamp,
                },
                folder: Register { value: None, stamp },
                position: Register {
                    value: position,
                    stamp,
                },
                presence: Register {
                    value: Presence::Live,
                    stamp,
                },
                note: Register {
                    value: String::new(),
                    stamp,
                },
            },
        ))
    }
}
