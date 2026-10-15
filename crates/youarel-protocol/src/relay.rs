//! The messages of the optional relay in `specs/WIRE.md`: a member's signed request, the bundles the relay hands back, and the unsigned mail of a pairing.

use crate::message::{Digest, MAGIC, VERSION, begin, magic, read_signature, write_signature};
use crate::wire::{Reader, WireError, Writer, ascending};
use youarel_core::relay_rules::{MOST_SLOTS, NONCE, Nonce};
use youarel_crypto::sign::Signature;

/// The magic of each relay message.
const REQUEST: [u8; MAGIC] = *b"YRQ1";
const COLLECTED: [u8; MAGIC] = *b"YRL1";
const MAIL: [u8; MAGIC] = *b"YRM1";

/// The longest payload a request or a piece of mail carries: a bundle, or the entry of a pairing.
pub const LONGEST_PAYLOAD: usize = 32 * 1024 * 1024;

/// The most bytes a relay message adds around its payload or a slot around its bundle.
pub const LONGEST_FRAME: usize = 64 * 1024;

/// The tags of the verbs and sides.
const PUBLISH: u8 = 0;
const COLLECT: u8 = 1;
const ERASE: u8 = 2;
const OPEN: u8 = 0;
const DROP: u8 = 1;
const PICK: u8 = 2;
const TO_INVITER: u8 = 0;
const TO_JOINER: u8 = 1;

/// What a member asks of the relay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verb {
    /// Keep the payload, the member's newest bundle, in its slot.
    Publish,
    /// Hand back the newest bundle of every member.
    Collect,
    /// Delete the account and everything the relay keeps of it.
    Erase,
}

impl Verb {
    const fn tag(self) -> u8 {
        match self {
            Self::Publish => PUBLISH,
            Self::Collect => COLLECT,
            Self::Erase => ERASE,
        }
    }

    const fn from_tag(tag: u8) -> Result<Self, WireError> {
        match tag {
            PUBLISH => Ok(Self::Publish),
            COLLECT => Ok(Self::Collect),
            ERASE => Ok(Self::Erase),
            other => Err(WireError::Tag(other)),
        }
    }
}

/// A member's request, which it signs so that the relay admits it only once and only from a member.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub account: Digest,
    pub device: Digest,
    pub verb: Verb,
    /// The member's clock when it signed, in milliseconds since the Unix epoch.
    pub stamped: u64,
    pub nonce: Nonce,
    pub payload: Vec<u8>,
    pub signature: Signature,
}

/// The fields of a request that its signature covers.
#[derive(Clone, Copy, Debug)]
pub struct UnsignedRequest<'a> {
    pub account: &'a Digest,
    pub device: &'a Digest,
    pub verb: Verb,
    pub stamped: u64,
    pub nonce: &'a Nonce,
    pub payload: &'a [u8],
}

impl UnsignedRequest<'_> {
    /// The bytes the member signs.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Length`] for a payload beyond its limit.
    pub fn bytes(&self) -> Result<Vec<u8>, WireError> {
        if self.payload.len() > LONGEST_PAYLOAD {
            return Err(WireError::Length);
        }
        let mut writer = begin(REQUEST);
        writer
            .raw(self.account)
            .raw(self.device)
            .byte(self.verb.tag())
            .long(self.stamped)
            .raw(self.nonce);
        writer.bytes(self.payload)?;
        Ok(writer.finish())
    }
}

impl Request {
    #[must_use]
    pub fn unsigned(&self) -> UnsignedRequest<'_> {
        UnsignedRequest {
            account: &self.account,
            device: &self.device,
            verb: self.verb,
            stamped: self.stamped,
            nonce: &self.nonce,
            payload: &self.payload,
        }
    }

    /// The request's bytes.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Length`] for a payload beyond its limit.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = Writer::new();
        writer.raw(&self.unsigned().bytes()?);
        write_signature(&mut writer, &self.signature);
        Ok(writer.finish())
    }

    /// The request `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for any bytes that encoding never writes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        magic(&mut reader, REQUEST)?;
        let account = reader.array()?;
        let device = reader.array()?;
        let verb = Verb::from_tag(reader.byte()?)?;
        let stamped = reader.long()?;
        let nonce = reader.array::<NONCE>()?;
        let payload = reader.bytes(LONGEST_PAYLOAD)?.to_vec();
        let signature = read_signature(&mut reader)?;
        reader.finish()?;
        Ok(Self {
            account,
            device,
            verb,
            stamped,
            nonce,
            payload,
            signature,
        })
    }
}

/// One member's newest bundle as the relay keeps it, with when it arrived.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Slot {
    pub device: Digest,
    /// The relay's clock when the bundle arrived, in milliseconds since the Unix epoch.
    pub published: u64,
    pub bundle: Vec<u8>,
}

