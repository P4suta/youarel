//! Per-device immutable files and a committed head in a shared folder.

use super::{Delivery, Report, SegmentTarget};
use anyhow::{Context, Result, bail, ensure};
use std::path::{Path, PathBuf};
use youarel_core::folder_rules::{self, HEAD_FILE, segment_name, segment_sequence};
use youarel_core::segment_rules::{Fetch, Sequence, Verdict, fetch};
use youarel_protocol::message::Digest;
use youarel_protocol::segment::{Head, LONGEST_MESSAGE, Segment};
use youarel_store::{Store, fs};
use youarel_sync::account::Account;
use youarel_sync::{hex, unhex};

const LONGEST_HEAD: u64 = 128;
const MOST_SEGMENTS_PER_PASS: usize = 64;

#[derive(Debug)]
pub struct Directory {
    root: PathBuf,
    identity: String,
}

impl Directory {
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
            identity: root.to_string_lossy().into_owned(),
        }
    }
}

fn read_head(directory: &Path) -> Result<Option<Head>> {
    fs::read_bounded_if_present(&directory.join(HEAD_FILE), LONGEST_HEAD)?
        .map(|bytes| Head::decode(&bytes).context("the chain head is malformed"))
        .transpose()
}

pub(crate) fn newest(directory: &Path, sender: Digest) -> Result<Vec<u8>> {
    let peer = directory.join(hex(&sender));
    let head = read_head(&peer)?.context("the peer's head has not arrived")?;
    let bytes = fs::read_bounded(
        &peer.join(segment_name(head.latest()).as_str()),
        u64::try_from(LONGEST_MESSAGE)?,
    )?;
    let segment = Segment::decode(&bytes)?;
    ensure!(
        segment.sender == sender && segment.header.sequence() == head.latest(),
        "the newest segment names another chain or sequence"
    );
    Ok(bytes)
}

impl SegmentTarget for Directory {
    fn identity(&self) -> &str {
        &self.identity
    }

    fn head(&mut self, sender: &Digest) -> Result<Option<Head>> {
        read_head(&self.root.join(hex(sender)))
    }

    fn write(&mut self, sender: &Digest, sequence: Sequence, bytes: &[u8]) -> Result<()> {
        let directory = self.root.join(hex(sender));
        fs::create_dir_all(&directory)?;
        let path = directory.join(segment_name(sequence).as_str());
        let held = fs::read_bounded_if_present(&path, u64::try_from(LONGEST_MESSAGE)?)?;
        if let Some(held) = held {
            ensure!(
                held == bytes,
                "segment {} already holds different bytes",
                path.display()
            );
        } else {
            fs::replace(&path, bytes)?;
        }
        Ok(())
    }

    fn finish(&mut self, sender: &Digest, head: Head) -> Result<()> {
        let directory = self.root.join(hex(sender));
        fs::replace(&directory.join(HEAD_FILE), &head.encode())?;
        for name in fs::list(&directory)? {
            if segment_sequence(&name).is_some_and(|sequence| sequence < head.first()) {
                fs::remove_if_present(&directory.join(name))?;
            }
        }
        Ok(())
    }
}

struct Reader<'a> {
    store: &'a Store,
    now: u64,
    account: Account,
    directory: &'a Path,
}

struct PeerRead {
    applied: usize,
    more: bool,
}

impl Reader<'_> {
    fn peer(&mut self, sender: Digest) -> Result<PeerRead> {
        let directory = self.directory.join(hex(&sender));
        let head = read_head(&directory)?.context("the peer's head has not arrived")?;
        let mut applied: usize = 0;
        for count in 0..=MOST_SEGMENTS_PER_PASS {
            let cursor = self
                .store
                .session(self.now, |session| session.peer_cursor(&sender))?;
            let sequence = match fetch(cursor, head.available(), head.latest()) {
                Fetch::Complete => break,
                Fetch::Lost => bail!(
                    "this chain no longer holds the needed history; sync once through another folder or a snapshot directory"
                ),
                Fetch::Segment(sequence) => sequence,
            };
            if count == MOST_SEGMENTS_PER_PASS {
                return Ok(PeerRead {
                    applied,
                    more: true,
                });
            }
            let path = directory.join(segment_name(sequence).as_str());
            let bytes = fs::read_bounded(&path, u64::try_from(LONGEST_MESSAGE)?)?;
            let decision = super::apply(
                self.store,
                self.now,
                &mut self.account,
                Delivery {
                    sender,
                    head,
                    bytes: &bytes,
                },
            )?;
            match decision.verdict {
                Verdict::Apply => applied = applied.saturating_add(1),
                Verdict::Reapply | Verdict::Old => {}
                Verdict::Gap | Verdict::Jump(_) | Verdict::Lost | Verdict::Forked => bail!(
                    "segment {} was refused: {:?}",
                    path.display(),
                    decision.verdict
                ),
            }
        }
        Ok(PeerRead {
            applied,
            more: false,
        })
    }
}

/// Merges peer records and cursors without writing anything to the shared directory.
///
/// # Errors
///
/// Refuses an unreadable directory or local account; an incomplete or broken peer is reported and leaves the other peers readable.
pub fn merge(store: &Store, now: u64, directory: &Path) -> Result<Vec<String>> {
    Ok(report(store, now, directory, true)?.into_lines())
}

/// Merges every chain of an exported snapshot directory, including this device's own backups.
///
/// # Errors
///
/// Reports an unreadable directory or local account; incomplete peers are reported individually.
pub fn import(store: &Store, now: u64, directory: &Path) -> Result<Vec<String>> {
    Ok(report(store, now, directory, false)?.into_lines())
}

pub(crate) fn report(store: &Store, now: u64, directory: &Path, skip_own: bool) -> Result<Report> {
    let mut reader = Reader {
        store,
        now,
        account: super::super::account(store)?,
        directory,
    };
    let own = reader.account.device();
    let mut report = Report::default();
    for name in fs::list(directory)? {
        if !folder_rules::peer(&name) {
            continue;
        }
        let Some(sender) = unhex(&name).filter(|sender| !skip_own || *sender != own) else {
            continue;
        };
        match reader.peer(sender) {
            Ok(read) => {
                if read.applied > 0 {
                    report.lines.push(format!("merged {name}"));
                }
                report.more |= read.more;
            }
            Err(error) => report.problems.push(format!("skipped {name}: {error:#}")),
        }
    }
    Ok(report)
}
