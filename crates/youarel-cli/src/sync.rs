//! The sync commands: the account, the pairing of a device through files, bundles, a shared folder, and the removal of a device.
//!
//! Each command opens the account beside the store and reads or merges the store's state in one locked session.

use crate::{clock, web};
use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;
use youarel_core::relay_rules;
use youarel_crypto::pairing::Code;
use youarel_protocol::relay::{self, Collected, Mail, Side, Verb, read_nameplate};
use youarel_relay::Status;
use youarel_store::{Store, codec, fs};
use youarel_sync::account::Account;
use youarel_sync::keystore::KeyStore;
use youarel_sync::{SyncError, hex};

/// The suffix of a bundle file in a shared folder.
pub const BUNDLE_SUFFIX: &str = ".youarel";
/// The longest bundle, join, or welcome file a command reads.
const LONGEST_MESSAGE: u64 = 64 * 1024 * 1024;
/// The characters of a device identifier a command shows.
const SHORT_ID: usize = 16;

fn sync_error(error: SyncError) -> anyhow::Error {
    anyhow!(error)
}

/// The account beside the store.
fn account(store: &Store) -> Result<Account> {
    let directory = store.directory().join("account");
    let keys = KeyStore::for_directory(&directory).map_err(sync_error)?;
    Account::open(&directory, &keys).map_err(sync_error)
}

/// The store's state in its JSON form.
fn state(store: &Store, now: u64) -> Result<Vec<u8>> {
    store
        .session(now, |session| {
            codec::encode(session.state()).map_err(youarel_store::StoreError::Encode)
        })
        .map(String::into_bytes)
        .map_err(|error| anyhow!(error))
}

/// Merges a state in its JSON form into the store.
fn merge(store: &Store, now: u64, state: &[u8]) -> Result<()> {
    let text = core::str::from_utf8(state).context("the received state is not text")?;
    let received =
        codec::decode(text).map_err(|error| anyhow!("the received state is malformed: {error}"))?;
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
    let mut lines = vec![
        format!("account {}", hex(&status.account)),
        format!("epoch {}", status.epoch),
    ];
    for member in &status.members {
        let marker = if *member == status.this_device {
            "  (this device)"
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
    merge(store, now, &state)?;
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
    merge(store, now, &imported.state)?;
    Ok(vec![format!(
        "merged the state of {}",
        short(&imported.sender)
    )])
}

/// Syncs through a shared folder: reads every other device's bundle there, then writes this device's own.
///
/// A bundle that fails is reported and skipped, so one broken file stops nothing.
///
/// # Errors
///
/// Returns an error when the folder cannot be read or this device's bundle cannot be written.
pub fn folder(store: &Store, now: u64, directory: &Path) -> Result<Vec<String>> {
    let own = format!("{}{BUNDLE_SUFFIX}", hex(&account(store)?.device()));
    let mut lines = Vec::new();
    for name in fs::list(directory)? {
        if !name.ends_with(BUNDLE_SUFFIX) || name == own {
            continue;
        }
        let path = directory.join(&name);
        match account(store)?.import(&read(&path)?) {
            Ok(imported) => {
                merge(store, now, &imported.state)?;
                lines.push(format!("merged {name}"));
            }
            Err(error) => lines.push(format!("skipped {name}: {error}")),
        }
    }
    let state = state(store, now)?;
    let bytes = match account(store)?.export(now, &state) {
        Ok(bytes) => bytes,
        Err(error) => {
            lines.push(format!("wrote no bundle: {error}"));
            bail!("{}", lines.join("\n"));
        }
    };
    write(&directory.join(&own), &bytes)?;
    lines.push(format!("wrote {own}"));
    Ok(lines)
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

/// Syncs through a relay: merges every other member's newest bundle there, then publishes this device's own.
///
/// A bundle that fails is reported and skipped, as a shared folder's are.
///
/// # Errors
///
/// Returns an error when the relay cannot be reached or refuses this device's requests.
pub fn relay(store: &Store, now: u64, relay: &str) -> Result<Vec<String>> {
    let requests = endpoint(relay, youarel_relay::REQUESTS)?;
    let limit = longest_answer()?;
    let collect = account(store)?
        .request(now, Verb::Collect, Vec::new())
        .map_err(sync_error)?;
    let (status, body) = web::post(&requests, &collect, limit)?;
    let mut lines = Vec::new();
    if status == Status::Ok.code() {
        let own = account(store)?.device();
        for slot in Collected::decode(&body)?.slots {
            if slot.device == own {
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
                    merge(store, now, &imported.state)?;
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
    let state = state(store, now)?;
    let bundle = account(store)?.export(now, &state).map_err(sync_error)?;
    let publish = account(store)?
        .request(now, Verb::Publish, bundle)
        .map_err(sync_error)?;
    let (published, answer) = web::post(&requests, &publish, limit)?;
    if published != Status::Ok.code() {
        bail!(
            "the relay refused this device's bundle: {}",
            reason(&answer)
        );
    }
    lines.push(String::from("published this device's bundle"));
    Ok(lines)
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
    merge(store, clock::now()?, &state)?;
    Ok(vec![String::from("joined the account")])
}
