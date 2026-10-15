//! A device's account: its identity, the membership log, the epoch keys, and the bundles, joins, and welcomes it reads and writes.
//!
//! The files sit in one directory beside the store:
//! `identity` and `epochs` sealed under the key the credential store holds, `genesis` naming the account, and each operation of the log under `log/`.
//! Pending invitations and a joining device's state sit under `invitations/` and in `joining`, sealed as well.

use crate::keystore::KeyStore;
use crate::{SyncError, hex, sealed, unhex};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use youarel_core::membership_rules::{self, Resolution};
use youarel_crypto::kem::{self, KeyPair};
use youarel_crypto::pairing::{self, Code};
use youarel_crypto::seal::{self, COMMITMENT};
use youarel_crypto::sign::{ED25519_KEY, ML_DSA_SIGNING_KEY, SigningKeys};
use youarel_crypto::slot::{self, Binding};
use youarel_crypto::{DIGEST, KEY, Label};
use youarel_protocol::message::{
    Action, Admission, Bundle, DevicePublic, Digest, Entry, Join, Joined, Operation, Rotation,
    Unsigned, UnsignedBundle, Welcome, account_of, operation_digest,
};
use youarel_protocol::wire::{Reader, Writer};
use youarel_store::fs;
use zeroize::Zeroizing;

/// The purposes the sealed files of an account bind.
const IDENTITY: &[u8] = b"youarel/v1/file/identity";
const EPOCHS: &[u8] = b"youarel/v1/file/epochs";
const INVITATION: &[u8] = b"youarel/v1/file/invitation";
const JOINING: &[u8] = b"youarel/v1/file/joining";
/// The longest operation file a reader accepts.
const LONGEST_OPERATION: u64 = 2 * 1024 * 1024;
/// The suffix of an operation file.
const OPERATION_SUFFIX: &str = ".op";
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

/// The keys of this device.
struct Identity {
    signing: SigningKeys,
    kem: KeyPair,
    public: DevicePublic,
}

