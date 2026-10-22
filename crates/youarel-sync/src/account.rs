//! A device's identity, membership log, epoch keys, segment envelopes, and pairing messages.
//!
//! The files sit in one directory beside the store:
//! `identity` and `epochs` with the device's keys, `genesis` naming the account, and each operation of the log under `log/`.
//! Pending invitations and a joining device's state sit under `invitations/` and in `joining`.
//! A file that holds a secret takes the permissions of the user's data directory, as the state beside it does.

use crate::{SyncError, hex, unhex};
mod segments;
pub use segments::OpenedSegment;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use youarel_core::membership_rules::Resolution;
use youarel_core::relay_rules::NONCE;
use youarel_crypto::kem::{self, KeyPair};
use youarel_crypto::pairing::{self, Code};
use youarel_crypto::recovery::RecoveryKey;
use youarel_crypto::seal::{self, COMMITMENT};
use youarel_crypto::sign::{ED25519_KEY, ML_DSA_SIGNING_KEY, SigningKeys};
use youarel_crypto::slot::{self, Binding};
use youarel_crypto::{DIGEST, KEY, Label};
use youarel_protocol::log::{self, Checked};
use youarel_protocol::message::{
    Action, Admission, DevicePublic, Digest, Entry, Join, Joined, Operation, Recovery, Rotation,
    SnapshotHint, Unsigned, Welcome, account_of, operation_digest,
};
use youarel_protocol::relay::{Request, UnsignedRequest, Verb};
pub use youarel_protocol::segment::body_digest;
use youarel_protocol::wire::{Reader, Writer};
use youarel_store::fs;
use zeroize::Zeroizing;

/// The file of the device's own keys.
const IDENTITY_FILE: &str = "identity";
/// The file of the epoch keys the device holds.
const EPOCHS_FILE: &str = "epochs";
/// The file of a joining device's state.
const JOINING_FILE: &str = "joining";
/// The directory of pending invitations, one file to an invitation.
const INVITATIONS: &str = "invitations";
/// The longest secret file of an account, which an identity or a long list of epoch keys fits.
const LONGEST_SECRET: u64 = 1024 * 1024;
/// The longest operation file a reader accepts.
const LONGEST_OPERATION: u64 = 2 * 1024 * 1024;
/// The suffix of an operation file.
const OPERATION_SUFFIX: &str = ".op";
/// The directory of the account's log, one file to an operation.
const LOG: &str = "log";
/// The file that names the account, which only a device of an account holds.
pub const GENESIS: &str = "genesis";

/// Whether the account directory belongs to an account, read without creating anything or reading the device's keys.
///
/// # Errors
///
/// Returns a [`SyncError`] when the file exists but cannot be read.
pub fn founded(directory: &Path) -> Result<bool, SyncError> {
    Ok(fs::read_if_present(&directory.join(GENESIS))?.is_some())
}

/// The bytes of a file that holds a secret, or `None` when the file does not exist.
fn read_secret(path: &Path) -> Result<Option<Zeroizing<Vec<u8>>>, SyncError> {
    Ok(fs::read_bounded_if_present(path, LONGEST_SECRET)?.map(Zeroizing::new))
}

/// The operations among the files `names` of the log directory `log` whose bytes digest to the names of their files.
fn read_log(log: &Path, names: Vec<String>) -> Result<BTreeMap<Digest, Vec<u8>>, SyncError> {
    let mut operations = BTreeMap::new();
    for name in names {
        let Some(digest) = digest_name(&name) else {
            continue;
        };
        let bytes = fs::read_bounded(&log.join(&name), LONGEST_OPERATION)?;
        if operation_digest(&bytes) == digest {
            operations.insert(digest, bytes);
        }
    }
    Ok(operations)
}

