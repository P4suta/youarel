//! The messages of `specs/WIRE.md`: operations of the membership log, bundles, and the four messages of a pairing, with the admission its entry seals.

use crate::wire::{Reader, WireError, Writer, ascending};
use youarel_core::membership_rules;
use youarel_crypto::seal::{COMMITMENT, NONCE, Sealed};
use youarel_crypto::sign::{
    ED25519_KEY, ED25519_SIGNATURE, ML_DSA_KEY, ML_DSA_SIGNATURE, Signature, VerifyingKeys,
};
use youarel_crypto::slot::{SEALED, Slot};
use youarel_crypto::{DIGEST, KEY, Label, digest, kem, pairing};

/// The version every message carries.
pub const VERSION: u16 = 1;
/// The bytes of the magic that starts each message.
const MAGIC: usize = 4;
/// The magic of each kind of message.
const OPERATION: [u8; MAGIC] = *b"YRO1";
const BUNDLE: [u8; MAGIC] = *b"YRB1";
const JOIN: [u8; MAGIC] = *b"YRJ1";
const WELCOME: [u8; MAGIC] = *b"YRW1";
const ADMISSION: [u8; MAGIC] = *b"YRA1";
const JOINED: [u8; MAGIC] = *b"YRC1";
const ENTRY: [u8; MAGIC] = *b"YRE1";
/// The most parents an operation names.
pub const MOST_PARENTS: usize = 64;
/// The most key slots a rotation holds, one per member.
pub const MOST_SLOTS: usize = 1024;
/// The most operations a bundle or an admission carries.
pub const MOST_OPERATIONS: usize = 65_536;
/// The longest operation, which a rotation of the most slots fits.
pub const LONGEST_OPERATION: usize = 2 * 1024 * 1024;
/// The longest sealed state.
pub const LONGEST_STATE: usize = 16 * 1024 * 1024;
/// The longest PAKE message.
pub const LONGEST_PAKE: usize = 64;
/// The most epoch keys an admission hands over.
pub const MOST_EPOCHS: usize = 4096;

/// A digest that names a device, an account, or an operation.
pub type Digest = [u8; DIGEST];

/// The tag of each action.
const GENESIS: u8 = 0;
const ADD: u8 = 1;
const REMOVE: u8 = 2;
const ROTATE: u8 = 3;

fn magic(reader: &mut Reader<'_>, expected: [u8; MAGIC]) -> Result<(), WireError> {
    if reader.array::<MAGIC>()? != expected {
        return Err(WireError::Magic);
    }
    let version = reader.short()?;
    if version != VERSION {
        return Err(WireError::Version(version));
    }
    Ok(())
}

fn begin(kind: [u8; MAGIC]) -> Writer {
    let mut writer = Writer::new();
    writer.raw(&kind).short(VERSION);
    writer
}

/// The public keys of a device: its two signing keys and its X-Wing key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevicePublic {
    pub signing: VerifyingKeys,
    pub kem: Box<[u8; kem::PUBLIC_KEY]>,
}

impl DevicePublic {
    /// The device's identifier: the digest of its public keys.
    #[must_use]
    pub fn id(&self) -> Digest {
        digest(
            Label::Device,
            &[
                &self.signing.ed25519,
                self.signing.ml_dsa.as_slice(),
                self.kem.as_slice(),
            ],
        )
    }

    fn write(&self, writer: &mut Writer) {
        writer
            .raw(&self.signing.ed25519)
            .raw(self.signing.ml_dsa.as_slice())
            .raw(self.kem.as_slice());
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, WireError> {
        Ok(Self {
            signing: VerifyingKeys {
                ed25519: reader.array::<ED25519_KEY>()?,
                ml_dsa: Box::new(reader.array::<ML_DSA_KEY>()?),
            },
            kem: Box::new(reader.array::<{ kem::PUBLIC_KEY }>()?),
        })
    }

    /// The keys in their wire form, which a pairing transcript binds.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        self.write(&mut writer);
        writer.finish()
    }

