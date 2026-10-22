//! The sync commands: the account, the pairing of a device through files, bundles, a shared folder, a relay, the removal of a device, and recovery.
//!
//! Each command opens the account beside the store and reads or merges the store's state in one locked session.

use crate::{clock, web};
use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;
use youarel_core::clock_rules::Stamp;
use youarel_core::folder_rules::{self, Seen};
use youarel_core::relay_rules;
use youarel_core::sync_rules::{self, Carried};
use youarel_crypto::pairing::Code;
use youarel_crypto::recovery::RecoveryKey;
use youarel_protocol::relay::{
    self, Collected, Locate, Mail, Side, Verb, read_located, read_nameplate,
};
use youarel_relay::Status;
use youarel_store::{Store, StoreError, fs, records};
use youarel_sync::account::{Account, founded, logged};
use youarel_sync::keystore::KeyStore;
use youarel_sync::{SyncError, hex};

pub use youarel_core::folder_rules::BUNDLE_SUFFIX;
/// The directory beside the store that holds the account's files.
pub const ACCOUNT_DIRECTORY: &str = "account";
/// The longest bundle, join, or welcome file a command reads.
const LONGEST_MESSAGE: u64 = 64 * 1024 * 1024;
/// The characters of a device identifier a command shows.
const SHORT_ID: usize = 16;

fn sync_error(error: SyncError) -> anyhow::Error {
    anyhow!(error)
}

/// The account beside the store.
fn account(store: &Store) -> Result<Account> {
    let directory = store.directory().join(ACCOUNT_DIRECTORY);
    let keys = KeyStore::for_directory(&directory).map_err(sync_error)?;
    Account::open(&directory, &keys).map_err(sync_error)
}

/// The store's state as its records.
fn state(store: &Store, now: u64) -> Result<Vec<u8>> {
    Ok(own_state(store, now)?.0)
}

/// The store's state as its records, with the latest stamp of this device's own writes in it, which a publish of the records carries.
fn own_state(store: &Store, now: u64) -> Result<(Vec<u8>, Option<Stamp>)> {
    store
        .session(now, |session| {
            let state = records::encode(session.state()).map_err(StoreError::Records)?;
            Ok((state, session.state().latest_of(session.device())))
        })
        .map_err(|error| anyhow!(error))
}

/// What a publish would carry now: the latest stamp of this device's own writes, and how many operations the account's log holds, read without the device's keys.
fn held(store: &Store, now: u64) -> Result<Carried> {
    let latest = store
        .session(now, |session| {
            Ok(session.state().latest_of(session.device()))
        })
        .map_err(|error| anyhow!(error))?;
    let operations = logged(&store.directory().join(ACCOUNT_DIRECTORY)).map_err(sync_error)?;
    Ok(Carried { latest, operations })
}

/// Whether this device has something new to say at `target`, where `missing` says the target lacks a bundle of it to keep.
fn pending_at(store: &Store, now: u64, target: &str, missing: bool) -> Result<bool> {
    let held = held(store, now)?;
    let published = store.published(target).map_err(|error| anyhow!(error))?;
    Ok(sync_rules::pending(held, published, missing))
}

/// Whether this device holds a write of its own or an operation of its account that no publish to `target` carried, read without the device's keys.
///
/// The window asks at its start, so a restart publishes at once what an earlier run left unpublished.
///
/// # Errors
///
/// Returns an error when the store's state, the record of its last publish, or the account's log cannot be read.
pub fn unpublished(store: &Store, now: u64, target: &str) -> Result<bool> {
    pending_at(store, now, target, false)
}

/// Seals this device's state and log, hands the bundle to `deliver`, and records what it carried for `target`, while no other process publishes for this device.
///
/// The record follows only a delivered bundle, so a publish that fails leaves its writes unpublished.
fn publish(
    store: &Store,
    now: u64,
    target: &str,
    deliver: impl FnOnce(&Account, Vec<u8>) -> Result<()>,
) -> Result<()> {
    let _publishing = store.publishing().map_err(|error| anyhow!(error))?;
    let (state, latest) = own_state(store, now)?;
    let mut account = account(store)?;
    let bundle = account.export(now, &state).map_err(sync_error)?;
    let carried = Carried {
        latest,
        operations: account.operations(),
    };
    deliver(&account, bundle)?;
    store
        .record_published(target, carried)
        .map_err(|error| anyhow!(error))
}