/// How many operations the account's log holds, read without creating anything or reading the device's keys, as an open account counts them.
///
/// # Errors
///
/// Returns a [`SyncError`] when the log exists but cannot be listed or an operation cannot be read.
pub fn logged(directory: &Path) -> Result<usize, SyncError> {
    let log = directory.join(LOG);
    let names = match fs::list(&log) {
        Ok(names) => names,
        Err(error) if error.source.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    Ok(read_log(&log, names)?.len())
}
/// The tag of a sealed record that still waits: an invitation without a join, or a join without a welcome.
const WAITING: u8 = 0;
/// The tag of a sealed record that moved on.
const ANSWERED: u8 = 1;
/// The longest code a sealed record holds as text.
const LONGEST_CODE: usize = 64;

/// A code that a sealed record holds as text.
fn read_code(reader: &mut Reader<'_>, what: &str) -> Result<Code, SyncError> {
    let refused = || SyncError::Refused(format!("{what} holds no code"));
    let text = core::str::from_utf8(reader.bytes(LONGEST_CODE)?).map_err(|_text| refused())?;
    Code::parse(text).ok_or_else(refused)
}

/// Epoch keys with their epochs.
type EpochKeys = Vec<(u32, Zeroizing<[u8; KEY]>)>;

/// The keys of this device.
struct Identity {
    signing: SigningKeys,
    kem: KeyPair,
    public: DevicePublic,
}

impl Identity {
    fn generate() -> Result<Self, SyncError> {
        Ok(Self::from_keys(
            SigningKeys::generate()?,
            KeyPair::generate()?,
        ))
    }

    /// The identity that the account in `directory` keeps, made and kept on the first open.
    fn kept(directory: &Path) -> Result<Self, SyncError> {
        let path = directory.join(IDENTITY_FILE);
        let Some(bytes) = read_secret(&path)? else {
            let identity = Self::generate()?;
            fs::replace(&path, &identity.encode())?;
            return Ok(identity);
        };
        Self::decode(&bytes)
    }

    fn from_keys(signing: SigningKeys, kem: KeyPair) -> Self {
        let public = DevicePublic {
            signing: signing.public().clone(),
            kem: Box::new(*kem.public()),
        };
        Self {
            signing,
            kem,
            public,
        }
    }

    /// The member a recovery key grows into, the same each time.
    fn from_recovery(key: &RecoveryKey) -> Result<Self, SyncError> {
        let seeds = key.seeds()?;
        Ok(Self::from_keys(
            SigningKeys::from_seeds(&seeds.ed25519, &seeds.ml_dsa),
            KeyPair::from_seed(seeds.kem.as_slice())?,
        ))
    }

    fn encode(&self) -> Zeroizing<Vec<u8>> {
        let (ed25519, ml_dsa) = self.signing.secret();
        let mut writer = Writer::new();
        writer
            .raw(&ed25519)
            .raw(&ml_dsa)
            .raw(&self.kem.seed())
            .raw(&self.public.encode());
        Zeroizing::new(writer.finish())
    }

    fn decode(bytes: &[u8]) -> Result<Self, SyncError> {
        let mut reader = Reader::new(bytes);
        let ed25519 = Zeroizing::new(reader.take(ED25519_KEY)?.to_vec());
        let ml_dsa = Zeroizing::new(reader.take(ML_DSA_SIGNING_KEY)?.to_vec());
        let seed = Zeroizing::new(reader.take(kem::SEED)?.to_vec());
        let public_bytes = reader.take(
            bytes
                .len()
                .saturating_sub(ED25519_KEY + ML_DSA_SIGNING_KEY + kem::SEED),
        )?;
        reader.finish()?;
        let public = public_from(public_bytes)?;
        let kem = KeyPair::from_seed(&seed)?;
        if kem.public() != public.kem.as_ref() {
            return Err(SyncError::Refused(String::from(
                "the identity's X-Wing key does not match its public half",
            )));
        }
        Ok(Self {
            signing: SigningKeys::from_secret(&ed25519, &ml_dsa, public.signing.clone())?,
            kem,
            public,
        })
    }
}

/// Public keys from their wire form.
fn public_from(bytes: &[u8]) -> Result<DevicePublic, SyncError> {
    Ok(DevicePublic::decode(bytes)?)
}

/// What an invitation waits for: a join, then the joiner's confirmation of the channel key, before which nothing secret leaves.
enum Invitation {
    Waiting(Code),
    Welcomed {
        joiner: DevicePublic,
        key: Zeroizing<[u8; KEY]>,
        confirmation: [u8; pairing::CONFIRMATION],
    },
}

/// Where a joining device stands.
enum Joining {
    /// It sent a join and waits for the welcome.
    Sent {
        code: Code,
        seed: Zeroizing<[u8; pairing::SEED]>,
    },
    /// It confirmed the channel key and waits for the entry it seals.
    Confirmed {
        invitation: u16,
        key: Zeroizing<[u8; KEY]>,
    },
}

/// The members of an account as this device sees them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Status {
    pub account: Digest,
    pub this_device: Digest,
    pub members: Vec<Digest>,
    pub epoch: u32,
    /// Whether this device is a member, which a removed device is not.
    pub member: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Member {
    pub device: Digest,
    pub added: u64,
    pub recovery: bool,
}

/// A device's account, open in its directory.
pub struct Account {
    directory: PathBuf,
    identity: Identity,
    genesis: Option<Digest>,
    log: BTreeMap<Digest, Vec<u8>>,
    /// Every epoch key this device holds, by its commitment, with its epoch.
    epochs: BTreeMap<[u8; COMMITMENT], (u32, Zeroizing<[u8; KEY]>)>,
}

impl core::fmt::Debug for Account {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Account")
            .field("directory", &self.directory)
            .field("device", &hex(&self.identity.public.id()))
            .finish_non_exhaustive()
    }
}

fn digest_name(name: &str) -> Option<Digest> {
    unhex::<DIGEST>(name.strip_suffix(OPERATION_SUFFIX)?)
}

impl Account {
    /// The account in `directory`, with a new identity when the device has none.
    ///
    /// # Errors
    ///
    /// Returns a [`SyncError`] when a file cannot be read, or the identity or the epoch keys it holds are malformed.
    pub fn open(directory: &Path) -> Result<Self, SyncError> {
        let log_directory = directory.join(LOG);
        fs::create_dir_all(&log_directory)?;
        fs::create_dir_all(&directory.join(INVITATIONS))?;
        let identity = Identity::kept(directory)?;
        let genesis = match fs::read_if_present(&directory.join(GENESIS))? {
            Some(text) => Some(unhex::<DIGEST>(text.trim()).ok_or_else(|| {
                SyncError::Refused(String::from("the genesis file holds no digest"))
            })?),
            None => None,
        };
        let log = read_log(&log_directory, fs::list(&log_directory)?)?;
        let mut epochs = BTreeMap::new();
        if let Some(bytes) = read_secret(&directory.join(EPOCHS_FILE))? {
            let mut reader = Reader::new(&bytes);
            let count = reader.count(youarel_protocol::message::MOST_EPOCHS)?;
            for _ in 0..count {
                let epoch = reader.word()?;
                let key = Zeroizing::new(reader.array::<KEY>()?);
                epochs.insert(seal::commitment(&key)?, (epoch, key));
            }
            reader.finish()?;
        }
        Ok(Self {
            directory: directory.to_path_buf(),
            identity,
            genesis,
            log,
            epochs,
        })
    }