    /// The keys that their wire form spells.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for bytes of another length.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        let public = Self::read(&mut reader)?;
        reader.finish()?;
        Ok(public)
    }
}

fn write_signature(writer: &mut Writer, signature: &Signature) {
    writer
        .raw(&signature.ed25519)
        .raw(signature.ml_dsa.as_slice());
}

fn read_signature(reader: &mut Reader<'_>) -> Result<Signature, WireError> {
    Ok(Signature {
        ed25519: reader.array::<ED25519_SIGNATURE>()?,
        ml_dsa: Box::new(reader.array::<ML_DSA_SIGNATURE>()?),
    })
}

/// A move to a new epoch: its number, the commitment to its key, and the key wrapped for each member it reaches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rotation {
    pub epoch: u32,
    pub commitment: [u8; COMMITMENT],
    /// One slot per device, in ascending order of the device.
    pub slots: Vec<Slot>,
}

/// What an operation does.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Genesis(DevicePublic),
    Add(DevicePublic),
    Remove(Digest),
    Rotate(Rotation),
}

impl Action {
    /// The action as the membership rules read it.
    #[must_use]
    pub fn rule(&self) -> membership_rules::Action {
        match self {
            Self::Genesis(_) => membership_rules::Action::Genesis,
            Self::Add(device) => membership_rules::Action::Add(device.id()),
            Self::Remove(device) => membership_rules::Action::Remove(*device),
            Self::Rotate(rotation) => membership_rules::Action::Rotate(rotation.epoch),
        }
    }

    fn write(&self, writer: &mut Writer) -> Result<(), WireError> {
        match self {
            Self::Genesis(device) => {
                writer.byte(GENESIS);
                device.write(writer);
            }
            Self::Add(device) => {
                writer.byte(ADD);
                device.write(writer);
            }
            Self::Remove(device) => {
                writer.byte(REMOVE).raw(device);
            }
            Self::Rotate(rotation) => {
                writer
                    .byte(ROTATE)
                    .word(rotation.epoch)
                    .raw(&rotation.commitment);
                writer.count(rotation.slots.len())?;
                for slot in &rotation.slots {
                    writer
                        .raw(&slot.device)
                        .raw(&slot.encapsulation)
                        .raw(&slot.sealed);
                }
            }
        }
        Ok(())
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, WireError> {
        match reader.byte()? {
            GENESIS => Ok(Self::Genesis(DevicePublic::read(reader)?)),
            ADD => Ok(Self::Add(DevicePublic::read(reader)?)),
            REMOVE => Ok(Self::Remove(reader.array()?)),
            ROTATE => {
                let epoch = reader.word()?;
                let commitment = reader.array()?;
                let count = reader.count(MOST_SLOTS)?;
                let mut slots = Vec::with_capacity(count);
                for _ in 0..count {
                    slots.push(Slot {
                        device: reader.array()?,
                        encapsulation: reader.take(kem::CIPHERTEXT)?.to_vec(),
                        sealed: reader.array::<SEALED>()?,
                    });
                }
                if !ascending(slots.iter().map(|slot| slot.device)) {
                    return Err(WireError::Order);
                }
                Ok(Self::Rotate(Rotation {
                    epoch,
                    commitment,
                    slots,
                }))
            }
            tag => Err(WireError::Tag(tag)),
        }
    }
}

/// One signed operation of the membership log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Operation {
    /// The account, or all zeros for the genesis, whose digest names the account.
    pub account: Digest,
    /// The operations its author saw, in ascending order.
    pub parents: Vec<Digest>,
    pub author: Digest,
    /// When the author wrote it, in milliseconds since the Unix epoch, for a person to read.
    pub wall: u64,
    pub action: Action,
    pub signature: Signature,
}

/// The fields of an operation that its signature covers.
#[derive(Clone, Copy, Debug)]
pub struct Unsigned<'a> {
    pub account: &'a Digest,
    pub parents: &'a [Digest],
    pub author: &'a Digest,
    pub wall: u64,
    pub action: &'a Action,
}

