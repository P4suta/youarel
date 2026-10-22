//! Canonical segments and the cursors a resumable transport exchanges.

use crate::message::{
    Digest, LONGEST_OPERATION, MAGIC, begin, magic, read_operations, read_signature,
    write_operations, write_signature,
};
use crate::wire::{Reader, WireError, Writer, ascending};
use std::collections::BTreeMap;
use youarel_core::segment_rules::{Available, Cursor, Header, Kind, Sequence};
use youarel_crypto::seal::{self, Sealed};
use youarel_crypto::sign::Signature;
use youarel_crypto::{Label, digest};

const SEGMENT: [u8; MAGIC] = *b"YRS1";
const CURSORS: [u8; MAGIC] = *b"YRT1";
const DELIVERED: [u8; MAGIC] = *b"YRD1";
const HEAD: [u8; MAGIC] = *b"YRP1";
pub use youarel_core::segment_rules::{LONGEST_ENVELOPE as LONGEST_MESSAGE, LONGEST_SEGMENT};
pub const MOST_DELIVERED: usize = 1024;
pub use youarel_core::relay_rules::LONGEST_DELIVERY;
const FRAME: usize = 4096;
const DELTA: u8 = 0;
const SNAPSHOT: u8 = 1;
const OFF: u8 = 0;
const ON: u8 = 1;
const COUNT_BYTES: usize = size_of::<u32>();

fn sequence(reader: &mut Reader<'_>) -> Result<Sequence, WireError> {
    Sequence::new(reader.long()?).ok_or(WireError::Value)
}

fn flag(reader: &mut Reader<'_>) -> Result<bool, WireError> {
    match reader.byte()? {
        OFF => Ok(false),
        ON => Ok(true),
        tag => Err(WireError::Tag(tag)),
    }
}

/// A transport's retained range and newest complete snapshot, when one is available.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Head {
    first: Sequence,
    latest: Sequence,
    snapshot: Option<Sequence>,
}

impl Head {
    #[must_use]
    pub fn new(first: Sequence, latest: Sequence, snapshot: Option<Sequence>) -> Option<Self> {
        (first <= latest && snapshot.is_none_or(|start| first <= start && start <= latest))
            .then_some(Self {
                first,
                latest,
                snapshot,
            })
    }

    #[must_use]
    pub const fn first(self) -> Sequence {
        self.first
    }

    #[must_use]
    pub const fn latest(self) -> Sequence {
        self.latest
    }

    #[must_use]
    pub const fn snapshot(self) -> Option<Sequence> {
        self.snapshot
    }

    #[must_use]
    pub const fn available(self) -> Available {
        Available {
            first: self.first,
            snapshot: self.snapshot,
        }
    }

    fn write(self, writer: &mut Writer) {
        writer.long(self.first.get()).long(self.latest.get());
        writer.byte(u8::from(self.snapshot.is_some()));
        if let Some(snapshot) = self.snapshot {
            writer.long(snapshot.get());
        }
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, WireError> {
        let first = sequence(reader)?;
        let latest = sequence(reader)?;
        let snapshot = if flag(reader)? {
            Some(sequence(reader)?)
        } else {
            None
        };
        Self::new(first, latest, snapshot).ok_or(WireError::Value)
    }

    #[must_use]
    pub fn encode(self) -> Vec<u8> {
        let mut writer = begin(HEAD);
        self.write(&mut writer);
        writer.finish()
    }

    /// Reads a committed transport head.
    ///
    /// # Errors
    ///
    /// Refuses zero or reversed sequences, unknown flags, and trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        magic(&mut reader, HEAD)?;
        let head = Self::read(&mut reader)?;
        reader.finish()?;
        Ok(head)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Cursors {
    pub peers: BTreeMap<Digest, Cursor>,
}

impl Cursors {
    /// The reader's cursors in device order.
    ///
    /// # Errors
    ///
    /// Refuses more peers than the membership log admits.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.peers.len() > crate::message::MOST_SLOTS {
            return Err(WireError::Length);
        }
        let mut writer = begin(CURSORS);
        writer.count(self.peers.len())?;
        for (sender, cursor) in &self.peers {
            writer
                .raw(sender)
                .long(cursor.sequence().get())
                .raw(&cursor.digest());
        }
        Ok(writer.finish())
    }