/// The relay's answer to a collection: every member's newest bundle, in ascending order of the devices.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Collected {
    pub slots: Vec<Slot>,
}

impl Collected {
    /// The collection's bytes.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for more slots than an account keeps, slots out of order, or a bundle beyond its limit.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.slots.len() > MOST_SLOTS || !ascending(self.slots.iter().map(|slot| slot.device)) {
            return Err(WireError::Order);
        }
        let mut writer = begin(COLLECTED);
        writer.count(self.slots.len())?;
        for slot in &self.slots {
            if slot.bundle.len() > LONGEST_PAYLOAD {
                return Err(WireError::Length);
            }
            writer.raw(&slot.device).long(slot.published);
            writer.bytes(&slot.bundle)?;
        }
        Ok(writer.finish())
    }

    /// The collection `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for any bytes that encoding never writes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        magic(&mut reader, COLLECTED)?;
        let count = reader.count(MOST_SLOTS)?;
        let mut slots = Vec::with_capacity(count);
        for _ in 0..count {
            slots.push(Slot {
                device: reader.array()?,
                published: reader.long()?,
                bundle: reader.bytes(LONGEST_PAYLOAD)?.to_vec(),
            });
        }
        reader.finish()?;
        if !ascending(slots.iter().map(|slot| slot.device)) {
            return Err(WireError::Order);
        }
        Ok(Self { slots })
    }
}

/// Which side of a pairing a piece of mail travels to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Inviter,
    Joiner,
}

impl Side {
    const fn tag(self) -> u8 {
        match self {
            Self::Inviter => TO_INVITER,
            Self::Joiner => TO_JOINER,
        }
    }

    const fn from_tag(tag: u8) -> Result<Self, WireError> {
        match tag {
            TO_INVITER => Ok(Self::Inviter),
            TO_JOINER => Ok(Self::Joiner),
            other => Err(WireError::Tag(other)),
        }
    }
}

/// A step of a pairing through the relay, which needs no signature: the pairing messages authenticate themselves through the code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Mail {
    /// Open a mailbox; the relay answers with its nameplate.
    Open,
    /// Leave a message in the mailbox for one side.
    Drop {
        nameplate: u16,
        to: Side,
        message: Vec<u8>,
    },
    /// Take the oldest message for one side, or nothing yet.
    Pick { nameplate: u16, to: Side },
}

impl Mail {
    /// The mail's bytes.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Length`] for a message beyond its limit.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = begin(MAIL);
        match self {
            Self::Open => {
                writer.byte(OPEN);
            }
            Self::Drop {
                nameplate,
                to,
                message,
            } => {
                if message.len() > LONGEST_PAYLOAD {
                    return Err(WireError::Length);
                }
                writer.byte(DROP).short(*nameplate).byte(to.tag());
                writer.bytes(message)?;
            }
            Self::Pick { nameplate, to } => {
                writer.byte(PICK).short(*nameplate).byte(to.tag());
            }
        }
        Ok(writer.finish())
    }

    /// The mail `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for any bytes that encoding never writes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        magic(&mut reader, MAIL)?;
        let mail = match reader.byte()? {
            OPEN => Self::Open,
            DROP => Self::Drop {
                nameplate: reader.short()?,
                to: Side::from_tag(reader.byte()?)?,
                message: reader.bytes(LONGEST_PAYLOAD)?.to_vec(),
            },
            PICK => Self::Pick {
                nameplate: reader.short()?,
                to: Side::from_tag(reader.byte()?)?,
            },
            other => return Err(WireError::Tag(other)),
        };
        reader.finish()?;
        Ok(mail)
    }
}

/// The relay's nameplate for a new mailbox, as the version and the number.
#[must_use]
pub fn nameplate_bytes(nameplate: u16) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.short(VERSION).short(nameplate);
    writer.finish()
}

/// The nameplate the relay answered an opening with.
///
/// # Errors
///
/// Returns a [`WireError`] for any other bytes.
pub fn read_nameplate(bytes: &[u8]) -> Result<u16, WireError> {
    let mut reader = Reader::new(bytes);
    let version = reader.short()?;
    if version != VERSION {
        return Err(WireError::Version(version));
    }
    let nameplate = reader.short()?;
    reader.finish()?;
    Ok(nameplate)
}

#[cfg(test)]
mod tests {
    use super::{
        Collected, LONGEST_FRAME, LONGEST_PAYLOAD, MOST_SLOTS, Mail, Request, Side, Slot, Verb,
        nameplate_bytes, read_nameplate,
    };