impl Unsigned<'_> {
    /// The bytes the author signs.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Length`] or [`WireError::Order`] for parents beyond the limit or out of order.
    pub fn bytes(&self) -> Result<Vec<u8>, WireError> {
        if self.parents.len() > MOST_PARENTS || !ascending(self.parents.iter()) {
            return Err(WireError::Order);
        }
        let mut writer = begin(OPERATION);
        writer.raw(self.account).count(self.parents.len())?;
        for parent in self.parents {
            writer.raw(parent);
        }
        writer.raw(self.author).long(self.wall);
        self.action.write(&mut writer)?;
        Ok(writer.finish())
    }
}

impl Operation {
    /// The fields the signature covers.
    #[must_use]
    pub fn unsigned(&self) -> Unsigned<'_> {
        Unsigned {
            account: &self.account,
            parents: &self.parents,
            author: &self.author,
            wall: self.wall,
            action: &self.action,
        }
    }

    /// The operation's bytes: the signed fields and the signature.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for parents or slots beyond their limits or out of order.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = Writer::new();
        writer.raw(&self.unsigned().bytes()?);
        write_signature(&mut writer, &self.signature);
        Ok(writer.finish())
    }

    /// The operation `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for any bytes that encoding never writes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() > LONGEST_OPERATION {
            return Err(WireError::Length);
        }
        let mut reader = Reader::new(bytes);
        magic(&mut reader, OPERATION)?;
        let account = reader.array()?;
        let count = reader.count(MOST_PARENTS)?;
        let mut parents = Vec::with_capacity(count);
        for _ in 0..count {
            parents.push(reader.array()?);
        }
        if !ascending(parents.iter()) {
            return Err(WireError::Order);
        }
        let author = reader.array()?;
        let wall = reader.long()?;
        let action = Action::read(&mut reader)?;
        let signature = read_signature(&mut reader)?;
        reader.finish()?;
        Ok(Self {
            account,
            parents,
            author,
            wall,
            action,
            signature,
        })
    }
}

/// The digest that names an operation, over its bytes.
#[must_use]
pub fn operation_digest(bytes: &[u8]) -> Digest {
    digest(Label::Operation, &[bytes])
}

/// The account a genesis operation founds, named by the digest of its bytes.
#[must_use]
pub fn account_of(genesis: &[u8]) -> Digest {
    digest(Label::Account, &[genesis])
}

fn write_operations(writer: &mut Writer, operations: &[Vec<u8>]) -> Result<(), WireError> {
    if operations.len() > MOST_OPERATIONS
        || !ascending(operations.iter().map(|bytes| operation_digest(bytes)))
    {
        return Err(WireError::Order);
    }
    writer.count(operations.len())?;
    for operation in operations {
        writer.bytes(operation)?;
    }
    Ok(())
}

fn read_operations(reader: &mut Reader<'_>) -> Result<Vec<Vec<u8>>, WireError> {
    let count = reader.count(MOST_OPERATIONS)?;
    let mut operations = Vec::with_capacity(count.min(MOST_PARENTS));
    for _ in 0..count {
        operations.push(reader.bytes(LONGEST_OPERATION)?.to_vec());
    }
    if !ascending(operations.iter().map(|bytes| operation_digest(bytes))) {
        return Err(WireError::Order);
    }
    Ok(operations)
}

fn write_sealed(writer: &mut Writer, sealed: &Sealed) -> Result<(), WireError> {
    writer.raw(&sealed.commitment).raw(&sealed.nonce);
    writer.bytes(&sealed.ciphertext)?;
    Ok(())
}

fn read_sealed(reader: &mut Reader<'_>) -> Result<Sealed, WireError> {
    Ok(Sealed {
        commitment: reader.array()?,
        nonce: reader.array::<NONCE>()?,
        ciphertext: reader.bytes(LONGEST_STATE)?.to_vec(),
    })
}

/// The joiner's answer to a welcome: its proof that it holds the channel key, which the inviter requires before it sends anything secret.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Joined {
    pub invitation: u16,
    pub confirmation: [u8; pairing::CONFIRMATION],
}

