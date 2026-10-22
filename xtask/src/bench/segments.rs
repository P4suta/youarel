use super::{Campaign, Measurement, NOW};
use crate::fs;
use anyhow::{Context as _, Result, ensure};
use std::path::PathBuf;
use youarel_core::folder_rules::segment_name;
use youarel_core::segment_rules::{Available, Sequence, Verdict};
use youarel_core::state::ItemId;
use youarel_store::{PendingPublish, PublishRequest, ReceivedSegment};
use youarel_sync::account::{Account, body_digest};

const TARGET: &str = "benchmark";
const ACCOUNT: &str = "account";
const SEGMENTS: &str = "segments";

#[derive(Debug)]
struct Transfer {
    sender: [u8; 32],
    first: Sequence,
    last: Sequence,
    available: Available,
    files: Vec<PathBuf>,
}

fn paired(first: &mut Account, second: &mut Account) -> Result<()> {
    first.found(NOW)?;
    let code = first.invite()?;
    let join = second.join(code)?;
    let welcome = first.accept(&join)?;
    let confirmation = second.confirm(&welcome)?;
    let entry = first.admit(NOW, &confirmation, None)?;
    second.finish(&entry)?;
    Ok(())
}

impl Campaign {
    fn deliver(
        &self,
        account: &mut Account,
        pending: &PendingPublish,
        metric: &mut Measurement,
    ) -> Result<Vec<PathBuf>> {
        let source = self.store("source");
        let directory = self.root.join(SEGMENTS);
        fs::create_dir_all(&directory)?;
        let mut files = Vec::new();
        for sequence in pending.sequences() {
            self.guard.check()?;
            let part =
                source.session(NOW, |session| session.publication_part(pending, sequence))?;
            ensure!(
                body_digest(&part.body) == part.digest,
                "reserved synthetic plaintext changed"
            );
            let bytes = if let Some(bytes) = part.sealed {
                bytes
            } else {
                let bytes = account.publish(NOW, part.header, &part.body)?;
                source.session(NOW, |session| {
                    session.seal_publication_part(pending, sequence, &bytes)
                })?;
                bytes
            };
            self.guard.spool(bytes.len())?;
            let path = directory.join(segment_name(sequence).as_str());
            fs::write(&path, &bytes)?;
            files.push(path);
            metric.bytes = metric.bytes.saturating_add(u64::try_from(bytes.len())?);
        }
        metric.segments = files.len();
        Ok(files)
    }

    fn publish(
        &self,
        account: &mut Account,
        snapshot: bool,
        metric: &mut Measurement,
    ) -> Result<Transfer> {
        let source = self.store("source");
        let _publishing = source.publishing()?;
        account.prepare_publish(NOW)?;
        let previous = source.session(NOW, |session| session.publication_state())?;
        let mut request = PublishRequest::new(TARGET, account.operations());
        request.force_snapshot = snapshot;
        let pending = source
            .session(NOW, |session| {
                session
                    .prepare_publication_checked(request, body_digest, || self.guard.publication())
            })?
            .context("the synthetic publication was unexpectedly quiet")?;
        ensure!(
            pending.is_snapshot() == snapshot,
            "the requested synthetic publication has another kind"
        );
        let available = if pending.is_snapshot() {
            Available {
                first: pending.first(),
                snapshot: Some(pending.first()),
            }
        } else {
            Available {
                first: previous
                    .first()
                    .context("the synthetic chain has no beginning")?,
                snapshot: previous.snapshot(),
            }
        };
        let files = self.deliver(account, &pending, metric)?;
        source.session(NOW, |session| session.commit_publication(&pending))?;
        Ok(Transfer {
            sender: account.device(),
            first: pending.first(),
            last: pending.last(),
            available,
            files,
        })
    }

    fn receive(
        &self,
        account: &mut Account,
        transfer: &Transfer,
        metric: &mut Measurement,
    ) -> Result<()> {
        let peer = self.store("peer");
        for path in &transfer.files {
            self.guard.check()?;
            let bytes = fs::read(path)?;
            let opened = account.open_segment(&bytes)?;
            ensure!(
                opened.sender == transfer.sender
                    && transfer.first <= opened.header.sequence()
                    && opened.header.sequence() <= transfer.last,
                "the synthetic segment names another chain range"
            );
            let result = peer.session(NOW, |session| {
                session.receive_segment(
                    ReceivedSegment {
                        sender: opened.sender,
                        header: opened.header,
                        available: transfer.available,
                        body: &opened.body,
                    },
                    body_digest,
                )
            })?;
            ensure!(
                result.verdict == Verdict::Apply,
                "the synthetic receiver refused a segment: {:?}",
                result.verdict
            );
            metric.bytes = metric.bytes.saturating_add(u64::try_from(bytes.len())?);
            metric.segments = metric.segments.saturating_add(1);
        }
        Ok(())
    }

    pub(super) fn sync_and_edits(&self, folders: &[ItemId]) -> Result<()> {
        let mut source = Account::open(&self.store("source").directory().join(ACCOUNT))?;
        let mut peer = Account::open(&self.store("peer").directory().join(ACCOUNT))?;
        paired(&mut source, &mut peer)?;
        let count = self
            .options
            .workload
            .shortcuts()
            .saturating_add(self.options.workload.folders());
        let snapshot = self.measure(
            Measurement::operation("snapshot_publish", count),
            |metric| {
                metric.rows = usize::try_from(count)?;
                self.publish(&mut source, true, metric)
            },
        )?;
        self.measure(Measurement::operation("snapshot_apply", count), |metric| {
            metric.rows = usize::try_from(count)?;
            self.receive(&mut peer, &snapshot, metric)
        })?;
        self.own_edits(folders)?;
        let delta = self.measure(Measurement::operation("delta_publish", 1), |metric| {
            metric.rows = 1;
            self.publish(&mut source, false, metric)
        })?;
        self.measure(Measurement::operation("delta_apply", 1), |metric| {
            metric.rows = 1;
            self.receive(&mut peer, &delta, metric)
        })?;
        ensure!(
            self.fingerprint(&self.store("source"))? == self.fingerprint(&self.store("peer"))?,
            "the measured segment exchange did not converge"
        );
        Ok(())
    }
}