/// Merges a received state into the store, as its records or in the JSON form of a build from before the records, whose icons stay behind.
fn merge_state(store: &Store, now: u64, state: &[u8]) -> Result<()> {
    let received = records::decode_received(state)
        .map_err(|error| anyhow!("the received state is malformed: {error}"))?;
    store
        .session(now, |session| {
            session.merge(&received);
            Ok(())
        })
        .map_err(|error| anyhow!(error))
}

fn read(path: &Path) -> Result<Vec<u8>> {
    Ok(fs::read_bounded(path, LONGEST_MESSAGE)?)
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    Ok(fs::replace(path, bytes)?)
}

fn short(device: &[u8; 32]) -> String {
    hex(device).chars().take(SHORT_ID).collect()
}

/// Founds an account with this device as its only member.
///
/// # Errors
///
/// Returns an error when the device belongs to an account already.
pub fn create(store: &Store, now: u64) -> Result<Vec<String>> {
    let id = account(store)?.found(now).map_err(sync_error)?;
    Ok(vec![format!("created account {}", hex(&id))])
}

/// The account, its members, and its epoch.
///
/// # Errors
///
/// Returns an error when the device belongs to no account.
pub fn show(store: &Store) -> Result<Vec<String>> {
    let account = account(store)?;
    if !account.has_account() {
        return Ok(vec![format!(
            "this device {} belongs to no account; create one or join one",
            short(&account.device())
        )]);
    }
    let status = account.status().map_err(sync_error)?;
    let recoveries = account.recovery_members().map_err(sync_error)?;
    let mut lines = vec![
        format!("account {}", hex(&status.account)),
        format!("epoch {}", status.epoch),
    ];
    for member in &status.members {
        let marker = if *member == status.this_device {
            "  (this device)"
        } else if recoveries.contains(member) {
            "  (recovery key)"
        } else {
            ""
        };
        lines.push(format!("member {}{marker}", hex(member)));
    }
    if !status.member {
        lines.push(String::from(
            "this device is no member any more; a member removed it, so pair it again to rejoin",
        ));
    }
    Ok(lines)
}

/// A new invitation code to type on the joining device.
///
/// # Errors
///
/// Returns an error when this device is no member.
pub fn invite(store: &Store) -> Result<Vec<String>> {
    let code = account(store)?.invite().map_err(sync_error)?;
    Ok(vec![
        format!("invitation code {}", code.text()),
        String::from("type it on the new device with `youarel pair join CODE --out FILE`"),
    ])
}

/// Writes the join file for a code.
///
/// # Errors
///
/// Returns an error when the code is malformed or the device belongs to an account.
pub fn join(store: &Store, code: &str, out: &Path) -> Result<Vec<String>> {
    let code = Code::parse(code).with_context(|| format!("`{code}` is not an invitation code"))?;
    let bytes = account(store)?.join(code).map_err(sync_error)?;
    write(out, &bytes)?;
    Ok(vec![format!(
        "wrote {}; carry it to the inviting device and run `youarel pair accept`",
        out.display()
    )])
}

/// Writes the welcome file for a join file.
///
/// # Errors
///
/// Returns an error when no invitation waits for the join.
pub fn accept(store: &Store, join: &Path, out: &Path) -> Result<Vec<String>> {
    let bytes = account(store)?.accept(&read(join)?).map_err(sync_error)?;
    write(out, &bytes)?;
    Ok(vec![format!(
        "wrote {}; carry it to the new device and run `youarel pair confirm`",
        out.display()
    )])
}

/// Writes the confirmation file for a welcome file.
///
/// # Errors
///
/// Returns an error when the welcome does not authenticate.
pub fn confirm(store: &Store, welcome: &Path, out: &Path) -> Result<Vec<String>> {
    let bytes = account(store)?
        .confirm(&read(welcome)?)
        .map_err(sync_error)?;
    write(out, &bytes)?;
    Ok(vec![format!(
        "wrote {}; carry it to the inviting device and run `youarel pair admit`",
        out.display()
    )])
}