impl Identity {
    fn generate() -> Result<Self, SyncError> {
        let signing = SigningKeys::generate()?;
        let kem = KeyPair::generate()?;
        let public = DevicePublic {
            signing: signing.public().clone(),
            kem: Box::new(*kem.public()),
        };
        Ok(Self {
            signing,
            kem,
            public,
        })
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

/// What a bundle brought.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Imported {
    pub sender: Digest,
    /// The sender's state in the JSON form of the store.
    pub state: Zeroizing<Vec<u8>>,
}

/// The log after its checks: every operation that verified, and its resolution.
struct Checked {
    operations: BTreeMap<Digest, Operation>,
    keys: BTreeMap<Digest, DevicePublic>,
    resolution: Resolution,
}

/// A device's account, open in its directory.
pub struct Account {
    directory: PathBuf,
    sealing: Zeroizing<[u8; KEY]>,
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
    /// Returns a [`SyncError`] when a file cannot be read or opened, or the key store refuses.
    pub fn open(directory: &Path, store: &KeyStore) -> Result<Self, SyncError> {
        fs::create_dir_all(&directory.join("log"))?;
        fs::create_dir_all(&directory.join("invitations"))?;
        let sealing = store.key()?;
        let identity_path = directory.join("identity");
        let identity = if let Some(bytes) = sealed::read(&identity_path, &sealing, IDENTITY)? {
            Identity::decode(&bytes)?
        } else {
            let identity = Identity::generate()?;
            sealed::write(&identity_path, &sealing, IDENTITY, &identity.encode())?;
            identity
        };
        let genesis = match fs::read_if_present(&directory.join("genesis"))? {
            Some(text) => Some(unhex::<DIGEST>(text.trim()).ok_or_else(|| {
                SyncError::Refused(String::from("the genesis file holds no digest"))
            })?),
            None => None,
        };
        let mut log = BTreeMap::new();
        for name in fs::list(&directory.join("log"))? {
            let Some(digest) = digest_name(&name) else {
                continue;
            };
            let bytes = fs::read_bounded(&directory.join("log").join(&name), LONGEST_OPERATION)?;
            if operation_digest(&bytes) == digest {
                log.insert(digest, bytes);
            }
        }
        let mut epochs = BTreeMap::new();
        if let Some(bytes) = sealed::read(&directory.join("epochs"), &sealing, EPOCHS)? {
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
            sealing,
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

    fn save_epochs(&self) -> Result<(), SyncError> {
        let mut writer = Writer::new();
        writer.count(self.epochs.len())?;
        for (epoch, key) in self.epochs.values() {
            writer.word(*epoch).raw(key.as_slice());
        }
        sealed::write(
            &self.directory.join("epochs"),
            &self.sealing,
            EPOCHS,
            &Zeroizing::new(writer.finish()),
        )
    }

    fn keep_operation(&mut self, bytes: Vec<u8>) -> Result<Digest, SyncError> {
        let digest = operation_digest(&bytes);
        if !self.log.contains_key(&digest) {
            fs::replace(
                &self
                    .directory
                    .join("log")
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
        let genesis = self.genesis()?;
        let account = self.account_id()?;
        let mut decoded = BTreeMap::new();
        let mut keys = BTreeMap::new();
        for (digest, bytes) in &self.log {
            let Ok(operation) = Operation::decode(bytes) else {
                continue;
            };
            if let Action::Genesis(device) | Action::Add(device) = &operation.action {
                keys.insert(device.id(), device.clone());
            }
            decoded.insert(*digest, operation);
        }
        let mut operations = BTreeMap::new();
        for (digest, operation) in decoded {
            let expected = if digest == genesis { [0; 32] } else { account };
            let verifies = operation.account == expected
                && operation.unsigned().bytes().is_ok_and(|unsigned| {
                    keys.get(&operation.author).is_some_and(|author| {
                        author
                            .signing
                            .verify(Label::Operation, &unsigned, &operation.signature)
                    })
                });
            if verifies {
                operations.insert(digest, operation);
            }
        }
        let rules: Vec<membership_rules::Operation> = operations
            .iter()
            .map(|(digest, operation)| membership_rules::Operation {
                digest: *digest,
                parents: operation.parents.clone(),
                author: operation.author,
                action: operation.action.rule(),
            })
            .collect();
        let resolution = membership_rules::resolve(genesis, &rules);
        Ok(Checked {
            operations,
            keys,
            resolution,
        })
    }

    /// The operations no other operation names as a parent, which a new operation names.
    fn heads(checked: &Checked) -> Vec<Digest> {
        let named: BTreeSet<Digest> = checked
            .operations
            .values()
            .flat_map(|operation| operation.parents.iter().copied())
            .collect();
        checked
            .operations
            .keys()
            .filter(|digest| !named.contains(*digest))
            .copied()
            .collect()
    }

    /// Writes an operation of this device into the log.
    fn write_operation(
        &mut self,
        checked: &Checked,
        now: u64,
        action: Action,
    ) -> Result<Digest, SyncError> {
        let account = if matches!(action, Action::Genesis(_)) {
            [0; 32]
        } else {
            self.account_id()?
        };
        let parents = Self::heads(checked);
        let author = self.device();
        let unsigned = Unsigned {
            account: &account,
            parents: &parents,
            author: &author,
            wall: now,
            action: &action,
        };
        let signature = self
            .identity
            .signing
            .sign(Label::Operation, &unsigned.bytes()?)?;
        let operation = Operation {
            account,
            parents,
            author,
            wall: now,
            action,
            signature,
        };
        self.keep_operation(operation.encode()?)
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
            &self.directory.join("genesis"),
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
        let account = self.account_id()?;
        let device = self.device();
        let mut learned = false;
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
                account: &account,
                epoch: rotation.epoch,
                device: &device,
            };
            let key = slot::unwrap(slot, &self.identity.kem, &binding)?;
            learned |= self.keep_key(rotation.epoch, key)?;
        }
        if learned {
            self.save_epochs()?;
        }
        Ok(())
    }

    /// The associated data that binds a sealed state to its account, epoch, and sender.
    fn state_binding(account: &Digest, epoch: u32, sender: &Digest) -> Vec<u8> {
        let mut writer = Writer::new();
        writer
            .raw(Label::State.text().as_bytes())
            .raw(account)
            .word(epoch)
            .raw(sender);
        writer.finish()
    }

    /// A bundle of the account's log and this device's state, sealed under the current epoch key and signed.
    ///
    /// The device rotates first when a member lacks the current key.
    ///
    /// # Errors
    ///
    /// Returns a [`SyncError`] when the device belongs to no account or holds no key of the current epoch.
    pub fn export(&mut self, now: u64, state: &[u8]) -> Result<Vec<u8>, SyncError> {
        self.rotate_if_due(now)?;
        let checked = self.check()?;
        let account = self.account_id()?;
        let device = self.device();
        let rotation = checked
            .resolution
            .rotation
            .and_then(|digest| checked.operations.get(&digest))
            .and_then(|operation| match &operation.action {
                Action::Rotate(rotation) => Some(rotation.commitment),
                Action::Genesis(_) | Action::Add(_) | Action::Remove(_) => None,
            })
            .ok_or_else(|| SyncError::Refused(String::from("the account has no epoch key yet")))?;
        let (epoch, key) = self.epochs.get(&rotation).ok_or_else(|| {
            SyncError::Refused(String::from(
                "this device holds no key of the current epoch; import a newer bundle first",
            ))
        })?;
        let sealed = seal::seal(key, &Self::state_binding(&account, *epoch, &device), state)?;
        let operations: Vec<Vec<u8>> = checked
            .operations
            .keys()
            .filter_map(|digest| self.log.get(digest).cloned())
            .collect();
        let unsigned = UnsignedBundle {
            account: &account,
            operations: &operations,
            sender: &device,
            epoch: *epoch,
            sealed: &sealed,
        };
        let signature = self
            .identity
            .signing
            .sign(Label::Bundle, &unsigned.bytes()?)?;
        let bundle = Bundle {
            account,
            operations,
            sender: device,
            epoch: *epoch,
            sealed,
            signature,
        };
        Ok(bundle.encode()?)
    }

    /// Reads a bundle: merges its log, checks the sender, and opens its state.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] for a bundle of another account, from a device that is no member, or under a key this device lacks, and another [`SyncError`] when it is malformed or forged.
    pub fn import(&mut self, bytes: &[u8]) -> Result<Imported, SyncError> {
        let bundle = Bundle::decode(bytes)?;
        let account = self.account_id()?;
        if bundle.account != account {
            return Err(SyncError::Refused(String::from(
                "the bundle belongs to another account",
            )));
        }
        for operation in &bundle.operations {
            if Operation::decode(operation).is_ok() {
                self.keep_operation(operation.clone())?;
            }
        }
        let checked = self.check()?;
        let sender = checked.keys.get(&bundle.sender).ok_or_else(|| {
            SyncError::Refused(String::from(
                "the bundle comes from a device this account never listed",
            ))
        })?;
        if !sender.signing.verify(
            Label::Bundle,
            &bundle.unsigned().bytes()?,
            &bundle.signature,
        ) {
            return Err(SyncError::Refused(String::from(
                "the bundle's signature does not verify",
            )));
        }
        if !checked.resolution.members.contains(&bundle.sender) {
            return Err(SyncError::Refused(String::from(
                "the bundle's sender is no member of the account",
            )));
        }
        self.learn_keys(&checked)?;
        let (_, key) = self.epochs.get(&bundle.sealed.commitment).ok_or_else(|| {
            SyncError::Refused(String::from(
                "this device holds no key of the bundle's epoch",
            ))
        })?;
        let state = seal::open(
            key,
            &Self::state_binding(&account, bundle.epoch, &bundle.sender),
            &bundle.sealed,
        )?;
        Ok(Imported {
            sender: bundle.sender,
            state,
        })
    }

    fn invitation_path(&self, number: u16) -> PathBuf {
        self.directory.join("invitations").join(number.to_string())
    }

    fn invitation(&self, number: u16) -> Result<Option<Invitation>, SyncError> {
        let Some(bytes) = sealed::read(&self.invitation_path(number), &self.sealing, INVITATION)?
        else {
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
        sealed::write(
            &self.invitation_path(number),
            &self.sealing,
            INVITATION,
            &Zeroizing::new(writer.finish()),
        )
    }

    /// A new invitation code, which a person types on the joining device.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when this device is no member of an account.
    pub fn invite(&self) -> Result<Code, SyncError> {
        if !self.status()?.member {
            return Err(SyncError::Refused(String::from(
                "only a member invites a device",
            )));
        }
        let code = Code::generate()?;
        self.save_invitation(code.number, &Invitation::Waiting(code))?;
        Ok(code)
    }

    fn joining(&self) -> Result<Option<Joining>, SyncError> {
        let Some(bytes) = sealed::read(&self.directory.join("joining"), &self.sealing, JOINING)?
        else {
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
        sealed::write(
            &self.directory.join("joining"),
            &self.sealing,
            JOINING,
            &Zeroizing::new(writer.finish()),
        )
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

    /// The entry for a joiner that confirmed the channel key: this device adds it to the account, then seals the account's log, its epoch keys, and `state` for it alone.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when no welcomed invitation waits for this confirmation, the confirmation fails, or this device is no member.
    pub fn admit(&mut self, now: u64, bytes: &[u8], state: &[u8]) -> Result<Vec<u8>, SyncError> {
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
            state: state.to_vec(),
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

    /// Installs an entry: the account's log, which lists this device, its epoch keys, and the inviter's state, which it hands back.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Refused`] when this device confirmed no welcome of this invitation, and another [`SyncError`] when the entry does not authenticate.
    pub fn finish(&mut self, bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>, SyncError> {
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
        let genesis = admission
            .operations
            .iter()
            .find(|operation| account_of(operation) == admission.account)
            .map(|operation| operation_digest(operation))
            .ok_or_else(|| {
                SyncError::Refused(String::from("the entry lacks the account's genesis"))
            })?;
        for operation in admission.operations {
            self.keep_operation(operation)?;
        }
        fs::replace(
            &self.directory.join("genesis"),
            format!("{}\n", hex(&genesis)).as_bytes(),
        )?;
        self.genesis = Some(genesis);
        for (epoch, key) in admission.epochs {
            self.keep_key(epoch, Zeroizing::new(key))?;
        }
        self.save_epochs()?;
        fs::remove_if_present(&self.directory.join("joining"))?;
        Ok(Zeroizing::new(admission.state))
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
}

#[cfg(test)]
mod tests {
    use super::LONGEST_OPERATION;

    #[test]
    fn the_log_reads_every_operation_the_wire_admits() {
        assert_eq!(
            usize::try_from(LONGEST_OPERATION).unwrap(),
            youarel_protocol::message::LONGEST_OPERATION
        );
    }
}