    /// Reads a reader's cursors in their one order.
    ///
    /// # Errors
    ///
    /// Refuses duplicate or unordered peers, zero sequences, and oversized lists.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        magic(&mut reader, CURSORS)?;
        let count = reader.count(crate::message::MOST_SLOTS)?;
        let mut peers = BTreeMap::new();
        let mut previous = None;
        for _ in 0..count {
            let sender = reader.array()?;
            if previous.is_some_and(|previous| previous >= sender) {
                return Err(WireError::Order);
            }
            previous = Some(sender);
            let cursor = Cursor::new(sequence(&mut reader)?, reader.array()?);
            peers.insert(sender, cursor);
        }
        reader.finish()?;
        Ok(Self { peers })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chain {
    pub sender: Digest,
    pub incomplete: bool,
    pub published: u64,
    pub head: Head,
    pub segments: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Delivered {
    pub account: Digest,
    pub operations: Vec<Vec<u8>>,
    pub chains: Vec<Chain>,
    pub more: bool,
}

impl Delivered {
    fn bounded(&self) -> Result<(), WireError> {
        if self.chains.len() > youarel_core::relay_rules::MOST_SLOTS
            || !ascending(self.chains.iter().map(|chain| chain.sender))
        {
            return Err(WireError::Order);
        }
        let operations = self
            .operations
            .iter()
            .try_fold(crate::relay::LONGEST_FRAME, |size, operation| {
                if operation.len() > LONGEST_OPERATION {
                    return None;
                }
                size.checked_add(operation.len())?.checked_add(COUNT_BYTES)
            })
            .ok_or(WireError::Length)?;
        let mut segments = self.chains.iter().flat_map(|chain| &chain.segments);
        let (count, bytes) = segments
            .try_fold((0_usize, operations), |(count, size), segment| {
                if segment.len() > LONGEST_MESSAGE {
                    return None;
                }
                Some((
                    count.checked_add(1)?,
                    size.checked_add(segment.len())?.checked_add(COUNT_BYTES)?,
                ))
            })
            .ok_or(WireError::Length)?;
        if count > MOST_DELIVERED || bytes > LONGEST_DELIVERY {
            return Err(WireError::Length);
        }
        Ok(())
    }

    /// A bounded page of peer chains and the public membership log.
    ///
    /// # Errors
    ///
    /// Refuses unordered peers or operations, or a page past its byte or segment limits.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        self.bounded()?;
        let mut writer = begin(DELIVERED);
        writer.raw(&self.account);
        write_operations(&mut writer, &self.operations)?;
        writer.count(self.chains.len())?;
        for chain in &self.chains {
            writer
                .raw(&chain.sender)
                .long(chain.published)
                .byte(u8::from(chain.incomplete));
            chain.head.write(&mut writer);
            writer.count(chain.segments.len())?;
            for segment in &chain.segments {
                writer.bytes(segment)?;
            }
        }
        writer.byte(u8::from(self.more));
        Ok(writer.finish())
    }

    /// Reads a transport page without trusting its segment signatures or its head hints.
    ///
    /// # Errors
    ///
    /// Refuses malformed fields, oversized pages, unordered lists, and trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() > LONGEST_DELIVERY {
            return Err(WireError::Length);
        }
        let mut reader = Reader::new(bytes);
        magic(&mut reader, DELIVERED)?;
        let account = reader.array()?;
        let operations = read_operations(&mut reader)?;
        let count = reader.count(youarel_core::relay_rules::MOST_SLOTS)?;
        let mut chains = Vec::with_capacity(count);
        for _ in 0..count {
            let sender = reader.array()?;
            let published = reader.long()?;
            let incomplete = flag(&mut reader)?;
            let head = Head::read(&mut reader)?;
            let segment_count = reader.count(MOST_DELIVERED)?;
            let mut segments = Vec::with_capacity(segment_count);
            for _ in 0..segment_count {
                segments.push(reader.bytes(LONGEST_MESSAGE)?.to_vec());
            }
            chains.push(Chain {
                sender,
                incomplete,
                published,
                head,
                segments,
            });
        }
        let more = flag(&mut reader)?;
        reader.finish()?;
        let delivered = Self {
            account,
            operations,
            chains,
            more,
        };
        delivered.bounded()?;
        Ok(delivered)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Segment {
    pub account: Digest,
    pub operations: Vec<Vec<u8>>,
    pub sender: Digest,
    pub epoch: u32,
    pub header: Header,
    pub sealed: Sealed,
    pub signature: Signature,
}

#[derive(Clone, Copy, Debug)]
pub struct Unsigned<'a> {
    pub account: &'a Digest,
    pub operations: &'a [Vec<u8>],
    pub sender: &'a Digest,
    pub epoch: u32,
    pub header: Header,
    pub sealed: &'a Sealed,
}

