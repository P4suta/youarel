//! Cursor pages and immutable publications over the optional relay.

use super::{Delivery, Report, SegmentTarget};
use crate::sync::{account, endpoint, longest_answer, reason, short, sync_error};
use crate::web;
use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeMap;
use youarel_core::relay_rules;
use youarel_core::segment_rules::{Cursor, Fetch, Kind, Sequence, Verdict, fetch};
use youarel_core::sync_rules;
use youarel_crypto::recovery::RecoveryKey;
use youarel_protocol::message::Digest;
use youarel_protocol::relay::{Locate, Verb, read_located};
use youarel_protocol::segment::{Chain, Cursors, Delivered, Head, Segment};
use youarel_relay::Status;
use youarel_store::{PublicationState, Store};
use youarel_sync::account::Account;

const PAGES_PER_PASS: usize = 8;

#[derive(Clone, Copy, Debug)]
pub(crate) struct OwnChain {
    head: Head,
    published: u64,
}

#[derive(Debug)]
pub(crate) struct Read {
    pub report: Report,
    pub own: Option<OwnChain>,
}

impl Read {
    pub(crate) fn missing(&self, state: &PublicationState, now: u64) -> bool {
        self.own.is_none_or(|own| {
            state
                .cursor()
                .is_none_or(|cursor| cursor.sequence() != own.head.latest())
                || sync_rules::stale(own.published, now)
        })
    }
}

fn post(url: &str, bytes: &[u8]) -> Result<(u16, Vec<u8>)> {
    web::post(
        &endpoint(url, youarel_relay::REQUESTS)?,
        bytes,
        longest_answer()?,
    )
}

fn cursors(store: &Store, now: u64, account: &Account) -> Result<Cursors> {
    let mut peers = store.session(now, |session| session.peer_cursors())?;
    let members = account.status().map_err(sync_error)?.members;
    peers.retain(|sender, _cursor| members.contains(sender));
    Ok(Cursors { peers })
}

fn request(account: &Account, now: u64, peers: &Cursors) -> Result<Vec<u8>> {
    account
        .request(now, Verb::Collect, peers.encode()?)
        .map_err(sync_error)
}

struct Reader<'a> {
    store: &'a Store,
    now: u64,
    account: Account,
    skipped: BTreeMap<Digest, Cursor>,
    read: Read,
}

impl Reader<'_> {
    fn peer(&mut self, chain: &Chain, more: bool) -> Result<usize> {
        let cursor = self
            .store
            .session(self.now, |session| session.peer_cursor(&chain.sender))?;
        ensure!(
            fetch(cursor, chain.head.available(), chain.head.latest()) != Fetch::Lost,
            "this chain no longer holds the needed history; sync once through a shared folder or snapshot directory"
        );
        ensure!(
            more || !chain.segments.is_empty()
                || fetch(cursor, chain.head.available(), chain.head.latest()) == Fetch::Complete,
            "the relay omitted retained history"
        );
        let mut applied = 0usize;
        for bytes in &chain.segments {
            let header = Segment::decode(bytes)?.header;
            ensure!(
                header.sequence() != chain.head.latest()
                    || relay_rules::incomplete_snapshot(header) == chain.incomplete,
                "the relay's snapshot progress disagrees with its latest segment"
            );
            let decision = super::apply(
                self.store,
                self.now,
                &mut self.account,
                Delivery {
                    sender: chain.sender,
                    head: chain.head,
                    bytes,
                },
            )?;
            match decision.verdict {
                Verdict::Apply => applied = applied.saturating_add(1),
                Verdict::Reapply | Verdict::Old => {}
                Verdict::Gap | Verdict::Jump(_) | Verdict::Lost | Verdict::Forked => {
                    bail!("the segment was refused: {:?}", decision.verdict)
                }
            }
        }
        Ok(applied)
    }

    fn page(&mut self, delivered: &Delivered) -> Result<usize> {
        self.account
            .learn_membership(delivered.account, &delivered.operations)
            .map_err(sync_error)?;
        let mut applied = 0usize;
        for chain in &delivered.chains {
            if chain.sender == self.account.device() {
                self.read.own = Some(OwnChain {
                    head: chain.head,
                    published: chain.published,
                });
                continue;
            }
            if self.skipped.contains_key(&chain.sender) {
                continue;
            }
            self.read.report.more |= chain.incomplete;
            if relay_rules::inactive(self.now, chain.published) {
                self.read.report.lines.push(format!(
                    "{} has published nothing here for half a year",
                    short(&chain.sender)
                ));
            }
            match self.peer(chain, delivered.more) {
                Ok(count) => {
                    applied = applied.saturating_add(count);
                    if count > 0 {
                        self.read
                            .report
                            .lines
                            .push(format!("merged {}", short(&chain.sender)));
                    }
                }
                Err(error) => {
                    self.read
                        .report
                        .problems
                        .push(format!("skipped {}: {error:#}", short(&chain.sender)));
                    self.skipped
                        .insert(chain.sender, Cursor::new(chain.head.latest(), [0; 32]));
                }
            }
        }
        Ok(applied)
    }

    fn positions(&self) -> Result<Cursors> {
        let mut positions = cursors(self.store, self.now, &self.account)?;
        if let Some(cursor) = self.store.session(self.now, |session| {
            Ok(session.publication_state()?.chain_cursor())
        })? {
            positions.peers.insert(self.account.device(), cursor);
        }
        positions.peers.extend(
            self.skipped
                .iter()
                .map(|(sender, cursor)| (*sender, *cursor)),
        );
        Ok(positions)
    }
}

