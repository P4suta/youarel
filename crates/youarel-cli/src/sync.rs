//! Membership, pairing, bounded segment transports, removal, and recovery.

use crate::{clock, web};
pub mod devices;
pub mod pairing;
pub mod segments;
use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;
use youarel_core::folder_rules;
use youarel_core::relay_rules;
use youarel_core::segment_rules::Destination;
use youarel_crypto::pairing::Code;
use youarel_crypto::recovery::RecoveryKey;
use youarel_protocol::message::SnapshotHint;
use youarel_protocol::relay::{self, Mail, Side, read_nameplate};
use youarel_relay::Status;
use youarel_store::{PublishRequest, Store, fs};
use youarel_sync::account::{Account, founded, logged};
use youarel_sync::{SyncError, hex};

/// The directory beside the store that holds the account's files.
pub const ACCOUNT_DIRECTORY: &str = "account";
/// The longest pairing file a command reads.
const LONGEST_MESSAGE: u64 = 64 * 1024 * 1024;
/// The characters of a device identifier a command shows.
const SHORT_ID: usize = 16;

fn sync_error(error: SyncError) -> anyhow::Error {
    anyhow!(error)
}

/// The account beside the store.
fn account(store: &Store) -> Result<Account> {
    Account::open(&store.directory().join(ACCOUNT_DIRECTORY)).map_err(sync_error)
}

/// Whether the target needs a publication or an outgoing chain is missing.
fn pending_at(store: &Store, now: u64, target: &str, missing: bool) -> Result<bool> {
    let operations = logged(&store.directory().join(ACCOUNT_DIRECTORY)).map_err(sync_error)?;
    let mut request = PublishRequest::new(target, operations);
    if missing {
        request.destination = Destination::Missing;
    }
    store
        .session(now, |session| {
            request.deltas_only = is_relay(target)
                && session.publication_state()?.snapshot_bytes() > relay_rules::MOST_BYTES;
            session.publication_due(request)
        })
        .map_err(anyhow::Error::from)
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

fn snapshot_hint(store: &Store, now: u64) -> Result<Option<SnapshotHint>> {
    let sender = account(store)?.device();
    Ok(store
        .session(now, |session| Ok(session.publication_state()?.snapshot()))?
        .map(|sequence| SnapshotHint { sender, sequence }))
}

/// Adds the confirmed device and writes its membership, epoch keys, and snapshot hint.
///
/// # Errors
///
/// Returns an error when the confirmation does not verify.
pub fn admit(store: &Store, now: u64, confirmation: &Path, out: &Path) -> Result<Vec<String>> {
    let hint = snapshot_hint(store, now)?;
    let bytes = account(store)?
        .admit(now, &read(confirmation)?, hint)
        .map_err(sync_error)?;
    write(out, &bytes)?;
    Ok(vec![format!(
        "wrote {}; carry it to the new device and run `youarel pair finish`",
        out.display()
    )])
}

/// Installs an entry file before a separate sync fetches the library.
///
/// # Errors
///
/// Returns an error when the entry does not authenticate.
pub fn finish(store: &Store, _now: u64, entry: &Path) -> Result<Vec<String>> {
    account(store)?.finish(&read(entry)?).map_err(sync_error)?;
    Ok(vec![String::from(
        "joined the account; sync through a shared folder or relay to fetch the library",
    )])
}

/// Writes a snapshot directory of bounded sealed segments.
///
/// # Errors
///
/// Returns an error when the device belongs to no account or lacks the current key.
pub fn export(store: &Store, now: u64, out: &Path) -> Result<Vec<String>> {
    segments::snapshot(store, now, &mut segments::folder::Directory::new(out))?;
    Ok(vec![format!("wrote snapshot directory {}", out.display())])
}

/// Reads the peer chains of a snapshot directory without publishing to it.
///
/// # Errors
///
/// Reports an unreadable or refused snapshot directory.
pub fn import(store: &Store, now: u64, file: &Path) -> Result<Vec<String>> {
    segments::folder::import(store, now, file)
}

/// The name of this device's own chain directory.
///
/// # Errors
///
/// Returns an error when the account cannot be opened.
pub fn own_chain(store: &Store) -> Result<String> {
    Ok(hex(&account(store)?.device()))
}

/// What a merge read, and whether this device's transport head is missing or behind.
type Read = (segments::Report, bool);

fn merge_folder(store: &Store, now: u64, directory: &Path) -> Result<Read> {
    use segments::SegmentTarget as _;
    let report = segments::folder::report(store, now, directory, true)?;
    let head = segments::folder::Directory::new(directory).head(&account(store)?.device())?;
    let state = store.session(now, |session| session.publication_state())?;
    let missing = head.is_none_or(|head| {
        state
            .cursor()
            .is_none_or(|cursor| cursor.sequence() != head.latest())
    });
    Ok((report, missing))
}

/// Merges peer chains and publishes a bounded delta or snapshot only when due.
///
/// A refused peer is reported and skipped while other peers continue.
///
/// # Errors
///
/// Reports an unreadable folder or a failed publication.
pub fn folder(store: &Store, now: u64, directory: &Path) -> Result<Vec<String>> {
    Ok(folder_report(store, now, directory)?.into_lines())
}

fn folder_report(store: &Store, now: u64, directory: &Path) -> Result<segments::Report> {
    let (mut report, _) = merge_folder(store, now, directory)?;
    match segments::publish(store, now, &mut segments::folder::Directory::new(directory)) {
        Ok(Some(head)) => report.lines.push(format!(
            "wrote {}/{}",
            own_chain(store)?,
            folder_rules::segment_name(head.latest())
        )),
        Ok(None) => report.lines.push(String::from("up to date")),
        Err(error) => {
            report.lines.push(format!("wrote no segments: {error:#}"));
            bail!("{}", report.into_lines().join("\n"));
        }
    }
    Ok(report)
}

/// What a merge read, and whether this device has something new to say at the target, which a publish there then carries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Merged {
    /// The received or refused peer histories.
    pub lines: Vec<String>,
    /// Whether this device has a pending write, membership change, or missing outgoing chain.
    pub pending: bool,
    pub problems: Vec<String>,
    pub more: bool,
}

