use super::super::{LONGEST_MESSAGE, endpoint, is_relay, longest_answer, reason};
use crate::web;
use anyhow::{Context as _, Result, bail, ensure};
use std::path::{Path, PathBuf};
use youarel_core::{
    device_rules::{PAIRING_PATIENCE, Role},
    wire_rules::{Reader, Writer},
};
use youarel_crypto::pairing::Code;
use youarel_protocol::relay::{Mail, Side, read_nameplate};
use youarel_relay::Status;
use youarel_store::fs;
use youarel_sync::hex;

const DIRECTORY: &str = "pairing";
const OFFER: &str = "offer";
const OFFER_BYTES: u64 = 40;
const CLAIM_ATTEMPTS: usize = 16;
const NONCE_BYTES: usize = 32;

#[derive(Clone, Copy, Debug)]
pub(super) enum Packet {
    Join,
    Welcome,
    Confirmation,
    Entry,
}

impl Packet {
    const fn name(self) -> &'static str {
        match self {
            Self::Join => "join",
            Self::Welcome => "welcome",
            Self::Confirmation => "confirmation",
            Self::Entry => "entry",
        }
    }
    const fn side(self) -> Side {
        match self {
            Self::Join | Self::Confirmation => Side::Inviter,
            Self::Welcome | Self::Entry => Side::Joiner,
        }
    }
}

#[derive(Debug)]
pub(super) struct Folder {
    slot: PathBuf,
    offer: Option<Vec<u8>>,
    nonce: Option<[u8; NONCE_BYTES]>,
}