pub(crate) fn collect(store: &Store, now: u64, url: &str) -> Result<Read> {
    let mut reader = Reader {
        store,
        now,
        account: account(store)?,
        skipped: BTreeMap::new(),
        read: Read {
            report: Report::default(),
            own: None,
        },
    };
    for _ in 0..PAGES_PER_PASS {
        let (status, body) = post(url, &request(&reader.account, now, &reader.positions()?)?)?;
        if status == Status::NotFound.code() {
            reader
                .read
                .report
                .lines
                .push(String::from("the relay holds no copy of this account yet"));
            break;
        }
        ensure!(
            status == Status::Ok.code(),
            "the relay refused collection: {}",
            reason(&body)
        );
        let delivered = Delivered::decode(&body)?;
        reader.read.report.more = delivered.more;
        let skipped = reader.skipped.len();
        let advanced = reader.page(&delivered)?;
        if !delivered.more || (advanced == 0 && skipped == reader.skipped.len()) {
            break;
        }
    }
    Ok(reader.read)
}

struct Relay<'a> {
    store: &'a Store,
    now: u64,
    url: &'a str,
    remote: Option<Head>,
}

impl SegmentTarget for Relay<'_> {
    fn identity(&self) -> &str {
        self.url
    }

    fn deltas_only(&self, state: &PublicationState) -> bool {
        state.snapshot_bytes() > relay_rules::MOST_BYTES
            || self.remote.is_some_and(|head| head.snapshot().is_none())
    }

    fn head(&mut self, _sender: &Digest) -> Result<Option<Head>> {
        Ok(self.remote)
    }

    fn write(&mut self, sender: &Digest, sequence: Sequence, bytes: &[u8]) -> Result<()> {
        let segment = Segment::decode(bytes)?;
        ensure!(
            segment.sender == *sender && segment.header.sequence() == sequence,
            "the reserved segment names another position"
        );
        let bridge = segment.header.kind() == Kind::Delta
            && self.remote.is_none_or(|head| {
                head.latest() < sequence && head.latest().next() != Some(sequence)
            });
        let verb = if bridge { Verb::Bridge } else { Verb::Publish };
        let request = account(self.store)?
            .request(self.now, verb, bytes.to_vec())
            .map_err(sync_error)?;
        let (status, body) = post(self.url, &request)?;
        ensure!(
            status == Status::Ok.code(),
            "the relay refused this segment: {}",
            reason(&body)
        );
        if self.remote.is_none_or(|head| head.latest() < sequence) {
            let first = if bridge {
                sequence
            } else {
                self.remote.map_or(sequence, Head::first)
            };
            self.remote = Head::new(first, sequence, None);
        }
        Ok(())
    }

    fn finish(&mut self, _sender: &Digest, head: Head) -> Result<()> {
        ensure!(
            self.remote
                .is_some_and(|remote| remote.latest() == head.latest()),
            "the relay did not acknowledge the complete publication"
        );
        Ok(())
    }
}