impl Joined {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = begin(JOINED);
        writer.short(self.invitation).raw(&self.confirmation);
        writer.finish()
    }

    /// The confirmation `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for any bytes that encoding never writes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        magic(&mut reader, JOINED)?;
        let joined = Self {
            invitation: reader.short()?,
            confirmation: reader.array()?,
        };
        reader.finish()?;
        Ok(joined)
    }
}

/// The inviter's last message: the admission, sealed under the channel key for the joiner that confirmed it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub invitation: u16,
    pub sealed: Sealed,
}

impl Entry {
    /// The entry's bytes.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Length`] for a sealed admission beyond its limit.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = begin(ENTRY);
        writer.short(self.invitation);
        write_sealed(&mut writer, &self.sealed)?;
        Ok(writer.finish())
    }

    /// The entry `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for any bytes that encoding never writes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        magic(&mut reader, ENTRY)?;
        let entry = Self {
            invitation: reader.short()?,
            sealed: read_sealed(&mut reader)?,
        };
        reader.finish()?;
        Ok(entry)
    }
}

/// One device's state sealed under an epoch key, with the account's membership log, signed by the sender.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bundle {
    pub account: Digest,
    /// The operations of the log, in ascending order of their digests.
    pub operations: Vec<Vec<u8>>,
    pub sender: Digest,
    pub epoch: u32,
    pub sealed: Sealed,
    pub signature: Signature,
}

/// The fields of a bundle that its signature covers.
#[derive(Clone, Copy, Debug)]
pub struct UnsignedBundle<'a> {
    pub account: &'a Digest,
    pub operations: &'a [Vec<u8>],
    pub sender: &'a Digest,
    pub epoch: u32,
    pub sealed: &'a Sealed,
}

impl UnsignedBundle<'_> {
    /// The bytes the sender signs.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for operations beyond the limit or out of order.
    pub fn bytes(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = begin(BUNDLE);
        writer.raw(self.account);
        write_operations(&mut writer, self.operations)?;
        writer.raw(self.sender).word(self.epoch);
        write_sealed(&mut writer, self.sealed)?;
        Ok(writer.finish())
    }
}

impl Bundle {
    #[must_use]
    pub fn unsigned(&self) -> UnsignedBundle<'_> {
        UnsignedBundle {
            account: &self.account,
            operations: &self.operations,
            sender: &self.sender,
            epoch: self.epoch,
            sealed: &self.sealed,
        }
    }

    /// The bundle's bytes.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for operations beyond the limit or out of order.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut writer = Writer::new();
        writer.raw(&self.unsigned().bytes()?);
        write_signature(&mut writer, &self.signature);
        Ok(writer.finish())
    }

    /// The bundle `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for any bytes that encoding never writes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        magic(&mut reader, BUNDLE)?;
        let account = reader.array()?;
        let operations = read_operations(&mut reader)?;
        let sender = reader.array()?;
        let epoch = reader.word()?;
        let sealed = read_sealed(&mut reader)?;
        let signature = read_signature(&mut reader)?;
        reader.finish()?;
        Ok(Self {
            account,
            operations,
            sender,
            epoch,
            sealed,
            signature,
        })
    }
}

/// A joining device's first message: the invitation it answers, its public keys, and its PAKE message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Join {
    pub invitation: u16,
    pub joiner: DevicePublic,
    pub pake: Vec<u8>,
}

impl Join {
    /// The join's bytes.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Length`] for a PAKE message beyond its limit.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.pake.len() > LONGEST_PAKE {
            return Err(WireError::Length);
        }
        let mut writer = begin(JOIN);
        writer.short(self.invitation);
        self.joiner.write(&mut writer);
        writer.bytes(&self.pake)?;
        Ok(writer.finish())
    }

    /// The join `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for any bytes that encoding never writes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        magic(&mut reader, JOIN)?;
        let invitation = reader.short()?;
        let joiner = DevicePublic::read(&mut reader)?;
        let pake = reader.bytes(LONGEST_PAKE)?.to_vec();
        reader.finish()?;
        Ok(Self {
            invitation,
            joiner,
            pake,
        })
    }
}