    /// This device's identifier.
    #[must_use]
    pub fn device(&self) -> Digest {
        self.identity.public.id()
    }

    /// Whether the device belongs to an account yet, as its founder or through a pairing.
    #[must_use]
    pub const fn has_account(&self) -> bool {
        self.genesis.is_some()
    }

    /// How many operations the account's log holds, which [`logged`] counts the same way without the device's keys.
    #[must_use]
    pub fn operations(&self) -> usize {
        self.log.len()
    }

    fn save_epochs(&self) -> Result<(), SyncError> {
        let mut writer = Writer::new();
        writer.count(self.epochs.len())?;
        for (epoch, key) in self.epochs.values() {
            writer.word(*epoch).raw(key.as_slice());
        }
        Ok(fs::replace(
            &self.directory.join(EPOCHS_FILE),
            &Zeroizing::new(writer.finish()),
        )?)
    }

    fn keep_operation(&mut self, bytes: Vec<u8>) -> Result<Digest, SyncError> {
        let digest = operation_digest(&bytes);
        if !self.log.contains_key(&digest) {
            fs::replace(
                &self
                    .directory
                    .join(LOG)
                    .join(format!("{}{OPERATION_SUFFIX}", hex(&digest))),
                &bytes,
            )?;
            self.log.insert(digest, bytes);
        }
        Ok(digest)
    }

    fn keep_key(&mut self, epoch: u32, key: Zeroizing<[u8; KEY]>) -> Result<bool, SyncError> {
        let commitment = seal::commitment(&key)?;
        if self.epochs.contains_key(&commitment) {
            return Ok(false);
        }
        self.epochs.insert(commitment, (epoch, key));
        Ok(true)
    }

    fn genesis(&self) -> Result<Digest, SyncError> {
        self.genesis.ok_or_else(|| {
            SyncError::Refused(String::from("this device belongs to no account yet"))
        })
    }

    /// The account's identifier, from the bytes of its genesis.
    fn account_id(&self) -> Result<Digest, SyncError> {
        let genesis = self.genesis()?;
        let bytes = self
            .log
            .get(&genesis)
            .ok_or_else(|| SyncError::Refused(String::from("the log lacks its genesis")))?;
        Ok(account_of(bytes))
    }

    /// The log with every operation that verifies, under the key its author published, and its resolution.
    fn check(&self) -> Result<Checked, SyncError> {
        Ok(log::check(self.genesis()?, self.account_id()?, &self.log))
    }

    /// Writes an operation of this device into the log.
    fn write_operation(
        &mut self,
        checked: &Checked,
        now: u64,
        action: Action,
    ) -> Result<Digest, SyncError> {
        let account = if matches!(action, Action::Genesis(_)) {
            log::NO_ACCOUNT
        } else {
            self.account_id()?
        };
        let bytes = Self::operation_by(&self.identity, account, checked, (now, action))?;
        self.keep_operation(bytes)
    }

    /// An operation in `account` that `author` signs at `now`, after every head of the log.
    fn operation_by(
        author: &Identity,
        account: Digest,
        checked: &Checked,
        (now, action): (u64, Action),
    ) -> Result<Vec<u8>, SyncError> {
        let parents = checked.heads();
        let id = author.public.id();
        let unsigned = Unsigned {
            account: &account,
            parents: &parents,
            author: &id,
            wall: now,
            action: &action,
        };
        let signature = author.signing.sign(Label::Operation, &unsigned.bytes()?)?;
        let operation = Operation {
            account,
            parents,
            author: id,
            wall: now,
            action,
            signature,
        };
        Ok(operation.encode()?)
    }

    /// Founds a new account with this device as its only member.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when the device belongs to an account already.
    pub fn found(&mut self, now: u64) -> Result<Digest, SyncError> {
        if self.genesis.is_some() {
            return Err(SyncError::Refused(String::from(
                "this device belongs to an account already",
            )));
        }
        let empty = Checked {
            operations: BTreeMap::new(),
            keys: BTreeMap::new(),
            resolution: Resolution::default(),
        };
        let genesis =
            self.write_operation(&empty, now, Action::Genesis(self.identity.public.clone()))?;
        fs::replace(
            &self.directory.join(GENESIS),
            format!("{}\n", hex(&genesis)).as_bytes(),
        )?;
        self.genesis = Some(genesis);
        self.rotate_if_due(now)?;
        self.account_id()
    }

