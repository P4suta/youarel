use super::{Record, Session, StoreError};
use crate::database::{PEER_CURSORS, failed, open};
use redb::ReadableTable as _;
use std::collections::BTreeMap;
use youarel_core::segment_rules::{
    self, Available, Cursor, Decision, Digest, Header, Sequence, Verdict,
};
use youarel_core::wire_rules::{Reader, WireError, Writer};

#[derive(Clone, Copy, Debug)]
pub struct ReceivedSegment<'a> {
    pub sender: Digest,
    pub header: Header,
    pub available: Available,
    pub body: &'a [u8],
}

fn cursor(bytes: &[u8]) -> Result<Cursor, StoreError> {
    let mut reader = Reader::new(bytes);
    let sequence = Sequence::new(reader.long().map_err(StoreError::Damaged)?)
        .ok_or(StoreError::Damaged(WireError::Value))?;
    let digest = reader.array().map_err(StoreError::Damaged)?;
    reader.finish().map_err(StoreError::Damaged)?;
    Ok(Cursor::new(sequence, digest))
}

fn encode(cursor: Cursor) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.long(cursor.sequence().get()).raw(&cursor.digest());
    writer.finish()
}

impl Session {
    /// The last segment committed with this peer's records.
    ///
    /// # Errors
    ///
    /// Returns a database or cursor error.
    pub fn peer_cursor(&self, peer: &Digest) -> Result<Option<Cursor>, StoreError> {
        open(&self.transaction, PEER_CURSORS)?
            .get(peer.as_slice())
            .map_err(failed)?
            .map_or_else(|| Ok(None), |held| cursor(held.value()).map(Some))
    }

    /// Every peer's committed cursor in device order.
    ///
    /// # Errors
    ///
    /// Returns a database or cursor error.
    pub fn peer_cursors(&self) -> Result<BTreeMap<Digest, Cursor>, StoreError> {
        let mut peers = BTreeMap::new();
        for entry in open(&self.transaction, PEER_CURSORS)?
            .iter()
            .map_err(failed)?
        {
            let (key, value) = entry.map_err(failed)?;
            let peer = key
                .value()
                .try_into()
                .map_err(|_length| StoreError::Damaged(WireError::Value))?;
            peers.insert(peer, cursor(value.value())?);
        }
        Ok(peers)
    }

    /// Admits authenticated plaintext and commits its record merges and cursor together.
    ///
    /// # Errors
    ///
    /// Refuses malformed records, unadmitted links, or a database error without committing a partial merge.
    pub fn receive_segment(
        &mut self,
        received: ReceivedSegment<'_>,
        digest: impl Fn(&[u8]) -> Digest,
    ) -> Result<Decision, StoreError> {
        if received.body.len() > segment_rules::LONGEST_SEGMENT {
            return Err(StoreError::Received(crate::codec::DecodeError::Records(
                WireError::Length,
            )));
        }
        let body = crate::records::decode_body(received.body).map_err(StoreError::Received)?;
        let decision = segment_rules::step(
            self.peer_cursor(&received.sender)?,
            received.header,
            digest(received.body),
            received.available,
        );
        if decision.verdict == Verdict::Apply {
            for (id, name) in body.devices {
                self.merge_record(Record::Device(id, name))?;
            }
            for (id, folder) in body.folders {
                self.merge_record(Record::Folder(id, folder))?;
            }
            for (id, shortcut) in body.shortcuts {
                self.merge_record(Record::Shortcut(id, shortcut))?;
            }
            let cursor = decision
                .cursor
                .ok_or(StoreError::Missing("an applied segment cursor"))?;
            open(&self.transaction, PEER_CURSORS)?
                .insert(received.sender.as_slice(), encode(cursor).as_slice())
                .map_err(failed)?;
            self.changed = true;
        }
        Ok(decision)
    }
}