/// The inviter's answer to a join: its PAKE message, the encapsulation to the joiner, and its proof of the channel key, which carry nothing secret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Welcome {
    pub invitation: u16,
    pub pake: Vec<u8>,
    pub encapsulation: Vec<u8>,
    pub welcomed: [u8; pairing::CONFIRMATION],
}

impl Welcome {
    /// The welcome's bytes.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Length`] for a PAKE message or an encapsulation of the wrong size.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.pake.len() > LONGEST_PAKE || self.encapsulation.len() != kem::CIPHERTEXT {
            return Err(WireError::Length);
        }
        let mut writer = begin(WELCOME);
        writer.short(self.invitation);
        writer.bytes(&self.pake)?;
        writer.raw(&self.encapsulation).raw(&self.welcomed);
        Ok(writer.finish())
    }

    /// The welcome `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for any bytes that encoding never writes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        magic(&mut reader, WELCOME)?;
        let invitation = reader.short()?;
        let pake = reader.bytes(LONGEST_PAKE)?.to_vec();
        let encapsulation = reader.take(kem::CIPHERTEXT)?.to_vec();
        let welcomed = reader.array()?;
        reader.finish()?;
        Ok(Self {
            invitation,
            pake,
            encapsulation,
            welcomed,
        })
    }
}

/// What a welcome seals for the joiner: the account's log, its epoch keys, and the inviter's state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Admission {
    pub account: Digest,
    pub operations: Vec<Vec<u8>>,
    /// Every epoch key the inviter holds, in ascending order of the epoch and then the key, since two members may rotate to one epoch at once.
    pub epochs: Vec<(u32, [u8; KEY])>,
    /// The inviter's state in the JSON form of the store.
    pub state: Vec<u8>,
}