/// Adds the confirmed device and writes the entry file, which carries this device's state.
///
/// # Errors
///
/// Returns an error when the confirmation does not verify.
pub fn admit(store: &Store, now: u64, confirmation: &Path, out: &Path) -> Result<Vec<String>> {
    let state = state(store, now)?;
    let bytes = account(store)?
        .admit(now, &read(confirmation)?, &state)
        .map_err(sync_error)?;
    write(out, &bytes)?;
    Ok(vec![format!(
        "wrote {}; carry it to the new device and run `youarel pair finish`",
        out.display()
    )])
}

/// Installs an entry file and merges the inviter's state.
///
/// # Errors
///
/// Returns an error when the entry does not authenticate.
pub fn finish(store: &Store, now: u64, entry: &Path) -> Result<Vec<String>> {
    let state = account(store)?.finish(&read(entry)?).map_err(sync_error)?;
    merge_state(store, now, &state)?;
    Ok(vec![String::from("joined the account")])
}

/// Writes a bundle of this device's state.
///
/// # Errors
///
/// Returns an error when the device belongs to no account or lacks the current key.
pub fn export(store: &Store, now: u64, out: &Path) -> Result<Vec<String>> {
    let state = state(store, now)?;
    let bytes = account(store)?.export(now, &state).map_err(sync_error)?;
    write(out, &bytes)?;
    Ok(vec![format!("wrote {}", out.display())])
}

/// Reads a bundle and merges its state.
///
/// # Errors
///
/// Returns an error when the bundle is refused.
pub fn import(store: &Store, now: u64, file: &Path) -> Result<Vec<String>> {
    let imported = account(store)?.import(&read(file)?).map_err(sync_error)?;
    merge_state(store, now, &imported.state)?;
    Ok(vec![format!(
        "merged the state of {}",
        short(&imported.sender)
    )])
}

/// The name of this device's own bundle in a shared folder.
///
/// # Errors
///
/// Returns an error when the account cannot be opened.
pub fn own_bundle(store: &Store) -> Result<String> {
    Ok(format!("{}{BUNDLE_SUFFIX}", hex(&account(store)?.device())))
}

/// What a merge read: a line for each bundle, and whether the target lacks a bundle of this device to keep.
type Read = (Vec<String>, bool);

/// Merges every other device's bundle in a shared folder, which `own` names apart from this device's, and says whether the folder lacks this device's own.
///
/// A bundle that cannot be read or opened is reported and skipped, since a cloud drive may still be writing it or hold it locked, so one such file stops nothing.
fn merge_folder(store: &Store, now: u64, directory: &Path, own: &str) -> Result<Read> {
    let names = fs::list(directory)?;
    let missing = !names.iter().any(|name| name == own);
    let mut lines = Vec::new();
    for name in names {
        if !folder_rules::counts(Seen::Touched(&name), Some(own)) {
            continue;
        }
        let bytes = match read(&directory.join(&name)) {
            Ok(bytes) => bytes,
            Err(error) => {
                lines.push(format!("skipped {name}: {error:#}"));
                continue;
            }
        };
        match account(store)?.import(&bytes) {
            Ok(imported) => {
                merge_state(store, now, &imported.state)?;
                lines.push(format!("merged {name}"));
            }
            Err(error) => lines.push(format!("skipped {name}: {error}")),
        }
    }
    Ok((lines, missing))
}

/// Syncs through a shared folder: reads every other device's bundle there, then writes this device's own and records what it carried.
///
/// A bundle that fails is reported and skipped, so one broken file stops nothing.
///
/// # Errors
///
/// Returns an error when the folder cannot be listed or this device's bundle cannot be written.
pub fn folder(store: &Store, now: u64, directory: &Path) -> Result<Vec<String>> {
    let own = own_bundle(store)?;
    let (mut lines, _) = merge_folder(store, now, directory, &own)?;
    let target = directory.to_string_lossy();
    let written = publish(store, now, &target, |_, bundle| {
        write(&directory.join(&own), &bundle)
    });
    if let Err(error) = written {
        lines.push(format!("wrote no bundle: {error:#}"));
        bail!("{}", lines.join("\n"));
    }
    lines.push(format!("wrote {own}"));
    Ok(lines)
}

