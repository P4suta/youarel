//! The relay's storage traits over a Durable Object's SQLite database.

use std::collections::BTreeMap;
use worker::{SqlStorage, SqlStorageValue};
use youarel_core::relay_rules::{NONCE, Seen};
use youarel_protocol::message::Digest;
use youarel_protocol::relay::{Side, Slot};
use youarel_relay::StoreError;
use youarel_relay::account::AccountStore;
use youarel_relay::directory::DirectoryStore;
use youarel_relay::mailbox::MailStore;

/// The key under which an account's database keeps its genesis.
const GENESIS: &str = "genesis";

/// How the messages table names the side a message travels to.
const TO_INVITER: i64 = 0;
const TO_JOINER: i64 = 1;

fn failed(error: &worker::Error) -> StoreError {
    StoreError(error.to_string())
}

fn rows(
    sql: &SqlStorage,
    query: &str,
    bindings: Vec<SqlStorageValue>,
) -> Result<Vec<Vec<SqlStorageValue>>, StoreError> {
    let cursor = sql.exec(query, bindings).map_err(|error| failed(&error))?;
    cursor
        .raw()
        .map(|row| row.map_err(|error| failed(&error)))
        .collect()
}

fn run(sql: &SqlStorage, query: &str, bindings: Vec<SqlStorageValue>) -> Result<(), StoreError> {
    sql.exec(query, bindings).map_err(|error| failed(&error))?;
    Ok(())
}

/// The `N` columns of a row that a query selects.
fn columns<const N: usize>(row: &[SqlStorageValue]) -> Result<&[SqlStorageValue; N], StoreError> {
    row.try_into()
        .map_err(|_short| StoreError(format!("a row holds no {N} columns")))
}

fn blob(value: &SqlStorageValue) -> Result<Vec<u8>, StoreError> {
    match value {
        SqlStorageValue::Blob(bytes) => Ok(bytes.clone()),
        _ => Err(StoreError(String::from("a column holds no bytes"))),
    }
}

fn array<const N: usize>(value: &SqlStorageValue) -> Result<[u8; N], StoreError> {
    blob(value)?
        .try_into()
        .map_err(|_bytes: Vec<u8>| StoreError(format!("a column holds no {N} bytes")))
}

fn integer(value: &SqlStorageValue) -> Result<u64, StoreError> {
    match value {
        SqlStorageValue::Integer(number) => {
            u64::try_from(*number).map_err(|error| StoreError(error.to_string()))
        }
        _ => Err(StoreError(String::from("a column holds no integer"))),
    }
}

fn stored(number: u64) -> Result<SqlStorageValue, StoreError> {
    let signed = i64::try_from(number).map_err(|error| StoreError(error.to_string()))?;
    SqlStorageValue::try_from_i64(signed).map_err(|error| failed(&error))
}

const fn side(to: Side) -> i64 {
    match to {
        Side::Inviter => TO_INVITER,
        Side::Joiner => TO_JOINER,
    }
}

/// One account's database.
#[derive(Debug)]
pub struct SqlAccount {
    sql: SqlStorage,
}

impl SqlAccount {
    /// The account's database, with its tables made when it is new.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when a table cannot be made.
    pub fn open(sql: SqlStorage) -> Result<Self, StoreError> {
        for table in [
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value BLOB NOT NULL)",
            "CREATE TABLE IF NOT EXISTS log (digest BLOB PRIMARY KEY, bytes BLOB NOT NULL)",
            "CREATE TABLE IF NOT EXISTS slots (device BLOB PRIMARY KEY, published INTEGER NOT NULL, bundle BLOB NOT NULL)",
            "CREATE TABLE IF NOT EXISTS seen (nonce BLOB PRIMARY KEY, stamped INTEGER NOT NULL)",
        ] {
            run(&sql, table, Vec::new())?;
        }
        Ok(Self { sql })
    }
}

impl AccountStore for SqlAccount {
    fn genesis(&self) -> Result<Option<Digest>, StoreError> {
        let found = rows(
            &self.sql,
            "SELECT value FROM meta WHERE key = ?",
            vec![GENESIS.into()],
        )?;
        found
            .first()
            .map(|row| {
                let [value] = columns(row)?;
                array(value)
            })
            .transpose()
    }