    /// The account's members as this device resolves them.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when the device belongs to no account.
    pub fn status(&self) -> Result<Status, SyncError> {
        let checked = self.check()?;
        let device = self.device();
        Ok(Status {
            account: self.account_id()?,
            this_device: device,
            member: checked.resolution.members.contains(&device),
            members: checked.resolution.members,
            epoch: checked.resolution.epoch,
        })
    }

    /// # Errors
    ///
    /// Refuses an absent or invalid membership log.
    pub fn members(&self) -> Result<Vec<Member>, SyncError> {
        let checked = self.check()?;
        let mut added = BTreeMap::new();
        for digest in &checked.resolution.applied {
            let Some(operation) = checked.operations.get(digest) else {
                continue;
            };
            match &operation.action {
                Action::Genesis(device) | Action::Add(device) => {
                    added.insert(
                        device.id(),
                        Member {
                            device: device.id(),
                            added: operation.wall,
                            recovery: false,
                        },
                    );
                }
                Action::AddRecovery(recovery) => {
                    let device = recovery.device.id();
                    added.insert(
                        device,
                        Member {
                            device,
                            added: operation.wall,
                            recovery: true,
                        },
                    );
                }
                Action::Remove(device) => {
                    added.remove(device);
                }
                Action::Rotate(_) => {}
            }
        }
        Ok(checked
            .resolution
            .members
            .iter()
            .filter_map(|device| added.get(device).copied())
            .collect())
    }

    /// Whether the canonical rotation leaves a member without a key, or a removal followed it.
    fn rotation_due(checked: &Checked) -> bool {
        let resolution = &checked.resolution;
        let Some(rotation) = resolution.rotation else {
            return true;
        };
        let slotted: BTreeSet<Digest> = match checked
            .operations
            .get(&rotation)
            .map(|operation| &operation.action)
        {
            Some(Action::Rotate(rotation)) => {
                rotation.slots.iter().map(|slot| slot.device).collect()
            }
            _ => BTreeSet::new(),
        };
        let unslotted = resolution
            .members
            .iter()
            .any(|member| !slotted.contains(member));
        let removed_since = resolution
            .applied
            .iter()
            .skip_while(|digest| **digest != rotation)
            .any(|digest| {
                matches!(
                    checked
                        .operations
                        .get(digest)
                        .map(|operation| &operation.action),
                    Some(Action::Remove(_))
                )
            });
        unslotted || removed_since
    }

    /// Rotates to a new epoch whose key reaches every member, when a member lacks the current key or a removal followed it.
    fn rotate_if_due(&mut self, now: u64) -> Result<(), SyncError> {
        let checked = self.check()?;
        if !checked.resolution.members.contains(&self.device()) || !Self::rotation_due(&checked) {
            return Ok(());
        }
        let account = self.account_id()?;
        let epoch = checked.resolution.epoch.saturating_add(1);
        let key = Zeroizing::new(youarel_crypto::random::<KEY>()?);
        let mut slots = Vec::new();
        for member in &checked.resolution.members {
            let public = checked.keys.get(member).ok_or_else(|| {
                SyncError::Refused(format!("the log lacks the keys of member {}", hex(member)))
            })?;
            let binding = Binding {
                account: &account,
                epoch,
                device: member,
            };
            slots.push(slot::wrap(&key, public.kem.as_slice(), &binding)?);
        }
        let rotation = Rotation {
            epoch,
            commitment: seal::commitment(&key)?,
            slots,
        };
        self.write_operation(&checked, now, Action::Rotate(rotation))?;
        self.keep_key(epoch, key)?;
        self.save_epochs()
    }

    /// Learns the key of every rotation by a member that holds a slot for this device.
    fn learn_keys(&mut self, checked: &Checked) -> Result<(), SyncError> {
        let keys = self.unwrapped(&self.account_id()?, checked, &self.identity)?;
        self.keep_keys(keys)
    }

    /// The key of every rotation this device lacks that holds a slot for `identity`, with its epoch.
    fn unwrapped(
        &self,
        account: &Digest,
        checked: &Checked,
        identity: &Identity,
    ) -> Result<EpochKeys, SyncError> {
        let device = identity.public.id();
        let mut keys = Vec::new();
        for digest in &checked.resolution.rotations {
            let Some(Action::Rotate(rotation)) = checked
                .operations
                .get(digest)
                .map(|operation| &operation.action)
            else {
                continue;
            };
            if self.epochs.contains_key(&rotation.commitment) {
                continue;
            }
            let Some(slot) = rotation.slots.iter().find(|slot| slot.device == device) else {
                continue;
            };
            let binding = Binding {
                account,
                epoch: rotation.epoch,
                device: &device,
            };
            keys.push((rotation.epoch, slot::unwrap(slot, &identity.kem, &binding)?));
        }
        Ok(keys)
    }

    fn keep_keys(&mut self, keys: EpochKeys) -> Result<(), SyncError> {
        let mut learned = false;
        for (epoch, key) in keys {
            learned |= self.keep_key(epoch, key)?;
        }
        if learned {
            self.save_epochs()?;
        }
        Ok(())
    }

    fn invitation_path(&self, number: u16) -> PathBuf {
        self.directory.join(INVITATIONS).join(number.to_string())
    }

