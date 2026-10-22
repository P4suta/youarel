use super::{CHUNK_BYTES, blob, columns, integer, rows, run, side, stored};
use worker::{SqlStorage, SqlStorageValue};
use youarel_protocol::relay::{LONGEST_PAYLOAD, Side};
use youarel_relay::StoreError;

pub(super) fn push(
    sql: &SqlStorage,
    nameplate: u16,
    to: Side,
    message: &[u8],
) -> Result<(), StoreError> {
    let size = u64::try_from(message.len()).map_err(|error| StoreError(error.to_string()))?;
    let inserted = rows(
        sql,
        "INSERT INTO mail_messages (nameplate, side, size) VALUES (?, ?, ?) RETURNING id",
        vec![i64::from(nameplate).into(), side(to).into(), stored(size)?],
    )?;
    let row = inserted
        .first()
        .ok_or_else(|| StoreError(String::from("the mailbox lost its new message")))?;
    let [id] = columns(row)?;
    for (index, bytes) in message.chunks(CHUNK_BYTES).enumerate() {
        let part = u64::try_from(index).map_err(|error| StoreError(error.to_string()))?;
        run(
            sql,
            "INSERT INTO mail_parts (message, part, bytes) VALUES (?, ?, ?)",
            vec![id.clone(), stored(part)?, bytes.to_vec().into()],
        )?;
    }
    Ok(())
}

fn read(sql: &SqlStorage, id: &SqlStorageValue, expected: usize) -> Result<Vec<u8>, StoreError> {
    if expected > LONGEST_PAYLOAD {
        return Err(StoreError(String::from(
            "a mailbox message exceeds its limit",
        )));
    }
    let chunks = rows(
        sql,
        "SELECT part, bytes FROM mail_parts WHERE message = ? ORDER BY part",
        vec![id.clone()],
    )?;
    let mut message = Vec::new();
    for (index, row) in chunks.iter().enumerate() {
        let [part, bytes] = columns(row)?;
        let bytes = blob(bytes)?;
        let part =
            usize::try_from(integer(part)?).map_err(|error| StoreError(error.to_string()))?;
        if part != index
            || bytes.is_empty()
            || bytes.len() > CHUNK_BYTES
            || message.len().saturating_add(bytes.len()) > expected
        {
            return Err(StoreError(String::from(
                "a mailbox message's chunks are invalid",
            )));
        }
        message.extend_from_slice(&bytes);
    }
    if message.len() != expected {
        return Err(StoreError(String::from("a mailbox message is incomplete")));
    }
    Ok(message)
}

pub(super) fn pop(
    sql: &SqlStorage,
    nameplate: u16,
    to: Side,
) -> Result<Option<Vec<u8>>, StoreError> {
    let found = rows(
        sql,
        "SELECT id, size FROM mail_messages WHERE nameplate = ? AND side = ? ORDER BY id LIMIT 1",
        vec![i64::from(nameplate).into(), side(to).into()],
    )?;
    let Some(row) = found.first() else {
        return Ok(None);
    };
    let [id, size] = columns(row)?;
    let expected =
        usize::try_from(integer(size)?).map_err(|error| StoreError(error.to_string()))?;
    let message = read(sql, id, expected)?;
    run(
        sql,
        "DELETE FROM mail_parts WHERE message = ?",
        vec![id.clone()],
    )?;
    run(
        sql,
        "DELETE FROM mail_messages WHERE id = ?",
        vec![id.clone()],
    )?;
    Ok(Some(message))
}