/// What a merge read, and whether this device has something new to say at the target, which a publish there then carries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Merged {
    /// A line for each bundle the merge read or skipped.
    pub lines: Vec<String>,
    /// Whether this device holds a write or an operation that no publish to the target carried, or the target lacks a bundle of it to keep.
    pub pending: bool,
}

/// Merges what the other devices left at `target`, a relay's address or a shared folder, and writes nothing there.
///
/// The window merges so on its interval and when its watch notices another device's bundle, since every bundle it wrote in answer would bring the other devices to answer in turn.
/// The merge then reads from the store whether this device has something new to say there, which the window publishes.
///
/// # Errors
///
/// Returns an error when the relay or the folder cannot be reached, or the device belongs to no account.
pub fn merge(store: &Store, now: u64, target: &str) -> Result<Merged> {
    let (lines, missing) = if is_relay(target) {
        collect(store, now, target)?
    } else {
        merge_folder(store, now, Path::new(target), &own_bundle(store)?)?
    };
    Ok(Merged {
        lines,
        pending: pending_at(store, now, target, missing)?,
    })
}

/// Removes the member whose identifier starts with `prefix`.
///
/// # Errors
///
/// Returns an error when the prefix names no member or more than one.
pub fn remove(store: &Store, now: u64, prefix: &str) -> Result<Vec<String>> {
    let mut account = account(store)?;
    let matching = account.members_matching(prefix).map_err(sync_error)?;
    let [device] = matching.as_slice() else {
        bail!(
            "`{prefix}` names {} members; give more of the identifier",
            matching.len()
        );
    };
    account.remove(now, *device).map_err(sync_error)?;
    Ok(vec![format!(
        "removed {}; the next bundle carries a key it never sees",
        hex(device)
    )])
}

/// Syncs through a relay when `target` is an address, and through a shared folder otherwise.
///
/// # Errors
///
/// Returns an error when the relay or the folder cannot be reached, or the device belongs to no account.
pub fn sync(store: &Store, now: u64, target: &str) -> Result<Vec<String>> {
    if is_relay(target) {
        relay(store, now, target)
    } else {
        folder(store, now, Path::new(target))
    }
}

/// Whether this device belongs to an account, read without creating anything.
///
/// # Errors
///
/// Returns an error when the account's files exist but cannot be read.
pub fn has_account(store: &Store) -> Result<bool> {
    founded(&store.directory().join(ACCOUNT_DIRECTORY)).map_err(sync_error)
}

/// Whether `target` names a relay rather than a folder.
#[must_use]
pub fn is_relay(target: &str) -> bool {
    ["https://", "http://"]
        .iter()
        .any(|scheme| target.starts_with(scheme))
}

/// The address of `path` at the relay, which takes HTTPS, or plain HTTP on this host alone.
fn endpoint(relay: &str, path: &str) -> Result<String> {
    let parsed = url::Url::parse(relay).with_context(|| format!("`{relay}` is no address"))?;
    let local = match parsed.host() {
        Some(url::Host::Domain(name)) => name == "localhost",
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    };
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && local) {
        bail!("a relay takes https, or plain http only on this host");
    }
    Ok(format!("{}{path}", relay.trim_end_matches('/')))
}

/// The most bytes a relay's answer holds: every member's bundle, and room for their frames.
fn longest_answer() -> Result<u64> {
    let frames = relay_rules::MOST_SLOTS.saturating_mul(relay::LONGEST_FRAME);
    Ok(u64::try_from(
        relay_rules::MOST_BYTES.saturating_add(frames),
    )?)
}

/// The reason in a relay's refusal.
fn reason(body: &[u8]) -> String {
    String::from_utf8_lossy(body).into_owned()
}