    fn set_genesis(&mut self, genesis: Digest) -> Result<(), StoreError> {
        run(
            &self.sql,
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)",
            vec![GENESIS.into(), genesis.to_vec().into()],
        )
    }

    fn log(&self) -> Result<BTreeMap<Digest, Vec<u8>>, StoreError> {
        rows(&self.sql, "SELECT digest, bytes FROM log", Vec::new())?
            .iter()
            .map(|row| {
                let [digest, bytes] = columns(row)?;
                Ok((array(digest)?, blob(bytes)?))
            })
            .collect()
    }

    fn keep_operation(&mut self, digest: Digest, bytes: &[u8]) -> Result<(), StoreError> {
        run(
            &self.sql,
            "INSERT OR IGNORE INTO log (digest, bytes) VALUES (?, ?)",
            vec![digest.to_vec().into(), bytes.to_vec().into()],
        )
    }

    fn slots(&self) -> Result<Vec<Slot>, StoreError> {
        rows(
            &self.sql,
            "SELECT device, published, bundle FROM slots",
            Vec::new(),
        )?
        .iter()
        .map(|row| {
            let [device, published, bundle] = columns(row)?;
            Ok(Slot {
                device: array(device)?,
                published: integer(published)?,
                bundle: blob(bundle)?,
            })
        })
        .collect()
    }

    fn put_slot(&mut self, slot: Slot) -> Result<(), StoreError> {
        run(
            &self.sql,
            "INSERT OR REPLACE INTO slots (device, published, bundle) VALUES (?, ?, ?)",
            vec![
                slot.device.to_vec().into(),
                stored(slot.published)?,
                slot.bundle.into(),
            ],
        )
    }

    fn remove_slot(&mut self, device: &Digest) -> Result<(), StoreError> {
        run(
            &self.sql,
            "DELETE FROM slots WHERE device = ?",
            vec![device.to_vec().into()],
        )
    }

    fn seen(&self) -> Result<Vec<Seen>, StoreError> {
        rows(&self.sql, "SELECT nonce, stamped FROM seen", Vec::new())?
            .iter()
            .map(|row| {
                let [nonce, stamped] = columns(row)?;
                Ok(Seen {
                    nonce: array::<NONCE>(nonce)?,
                    stamped: integer(stamped)?,
                })
            })
            .collect()
    }

    fn set_seen(&mut self, seen: &[Seen]) -> Result<(), StoreError> {
        run(&self.sql, "DELETE FROM seen", Vec::new())?;
        for entry in seen {
            run(
                &self.sql,
                "INSERT OR REPLACE INTO seen (nonce, stamped) VALUES (?, ?)",
                vec![entry.nonce.to_vec().into(), stored(entry.stamped)?],
            )?;
        }
        Ok(())
    }

    fn erase(&mut self) -> Result<(), StoreError> {
        for table in ["meta", "log", "slots", "seen"] {
            run(&self.sql, &format!("DELETE FROM {table}"), Vec::new())?;
        }
        Ok(())
    }
}

/// The directory's database.
#[derive(Debug)]
pub struct SqlDirectory {
    sql: SqlStorage,
}

impl SqlDirectory {
    /// The directory's database, with its table made when it is new.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when the table cannot be made.
    pub fn open(sql: SqlStorage) -> Result<Self, StoreError> {
        run(
            &sql,
            "CREATE TABLE IF NOT EXISTS directory (member BLOB PRIMARY KEY, account BLOB NOT NULL)",
            Vec::new(),
        )?;
        Ok(Self { sql })
    }
}

impl DirectoryStore for SqlDirectory {
    fn account_of(&self, member: &Digest) -> Result<Option<Digest>, StoreError> {
        let found = rows(
            &self.sql,
            "SELECT account FROM directory WHERE member = ?",
            vec![member.to_vec().into()],
        )?;
        found
            .first()
            .map(|row| {
                let [account] = columns(row)?;
                array(account)
            })
            .transpose()
    }

