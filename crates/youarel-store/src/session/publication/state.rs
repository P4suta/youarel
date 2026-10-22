use youarel_core::clock_rules::{DeviceId, Stamp};
use youarel_core::segment_rules::{Cursor, Destination, Sequence};
use youarel_core::wire_rules::{Reader, WireError, Writer};

pub(super) const LONGEST_TARGET: usize = 64 * 1024;
const VERSION: u16 = 1;
const NONE: u8 = 0;
const SOME: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicationError {
    Interrupted,
    Target,
    Exhausted,
    Stale,
    MissingPart(Sequence),
    SealedConflict(Sequence),
    UnsealedPart(Sequence),
}

impl core::fmt::Display for PublicationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Interrupted => {
                formatter.write_str("publication stopped before completing its reservation")
            }
            Self::Target => formatter.write_str("the sync target is empty or too long"),
            Self::Exhausted => formatter.write_str("the segment chain has no sequence left"),
            Self::Stale => formatter.write_str("the pending publication changed; read it again"),
            Self::MissingPart(sequence) => {
                write!(formatter, "pending segment {} is missing", sequence.get())
            }
            Self::SealedConflict(sequence) => write!(
                formatter,
                "pending segment {} already has different sealed bytes",
                sequence.get()
            ),
            Self::UnsealedPart(sequence) => write!(
                formatter,
                "pending segment {} has not been sealed",
                sequence.get()
            ),
        }
    }
}

impl std::error::Error for PublicationError {}

#[derive(Clone, Copy, Debug)]
pub struct PublishRequest<'a> {
    pub target: &'a str,
    pub operations: usize,
    pub destination: Destination,
    pub force_snapshot: bool,
    pub deltas_only: bool,
    pub limit: usize,
}

