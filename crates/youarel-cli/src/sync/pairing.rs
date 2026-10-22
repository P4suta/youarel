//! Bounded pairing steps for the window and other interactive clients.

mod control;
pub use control::Control;
mod transport;

use super::{account, devices::DeviceStore, snapshot_hint};
use anyhow::{Context as _, Result, ensure};
use transport::{Packet, Transport};
use youarel_core::device_rules::{Event, Role, Stage};
use youarel_crypto::pairing::Code;
use youarel_store::Store;
use youarel_sync::account::Account;

fn local<T>(
    (store, control): (&Store, &Control),
    transition: (Stage, Event),
    now: u64,
    work: impl FnOnce(&mut Account) -> Result<T>,
) -> Result<Option<T>> {
    let _publishing = store.publishing()?;
    let checked = DeviceStore::open(store, now)?;
    control.local(transition, now, || work(&mut checked.account()?))
}

#[derive(Debug)]
pub struct Pairing {
    store: Store,
    target: String,
    transport: Transport,
    role: Role,
    control: Control,
    number: u16,
    pending_join: Option<Vec<u8>>,
    pending_entry: Option<Vec<u8>>,
    joined: bool,
}

impl Pairing {
    /// # Errors
    ///
    /// Refuses an unusable target, a removed device, or a failed invitation.
    pub fn invite(
        store: Store,
        target: String,
        control: Control,
        now: u64,
    ) -> Result<(Self, String)> {
        DeviceStore::open(&store, now)?;
        let (transport, code) = Transport::invite(&target, now)?;
        let saved = local(
            (&store, &control),
            (Stage::Offered, Event::Ready),
            now,
            |account| {
                if !account.has_account() {
                    account.found(now)?;
                }
                account.invite_as(code)?;
                Ok(())
            },
        );
        match saved {
            Ok(Some(())) => {}
            Ok(None) => {
                transport.cleanup(Role::Invite, false)?;
                anyhow::bail!("The invitation stopped.");
            }
            Err(error) => {
                transport.cleanup(Role::Invite, false)?;
                return Err(error);
            }
        }
        Ok((
            Self {
                store,
                target,
                transport,
                role: Role::Invite,
                control,
                number: code.number,
                pending_join: None,
                pending_entry: None,
                joined: false,
            },
            code.text(),
        ))
    }

    /// # Errors
    ///
    /// Refuses a malformed code, an existing membership, or an unusable target.
    pub fn join(
        store: Store,
        (target, text): (String, &str),
        control: Control,
        now: u64,
    ) -> Result<Self> {
        let code = Code::parse(text).context("Check the invitation code and try again.")?;
        DeviceStore::open(&store, now)?;
        let transport = Transport::join(&target, code.number)?;
        let pending_join = local(
            (&store, &control),
            (Stage::Waiting, Event::Poll),
            now,
            |account| {
                ensure!(
                    !account.has_account(),
                    "This device has already joined. Use a fresh device to join."
                );
                Ok(account.join(code)?)
            },
        )?;
        Ok(Self {
            store,
            target,
            transport,
            role: Role::Join,
            control,
            number: code.number,
            pending_join,
            pending_entry: None,
            joined: false,
        })
    }

    /// # Errors
    ///
    /// Reports one refused protocol step or sync without holding a window's library lock.
    pub fn advance(&mut self, now: u64) -> Result<Stage> {
        self.control.event(Event::Poll, now);
        let stage = self.control.stage();
        if stage.terminal() {
            return Ok(stage);
        }
        DeviceStore::open(&self.store, now)?;
        if !self.transport.ready(now)? {
            return Ok(stage);
        }
        if let Some(join) = self.pending_join.as_ref()
            && !self.control.stage().terminal()
        {
            self.transport.send(Packet::Join, join.clone())?;
            self.pending_join = None;
        }
        match (self.role, stage) {
            (Role::Invite, Stage::Waiting) => self.welcome(now)?,
            (Role::Join, Stage::Waiting) => self.confirm(now)?,
            (Role::Invite, Stage::Confirming) => self.admit(now)?,
            (Role::Join, Stage::Confirming) => self.finish(now)?,
            (_, Stage::Syncing) => self.sync(now)?,
            _ => {}
        }
        Ok(self.control.stage())
    }

    fn welcome(&self, now: u64) -> Result<()> {
        let Some(join) = self.transport.take(Packet::Join)? else {
            return Ok(());
        };
        let welcome = local(
            (&self.store, &self.control),
            (Stage::Waiting, Event::Peer),
            now,
            |account| Ok(account.accept(&join)?),
        )?;
        if let Some(welcome) = welcome {
            self.transport.send(Packet::Welcome, welcome)?;
        }
        Ok(())
    }

    fn confirm(&self, now: u64) -> Result<()> {
        let Some(welcome) = self.transport.take(Packet::Welcome)? else {
            return Ok(());
        };
        let confirmation = local(
            (&self.store, &self.control),
            (Stage::Waiting, Event::Peer),
            now,
            |account| Ok(account.confirm(&welcome)?),
        )?;
        if let Some(confirmation) = confirmation {
            self.transport.send(Packet::Confirmation, confirmation)?;
        }
        Ok(())
    }

    fn admit(&mut self, now: u64) -> Result<()> {
        let Some(confirmation) = self.transport.take(Packet::Confirmation)? else {
            return Ok(());
        };
        let hint = snapshot_hint(&self.store, now)?;
        let entry = local(
            (&self.store, &self.control),
            (Stage::Confirming, Event::Joined),
            now,
            |account| Ok(account.admit(now, &confirmation, hint)?),
        )?;
        if let Some(entry) = entry {
            self.joined = true;
            self.pending_entry = Some(entry);
        }
        Ok(())
    }

    fn finish(&mut self, now: u64) -> Result<()> {
        let Some(entry) = self.transport.take(Packet::Entry)? else {
            return Ok(());
        };
        let finished = local(
            (&self.store, &self.control),
            (Stage::Confirming, Event::Joined),
            now,
            |account| Ok(account.finish(&entry)?),
        )?;
        self.joined = finished.is_some();
        Ok(())
    }

    fn sync(&mut self, now: u64) -> Result<()> {
        if let Some(entry) = self.pending_entry.as_ref() {
            if super::is_relay(&self.target) {
                super::publish_to(&self.store, now, &self.target)?;
            }
            self.transport.send(Packet::Entry, entry.clone())?;
            self.pending_entry = None;
        }
        let report = if super::is_relay(&self.target) {
            super::segments::relay::sync(&self.store, now, &self.target)?
        } else {
            super::folder_report(&self.store, now, std::path::Path::new(&self.target))?
        };
        ensure!(
            report.problems.is_empty(),
            "Device joined; sync needs attention: {}",
            report.problems.join("\n")
        );
        if !report.more {
            self.control.event(Event::Synced, now);
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Reports a failed cleanup without removing membership or keys from a completed join.
    pub fn cleanup(&self) -> Result<()> {
        let _publishing = self.store.publishing()?;
        match self.role {
            Role::Invite => account(&self.store)?.cancel_invitation(self.number)?,
            Role::Join => account(&self.store)?.cancel_join()?,
        }
        self.transport.cleanup(self.role, self.joined)
    }
}

#[cfg(test)]
mod tests;