fn write_header(writer: &mut Writer, header: Header) {
    writer.long(header.sequence().get()).raw(&header.previous());
    match header.kind() {
        Kind::Delta => {
            writer.byte(DELTA);
        }
        Kind::Snapshot { part, parts } => {
            writer.byte(SNAPSHOT).word(part).word(parts);
        }
    }
}

fn read_header(reader: &mut Reader<'_>) -> Result<Header, WireError> {
    let sequence = Sequence::new(reader.long()?).ok_or(WireError::Value)?;
    let previous = reader.array()?;
    let kind = match reader.byte()? {
        DELTA => Kind::Delta,
        SNAPSHOT => Kind::Snapshot {
            part: reader.word()?,
            parts: reader.word()?,
        },
        tag => return Err(WireError::Tag(tag)),
    };
    Header::new(sequence, previous, kind).ok_or(WireError::Value)
}

impl Unsigned<'_> {
    fn bounded(&self) -> Result<(), WireError> {
        if self.sealed.ciphertext.len() < seal::TAG
            || self.sealed.ciphertext.len() > LONGEST_SEGMENT.saturating_add(seal::TAG)
            || self
                .operations
                .iter()
                .any(|operation| operation.len() > LONGEST_OPERATION)
        {
            return Err(WireError::Length);
        }
        let initial = self
            .sealed
            .ciphertext
            .len()
            .checked_add(FRAME)
            .ok_or(WireError::Length)?;
        let size = self.operations.iter().try_fold(initial, |size, operation| {
            size.checked_add(operation.len())?.checked_add(COUNT_BYTES)
        });
        if size.is_none_or(|size| size > LONGEST_MESSAGE) {
            return Err(WireError::Length);
        }
        Ok(())
    }

    /// The bytes authenticated by the sender.
    ///
    /// # Errors
    ///
    /// Refuses oversized fields or a membership log out of digest order.
    pub fn bytes(&self) -> Result<Vec<u8>, WireError> {
        self.bounded()?;
        let mut writer = begin(SEGMENT);
        writer.raw(self.account);
        write_operations(&mut writer, self.operations)?;
        writer.raw(self.sender).word(self.epoch);
        write_header(&mut writer, self.header);
        writer.raw(&self.sealed.commitment).raw(&self.sealed.nonce);
        writer.bytes(&self.sealed.ciphertext)?;
        Ok(writer.finish())
    }
}

/// The state seal's associated data, including the complete chain header.
#[must_use]
pub fn binding(account: &Digest, sender: &Digest, epoch: u32, header: Header) -> Vec<u8> {
    let mut writer = Writer::new();
    writer
        .raw(Label::State.text().as_bytes())
        .byte(0)
        .short(crate::message::VERSION)
        .raw(account)
        .raw(sender)
        .word(epoch);
    write_header(&mut writer, header);
    writer.finish()
}

/// The deterministic plaintext identity that a successor names, independent of a sealing nonce.
#[must_use]
pub fn body_digest(body: &[u8]) -> Digest {
    digest(Label::Segment, &[body])
}