/// Merges every other member's newest bundle at a relay, reporting and skipping a bundle that fails, as a shared folder's are, and says whether the relay lacks a bundle of this device to keep.
///
/// The relay stamps a slot only when its member publishes, so this device's bundle counts as lacking once it grew stale, and the next publish renews it before the other members hear of the device as gone.
fn collect(store: &Store, now: u64, relay: &str) -> Result<Read> {
    let requests = endpoint(relay, youarel_relay::REQUESTS)?;
    let collect = account(store)?
        .request(now, Verb::Collect, Vec::new())
        .map_err(sync_error)?;
    let (status, body) = web::post(&requests, &collect, longest_answer()?)?;
    let mut lines = Vec::new();
    let mut missing = true;
    if status == Status::Ok.code() {
        let own = account(store)?.device();
        for slot in Collected::decode(&body)?.slots {
            if slot.device == own {
                missing = sync_rules::stale(slot.published, now);
                continue;
            }
            // The relay's clock is its own claim, so a long silence is only reported, and a person decides on the removal.
            if relay_rules::inactive(now, slot.published) {
                lines.push(format!(
                    "{} has published nothing here for half a year; `youarel remove {}` ends its membership",
                    short(&slot.device),
                    short(&slot.device)
                ));
            }
            match account(store)?.import(&slot.bundle) {
                Ok(imported) => {
                    merge_state(store, now, &imported.state)?;
                    lines.push(format!("merged {}", short(&slot.device)));
                }
                Err(error) => lines.push(format!("skipped {}: {error}", short(&slot.device))),
            }
        }
    } else if status == Status::NotFound.code() {
        lines.push(String::from("the relay holds no copy of this account yet"));
    } else {
        bail!(
            "the relay refused to hand out the bundles: {}",
            reason(&body)
        );
    }
    Ok((lines, missing))
}

/// Publishes this device's bundle at a relay and records what it carried.
fn publish_to(store: &Store, now: u64, relay: &str) -> Result<()> {
    let requests = endpoint(relay, youarel_relay::REQUESTS)?;
    let limit = longest_answer()?;
    publish(store, now, relay, |account, bundle| {
        let request = account
            .request(now, Verb::Publish, bundle)
            .map_err(sync_error)?;
        let (published, answer) = web::post(&requests, &request, limit)?;
        if published != Status::Ok.code() {
            bail!(
                "the relay refused this device's bundle: {}",
                reason(&answer)
            );
        }
        Ok(())
    })
}

/// Syncs through a relay: merges every other member's newest bundle there, then publishes this device's own and records what it carried.
///
/// A bundle that fails is reported and skipped, as a shared folder's are.
///
/// # Errors
///
/// Returns an error when the relay cannot be reached or refuses this device's requests.
pub fn relay(store: &Store, now: u64, relay: &str) -> Result<Vec<String>> {
    let (mut lines, _) = collect(store, now, relay)?;
    publish_to(store, now, relay)?;
    lines.push(String::from("published this device's bundle"));
    Ok(lines)
}

/// Creates a recovery key whose member joins the account, and shows the key once.
///
/// # Errors
///
/// Returns an error when this device is no member.
pub fn recovery_create(store: &Store, now: u64) -> Result<Vec<String>> {
    let key = account(store)?.enroll_recovery(now).map_err(sync_error)?;
    let member = Account::recovery_member(&key).map_err(sync_error)?;
    Ok(vec![
        format!("recovery key {}", key.text().as_str()),
        format!("its member {}", hex(&member)),
        String::from(
            "write the key down and keep it apart from your devices; `youarel recover` brings a new device in with it when every device is gone",
        ),
        String::from(
            "whoever reads the key can join the account, so `youarel remove` its member once the key may have been seen",
        ),
        String::from(
            "the other devices learn of the key with the next bundle or sync, and a relay finds the account by it once this device syncs there",
        ),
    ])
}

/// The recovery key that `text` spells.
fn recovery_key(text: &str) -> Result<RecoveryKey> {
    RecoveryKey::parse(text.trim())
        .context("that is no recovery key; check each group of five for a mistyped symbol")
}

/// Recovers from the first of `bundles` that the key opens, and merges its state.
fn recover_from(
    store: &Store,
    now: u64,
    key: &RecoveryKey,
    bundles: impl IntoIterator<Item = (String, Vec<u8>)>,
) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    for (name, bundle) in bundles {
        match account(store)?.recover(now, key, &bundle) {
            Ok(imported) => {
                merge_state(store, now, &imported.state)?;
                lines.push(format!("recovered the account from {name}"));
                lines.push(String::from(
                    "remove each lost device with `youarel remove`, and create a new recovery key if this one may have been seen",
                ));
                return Ok(lines);
            }
            Err(error) => lines.push(format!("skipped {name}: {error}")),
        }
    }
    lines.push(String::from(
        "no bundle there belongs to this key's account",
    ));
    bail!("{}", lines.join("\n"))
}

