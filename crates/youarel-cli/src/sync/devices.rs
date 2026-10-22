//! Structured device facts and actions shared by interactive clients.

use super::{Account, account};
use anyhow::{Context as _, Result, ensure};
use std::collections::BTreeMap;
use youarel_core::{device_rules::MemberId, state::Edit};
use youarel_store::{Session, Store, StoreError};
use youarel_sync::account::Member;

#[derive(Clone, Copy, Debug)]
pub(super) struct DeviceStore<'store>(&'store Store);

impl<'store> DeviceStore<'store> {
    pub(super) fn open(store: &'store Store, now: u64) -> Result<Self> {
        Self::read(store, now, |_| Ok(())).map(|(checked, ())| checked)
    }

    fn read<T>(
        store: &'store Store,
        now: u64,
        read: impl FnOnce(&mut Session) -> Result<T, StoreError>,
    ) -> Result<(Self, T)> {
        let facts = store.session(now, read)?;
        Ok((Self(store), facts))
    }

    pub(super) fn account(self) -> Result<Account> {
        account(self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Facts {
    pub own: [u8; 32],
    pub joined: bool,
    pub member: bool,
    pub members: Vec<Member>,
    pub names: BTreeMap<MemberId, String>,
}

#[must_use]
pub fn identity_text(id: MemberId) -> String {
    youarel_sync::hex(&id.0)
}

/// # Errors
///
/// Refuses text that does not encode a full membership identifier.
pub fn identity(text: &str) -> Result<MemberId> {
    youarel_sync::unhex(text)
        .map(MemberId)
        .context("The device identifier is invalid.")
}

/// # Errors
///
/// Reports unreadable membership or sealed name records.
pub fn inspect(store: &Store, now: u64) -> Result<Facts> {
    let (checked, names) = DeviceStore::read(store, now, |session| session.device_names())?;
    let account = checked.account()?;
    let own = account.device();
    let names = names
        .into_iter()
        .map(|(id, name)| (id, name.value))
        .collect();
    let joined = account.has_account();
    let member = joined && account.status()?.member;
    let mut members = if joined {
        account.members()?
    } else {
        Vec::new()
    };
    if !members.iter().any(|held| held.device == own) {
        members.push(Member {
            device: own,
            added: 0,
            recovery: false,
        });
    }
    Ok(Facts {
        own,
        joined,
        member,
        members,
        names,
    })
}

/// # Errors
///
/// Refuses an unknown device, an invalid name, or an unreadable store.
pub fn name(store: &Store, now: u64, (id, name): (MemberId, &str)) -> Result<()> {
    let account = DeviceStore::open(store, now)?.account()?;
    ensure!(
        id.0 == account.device() || account.status()?.members.contains(&id.0),
        "Choose a current device."
    );
    store.session(now, |session| {
        session.edit(Edit::NameDevice {
            id,
            name: name.trim().to_owned(),
        })
    })?;
    Ok(())
}

/// # Errors
///
/// Refuses the current device, absent membership, or an unreadable identity.
pub fn remove(store: &Store, now: u64, id: MemberId) -> Result<()> {
    let _publishing = store.publishing()?;
    let mut account = DeviceStore::open(store, now)?.account()?;
    ensure!(
        id.0 != account.device(),
        "Remove this device from another device."
    );
    Ok(account.remove(now, id.0)?)
}

/// # Errors
///
/// Refuses a nonmember or failure to enroll the recovery identity.
pub fn recovery_key(store: &Store, now: u64) -> Result<String> {
    let _publishing = store.publishing()?;
    let key = DeviceStore::open(store, now)?
        .account()?
        .enroll_recovery(now)?;
    Ok(key.text().as_str().to_owned())
}

/// # Errors
///
/// Refuses an invalid recovery key, existing membership, or an unavailable bootstrap source.
pub fn recover(store: &Store, now: u64, (key, target): (&str, &str)) -> Result<()> {
    let checked = DeviceStore::open(store, now)?;
    let source = if super::is_relay(target) {
        super::Source::Relay(target)
    } else {
        super::Source::Folder(std::path::Path::new(target))
    };
    super::recover(checked.0, now, key, source)?;
    Ok(())
}

#[cfg(test)]
mod tests;