    fn invitation(&self, number: u16) -> Result<Option<Invitation>, SyncError> {
        let Some(bytes) = read_secret(&self.invitation_path(number))? else {
            return Ok(None);
        };
        let mut reader = Reader::new(&bytes);
        let invitation = if reader.byte()? == WAITING {
            Invitation::Waiting(read_code(&mut reader, "an invitation")?)
        } else {
            let joiner = public_from(reader.bytes(youarel_protocol::message::LONGEST_OPERATION)?)?;
            Invitation::Welcomed {
                joiner,
                key: Zeroizing::new(reader.array()?),
                confirmation: reader.array()?,
            }
        };
        reader.finish()?;
        Ok(Some(invitation))
    }

    fn save_invitation(&self, number: u16, invitation: &Invitation) -> Result<(), SyncError> {
        let mut writer = Writer::new();
        match invitation {
            Invitation::Waiting(code) => {
                writer.byte(WAITING);
                writer.bytes(code.text().as_bytes())?;
            }
            Invitation::Welcomed {
                joiner,
                key,
                confirmation,
            } => {
                writer.byte(ANSWERED);
                writer.bytes(&joiner.encode())?;
                writer.raw(key.as_slice()).raw(confirmation);
            }
        }
        Ok(fs::replace(
            &self.invitation_path(number),
            &Zeroizing::new(writer.finish()),
        )?)
    }

    /// A new invitation code, which a person types on the joining device.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when this device is no member of an account.
    pub fn invite(&self) -> Result<Code, SyncError> {
        self.invite_as(Code::generate()?)
    }

    /// # Errors
    ///
    /// Reports failure to remove this invitation's local channel state.
    pub fn cancel_invitation(&self, number: u16) -> Result<(), SyncError> {
        Ok(fs::remove_if_present(&self.invitation_path(number))?)
    }

    /// # Errors
    ///
    /// Reports failure to remove this device's unfinished join state.
    pub fn cancel_join(&self) -> Result<(), SyncError> {
        Ok(fs::remove_if_present(&self.directory.join(JOINING_FILE))?)
    }

    /// An invitation under `code`, whose number a relay's mailbox holds.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when this device is no member of an account.
    pub fn invite_as(&self, code: Code) -> Result<Code, SyncError> {
        if !self.status()?.member {
            return Err(SyncError::Refused(String::from(
                "only a member invites a device",
            )));
        }
        self.save_invitation(code.number, &Invitation::Waiting(code))?;
        Ok(code)
    }

    /// A request to the relay with `payload`, stamped `now` and signed by this device, under a nonce of its own.
    ///
    /// # Errors
    ///
    /// Returns a [`SyncError`] when the device belongs to no account, the payload passes the request's limit, or signing fails.
    pub fn request(&self, now: u64, verb: Verb, payload: Vec<u8>) -> Result<Vec<u8>, SyncError> {
        let account = self.account_id()?;
        let device = self.device();
        let nonce = youarel_crypto::random::<NONCE>()?;
        let unsigned = UnsignedRequest {
            account: &account,
            device: &device,
            verb,
            stamped: now,
            nonce: &nonce,
            payload: &payload,
        };
        let signature = self
            .identity
            .signing
            .sign(Label::Request, &unsigned.bytes()?)?;
        Ok(Request {
            account,
            device,
            verb,
            stamped: now,
            nonce,
            payload,
            signature,
        }
        .encode()?)
    }

    fn joining(&self) -> Result<Option<Joining>, SyncError> {
        let Some(bytes) = read_secret(&self.directory.join(JOINING_FILE))? else {
            return Ok(None);
        };
        let mut reader = Reader::new(&bytes);
        let joining = if reader.byte()? == WAITING {
            Joining::Sent {
                code: read_code(&mut reader, "the joining state")?,
                seed: Zeroizing::new(reader.array()?),
            }
        } else {
            Joining::Confirmed {
                invitation: reader.short()?,
                key: Zeroizing::new(reader.array()?),
            }
        };
        reader.finish()?;
        Ok(Some(joining))
    }

    fn save_joining(&self, joining: &Joining) -> Result<(), SyncError> {
        let mut writer = Writer::new();
        match joining {
            Joining::Sent { code, seed } => {
                writer.byte(WAITING);
                writer.bytes(code.text().as_bytes())?;
                writer.raw(seed.as_slice());
            }
            Joining::Confirmed { invitation, key } => {
                writer.byte(ANSWERED).short(*invitation).raw(key.as_slice());
            }
        }
        Ok(fs::replace(
            &self.directory.join(JOINING_FILE),
            &Zeroizing::new(writer.finish()),
        )?)
    }

    /// The joining device's first message, for the code a member showed.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when this device belongs to an account already.
    pub fn join(&self, code: Code) -> Result<Vec<u8>, SyncError> {
        if self.genesis.is_some() {
            return Err(SyncError::Refused(String::from(
                "this device belongs to an account already",
            )));
        }
        let (seed, pake) = pairing::join(code)?;
        self.save_joining(&Joining::Sent { code, seed })?;
        let join = Join {
            invitation: code.number,
            joiner: self.identity.public.clone(),
            pake,
        };
        Ok(join.encode()?)
    }

