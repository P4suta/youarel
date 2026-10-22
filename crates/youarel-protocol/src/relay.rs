//! Signed relay requests, recovery discovery, and self-authenticating pairing mail.

use crate::message::{Digest, MAGIC, VERSION, begin, magic, read_signature, write_signature};
use crate::wire::{Reader, WireError, Writer, ascending};
use youarel_core::relay_rules::{NONCE, Nonce};
use youarel_crypto::sign::Signature;

/// The magic of each relay message.
const REQUEST: [u8; MAGIC] = *b"YRQ1";
const MAIL: [u8; MAGIC] = *b"YRM1";
const LOCATE: [u8; MAGIC] = *b"YRF1";
const REGISTRATION: [u8; MAGIC] = *b"YRG1";

/// The most members a registration names: as many as a rotation reaches.
const MOST_MEMBERS: usize = crate::message::MOST_SLOTS;

/// The longest segment envelope or pairing message a request carries.
pub const LONGEST_PAYLOAD: usize = youarel_core::segment_rules::LONGEST_ENVELOPE;

/// The framing allowance around a relay payload.
pub const LONGEST_FRAME: usize = 64 * 1024;

/// The tags of the verbs and sides.
const PUBLISH: u8 = 0;
const COLLECT: u8 = 1;
const ERASE: u8 = 2;
const BRIDGE: u8 = 3;
const OPEN: u8 = 0;
const DROP: u8 = 1;
const PICK: u8 = 2;
const POLL: u8 = 3;
const TO_INVITER: u8 = 0;
const TO_JOINER: u8 = 1;

/// What a member asks of the relay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verb {
    /// Append the signed segment or replay its exact retained envelope.
    Publish,
    /// Receive a page of peer chains after the supplied cursors.
    Collect,
    /// Delete the account and everything the relay keeps of it.
    Erase,
    /// Start a delta suffix after this device used another transport for a bootstrap.
    Bridge,
}

impl Verb {
    const fn tag(self) -> u8 {
        match self {
            Self::Publish => PUBLISH,
            Self::Collect => COLLECT,
            Self::Erase => ERASE,
            Self::Bridge => BRIDGE,
        }
    }

    const fn from_tag(tag: u8) -> Result<Self, WireError> {
        match tag {
            PUBLISH => Ok(Self::Publish),
            COLLECT => Ok(Self::Collect),
            ERASE => Ok(Self::Erase),
            BRIDGE => Ok(Self::Bridge),
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
    /// Take the oldest message immediately, without waiting when none exists.
    Poll { nameplate: u16, to: Side },
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
            Self::Poll { nameplate, to } => {
                writer.byte(POLL).short(*nameplate).byte(to.tag());
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
            POLL => Self::Poll {
                nameplate: reader.short()?,
                to: Side::from_tag(reader.byte()?)?,
            },
            other => return Err(WireError::Tag(other)),
        };
        reader.finish()?;
        Ok(mail)
    }
}

/// A device without an account asks which account lists a member, such as the one its recovery key grows into.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Locate {
    pub member: Digest,
}

impl Locate {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = begin(LOCATE);
        writer.raw(&self.member);
        writer.finish()
    }

    /// The question `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for any bytes that encoding never writes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        magic(&mut reader, LOCATE)?;
        let member = reader.array()?;
        reader.finish()?;
        Ok(Self { member })
    }
}

/// The recovery members of an account, which the relay's directory notes when a member publishes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Registration {
    pub account: Digest,
    /// The members, in ascending order.
    pub members: Vec<Digest>,
}

impl Registration {
    /// The registration's bytes.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Order`] for more members than a rotation reaches, or members out of order.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.members.len() > MOST_MEMBERS || !ascending(self.members.iter()) {
            return Err(WireError::Order);
        }
        let mut writer = begin(REGISTRATION);
        writer.raw(&self.account).count(self.members.len())?;
        for member in &self.members {
            writer.raw(member);
        }
        Ok(writer.finish())
    }

    /// The registration `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for any bytes that encoding never writes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        magic(&mut reader, REGISTRATION)?;
        let account = reader.array()?;
        let count = reader.count(MOST_MEMBERS)?;
        let mut members = Vec::with_capacity(count);
        for _ in 0..count {
            members.push(reader.array()?);
        }
        reader.finish()?;
        if !ascending(members.iter()) {
            return Err(WireError::Order);
        }
        Ok(Self { account, members })
    }
}