fn renew(store: &Store, now: u64, url: &str, own: OwnChain) -> Result<()> {
    let mut account = account(store)?;
    let mut positions = cursors(store, now, &account)?;
    let before = own
        .head
        .latest()
        .get()
        .checked_sub(1)
        .and_then(Sequence::new);
    positions.peers.remove(&account.device());
    if let Some(before) = before {
        positions
            .peers
            .insert(account.device(), Cursor::new(before, [0; 32]));
    }
    let (status, body) = post(url, &request(&account, now, &positions)?)?;
    ensure!(
        status == Status::Ok.code(),
        "the relay refused renewal: {}",
        reason(&body)
    );
    let delivered = Delivered::decode(&body)?;
    let chain = delivered
        .chains
        .iter()
        .find(|chain| chain.sender == account.device())
        .context("the relay lost this device's chain")?;
    let bytes = chain
        .segments
        .iter()
        .find(|bytes| {
            Segment::decode(bytes)
                .is_ok_and(|segment| segment.header.sequence() == own.head.latest())
        })
        .context("the relay lost this device's latest segment")?;
    let opened = account.open_segment(bytes).map_err(sync_error)?;
    let local = store.session(now, |session| Ok(session.publication_state()?.cursor()))?;
    ensure!(
        local == Some(Cursor::new(opened.header.sequence(), opened.digest)),
        "the relay returned a fork of this device's latest segment"
    );
    let request = account
        .request(now, Verb::Publish, bytes.clone())
        .map_err(sync_error)?;
    let (renewed, answer) = post(url, &request)?;
    ensure!(
        renewed == Status::Ok.code(),
        "the relay refused renewal: {}",
        reason(&answer)
    );
    Ok(())
}

fn publish_after(store: &Store, now: u64, url: &str, read: &mut Read) -> Result<()> {
    let mut target = Relay {
        store,
        now,
        url,
        remote: read.own.map(|own| own.head),
    };
    if super::publish(store, now, &mut target)?.is_some() {
        read.report
            .lines
            .push(String::from("published this device's segments"));
    } else if let Some(own) = read.own.filter(|own| sync_rules::stale(own.published, now)) {
        renew(store, now, url, own)?;
        read.report
            .lines
            .push(String::from("renewed this device's chain"));
    } else {
        read.report.lines.push(String::from("up to date"));
    }
    Ok(())
}

pub(crate) fn sync(store: &Store, now: u64, url: &str) -> Result<Report> {
    let mut read = collect(store, now, url)?;
    publish_after(store, now, url, &mut read)?;
    Ok(read.report)
}

pub(crate) fn publish(store: &Store, now: u64, url: &str) -> Result<()> {
    let mut read = collect(store, now, url)?;
    publish_after(store, now, url, &mut read)
}

pub(crate) fn recover(
    store: &Store,
    now: u64,
    key: &RecoveryKey,
    url: &str,
) -> Result<Vec<String>> {
    let member = Account::recovery_member(key).map_err(sync_error)?;
    let (found, location) = web::post(
        &endpoint(url, youarel_relay::LOCATE)?,
        &Locate { member }.encode(),
        longest_answer()?,
    )?;
    ensure!(
        found == Status::Ok.code(),
        "the relay found no account for this key: {}",
        reason(&location)
    );
    let id = read_located(&location)?;
    let request = Account::recovery_request(key, id, (now, Verb::Collect)).map_err(sync_error)?;
    let (status, body) = post(url, &request)?;
    ensure!(
        status == Status::Ok.code(),
        "the relay refused recovery: {}",
        reason(&body)
    );
    let delivered = Delivered::decode(&body)?;
    ensure!(
        delivered.account == id,
        "the relay returned another account"
    );
    let mut problems = Vec::new();
    for bytes in delivered.chains.iter().flat_map(|chain| &chain.segments) {
        match account(store)?.recover_segment(now, key, bytes) {
            Ok(_opened) => {
                let mut read = Read {
                    report: Report::default(),
                    own: None,
                };
                publish_after(store, now, url, &mut read)?;
                let report = sync(store, now, url)?;
                report.complete()?;
                let mut lines = vec![String::from("recovered the library through the relay")];
                lines.extend(report.into_lines());
                return Ok(lines);
            }
            Err(error) => problems.push(error.to_string()),
        }
    }
    bail!(
        "the relay has no usable bootstrap for this key; recover through a shared folder or snapshot directory: {}",
        problems.join("; ")
    )
}