impl Admission {
    /// The admission's bytes.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for lists beyond their limits or out of order.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.epochs.len() > MOST_EPOCHS || !ascending(self.epochs.iter()) {
            return Err(WireError::Order);
        }
        let mut writer = begin(ADMISSION);
        writer.raw(&self.account);
        write_operations(&mut writer, &self.operations)?;
        writer.count(self.epochs.len())?;
        for (epoch, key) in &self.epochs {
            writer.word(*epoch).raw(key);
        }
        writer.bytes(&self.state)?;
        Ok(writer.finish())
    }

    /// The admission `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError`] for any bytes that encoding never writes.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(bytes);
        magic(&mut reader, ADMISSION)?;
        let account = reader.array()?;
        let operations = read_operations(&mut reader)?;
        let count = reader.count(MOST_EPOCHS)?;
        let mut epochs = Vec::with_capacity(count);
        for _ in 0..count {
            epochs.push((reader.word()?, reader.array()?));
        }
        if !ascending(epochs.iter()) {
            return Err(WireError::Order);
        }
        let state = reader.bytes(LONGEST_STATE)?.to_vec();
        reader.finish()?;
        Ok(Self {
            account,
            operations,
            epochs,
            state,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Action, Admission, Bundle, DevicePublic, Entry, Join, Joined, LONGEST_OPERATION,
        LONGEST_PAKE, LONGEST_STATE, MOST_EPOCHS, MOST_OPERATIONS, MOST_PARENTS, MOST_SLOTS,
        Operation, Rotation, Welcome, account_of, operation_digest,
    };
    use crate::wire::WireError;
    use youarel_crypto::Label;
    use youarel_crypto::kem::KeyPair;
    use youarel_crypto::seal::Sealed;
    use youarel_crypto::sign::SigningKeys;
    use youarel_crypto::slot::Slot;

    fn device() -> (SigningKeys, DevicePublic) {
        let signing = SigningKeys::generate().unwrap();
        let kem = KeyPair::generate().unwrap();
        let public = DevicePublic {
            signing: signing.public().clone(),
            kem: Box::new(*kem.public()),
        };
        (signing, public)
    }

    /// An operation of `author` in `account`, signed by `keys`.
    fn signed(
        keys: &SigningKeys,
        (account, author): ([u8; 32], [u8; 32]),
        parents: Vec<[u8; 32]>,
        action: Action,
    ) -> Operation {
        let mut operation = Operation {
            account,
            parents,
            author,
            wall: 1_700_000_000_000,
            action,
            signature: keys.sign(Label::Operation, b"").unwrap(),
        };
        operation.signature = keys
            .sign(Label::Operation, &operation.unsigned().bytes().unwrap())
            .unwrap();
        operation
    }

    fn sealed() -> Sealed {
        Sealed {
            commitment: [1; 32],
            nonce: [2; 24],
            ciphertext: b"ciphertext".to_vec(),
        }
    }

    #[test]
    fn every_message_reads_back_as_written() {
        let (keys, public) = device();
        let genesis = signed(
            &keys,
            ([0; 32], public.id()),
            Vec::new(),
            Action::Genesis(public.clone()),
        );
        let genesis_bytes = genesis.encode().unwrap();
        assert_eq!(Operation::decode(&genesis_bytes).unwrap(), genesis);
        let account = account_of(&genesis_bytes);
        let rotation = signed(
            &keys,
            (account, public.id()),
            vec![operation_digest(&genesis_bytes)],
            Action::Rotate(Rotation {
                epoch: 1,
                commitment: [3; 32],
                slots: vec![Slot {
                    device: public.id(),
                    encapsulation: vec![4; youarel_crypto::kem::CIPHERTEXT],
                    sealed: [5; youarel_crypto::slot::SEALED],
                }],
            }),
        );
        assert_eq!(
            Operation::decode(&rotation.encode().unwrap()).unwrap(),
            rotation
        );
        let mut operations = vec![genesis_bytes, rotation.encode().unwrap()];
        operations.sort_by_key(|bytes| operation_digest(bytes));
        let bundle = Bundle {
            account,
            operations: operations.clone(),
            sender: public.id(),
            epoch: 1,
            sealed: sealed(),
            signature: keys.sign(Label::Bundle, b"x").unwrap(),
        };
        assert_eq!(Bundle::decode(&bundle.encode().unwrap()).unwrap(), bundle);
        let admission = Admission {
            account,
            operations,
            epochs: vec![(0, [12; 32]), (1, [13; 32])],
            state: b"{}".to_vec(),
        };
        assert_eq!(
            Admission::decode(&admission.encode().unwrap()).unwrap(),
            admission
        );
        assert!(keys.public().verify(
            Label::Operation,
            &rotation.unsigned().bytes().unwrap(),
            &rotation.signature
        ));
    }

    #[test]
    fn every_pairing_message_reads_back_as_written() {
        let (_, public) = device();
        let join = Join {
            invitation: 7,
            joiner: public,
            pake: vec![8; 33],
        };
        assert_eq!(Join::decode(&join.encode().unwrap()).unwrap(), join);
        let welcome = Welcome {
            invitation: 7,
            pake: vec![9; 33],
            encapsulation: vec![10; youarel_crypto::kem::CIPHERTEXT],
            welcomed: [11; 64],
        };
        assert_eq!(
            Welcome::decode(&welcome.encode().unwrap()).unwrap(),
            welcome
        );
        let joined = Joined {
            invitation: 7,
            confirmation: [6; 64],
        };
        assert_eq!(Joined::decode(&joined.encode()).unwrap(), joined);
        let entry = Entry {
            invitation: 7,
            sealed: sealed(),
        };
        assert_eq!(Entry::decode(&entry.encode().unwrap()).unwrap(), entry);
        assert_eq!(
            Joined::decode(&entry.encode().unwrap()),
            Err(WireError::Magic)
        );
    }

    /// The digest of the index, which orders a test's keys.
    fn key(index: usize) -> [u8; 32] {
        let mut key = [0; 32];
        key[..8].copy_from_slice(&u64::try_from(index).unwrap().to_be_bytes());
        key
    }

    #[test]
    fn every_operation_limit_admits_its_bound_and_refuses_one_more() {
        let (keys, public) = device();
        let parents = |count: usize| (0..count).map(key).collect::<Vec<_>>();
        let with_parents = signed(
            &keys,
            ([1; 32], public.id()),
            parents(MOST_PARENTS),
            Action::Remove([2; 32]),
        );
        assert_eq!(
            Operation::decode(&with_parents.encode().unwrap()).unwrap(),
            with_parents
        );
        let too_many = Operation {
            parents: parents(MOST_PARENTS + 1),
            ..with_parents
        };
        assert_eq!(too_many.encode(), Err(WireError::Order));
        let slots = (0..MOST_SLOTS)
            .map(|index| Slot {
                device: key(index),
                encapsulation: vec![4; youarel_crypto::kem::CIPHERTEXT],
                sealed: [5; youarel_crypto::slot::SEALED],
            })
            .collect();
        let widest = signed(
            &keys,
            ([1; 32], public.id()),
            Vec::new(),
            Action::Rotate(Rotation {
                epoch: 1,
                commitment: [3; 32],
                slots,
            }),
        );
        assert_eq!(
            Operation::decode(&widest.encode().unwrap()).unwrap(),
            widest
        );
        assert_ne!(
            Operation::decode(&vec![0; LONGEST_OPERATION]),
            Err(WireError::Length)
        );
        assert_eq!(
            Operation::decode(&vec![0; LONGEST_OPERATION + 1]),
            Err(WireError::Length)
        );
    }

    /// The state of the largest account a relay keeps, ten thousand shortcuts of five hundred bytes each, fits in one bundle, and a state past the limit does not.
    #[test]
    fn a_bundle_holds_the_largest_account_and_refuses_a_longer_state() {
        const SHORTCUTS: usize = 10_000;
        const SHORTCUT_BYTES: usize = 500;
        let (keys, public) = device();
        let bundle = |length: usize| Bundle {
            account: [1; 32],
            operations: Vec::new(),
            sender: public.id(),
            epoch: 1,
            sealed: Sealed {
                commitment: [1; 32],
                nonce: [2; 24],
                ciphertext: vec![7; length],
            },
            signature: keys.sign(Label::Bundle, b"x").unwrap(),
        };
        let largest = bundle(SHORTCUTS * SHORTCUT_BYTES);
        assert_eq!(Bundle::decode(&largest.encode().unwrap()).unwrap(), largest);
        assert_eq!(
            Bundle::decode(&bundle(LONGEST_STATE + 1).encode().unwrap()),
            Err(WireError::Length)
        );
    }

    #[test]
    fn a_bundle_holds_the_most_operations_and_the_longest_state() {
        let (keys, public) = device();
        let mut operations: Vec<Vec<u8>> = (0..MOST_OPERATIONS)
            .map(|index| key(index).to_vec())
            .collect();
        operations.sort_by_key(|bytes| operation_digest(bytes));
        let bundle = Bundle {
            account: [1; 32],
            operations: operations.clone(),
            sender: public.id(),
            epoch: 1,
            sealed: Sealed {
                commitment: [1; 32],
                nonce: [2; 24],
                ciphertext: vec![7; LONGEST_STATE],
            },
            signature: keys.sign(Label::Bundle, b"x").unwrap(),
        };
        assert_eq!(Bundle::decode(&bundle.encode().unwrap()).unwrap(), bundle);
        let mut more = operations.clone();
        more.push(key(MOST_OPERATIONS).to_vec());
        more.sort_by_key(|bytes| operation_digest(bytes));
        assert_eq!(
            Bundle {
                operations: more,
                ..bundle.clone()
            }
            .encode(),
            Err(WireError::Order)
        );
        let unordered: Vec<Vec<u8>> = operations.iter().take(2).rev().cloned().collect();
        assert_eq!(
            Bundle {
                operations: unordered,
                ..bundle
            }
            .encode(),
            Err(WireError::Order)
        );
    }

    #[test]
    fn every_pairing_limit_admits_its_bound_and_refuses_one_more() {
        let (_, public) = device();
        let join = Join {
            invitation: 7,
            joiner: public,
            pake: vec![8; LONGEST_PAKE],
        };
        assert_eq!(Join::decode(&join.encode().unwrap()).unwrap(), join);
        assert_eq!(
            Join {
                pake: vec![8; LONGEST_PAKE + 1],
                ..join
            }
            .encode(),
            Err(WireError::Length)
        );
        let welcome = Welcome {
            invitation: 7,
            pake: vec![9; LONGEST_PAKE],
            encapsulation: vec![10; youarel_crypto::kem::CIPHERTEXT],
            welcomed: [11; 64],
        };
        assert_eq!(
            Welcome::decode(&welcome.encode().unwrap()).unwrap(),
            welcome
        );
        assert_eq!(
            Welcome {
                pake: vec![9; LONGEST_PAKE + 1],
                ..welcome.clone()
            }
            .encode(),
            Err(WireError::Length)
        );
        assert_eq!(
            Welcome {
                encapsulation: vec![10; 3],
                ..welcome
            }
            .encode(),
            Err(WireError::Length)
        );
        let epochs: Vec<(u32, [u8; 32])> = (0..MOST_EPOCHS).map(|index| (0, key(index))).collect();
        let admission = Admission {
            account: [1; 32],
            operations: Vec::new(),
            epochs: epochs.clone(),
            state: Vec::new(),
        };
        assert_eq!(
            Admission::decode(&admission.encode().unwrap()).unwrap(),
            admission
        );
        let mut crowded = epochs;
        crowded.push((0, key(MOST_EPOCHS)));
        assert_eq!(
            Admission {
                epochs: crowded,
                ..admission
            }
            .encode(),
            Err(WireError::Order)
        );
    }

    #[test]
    fn decoding_refuses_what_encoding_never_writes() {
        let (keys, public) = device();
        let genesis = signed(
            &keys,
            ([0; 32], public.id()),
            Vec::new(),
            Action::Genesis(public.clone()),
        );
        let bytes = genesis.encode().unwrap();
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(Operation::decode(&trailing), Err(WireError::Trailing));
        assert_eq!(
            Operation::decode(&bytes[..bytes.len() - 1]),
            Err(WireError::Truncated)
        );
        let mut magic = bytes.clone();
        magic[0] = b'X';
        assert_eq!(Operation::decode(&magic), Err(WireError::Magic));
        let mut version = bytes.clone();
        version[5] = 2;
        assert_eq!(Operation::decode(&version), Err(WireError::Version(2)));
        let unordered = Operation {
            parents: vec![[2; 32], [1; 32]],
            ..genesis
        };
        assert_eq!(unordered.encode(), Err(WireError::Order));
        let tag_at = 4 + 2 + 32 + 4 + 32 + 8;
        let mut tag = bytes.clone();
        tag[tag_at] = 9;
        assert_eq!(Operation::decode(&tag), Err(WireError::Tag(9)));
        assert_eq!(Bundle::decode(&bytes), Err(WireError::Magic));
        let admission = Admission {
            account: [0; 32],
            operations: Vec::new(),
            epochs: vec![(1, [0; 32]), (1, [0; 32])],
            state: Vec::new(),
        };
        assert_eq!(admission.encode(), Err(WireError::Order));
        let long = Join {
            invitation: 1,
            joiner: public,
            pake: vec![0; 65],
        };
        assert_eq!(long.encode(), Err(WireError::Length));
    }

    #[test]
    fn a_device_is_named_by_all_three_of_its_keys() {
        let (_, public) = device();
        let mut other = public.clone();
        other.kem[0] ^= 1;
        assert_ne!(public.id(), other.id());
        assert_eq!(public.encode().len(), 32 + 1952 + 1216);
        assert_eq!(
            Action::Add(public.clone()).rule(),
            youarel_core::membership_rules::Action::Add(public.id())
        );
    }
}