/// Recovers the account through a relay: finds the account by the key's member, collects as that member, recovers from a bundle there, and publishes this device's own.
fn recover_through(store: &Store, now: u64, key: &RecoveryKey, relay: &str) -> Result<Vec<String>> {
    let limit = longest_answer()?;
    let member = Account::recovery_member(key).map_err(sync_error)?;
    let (found, location) = web::post(
        &endpoint(relay, youarel_relay::LOCATE)?,
        &Locate { member }.encode(),
        limit,
    )?;
    if found != Status::Ok.code() {
        bail!(
            "the relay found no account for this key: {}",
            reason(&location)
        );
    }
    let id = read_located(&location)?;
    let requests = endpoint(relay, youarel_relay::REQUESTS)?;
    let collect = Account::recovery_request(key, id, (now, Verb::Collect)).map_err(sync_error)?;
    let (collected, slots) = web::post(&requests, &collect, limit)?;
    if collected != Status::Ok.code() {
        bail!(
            "the relay refused to hand out the bundles: {}",
            reason(&slots)
        );
    }
    let bundles = Collected::decode(&slots)?
        .slots
        .into_iter()
        .map(|slot| (short(&slot.device), slot.bundle));
    let mut lines = recover_from(store, now, key, bundles)?;
    publish_to(store, now, relay)?;
    lines.extend(self::relay(store, now, relay)?);
    Ok(lines)
}