impl Folder {
    fn invite(root: &Path, now: u64) -> Result<(Self, Code)> {
        for _ in 0..CLAIM_ATTEMPTS {
            let code = Code::generate()?;
            let nonce = youarel_crypto::random::<NONCE_BYTES>()?;
            let slot = root.join(DIRECTORY).join(code.number.to_string());
            fs::create_dir_all(&slot)?;
            let mut writer = Writer::new();
            writer.long(now).raw(&nonce);
            let offer = writer.finish();
            match fs::write_new(&slot.join(OFFER), &offer) {
                Ok(()) => {
                    return Ok((
                        Self {
                            slot,
                            offer: Some(offer),
                            nonce: Some(nonce),
                        },
                        code,
                    ));
                }
                Err(error) if error.source.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        bail!("The shared folder has no free invitation number. Try again.")
    }

    fn ready(&mut self, now: u64) -> Result<bool> {
        if self.nonce.is_some() {
            return Ok(true);
        }
        let Some(bytes) = fs::read_bounded_if_present(&self.slot.join(OFFER), OFFER_BYTES)? else {
            return Ok(false);
        };
        if u64::try_from(bytes.len())? < OFFER_BYTES {
            return Ok(false);
        }
        let mut reader = Reader::new(&bytes);
        let opened = reader.long()?;
        let nonce = reader.array()?;
        reader.finish()?;
        ensure!(
            now.saturating_sub(opened) < PAIRING_PATIENCE,
            "This invitation expired. Ask for a new code."
        );
        self.nonce = Some(nonce);
        self.offer = Some(bytes);
        Ok(true)
    }

    fn directory(&self) -> Result<PathBuf> {
        Ok(self.slot.join(hex(&self
            .nonce
            .context("The invitation has not reached this folder yet.")?)))
    }

    fn cleanup(&self, role: Role, joined: bool) -> Result<()> {
        let Some(_nonce) = self.nonce else {
            return Ok(());
        };
        let directory = self.directory()?;
        for packet in [
            Packet::Join,
            Packet::Welcome,
            Packet::Confirmation,
            Packet::Entry,
        ] {
            let remove = match role {
                Role::Invite => !matches!(packet, Packet::Entry) || !joined,
                Role::Join => joined || matches!(packet, Packet::Join | Packet::Confirmation),
            };
            if remove {
                fs::remove_if_present(&directory.join(packet.name()))?;
            }
        }
        if matches!(role, Role::Invite) || joined {
            let current = fs::read_bounded_if_present(&self.slot.join(OFFER), OFFER_BYTES)?;
            if current == self.offer {
                fs::remove_if_present(&self.slot.join(OFFER))?;
            }
        }
        fs::remove_empty_dir(&directory)?;
        fs::remove_empty_dir(&self.slot)?;
        Ok(())
    }
}

#[derive(Debug)]
pub(super) enum Transport {
    Folder(Folder),
    Relay { endpoint: String, number: u16 },
}

impl Transport {
    pub(super) fn invite(target: &str, now: u64) -> Result<(Self, Code)> {
        ensure!(
            !target.trim().is_empty(),
            "Choose a shared folder or relay first."
        );
        if is_relay(target) {
            let endpoint = endpoint(target, youarel_relay::MAIL)?;
            let (status, bytes) = web::post(&endpoint, &Mail::Open.encode()?, longest_answer()?)?;
            ensure!(
                status == Status::Ok.code(),
                "The relay refused the invitation: {}",
                reason(&bytes)
            );
            let number = read_nameplate(&bytes)?;
            Ok((Self::Relay { endpoint, number }, Code::for_number(number)?))
        } else {
            let (folder, code) = Folder::invite(Path::new(target), now)?;
            Ok((Self::Folder(folder), code))
        }
    }

    pub(super) fn join(target: &str, number: u16) -> Result<Self> {
        ensure!(
            !target.trim().is_empty(),
            "Choose a shared folder or relay first."
        );
        Ok(if is_relay(target) {
            Self::Relay {
                endpoint: endpoint(target, youarel_relay::MAIL)?,
                number,
            }
        } else {
            Self::Folder(Folder {
                slot: Path::new(target).join(DIRECTORY).join(number.to_string()),
                nonce: None,
                offer: None,
            })
        })
    }

    pub(super) fn ready(&mut self, now: u64) -> Result<bool> {
        match self {
            Self::Folder(folder) => folder.ready(now),
            Self::Relay { .. } => Ok(true),
        }
    }

    pub(super) fn send(&self, packet: Packet, message: Vec<u8>) -> Result<()> {
        match self {
            Self::Folder(folder) => {
                let directory = folder.directory()?;
                fs::create_dir_all(&directory)?;
                fs::replace(&directory.join(packet.name()), &message)?;
            }
            Self::Relay { endpoint, number } => {
                let bytes = Mail::Drop {
                    nameplate: *number,
                    to: packet.side(),
                    message,
                }
                .encode()?;
                let (status, body) = web::post(endpoint, &bytes, longest_answer()?)?;
                ensure!(
                    status == Status::Ok.code(),
                    "The relay refused the message: {}",
                    reason(&body)
                );
            }
        }
        Ok(())
    }

    pub(super) fn take(&self, packet: Packet) -> Result<Option<Vec<u8>>> {
        match self {
            Self::Folder(folder) => {
                let path = folder.directory()?.join(packet.name());
                let message = fs::read_bounded_if_present(&path, LONGEST_MESSAGE)?;
                if message.is_some() {
                    fs::remove_if_present(&path)?;
                }
                Ok(message)
            }
            Self::Relay { endpoint, number } => {
                let bytes = Mail::Poll {
                    nameplate: *number,
                    to: packet.side(),
                }
                .encode()?;
                let (status, body) = web::post(endpoint, &bytes, longest_answer()?)?;
                if status == Status::NoContent.code() {
                    return Ok(None);
                }
                ensure!(
                    status == Status::Ok.code(),
                    "The invitation is unavailable: {}",
                    reason(&body)
                );
                Ok(Some(body))
            }
        }
    }

    pub(super) fn cleanup(&self, role: Role, joined: bool) -> Result<()> {
        match self {
            Self::Folder(folder) => folder.cleanup(role, joined),
            Self::Relay { .. } => Ok(()),
        }
    }
}