impl Segment {
    #[must_use]
    pub fn unsigned(&self) -> Unsigned<'_> {
        Unsigned {
            account: &self.account,
            operations: &self.operations,
            sender: &self.sender,
            epoch: self.epoch,
            header: self.header,
            sealed: &self.sealed,
        }
    }

    #[must_use]
    pub fn binding(&self) -> Vec<u8> {
        binding(&self.account, &self.sender, self.epoch, self.header)
    }

    /// The signed segment in its canonical form.
    ///
    /// # Errors
    ///
    /// Refuses the same fields as [`Unsigned::bytes`].
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = Writer::new();
        writer.raw(&self.unsigned().bytes()?);
        write_signature(&mut writer, &self.signature);
        Ok(writer.finish())
    }

    /// Reads a bounded, canonical segment without trusting its signature or opening its body.
    ///
    /// # Errors
    ///
    /// Refuses unknown versions, invalid headers, oversized fields, and trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() > LONGEST_MESSAGE {
            return Err(WireError::Length);
        }
        let mut reader = Reader::new(bytes);
        magic(&mut reader, SEGMENT)?;
        let account = reader.array()?;
        let operations = read_operations(&mut reader)?;
        let sender = reader.array()?;
        let epoch = reader.word()?;
        let header = read_header(&mut reader)?;
        let sealed = Sealed {
            commitment: reader.array()?,
            nonce: reader.array()?,
            ciphertext: reader
                .bytes(LONGEST_SEGMENT.saturating_add(seal::TAG))?
                .to_vec(),
        };
        let signature = read_signature(&mut reader)?;
        reader.finish()?;
        let segment = Self {
            account,
            operations,
            sender,
            epoch,
            header,
            sealed,
            signature,
        };
        segment.unsigned().bounded()?;
        Ok(segment)
    }
}

#[cfg(test)]
mod tests {
    use super::{Chain, Cursors, Delivered, Head, Segment};
    use crate::wire::WireError;
    use youarel_core::segment_rules::{Header, Kind, Sequence};
    use youarel_crypto::Label;
    use youarel_crypto::seal::{self, Sealed};
    use youarel_crypto::sign::SigningKeys;

    #[test]
    fn retired_bundle_and_collection_magics_are_refused() {
        for bytes in [b"YRB1\0\x01".as_slice(), b"YRL1\0\x01".as_slice()] {
            assert_eq!(Segment::decode(bytes), Err(WireError::Magic));
            assert_eq!(Delivered::decode(bytes), Err(WireError::Magic));
        }
    }

    #[test]
    fn delivery_peer_and_segment_counts_keep_their_wire_bounds() {
        let sequence = Sequence::FIRST;
        let chain = |number| Chain {
            sender: [number; 32],
            incomplete: false,
            published: u64::MAX,
            head: Head::new(sequence, sequence, Some(sequence)).unwrap(),
            segments: Vec::new(),
        };
        let full = Delivered {
            account: [0; 32],
            operations: Vec::new(),
            chains: (0..64).map(chain).collect(),
            more: false,
        };
        assert_eq!(Delivered::decode(&full.encode().unwrap()).unwrap(), full);
        let mut crowded = full;
        crowded.chains.push(chain(64));
        assert_eq!(crowded.encode(), Err(WireError::Order));
        let mut messages = Delivered {
            account: [0; 32],
            operations: Vec::new(),
            chains: vec![Chain {
                segments: vec![vec![0]; super::MOST_DELIVERED],
                ..chain(0)
            }],
            more: true,
        };
        assert_eq!(
            Delivered::decode(&messages.encode().unwrap()).unwrap(),
            messages
        );
        messages.chains.first_mut().unwrap().segments.push(vec![0]);
        assert_eq!(messages.encode(), Err(WireError::Length));
    }