    /// A member publishes its bundle in one request, so a request carries a bundle that holds the longest state.
    #[test]
    fn a_request_carries_a_bundle_with_the_longest_state() {
        let keys = SigningKeys::generate().unwrap();
        let bundle = crate::message::Bundle {
            account: [1; 32],
            operations: Vec::new(),
            sender: [2; 32],
            epoch: 1,
            sealed: youarel_crypto::seal::Sealed {
                commitment: [3; 32],
                nonce: [4; 24],
                ciphertext: vec![5; crate::message::LONGEST_STATE],
            },
            signature: keys.sign(Label::Bundle, b"x").unwrap(),
        };
        let request = Request {
            account: [1; 32],
            device: [2; 32],
            verb: Verb::Publish,
            stamped: 1,
            nonce: [6; 16],
            payload: bundle.encode().unwrap(),
            signature: keys.sign(Label::Request, b"x").unwrap(),
        };
        let encoded = request.encode().unwrap();
        assert!(encoded.len() > crate::message::LONGEST_STATE);
    }

    #[test]
    fn every_relay_limit_admits_its_bound_and_refuses_one_more() {
        let keys = SigningKeys::generate().unwrap();
        let request = |length: usize| Request {
            account: [1; 32],
            device: [2; 32],
            verb: Verb::Publish,
            stamped: 1,
            nonce: [3; 16],
            payload: vec![4; length],
            signature: keys.sign(Label::Request, b"x").unwrap(),
        };
        // A signature differs each time it is made, so the test compares against the request it encoded.
        let built = request(LONGEST_PAYLOAD);
        let longest = built.encode().unwrap();
        assert!(longest.len() <= LONGEST_PAYLOAD + LONGEST_FRAME);
        assert_eq!(Request::decode(&longest).unwrap(), built);
        assert_eq!(
            request(LONGEST_PAYLOAD + 1).encode(),
            Err(WireError::Length)
        );
        let slot = |index: usize, length: usize| Slot {
            device: [u8::try_from(index).unwrap(); 32],
            published: 1,
            bundle: vec![5; length],
        };
        let most = Collected {
            slots: (0..MOST_SLOTS).map(|index| slot(index, 1)).collect(),
        };
        assert_eq!(Collected::decode(&most.encode().unwrap()).unwrap(), most);
        let crowded = Collected {
            slots: (0..=MOST_SLOTS).map(|index| slot(index, 1)).collect(),
        };
        assert_eq!(crowded.encode(), Err(WireError::Order));
        let full = Collected {
            slots: vec![slot(0, LONGEST_PAYLOAD)],
        };
        assert_eq!(Collected::decode(&full.encode().unwrap()).unwrap(), full);
        let over = Collected {
            slots: vec![slot(0, LONGEST_PAYLOAD + 1)],
        };
        assert_eq!(over.encode(), Err(WireError::Length));
        let mail = |length: usize| Mail::Drop {
            nameplate: 1,
            to: Side::Joiner,
            message: vec![6; length],
        };
        assert_eq!(
            Mail::decode(&mail(LONGEST_PAYLOAD).encode().unwrap()).unwrap(),
            mail(LONGEST_PAYLOAD)
        );
    }
    use crate::wire::WireError;
    use youarel_crypto::Label;
    use youarel_crypto::sign::SigningKeys;

    #[test]
    fn every_relay_message_reads_back_as_written() {
        let keys = SigningKeys::generate().unwrap();
        for verb in [Verb::Publish, Verb::Collect, Verb::Erase] {
            let request = Request {
                account: [1; 32],
                device: [2; 32],
                verb,
                stamped: 1_700_000_000_000,
                nonce: [3; 16],
                payload: b"bundle".to_vec(),
                signature: keys.sign(Label::Request, b"x").unwrap(),
            };
            assert_eq!(
                Request::decode(&request.encode().unwrap()).unwrap(),
                request
            );
        }
        let collected = Collected {
            slots: vec![
                Slot {
                    device: [1; 32],
                    published: 5,
                    bundle: b"a".to_vec(),
                },
                Slot {
                    device: [2; 32],
                    published: 6,
                    bundle: b"b".to_vec(),
                },
            ],
        };
        assert_eq!(
            Collected::decode(&collected.encode().unwrap()).unwrap(),
            collected
        );
        let mut reversed = collected;
        reversed.slots.reverse();
        assert_eq!(reversed.encode(), Err(WireError::Order));
        for mail in [
            Mail::Open,
            Mail::Drop {
                nameplate: 7,
                to: Side::Inviter,
                message: b"join".to_vec(),
            },
            Mail::Pick {
                nameplate: 7,
                to: Side::Joiner,
            },
        ] {
            assert_eq!(Mail::decode(&mail.encode().unwrap()).unwrap(), mail);
        }
        assert_eq!(read_nameplate(&nameplate_bytes(42)), Ok(42));
        assert_eq!(
            Mail::Drop {
                nameplate: 7,
                to: Side::Joiner,
                message: vec![0; LONGEST_PAYLOAD + 1],
            }
            .encode(),
            Err(WireError::Length)
        );
    }
}