    /// The welcome for a join: the inviter's half of the key exchange and its proof of the channel key, which hold nothing secret.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when no invitation waits for this join.
    pub fn accept(&self, bytes: &[u8]) -> Result<Vec<u8>, SyncError> {
        let join = Join::decode(bytes)?;
        let Some(Invitation::Waiting(code)) = self.invitation(join.invitation)? else {
            return Err(SyncError::Refused(format!(
                "no invitation {} waits for a join",
                join.invitation
            )));
        };
        let accepted = pairing::accept(
            code,
            &join.pake,
            join.joiner.kem.as_slice(),
            &join.joiner.encode(),
        )?;
        let welcome = Welcome {
            invitation: join.invitation,
            pake: accepted.inviter_message,
            encapsulation: accepted.encapsulation,
            welcomed: accepted.channel.welcomed(),
        };
        self.save_invitation(
            join.invitation,
            &Invitation::Welcomed {
                joiner: join.joiner,
                key: Zeroizing::new(*accepted.channel.key()),
                confirmation: accepted.channel.joined(),
            },
        )?;
        Ok(welcome.encode()?)
    }

    /// Admits a confirmed joiner and seals the membership, epoch keys, and optional snapshot hint for it alone.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when no welcomed invitation waits for this confirmation, the confirmation fails, or this device is no member.
    pub fn admit(
        &mut self,
        now: u64,
        bytes: &[u8],
        hint: Option<SnapshotHint>,
    ) -> Result<Vec<u8>, SyncError> {
        let joined = Joined::decode(bytes)?;
        let Some(Invitation::Welcomed {
            joiner,
            key: channel,
            confirmation,
        }) = self.invitation(joined.invitation)?
        else {
            return Err(SyncError::Refused(format!(
                "no invitation {} waits for a confirmation",
                joined.invitation
            )));
        };
        if !youarel_crypto::equal(&confirmation, &joined.confirmation) {
            return Err(SyncError::Refused(String::from(
                "the joiner's confirmation does not verify, so it never held the code",
            )));
        }
        let checked = self.check()?;
        if !checked.resolution.members.contains(&self.device()) {
            return Err(SyncError::Refused(String::from(
                "only a member admits a device",
            )));
        }
        if !checked.resolution.members.contains(&joiner.id()) {
            self.write_operation(&checked, now, Action::Add(joiner))?;
        }
        let admitted = self.check()?;
        let admission = Admission {
            account: self.account_id()?,
            operations: admitted
                .operations
                .keys()
                .filter_map(|digest| self.log.get(digest).cloned())
                .collect(),
            epochs: {
                let mut epochs: Vec<(u32, [u8; KEY])> = self
                    .epochs
                    .values()
                    .map(|(epoch, key)| (*epoch, **key))
                    .collect();
                epochs.sort_unstable();
                epochs
            },
            hint,
        };
        let entry = Entry {
            invitation: joined.invitation,
            sealed: seal::seal(
                &channel,
                &joined.invitation.to_be_bytes(),
                &Zeroizing::new(admission.encode()?),
            )?,
        };
        fs::remove_if_present(&self.invitation_path(joined.invitation))?;
        Ok(entry.encode()?)
    }

    /// The joiner's confirmation for a welcome that proves the inviter holds the same channel key.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when no join waits for this welcome, and another [`SyncError`] when it does not authenticate.
    pub fn confirm(&self, bytes: &[u8]) -> Result<Vec<u8>, SyncError> {
        let welcome = Welcome::decode(bytes)?;
        let Some(Joining::Sent { code, seed }) = self.joining()? else {
            return Err(SyncError::Refused(String::from("this device sent no join")));
        };
        if code.number != welcome.invitation {
            return Err(SyncError::Refused(String::from(
                "the welcome answers another invitation",
            )));
        }
        let public = self.identity.public.encode();
        let joiner = pairing::Joiner {
            seed: &seed,
            keys: &self.identity.kem,
            public: &public,
        };
        let channel = pairing::finish(
            code,
            &joiner,
            &pairing::Welcome {
                inviter_message: &welcome.pake,
                encapsulation: &welcome.encapsulation,
                welcomed: &welcome.welcomed,
            },
        )?;
        self.save_joining(&Joining::Confirmed {
            invitation: welcome.invitation,
            key: Zeroizing::new(*channel.key()),
        })?;
        Ok(Joined {
            invitation: welcome.invitation,
            confirmation: channel.joined(),
        }
        .encode())
    }

    fn admission_genesis(&self, admission: &Admission) -> Result<Digest, SyncError> {
        let genesis = admission
            .operations
            .iter()
            .find(|operation| account_of(operation) == admission.account)
            .map(|operation| operation_digest(operation))
            .ok_or_else(|| {
                SyncError::Refused(String::from("the entry lacks the account's genesis"))
            })?;
        let operations: BTreeMap<Digest, Vec<u8>> = admission
            .operations
            .iter()
            .map(|operation| (operation_digest(operation), operation.clone()))
            .collect();
        let checked = log::check(genesis, admission.account, &operations);
        if !checked.member(&self.device()) {
            return Err(SyncError::Refused(String::from(
                "the entry does not admit this device",
            )));
        }
        Ok(genesis)
    }