    #[test]
    fn cursors_and_deliveries_keep_the_peer_boundaries_and_snapshot_hints() {
        let segment = segment(Kind::Snapshot { part: 0, parts: 1 });
        let sequence = segment.header.sequence();
        let head = Head::new(sequence, sequence, Some(sequence)).unwrap();
        let head_bytes = head.encode();
        assert_eq!(Head::decode(&head_bytes).unwrap(), head);
        assert_eq!(head.available().snapshot, Some(sequence));
        let mut cursors = Cursors::default();
        cursors.peers.insert(
            segment.sender,
            youarel_core::segment_rules::Cursor::new(sequence, [5; 32]),
        );
        assert_eq!(
            Cursors::decode(&cursors.encode().unwrap()).unwrap(),
            cursors
        );
        let delivered = Delivered {
            account: segment.account,
            operations: Vec::new(),
            chains: vec![Chain {
                sender: segment.sender,
                incomplete: true,
                published: 7,
                head,
                segments: vec![segment.encode().unwrap()],
            }],
            more: true,
        };
        let bytes = delivered.encode().unwrap();
        assert_eq!(Delivered::decode(&bytes).unwrap(), delivered);
        let mut trailing = bytes;
        trailing.push(0);
        assert_eq!(Delivered::decode(&trailing), Err(WireError::Trailing));
        assert!(Head::new(Sequence::new(5).unwrap(), sequence, None).is_none());
        assert!(Head::new(sequence, sequence, Sequence::new(3)).is_none());
        assert!(Head::new(sequence, sequence, Sequence::new(5)).is_none());
    }

    #[test]
    fn a_delivery_refuses_duplicate_peers_and_segment_length_overflows() {
        let segment = segment(Kind::Delta);
        let sequence = segment.header.sequence();
        let chain = Chain {
            sender: segment.sender,
            incomplete: false,
            published: 0,
            head: Head::new(sequence, sequence, None).unwrap(),
            segments: Vec::new(),
        };
        let delivered = Delivered {
            account: segment.account,
            operations: Vec::new(),
            chains: vec![chain.clone(), chain],
            more: false,
        };
        assert_eq!(delivered.encode(), Err(WireError::Order));
        let mut too_long = segment;
        too_long.sealed.ciphertext = vec![0; super::LONGEST_SEGMENT + seal::TAG + 1];
        assert_eq!(too_long.encode(), Err(WireError::Length));
    }

    fn segment(kind: Kind) -> Segment {
        let keys = SigningKeys::generate().unwrap();
        let mut segment = Segment {
            account: [1; 32],
            operations: Vec::new(),
            sender: [2; 32],
            epoch: 3,
            header: Header::new(Sequence::new(4).unwrap(), [3; 32], kind).unwrap(),
            sealed: Sealed {
                commitment: [5; 32],
                nonce: [6; 24],
                ciphertext: vec![7; seal::TAG],
            },
            signature: keys.sign(Label::Segment, b"").unwrap(),
        };
        segment.signature = keys
            .sign(Label::Segment, &segment.unsigned().bytes().unwrap())
            .unwrap();
        assert!(keys.public().verify(
            Label::Segment,
            &segment.unsigned().bytes().unwrap(),
            &segment.signature
        ));
        let mut changed = segment.clone();
        changed.header = Header::new(Sequence::new(5).unwrap(), [3; 32], kind).unwrap();
        assert!(!keys.public().verify(
            Label::Segment,
            &changed.unsigned().bytes().unwrap(),
            &segment.signature
        ));
        segment
    }

    #[test]
    fn each_segment_kind_reads_back_and_every_shorter_form_is_truncated() {
        for kind in [
            Kind::Delta,
            Kind::Snapshot { part: 0, parts: 2 },
            Kind::Snapshot { part: 1, parts: 2 },
        ] {
            let segment = segment(kind);
            let bytes = segment.encode().unwrap();
            assert_eq!(Segment::decode(&bytes).unwrap(), segment);
            for end in 0..bytes.len() {
                Segment::decode(&bytes[..end]).unwrap_err();
            }
            let mut trailing = bytes;
            trailing.push(0);
            assert_eq!(Segment::decode(&trailing), Err(WireError::Trailing));
        }
    }

    #[test]
    fn a_segment_binds_its_complete_chain_header_to_the_seal() {
        let segment = segment(Kind::Delta);
        let key = [8; 32];
        let binding = segment.binding();
        let sealed = seal::seal(&key, &binding, b"records").unwrap();
        assert_eq!(
            seal::open(&key, &binding, &sealed).unwrap().as_slice(),
            b"records"
        );
        let mut changed = segment;
        changed.header = Header::new(Sequence::new(4).unwrap(), [9; 32], Kind::Delta).unwrap();
        seal::open(&key, &changed.binding(), &sealed).unwrap_err();
    }
}
