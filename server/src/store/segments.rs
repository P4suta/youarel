use super::{CHUNK_BYTES, array, blob, columns, integer, rows, run, stored};
use std::num::NonZeroUsize;
use worker::{SqlStorage, SqlStorageValue};
use youarel_core::folder_rules::{segment_name, segment_sequence};
use youarel_core::relay_rules::RetainedSegment;
use youarel_core::segment_rules::{Header, Kind, LONGEST_ENVELOPE, Sequence};
use youarel_protocol::message::Digest;
use youarel_protocol::segment::Head;
use youarel_relay::StoreError;
use youarel_relay::account::{Append, StoredChain};

fn size(value: &SqlStorageValue) -> Result<usize, StoreError> {
    usize::try_from(integer(value)?).map_err(|error| StoreError(error.to_string()))
}

fn sequence(value: &SqlStorageValue) -> Result<Sequence, StoreError> {
    let SqlStorageValue::String(text) = value else {
        return Err(StoreError(String::from(
            "a sequence holds no canonical text",
        )));
    };
    segment_sequence(text).ok_or_else(|| StoreError(String::from("a sequence is invalid")))
}

fn kind(part: &SqlStorageValue, parts: &SqlStorageValue) -> Result<Kind, StoreError> {
    let part = u32::try_from(integer(part)?).map_err(|error| StoreError(error.to_string()))?;
    let parts = u32::try_from(integer(parts)?).map_err(|error| StoreError(error.to_string()))?;
    Ok(if part == 0 && parts == 0 {
        Kind::Delta
    } else {
        Kind::Snapshot { part, parts }
    })
}

pub(super) fn chains(sql: &SqlStorage) -> Result<Vec<StoredChain>, StoreError> {
    rows(sql, "SELECT c.device, c.published, c.head, COALESCE(SUM(s.size), 0), latest.part, latest.parts FROM chains c LEFT JOIN segments s ON s.device = c.device LEFT JOIN segments latest ON latest.device = c.device AND latest.sequence = (SELECT MAX(sequence) FROM segments WHERE device = c.device) GROUP BY c.device ORDER BY c.device", Vec::new())?
        .iter().map(|row| {
            let [sender, published, head, bytes, part, parts] = columns(row)?;
            let head = Head::decode(&blob(head)?).map_err(|error| StoreError(error.to_string()))?;
            let header = Header::new(head.latest(), [0; 32], kind(part, parts)?).ok_or_else(|| StoreError(String::from("a chain's latest header is invalid")))?;
            Ok(StoredChain {
                sender: array(sender)?, published: integer(published)?, bytes: size(bytes)?,
                head, incomplete: youarel_core::relay_rules::incomplete_snapshot(header),
            })
        }).collect()
}

pub(super) fn chain(sql: &SqlStorage, sender: &Digest) -> Result<Vec<RetainedSegment>, StoreError> {
    rows(sql, "SELECT sequence, previous, part, parts, size FROM segments WHERE device = ? ORDER BY sequence", vec![sender.to_vec().into()])?
        .iter().map(|row| {
            let [number, previous, part, parts, bytes] = columns(row)?;
            let header = Header::new(sequence(number)?, array(previous)?, kind(part, parts)?).ok_or_else(|| StoreError(String::from("an invalid header was stored")))?;
            let bytes = NonZeroUsize::new(size(bytes)?).ok_or_else(|| StoreError(String::from("an empty segment was stored")))?;
            Ok(RetainedSegment { header, bytes })
        }).collect()
}

pub(super) fn segment(
    sql: &SqlStorage,
    sender: &Digest,
    sequence: Sequence,
) -> Result<Option<Vec<u8>>, StoreError> {
    let chunks = rows(
        sql,
        "SELECT part, bytes FROM segment_parts WHERE device = ? AND sequence = ? ORDER BY part",
        vec![
            sender.to_vec().into(),
            segment_name(sequence).as_str().into(),
        ],
    )?;
    if chunks.is_empty() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    for (index, row) in chunks.iter().enumerate() {
        let [part, chunk] = columns(row)?;
        let chunk = blob(chunk)?;
        if size(part)? != index
            || chunk.is_empty()
            || chunk.len() > CHUNK_BYTES
            || bytes.len().saturating_add(chunk.len()) > LONGEST_ENVELOPE
        {
            return Err(StoreError(String::from(
                "a segment's storage chunks are invalid",
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Some(bytes))
}

pub(super) fn append(sql: &SqlStorage, append: &Append<'_>) -> Result<(), StoreError> {
    let sequence = segment_name(append.header.sequence());
    let (part, parts) = match append.header.kind() {
        Kind::Delta => (0, 0),
        Kind::Snapshot { part, parts } => (part, parts),
    };
    run(
        sql,
        "INSERT INTO segments (device, sequence, previous, part, parts, size) VALUES (?, ?, ?, ?, ?, ?)",
        vec![
            append.sender.to_vec().into(),
            sequence.as_str().into(),
            append.header.previous().to_vec().into(),
            i64::from(part).into(),
            i64::from(parts).into(),
            stored(
                u64::try_from(append.bytes.len()).map_err(|error| StoreError(error.to_string()))?,
            )?,
        ],
    )?;
    for (index, chunk) in append.bytes.chunks(CHUNK_BYTES).enumerate() {
        run(
            sql,
            "INSERT INTO segment_parts (device, sequence, part, bytes) VALUES (?, ?, ?, ?)",
            vec![
                append.sender.to_vec().into(),
                sequence.as_str().into(),
                stored(u64::try_from(index).map_err(|error| StoreError(error.to_string()))?)?,
                chunk.to_vec().into(),
            ],
        )?;
    }
    for table in ["segments", "segment_parts"] {
        run(
            sql,
            &format!("DELETE FROM {table} WHERE device = ? AND sequence < ?"),
            vec![
                append.sender.to_vec().into(),
                segment_name(append.head.first()).as_str().into(),
            ],
        )?;
    }
    run(
        sql,
        "INSERT OR REPLACE INTO chains (device, published, head) VALUES (?, ?, ?)",
        vec![
            append.sender.to_vec().into(),
            stored(append.published)?,
            append.head.encode().into(),
        ],
    )
}

pub(super) fn remove(sql: &SqlStorage, sender: &Digest) -> Result<(), StoreError> {
    for table in ["chains", "segments", "segment_parts"] {
        run(
            sql,
            &format!("DELETE FROM {table} WHERE device = ?"),
            vec![sender.to_vec().into()],
        )?;
    }
    Ok(())
}