    /// Installs checked membership and epoch keys, returning the hint for a separate bounded bootstrap.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when this device confirmed no welcome of this invitation, and another [`SyncError`] when the entry does not authenticate.
    pub fn finish(&mut self, bytes: &[u8]) -> Result<Option<SnapshotHint>, SyncError> {
        if self.genesis.is_some() {
            return Err(SyncError::Refused(String::from(
                "this device belongs to an account already",
            )));
        }
        let entry = Entry::decode(bytes)?;
        let Some(Joining::Confirmed {
            invitation,
            key: channel,
        }) = self.joining()?
        else {
            return Err(SyncError::Refused(String::from(
                "this device confirmed no welcome",
            )));
        };
        if invitation != entry.invitation {
            return Err(SyncError::Refused(String::from(
                "the entry answers another invitation",
            )));
        }
        let admission = Admission::decode(&seal::open(
            &channel,
            &entry.invitation.to_be_bytes(),
            &entry.sealed,
        )?)?;
        let genesis = self.admission_genesis(&admission)?;
        for operation in admission.operations {
            self.keep_operation(operation)?;
        }
        fs::replace(
            &self.directory.join(GENESIS),
            format!("{}\n", hex(&genesis)).as_bytes(),
        )?;
        self.genesis = Some(genesis);
        for (epoch, key) in admission.epochs {
            self.keep_key(epoch, Zeroizing::new(key))?;
        }
        self.save_epochs()?;
        fs::remove_if_present(&self.directory.join(JOINING_FILE))?;
        Ok(admission.hint)
    }

    /// Removes a device from the account; the next bundle rotates to a key it never sees.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when this device is no member or the device is none either.
    pub fn remove(&mut self, now: u64, device: Digest) -> Result<(), SyncError> {
        let checked = self.check()?;
        if !checked.resolution.members.contains(&self.device()) {
            return Err(SyncError::Refused(String::from(
                "only a member removes a device",
            )));
        }
        if !checked.resolution.members.contains(&device) {
            return Err(SyncError::Refused(format!(
                "{} is no member of the account",
                hex(&device)
            )));
        }
        self.write_operation(&checked, now, Action::Remove(device))?;
        self.rotate_if_due(now)
    }

    /// The members whose identifiers start with `prefix`.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when the device belongs to no account.
    pub fn members_matching(&self, prefix: &str) -> Result<Vec<Digest>, SyncError> {
        Ok(self
            .status()?
            .members
            .into_iter()
            .filter(|member| hex(member).starts_with(prefix))
            .collect())
    }

    /// A new recovery key, whose member joins the account and receives a slot of the rotation that follows at once.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when this device is no member.
    pub fn enroll_recovery(&mut self, now: u64) -> Result<RecoveryKey, SyncError> {
        let checked = self.check()?;
        if !checked.member(&self.device()) {
            return Err(SyncError::Refused(String::from(
                "only a member creates a recovery key",
            )));
        }
        let key = RecoveryKey::generate()?;
        let recovery = Identity::from_recovery(&key)?;
        let public = recovery.public.clone();
        let consent = recovery
            .signing
            .sign(Label::RecoveryConsent, &self.account_id()?)?;
        self.write_operation(
            &checked,
            now,
            Action::AddRecovery(Recovery {
                device: recovery.public,
                consent,
            }),
        )?;
        self.rotate_if_due(now)?;
        self.share_history(now, &public)?;
        Ok(key)
    }

    fn share_history(&mut self, now: u64, recovery: &DevicePublic) -> Result<(), SyncError> {
        let checked = self.check()?;
        let current = checked
            .resolution
            .rotation
            .and_then(|digest| checked.operations.get(&digest))
            .and_then(|operation| {
                if let Action::Rotate(rotation) = &operation.action {
                    Some(rotation.commitment)
                } else {
                    None
                }
            });
        let account = self.account_id()?;
        let device = recovery.id();
        let mut historical = Vec::new();
        for (commitment, (epoch, key)) in &self.epochs {
            if Some(*commitment) == current {
                continue;
            }
            let binding = Binding {
                account: &account,
                epoch: *epoch,
                device: &device,
            };
            historical.push(Rotation {
                epoch: *epoch,
                commitment: *commitment,
                slots: vec![slot::wrap(key, recovery.kem.as_slice(), &binding)?],
            });
        }
        for rotation in historical {
            self.write_operation(&self.check()?, now, Action::Rotate(rotation))?;
        }
        Ok(())
    }

    /// The identifier of the member that `key` grows into, under which a relay finds its account.
    ///
    /// # Errors
    ///
    /// Returns a [`SyncError`] when the key's identity cannot be made.
    pub fn recovery_member(key: &RecoveryKey) -> Result<Digest, SyncError> {
        Ok(Identity::from_recovery(key)?.public.id())
    }

    /// A recovery member's signed request for retained segment chains.
    ///
    /// # Errors
    ///
    /// Returns a [`SyncError`] when the key's identity cannot be made or signing fails.
    pub fn recovery_request(
        key: &RecoveryKey,
        account: Digest,
        (now, verb): (u64, Verb),
    ) -> Result<Vec<u8>, SyncError> {
        let recovery = Identity::from_recovery(key)?;
        let device = recovery.public.id();
        let nonce = youarel_crypto::random::<NONCE>()?;
        let payload = if verb == Verb::Collect {
            youarel_protocol::segment::Cursors::default().encode()?
        } else {
            Vec::new()
        };
        let unsigned = UnsignedRequest {
            account: &account,
            device: &device,
            verb,
            stamped: now,
            nonce: &nonce,
            payload: &payload,
        };
        let signature = recovery.signing.sign(Label::Request, &unsigned.bytes()?)?;
        Ok(Request {
            account,
            device,
            verb,
            stamped: now,
            nonce,
            payload,
            signature,
        }
        .encode()?)
    }