/// Where a recovering device finds a bundle of its account.
#[derive(Clone, Copy, Debug)]
pub enum Source<'a> {
    Bundle(&'a Path),
    Folder(&'a Path),
    Relay(&'a str),
}

/// Recovers the account with a recovery key from a bundle file, a shared folder, or a relay, and then syncs through the folder or the relay.
///
/// # Errors
///
/// Returns an error when the text is no recovery key, this device belongs to an account already, or nothing at the source belongs to the key's account.
pub fn recover(store: &Store, now: u64, text: &str, source: Source<'_>) -> Result<Vec<String>> {
    let key = recovery_key(text)?;
    match source {
        Source::Bundle(path) => {
            let name = path.display().to_string();
            recover_from(store, now, &key, [(name, read(path)?)])
        }
        Source::Folder(directory) => {
            let mut bundles = Vec::new();
            for name in fs::list(directory)? {
                if folder_rules::counts(Seen::Touched(&name), None) {
                    let bytes = read(&directory.join(&name))?;
                    bundles.push((name, bytes));
                }
            }
            let mut lines = recover_from(store, now, &key, bundles)?;
            lines.extend(folder(store, now, directory)?);
            Ok(lines)
        }
        Source::Relay(relay) => recover_through(store, now, &key, relay),
    }
}

/// Leaves a message in a pairing's mailbox at the relay.
fn send(mail: &str, nameplate: u16, to: Side, message: Vec<u8>) -> Result<()> {
    let dropped = Mail::Drop {
        nameplate,
        to,
        message,
    }
    .encode()?;
    let (status, body) = web::post(mail, &dropped, longest_answer()?)?;
    if status != Status::Ok.code() {
        bail!("the relay refused the pairing's message: {}", reason(&body));
    }
    Ok(())
}

/// The next message for one side of a pairing, which the relay hands out when it arrives, until the mailbox closes.
fn receive(mail: &str, nameplate: u16, to: Side) -> Result<Vec<u8>> {
    let started = clock::now()?;
    let picked = Mail::Pick { nameplate, to }.encode()?;
    loop {
        let (status, body) = web::post(mail, &picked, longest_answer()?)?;
        if status == Status::Ok.code() {
            return Ok(body);
        }
        if status != Status::NoContent.code() {
            bail!("the relay refused the pairing: {}", reason(&body));
        }
        if !relay_rules::open(started, clock::now()?) {
            bail!("the other device answered nothing before the pairing expired");
        }
    }
}

/// Invites a device through a relay: shows a code under a nameplate the relay hands out, then answers the joining device there and admits it.
///
/// # Errors
///
/// Returns an error when the relay refuses, the joining device sends nothing in time, or its confirmation fails.
pub fn invite_through(store: &Store, relay: &str) -> Result<Vec<String>> {
    let mail = endpoint(relay, youarel_relay::MAIL)?;
    let (status, body) = web::post(&mail, &Mail::Open.encode()?, longest_answer()?)?;
    if status != Status::Ok.code() {
        bail!("the relay opened no pairing: {}", reason(&body));
    }
    let nameplate = read_nameplate(&body)?;
    let code = account(store)?
        .invite_as(Code::for_number(nameplate).map_err(|error| anyhow!(error))?)
        .map_err(sync_error)?;
    crate::commands::print(&[
        format!("invitation code {}", code.text()),
        format!(
            "type it on the new device with `youarel pair join CODE --relay {relay}`; this waits for it"
        ),
    ])?;
    let join = receive(&mail, nameplate, Side::Inviter)?;
    let welcome = account(store)?.accept(&join).map_err(sync_error)?;
    send(&mail, nameplate, Side::Joiner, welcome)?;
    let confirmation = receive(&mail, nameplate, Side::Inviter)?;
    let now = clock::now()?;
    let state = state(store, now)?;
    let entry = account(store)?
        .admit(now, &confirmation, &state)
        .map_err(sync_error)?;
    send(&mail, nameplate, Side::Joiner, entry)?;
    Ok(vec![String::from("admitted the new device")])
}

/// Joins an account through a relay under the code its inviting device showed.
///
/// # Errors
///
/// Returns an error when the code is malformed, the relay refuses, the inviting device answers nothing in time, or the welcome does not authenticate.
pub fn join_through(store: &Store, code: &str, relay: &str) -> Result<Vec<String>> {
    let mail = endpoint(relay, youarel_relay::MAIL)?;
    let code = Code::parse(code).with_context(|| format!("`{code}` is not an invitation code"))?;
    let join = account(store)?.join(code).map_err(sync_error)?;
    send(&mail, code.number, Side::Inviter, join)?;
    let welcome = receive(&mail, code.number, Side::Joiner)?;
    let confirmation = account(store)?.confirm(&welcome).map_err(sync_error)?;
    send(&mail, code.number, Side::Inviter, confirmation)?;
    let entry = receive(&mail, code.number, Side::Joiner)?;
    let state = account(store)?.finish(&entry).map_err(sync_error)?;
    merge_state(store, clock::now()?, &state)?;
    Ok(vec![String::from("joined the account")])
}

#[cfg(test)]
mod tests {
    use super::{merge_state, state};
    use youarel_core::state::{Edit, State};
    use youarel_store::{Store, codec};

    /// A time for the stores' sessions.
    const NOW: u64 = 1_700_000_000_000;

    fn held(store: &Store) -> State {
        store
            .session(NOW, |session| Ok(session.state().clone()))
            .unwrap()
    }

    #[test]
    fn a_received_state_merges_as_records_or_in_the_json_form_of_an_older_build() {
        let sending = tempfile::tempdir().unwrap();
        let sender = Store::new(sending.path().to_path_buf());
        sender
            .session(NOW, |session| {
                let id = session.fresh_id()?;
                session.edit(Edit::AddShortcut {
                    id,
                    link: String::from("https://example.com/"),
                    title: String::from("Example"),
                    folder: None,
                })
            })
            .unwrap();
        let sent = held(&sender);
        let older = codec::encode(&sent).unwrap().into_bytes();
        for received in [state(&sender, NOW).unwrap(), older] {
            let receiving = tempfile::tempdir().unwrap();
            let receiver = Store::new(receiving.path().to_path_buf());
            merge_state(&receiver, NOW, &received).unwrap();
            assert_eq!(held(&receiver), sent);
        }
        let receiving = tempfile::tempdir().unwrap();
        let receiver = Store::new(receiving.path().to_path_buf());
        let error = merge_state(&receiver, NOW, b"\xffnot a state").unwrap_err();
        assert!(error.to_string().contains("malformed"), "{error}");
        assert_eq!(held(&receiver), State::default());
    }
}