impl Merged {
    /// # Errors
    ///
    /// Reports refused or incomplete peer histories before the window records success.
    pub fn checked_pending(&self) -> Result<bool> {
        anyhow::ensure!(self.problems.is_empty(), "{}", self.problems.join("\n"));
        anyhow::ensure!(!self.more, "Still receiving changes from other devices.");
        Ok(self.pending)
    }
}

/// Merges what the other devices left at `target`, a relay's address or a shared folder, and writes nothing there.
///
/// The window reads peers on its interval and after notices, publishing only its own pending changes.
///
/// # Errors
///
/// Returns an error when the relay or the folder cannot be reached, or the device belongs to no account.
pub fn merge(store: &Store, now: u64, target: &str) -> Result<Merged> {
    let (report, missing) = if is_relay(target) {
        collect(store, now, target)?
    } else {
        merge_folder(store, now, Path::new(target))?
    };
    Ok(Merged {
        lines: report.clone().into_lines(),
        problems: report.problems,
        more: report.more,
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
        "removed {}; the next publication carries a key it never sees",
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

/// Runs a sync and refuses to claim success while a peer needs more history.
///
/// # Errors
///
/// Reports transport failures, refused peer chains, or an unfinished receive page.
pub fn sync_checked(store: &Store, now: u64, target: &str) -> Result<Vec<String>> {
    let report = if is_relay(target) {
        segments::relay::sync(store, now, target)?
    } else {
        folder_report(store, now, Path::new(target))?
    };
    report.complete()?;
    Ok(report.into_lines())
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

/// The network limit for a bounded delivery with its frames.
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

/// Receives bounded cursor pages and reports missing or stale outgoing history.
fn collect(store: &Store, now: u64, relay: &str) -> Result<Read> {
    let read = segments::relay::collect(store, now, relay)?;
    let state = store.session(now, |session| session.publication_state())?;
    let missing = read.missing(&state, now);
    Ok((read.report, missing))
}

fn publish_to(store: &Store, now: u64, relay: &str) -> Result<()> {
    segments::relay::publish(store, now, relay)
}

/// Receives cursor pages and publishes only pending segments, reporting refused peers.
///
/// # Errors
///
/// Returns an error when the relay cannot be reached or refuses this device's requests.
pub fn relay(store: &Store, now: u64, relay: &str) -> Result<Vec<String>> {
    Ok(segments::relay::sync(store, now, relay)?.into_lines())
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
            "the other devices learn of the key with the next sync, and a relay locates its library once this device syncs there",
        ),
    ])
}

/// The recovery key that `text` spells.
fn recovery_key(text: &str) -> Result<RecoveryKey> {
    RecoveryKey::parse(text.trim())
        .context("that is no recovery key; check each group of five for a mistyped symbol")
}

/// Locates the recovery member and resumes a retained authenticated segment chain.
fn recover_through(store: &Store, now: u64, key: &RecoveryKey, relay: &str) -> Result<Vec<String>> {
    segments::relay::recover(store, now, key, relay)
}

fn recover_directory(
    store: &Store,
    now: u64,
    key: &RecoveryKey,
    directory: &Path,
) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    for name in fs::list(directory)? {
        if !folder_rules::peer(&name) {
            continue;
        }
        let Some(sender) = youarel_sync::unhex(&name) else {
            continue;
        };
        let recovered = segments::folder::newest(directory, sender).and_then(|bytes| {
            account(store)?
                .recover_segment(now, key, &bytes)
                .map_err(sync_error)
        });
        match recovered {
            Ok(_opened) => {
                lines.push(format!("recovered the account from {name}"));
                let report = segments::folder::report(store, now, directory, false)?;
                report.complete()?;
                lines.extend(report.into_lines());
                lines.push(String::from(
                    "remove lost devices and replace a recovery key that may have been seen",
                ));
                return Ok(lines);
            }
            Err(error) => lines.push(format!("skipped {name}: {error:#}")),
        }
    }
    lines.push(String::from(
        "no segment there belongs to this key's account",
    ));
    bail!("{}", lines.join("\n"))
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
        Source::Bundle(path) => recover_directory(store, now, &key, path),
        Source::Folder(directory) => {
            let mut lines = recover_directory(store, now, &key, directory)?;
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
    let hint = snapshot_hint(store, now)?;
    let entry = account(store)?
        .admit(now, &confirmation, hint)
        .map_err(sync_error)?;
    publish_to(store, now, relay)?;
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
    account(store)?.finish(&entry).map_err(sync_error)?;
    let mut lines = vec![String::from("joined the account")];
    lines.extend(self::relay(store, clock::now()?, relay)?);
    Ok(lines)
}