/// The relay's answer to a question of location: the version and the account.
#[must_use]
pub fn located_bytes(account: &Digest) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.short(VERSION).raw(account);
    writer.finish()
}

/// The account the relay located.
///
/// # Errors
///
/// Returns a [`WireError`] for any other bytes.
pub fn read_located(bytes: &[u8]) -> Result<Digest, WireError> {
    let mut reader = Reader::new(bytes);
    let version = reader.short()?;
    if version != VERSION {
        return Err(WireError::Version(version));
    }
    let account = reader.array()?;
    reader.finish()?;
    Ok(account)
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
        LONGEST_FRAME, LONGEST_PAYLOAD, Locate, MOST_MEMBERS, Mail, Registration, Request, Side,
        Verb, located_bytes, nameplate_bytes, read_located, read_nameplate,
    };

    #[test]
    fn a_location_and_a_registration_read_back_as_written() {
        let locate = Locate { member: [7; 32] };
        assert_eq!(Locate::decode(&locate.encode()).unwrap(), locate);
        assert_eq!(read_located(&located_bytes(&[8; 32])), Ok([8; 32]));
        let registration = Registration {
            account: [1; 32],
            members: vec![[2; 32], [3; 32]],
        };
        assert_eq!(
            Registration::decode(&registration.encode().unwrap()).unwrap(),
            registration
        );
        let unordered = Registration {
            account: [1; 32],
            members: vec![[3; 32], [2; 32]],
        };
        assert_eq!(unordered.encode(), Err(WireError::Order));
        let crowded = Registration {
            account: [1; 32],
            members: (0..=MOST_MEMBERS)
                .map(|index| {
                    let mut member = [0; 32];
                    member[..8].copy_from_slice(&u64::try_from(index).unwrap().to_be_bytes());
                    member
                })
                .collect(),
        };
        assert_eq!(crowded.encode(), Err(WireError::Order));
        let most = Registration {
            members: crowded.members.get(..MOST_MEMBERS).unwrap().to_vec(),
            ..crowded
        };
        assert_eq!(Registration::decode(&most.encode().unwrap()).unwrap(), most);
    }

    /// A publication carries the complete envelope, including its largest sealed body.
    #[test]
    fn a_request_carries_a_segment_with_the_longest_body() {
        use youarel_core::segment_rules::{Header, Kind, Sequence};
        let keys = SigningKeys::generate().unwrap();
        let segment = crate::segment::Segment {
            account: [1; 32],
            operations: Vec::new(),
            sender: [2; 32],
            epoch: 1,
            header: Header::new(Sequence::FIRST, [0; 32], Kind::Delta).unwrap(),
            sealed: youarel_crypto::seal::Sealed {
                commitment: [3; 32],
                nonce: [4; 24],
                ciphertext: vec![5; crate::segment::LONGEST_SEGMENT + youarel_crypto::seal::TAG],
            },
            signature: keys.sign(Label::Segment, b"x").unwrap(),
        };
        let request = Request {
            account: [1; 32],
            device: [2; 32],
            verb: Verb::Publish,
            stamped: 1,
            nonce: [6; 16],
            payload: segment.encode().unwrap(),
            signature: keys.sign(Label::Request, b"x").unwrap(),
        };
        let encoded = request.encode().unwrap();
        assert!(encoded.len() > crate::segment::LONGEST_SEGMENT);
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
        for verb in [Verb::Publish, Verb::Collect, Verb::Erase, Verb::Bridge] {
            let request = Request {
                account: [1; 32],
                device: [2; 32],
                verb,
                stamped: 1_700_000_000_000,
                nonce: [3; 16],
                payload: b"segment".to_vec(),
                signature: keys.sign(Label::Request, b"x").unwrap(),
            };
            assert_eq!(
                Request::decode(&request.encode().unwrap()).unwrap(),
                request
            );
        }
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
            Mail::Poll {
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