impl<'a> PublishRequest<'a> {
    #[must_use]
    pub const fn new(target: &'a str, operations: usize) -> Self {
        Self {
            target,
            operations,
            destination: Destination::Present,
            force_snapshot: false,
            deltas_only: false,
            limit: youarel_core::segment_rules::LONGEST_BODY,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingPublish {
    pub(super) target: String,
    pub(super) first: Sequence,
    pub(super) last: Sequence,
    pub(super) previous: Option<Cursor>,
    pub(super) watermark: Option<Stamp>,
    pub(super) operations: usize,
    pub(super) snapshot: bool,
    pub(super) body_bytes: usize,
}

impl PendingPublish {
    #[must_use]
    pub const fn first(&self) -> Sequence {
        self.first
    }

    #[must_use]
    pub const fn last(&self) -> Sequence {
        self.last
    }

    #[must_use]
    pub const fn is_snapshot(&self) -> bool {
        self.snapshot
    }

    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    #[must_use]
    pub const fn body_bytes(&self) -> usize {
        self.body_bytes
    }

    pub fn sequences(&self) -> impl Iterator<Item = Sequence> {
        (self.first.get()..=self.last.get()).filter_map(Sequence::new)
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = begin();
        writer.bytes(self.target.as_bytes())?;
        writer.long(self.first.get()).long(self.last.get());
        write_cursor(&mut writer, self.previous);
        writer.byte(u8::from(self.watermark.is_some()));
        if let Some(stamp) = self.watermark {
            writer
                .long(stamp.wall)
                .word(stamp.counter)
                .raw(&stamp.device.0);
        }
        write_size(&mut writer, self.operations)?;
        writer.byte(u8::from(self.snapshot));
        write_size(&mut writer, self.body_bytes)?;
        Ok(writer.finish())
    }

    pub(super) fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = reader(bytes)?;
        let target = target(&mut reader)?;
        let first = sequence(&mut reader)?;
        let last = sequence(&mut reader)?;
        let previous = read_cursor(&mut reader)?;
        let watermark = if flag(&mut reader)? {
            Some(Stamp {
                wall: reader.long()?,
                counter: reader.word()?,
                device: DeviceId(reader.array()?),
            })
        } else {
            None
        };
        let operations = read_size(&mut reader)?;
        let snapshot = flag(&mut reader)?;
        let body_bytes = read_size(&mut reader)?;
        reader.finish()?;
        if first > last
            || last
                .get()
                .checked_sub(first.get())
                .is_none_or(|parts| parts >= u64::from(u32::MAX))
            || previous.map_or_else(
                || first != Sequence::FIRST,
                |previous| previous.sequence().next() != Some(first),
            )
        {
            return Err(WireError::Value);
        }
        Ok(Self {
            target,
            first,
            last,
            previous,
            watermark,
            operations,
            snapshot,
            body_bytes,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PublicationState {
    pub(super) cursor: Option<Cursor>,
    pub(super) retired: Option<Cursor>,
    pub(super) first: Option<Sequence>,
    pub(super) snapshot: Option<Sequence>,
    pub(super) target: Option<String>,
    pub(super) operations: usize,
    pub(super) snapshot_bytes: usize,
    pub(super) delta_bytes: usize,
    pub(super) deltas: u64,
}

impl PublicationState {
    #[must_use]
    pub const fn cursor(&self) -> Option<Cursor> {
        self.cursor
    }

    #[must_use]
    pub const fn chain_cursor(&self) -> Option<Cursor> {
        match self.retired {
            Some(cursor) => Some(cursor),
            None => self.cursor,
        }
    }

    #[must_use]
    pub const fn first(&self) -> Option<Sequence> {
        self.first
    }

    #[must_use]
    pub const fn snapshot(&self) -> Option<Sequence> {
        self.snapshot
    }

    #[must_use]
    pub const fn snapshot_bytes(&self) -> usize {
        self.snapshot_bytes
    }

    #[must_use]
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = begin();
        write_cursor(&mut writer, self.cursor);
        write_cursor(&mut writer, self.retired);
        write_sequence(&mut writer, self.first);
        write_sequence(&mut writer, self.snapshot);
        writer.byte(u8::from(self.target.is_some()));
        if let Some(target) = &self.target {
            writer.bytes(target.as_bytes())?;
        }
        write_size(&mut writer, self.operations)?;
        write_size(&mut writer, self.snapshot_bytes)?;
        write_size(&mut writer, self.delta_bytes)?;
        writer.long(self.deltas);
        Ok(writer.finish())
    }

    pub(super) fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = reader(bytes)?;
        let cursor = read_cursor(&mut reader)?;
        let retired = read_cursor(&mut reader)?;
        let first = read_sequence(&mut reader)?;
        let snapshot = read_sequence(&mut reader)?;
        let target = if flag(&mut reader)? {
            Some(target(&mut reader)?)
        } else {
            None
        };
        let operations = read_size(&mut reader)?;
        let snapshot_bytes = read_size(&mut reader)?;
        let delta_bytes = read_size(&mut reader)?;
        let deltas = reader.long()?;
        reader.finish()?;
        let state = Self {
            cursor,
            retired,
            first,
            snapshot,
            target,
            operations,
            snapshot_bytes,
            delta_bytes,
            deltas,
        };
        if retired.is_some_and(|retired| {
            cursor.is_some_and(|cursor| cursor.sequence() >= retired.sequence())
        }) {
            return Err(WireError::Value);
        }
        match (cursor, first, state.target.as_ref()) {
            (Some(cursor), Some(first), Some(_))
                if first <= cursor.sequence()
                    && snapshot.is_none_or(|snapshot| {
                        first <= snapshot && snapshot <= cursor.sequence()
                    }) => {}
            (None, None, None)
                if snapshot.is_none()
                    && operations == 0
                    && snapshot_bytes == 0
                    && delta_bytes == 0
                    && deltas == 0 => {}
            _ => return Err(WireError::Value),
        }
        Ok(state)
    }
}

fn begin() -> Writer {
    let mut writer = Writer::new();
    writer.short(VERSION);
    writer
}

fn reader(bytes: &[u8]) -> Result<Reader<'_>, WireError> {
    let mut reader = Reader::new(bytes);
    let version = reader.short()?;
    if version != VERSION {
        return Err(WireError::Version(version));
    }
    Ok(reader)
}

fn flag(reader: &mut Reader<'_>) -> Result<bool, WireError> {
    match reader.byte()? {
        NONE => Ok(false),
        SOME => Ok(true),
        tag => Err(WireError::Tag(tag)),
    }
}

fn target(reader: &mut Reader<'_>) -> Result<String, WireError> {
    let bytes = reader.bytes(LONGEST_TARGET)?;
    if bytes.is_empty() {
        return Err(WireError::Value);
    }
    core::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_utf8| WireError::Value)
}

pub(super) fn sequence(reader: &mut Reader<'_>) -> Result<Sequence, WireError> {
    Sequence::new(reader.long()?).ok_or(WireError::Value)
}

fn write_size(writer: &mut Writer, size: usize) -> Result<(), WireError> {
    writer.long(u64::try_from(size).map_err(|_length| WireError::Length)?);
    Ok(())
}

fn read_size(reader: &mut Reader<'_>) -> Result<usize, WireError> {
    usize::try_from(reader.long()?).map_err(|_length| WireError::Length)
}

pub(super) fn write_cursor(writer: &mut Writer, cursor: Option<Cursor>) {
    writer.byte(u8::from(cursor.is_some()));
    if let Some(cursor) = cursor {
        writer.long(cursor.sequence().get()).raw(&cursor.digest());
    }
}

pub(super) fn read_cursor(reader: &mut Reader<'_>) -> Result<Option<Cursor>, WireError> {
    if flag(reader)? {
        Ok(Some(Cursor::new(sequence(reader)?, reader.array()?)))
    } else {
        Ok(None)
    }
}

fn write_sequence(writer: &mut Writer, sequence: Option<Sequence>) {
    writer.byte(u8::from(sequence.is_some()));
    if let Some(sequence) = sequence {
        writer.long(sequence.get());
    }
}

fn read_sequence(reader: &mut Reader<'_>) -> Result<Option<Sequence>, WireError> {
    if flag(reader)? {
        sequence(reader).map(Some)
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::{PendingPublish, PublicationState};
    use youarel_core::clock_rules::{DeviceId, Stamp};
    use youarel_core::segment_rules::{Cursor, Sequence};
    use youarel_core::wire_rules::WireError;

    #[test]
    fn publication_metadata_reads_back_and_refuses_truncated_or_trailing_bytes() {
        let first = Sequence::FIRST;
        let last = Sequence::new(2).unwrap();
        let pending = PendingPublish {
            target: "shared".to_owned(),
            first,
            last,
            previous: None,
            watermark: Some(Stamp {
                wall: 10,
                counter: 1,
                device: DeviceId([5; 16]),
            }),
            operations: 2,
            snapshot: true,
            body_bytes: 128,
        };
        let bytes = pending.encode().unwrap();
        assert_eq!(PendingPublish::decode(&bytes).unwrap(), pending);
        for end in 0..bytes.len() {
            PendingPublish::decode(&bytes[..end]).unwrap_err();
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert_eq!(PendingPublish::decode(&trailing), Err(WireError::Trailing));
        let state = PublicationState {
            cursor: Some(Cursor::new(last, [3; 32])),
            retired: Some(Cursor::new(Sequence::new(4).unwrap(), [4; 32])),
            first: Some(first),
            snapshot: Some(first),
            target: Some("shared".to_owned()),
            operations: 2,
            snapshot_bytes: 128,
            delta_bytes: 10,
            deltas: 1,
        };
        let state_bytes = state.encode().unwrap();
        assert_eq!(PublicationState::decode(&state_bytes).unwrap(), state);
        for end in 0..state_bytes.len() {
            PublicationState::decode(&state_bytes[..end]).unwrap_err();
        }
        assert_eq!(
            PublicationState::decode(&PublicationState::default().encode().unwrap()).unwrap(),
            PublicationState::default()
        );
        let mut invalid = state;
        invalid.retired = invalid.cursor;
        assert_eq!(
            PublicationState::decode(&invalid.encode().unwrap()),
            Err(WireError::Value)
        );
    }
}