    fn register(&mut self, account: &Digest, members: &[Digest]) -> Result<(), StoreError> {
        for member in members {
            run(
                &self.sql,
                "INSERT OR REPLACE INTO directory (member, account) VALUES (?, ?)",
                vec![member.to_vec().into(), account.to_vec().into()],
            )?;
        }
        Ok(())
    }
}

/// The mailboxes' database.
#[derive(Debug)]
pub struct SqlMail {
    sql: SqlStorage,
}

impl SqlMail {
    /// The mailboxes' database, with its tables made when it is new.
    ///
    /// # Errors
    ///
    /// Returns a [`StoreError`] when a table cannot be made.
    pub fn open(sql: SqlStorage) -> Result<Self, StoreError> {
        for table in [
            "CREATE TABLE IF NOT EXISTS mailboxes (nameplate INTEGER PRIMARY KEY, opened INTEGER NOT NULL)",
            "CREATE TABLE IF NOT EXISTS messages (id INTEGER PRIMARY KEY AUTOINCREMENT, nameplate INTEGER NOT NULL, side INTEGER NOT NULL, message BLOB NOT NULL)",
        ] {
            run(&sql, table, Vec::new())?;
        }
        Ok(Self { sql })
    }
}

impl MailStore for SqlMail {
    fn mailboxes(&self) -> Result<Vec<(u16, u64)>, StoreError> {
        rows(
            &self.sql,
            "SELECT nameplate, opened FROM mailboxes",
            Vec::new(),
        )?
        .iter()
        .map(|row| {
            let [nameplate, opened] = columns(row)?;
            let nameplate = u16::try_from(integer(nameplate)?)
                .map_err(|error| StoreError(error.to_string()))?;
            Ok((nameplate, integer(opened)?))
        })
        .collect()
    }

    fn open_mailbox(&mut self, nameplate: u16, now: u64) -> Result<(), StoreError> {
        run(
            &self.sql,
            "INSERT OR REPLACE INTO mailboxes (nameplate, opened) VALUES (?, ?)",
            vec![i64::from(nameplate).into(), stored(now)?],
        )
    }

    fn close_mailbox(&mut self, nameplate: u16) -> Result<(), StoreError> {
        run(
            &self.sql,
            "DELETE FROM messages WHERE nameplate = ?",
            vec![i64::from(nameplate).into()],
        )?;
        run(
            &self.sql,
            "DELETE FROM mailboxes WHERE nameplate = ?",
            vec![i64::from(nameplate).into()],
        )
    }

    fn held(&self, nameplate: u16) -> Result<usize, StoreError> {
        let found = rows(
            &self.sql,
            "SELECT COALESCE(SUM(LENGTH(message)), 0) FROM messages WHERE nameplate = ?",
            vec![i64::from(nameplate).into()],
        )?;
        let Some(row) = found.first() else {
            return Ok(0);
        };
        let [total] = columns(row)?;
        usize::try_from(integer(total)?).map_err(|error| StoreError(error.to_string()))
    }

    fn push(&mut self, nameplate: u16, to: Side, message: &[u8]) -> Result<(), StoreError> {
        run(
            &self.sql,
            "INSERT INTO messages (nameplate, side, message) VALUES (?, ?, ?)",
            vec![
                i64::from(nameplate).into(),
                side(to).into(),
                message.to_vec().into(),
            ],
        )
    }

    fn pop(&mut self, nameplate: u16, to: Side) -> Result<Option<Vec<u8>>, StoreError> {
        let found = rows(
            &self.sql,
            "SELECT id, message FROM messages WHERE nameplate = ? AND side = ? ORDER BY id LIMIT 1",
            vec![i64::from(nameplate).into(), side(to).into()],
        )?;
        let Some(row) = found.first() else {
            return Ok(None);
        };
        let [id, message] = columns(row)?;
        run(
            &self.sql,
            "DELETE FROM messages WHERE id = ?",
            vec![id.clone()],
        )?;
        Ok(Some(blob(message)?))
    }
}