    /// The members that recovery keys grow into, which `status` lists among the others.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when the device belongs to no account.
    pub fn recovery_members(&self) -> Result<Vec<Digest>, SyncError> {
        Ok(self.check()?.recoveries())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Account, EPOCHS_FILE, GENESIS, IDENTITY_FILE, INVITATIONS, Identity, JOINING_FILE, LOG,
        LONGEST_OPERATION, OPERATION_SUFFIX, founded, logged, operation_digest,
    };
    use crate::hex;
    use youarel_store::fs;

    const NOW: u64 = 1_700_000_000_000;

    #[test]
    fn the_log_counts_each_operation_whose_bytes_match_its_name() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(logged(&directory.path().join("absent")).unwrap(), 0);
        let log = directory.path().join(LOG);
        fs::create_dir_all(&log).unwrap();
        assert_eq!(logged(directory.path()).unwrap(), 0);
        for bytes in [b"one".as_slice(), b"two"] {
            let name = format!("{}{OPERATION_SUFFIX}", hex(&operation_digest(bytes)));
            fs::replace(&log.join(name), bytes).unwrap();
        }
        let forged = format!("{}{OPERATION_SUFFIX}", hex(&operation_digest(b"three")));
        fs::replace(&log.join(forged), b"not three").unwrap();
        fs::replace(&log.join("notes.txt"), b"one").unwrap();
        assert_eq!(logged(directory.path()).unwrap(), 2);
        let unlisted = tempfile::tempdir().unwrap();
        fs::replace(&unlisted.path().join(LOG), b"a file").unwrap();
        logged(unlisted.path()).unwrap_err();
    }

    #[test]
    fn an_open_account_counts_its_log_as_its_files_do() {
        let directory = tempfile::tempdir().unwrap();
        let mut account = Account::open(directory.path()).unwrap();
        assert_eq!(account.operations(), 0);
        account.found(NOW).unwrap();
        assert_eq!(
            account.operations(),
            2,
            "the genesis and the first rotation"
        );
        assert_eq!(logged(directory.path()).unwrap(), 2);
        let reopened = Account::open(directory.path()).unwrap();
        assert_eq!(reopened.operations(), 2);
    }

    #[test]
    fn the_log_reads_every_operation_the_wire_admits() {
        assert_eq!(
            usize::try_from(LONGEST_OPERATION).unwrap(),
            youarel_protocol::message::LONGEST_OPERATION
        );
    }

    #[test]
    fn a_directory_belongs_to_an_account_once_it_names_one() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!founded(&directory.path().join("absent")).unwrap());
        assert!(!founded(directory.path()).unwrap());
        fs::replace(&directory.path().join(GENESIS), b"named").unwrap();
        assert!(founded(directory.path()).unwrap());
    }

    #[test]
    fn an_account_keeps_its_keys_in_plain_files_and_reopens_them() {
        use youarel_core::segment_rules::{Header, Kind, Sequence};
        let (inviting, joining) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mut inviter = Account::open(inviting.path()).unwrap();
        inviter.found(NOW).unwrap();
        let header = Header::new(
            Sequence::FIRST,
            [0; 32],
            Kind::Snapshot { part: 0, parts: 1 },
        )
        .unwrap();
        let segment = inviter.publish(NOW, header, b"state").unwrap();
        let code = inviter.invite().unwrap();
        let join = Account::open(joining.path()).unwrap().join(code).unwrap();
        let identity = fs::read_bytes_if_present(&inviting.path().join(IDENTITY_FILE)).unwrap();
        assert_eq!(
            Identity::decode(&identity.unwrap()).unwrap().public.id(),
            inviter.device()
        );
        let secrets = [
            inviting.path().join(IDENTITY_FILE),
            inviting.path().join(EPOCHS_FILE),
            inviting
                .path()
                .join(INVITATIONS)
                .join(code.number.to_string()),
            joining.path().join(IDENTITY_FILE),
            joining.path().join(JOINING_FILE),
        ];
        for secret in &secrets {
            assert_ne!(
                fs::read_bytes_if_present(secret).unwrap(),
                None,
                "{secret:?}"
            );
        }
        let mut reopened = Account::open(inviting.path()).unwrap();
        assert_eq!(reopened.device(), inviter.device());
        assert_eq!(
            reopened.open_segment(&segment).unwrap().body.as_slice(),
            b"state"
        );
        reopened.accept(&join).unwrap();
    }

    #[test]
    fn an_identity_that_holds_no_keys_is_refused_and_kept() {
        let directory = tempfile::tempdir().unwrap();
        let identity = directory.path().join(IDENTITY_FILE);
        fs::replace(&identity, &[1; 100]).unwrap();
        Account::open(directory.path()).unwrap_err();
        assert_eq!(
            fs::read_bytes_if_present(&identity).unwrap(),
            Some(vec![1; 100]),
            "an open never makes a new identity over one it cannot read"
        );
    }
}
