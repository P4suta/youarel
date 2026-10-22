//! Local cache lookup, shared by the command line and bounded window work.

use crate::browsers::cache::{CachePath, Discovery};
use crate::icons::{identify, keep};
use anyhow::{Result, ensure};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use youarel_core::icon_rules::{HOSTS_PER_TICK, MOST_PROFILES, SIDE};
use youarel_import::icons::{Image, Snapshot};
use youarel_link::Link;
use youarel_store::{Store, fs};

const MOST_SNAPSHOT_BYTES: u64 = 512 * 1024 * 1024;
const MOST_ATTEMPTED_HOSTS: usize = 4096;
const SAFARI_IMAGES: &str = "favicons";
const MOST_PROBLEMS: usize = 32;

#[derive(Debug)]
enum Held {
    Unread,
    Ready(Snapshot),
    Failed,
}

#[derive(Debug)]
struct Cache {
    source: CachePath,
    held: Held,
}

impl Cache {
    fn candidates(
        &mut self,
        link: &Link,
        cancelled: &Arc<AtomicBool>,
        bytes: &mut u64,
    ) -> Result<Vec<youarel_import::icons::Candidate>> {
        if matches!(self.held, Held::Unread) {
            self.held = Held::Failed;
            let snapshot =
                Snapshot::open(&self.source.path, self.source.format, Arc::clone(cancelled))?;
            ensure!(
                bytes.saturating_add(snapshot.storage_bytes()) <= MOST_SNAPSHOT_BYTES,
                "browser icon snapshots pass the local storage limit"
            );
            *bytes = bytes.saturating_add(snapshot.storage_bytes());
            self.held = Held::Ready(snapshot);
        }
        let result = match &self.held {
            Held::Ready(snapshot) => snapshot.candidates(link),
            Held::Unread | Held::Failed => Ok(Vec::new()),
        };
        if result.is_err() {
            if let Held::Ready(snapshot) = &self.held {
                *bytes = bytes.saturating_sub(snapshot.storage_bytes());
            }
            self.held = Held::Failed;
        }
        Ok(result?)
    }

    fn image(&self, image: Image) -> Result<Vec<u8>> {
        let bytes = match image {
            Image::Bytes(bytes) => bytes,
            Image::SafariFile(name) => {
                let parent = self
                    .source
                    .path
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("the Safari cache has no parent"))?;
                let limit = u64::try_from(youarel_icon::LONGEST_SOURCE)?;
                fs::read_bounded(&parent.join(SAFARI_IMAGES).join(name), limit)?
            }
        };
        Ok(youarel_icon::normalize(&bytes)?)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Report {
    pub kept: usize,
    pub examined: usize,
    pub problems: Vec<String>,
    pub failures: usize,
}

impl Report {
    pub fn join(&mut self, other: Self) {
        self.kept = self.kept.saturating_add(other.kept);
        self.examined = self.examined.saturating_add(other.examined);
        self.failures = self.failures.saturating_add(other.failures);
        let room = MOST_PROBLEMS.saturating_sub(self.problems.len());
        self.problems.extend(other.problems.into_iter().take(room));
    }

    #[must_use]
    pub fn lines(self) -> Vec<String> {
        let mut lines = vec![format!(
            "Kept {} local icons after checking {} hosts; {} cache problems",
            self.kept, self.examined, self.failures
        )];
        lines.extend(self.problems);
        lines
    }
}

#[derive(Debug)]
pub struct Caches {
    sources: Vec<Cache>,
    problems: Vec<String>,
    attempted: BTreeSet<String>,
    cancelled: Arc<AtomicBool>,
    bytes: u64,
}

impl Caches {
    #[must_use]
    pub fn available(&self) -> bool {
        self.sources
            .iter()
            .any(|cache| !matches!(cache.held, Held::Failed))
    }

    #[must_use]
    pub fn new(mut discovery: Discovery, cancelled: Arc<AtomicBool>) -> Self {
        if discovery.caches.len() > MOST_PROFILES {
            discovery.problems.push(format!(
                "only the first {MOST_PROFILES} browser icon caches fit this pass"
            ));
            discovery.caches.truncate(MOST_PROFILES);
        }
        Self {
            sources: discovery
                .caches
                .into_iter()
                .map(|source| Cache {
                    source,
                    held: Held::Unread,
                })
                .collect(),
            problems: discovery.problems,
            attempted: BTreeSet::new(),
            cancelled,
            bytes: 0,
        }
    }

    /// Fills a bounded batch of missing hosts without contacting any site.
    ///
    /// # Errors
    ///
    /// Refuses an excessive batch, cancellation, or an unreadable library.
    pub fn fill(&mut self, store: &Store, now: u64, links: &[Link]) -> Result<Report> {
        ensure!(
            links.len() <= HOSTS_PER_TICK,
            "too many icon links in one tick"
        );
        let mut report = Report {
            problems: std::mem::take(&mut self.problems),
            ..Report::default()
        };
        for link in links {
            ensure!(
                !self.cancelled.load(Ordering::Relaxed),
                "the local icon lookup was cancelled"
            );
            if self.attempted.contains(link.host()) || !crate::icons::lacking(store, now, link)? {
                continue;
            }
            report.examined = report.examined.saturating_add(1);
            let image = self.find(link, &mut report.problems);
            ensure!(
                !self.cancelled.load(Ordering::Relaxed),
                "the local icon lookup was cancelled"
            );
            if let Some(image) = image {
                let kept = store.session(now, |session| {
                    if session.icon(link.host())?.is_some() {
                        return Ok(false);
                    }
                    keep(session, link, (identify(&image), image))?;
                    Ok(true)
                })?;
                if kept {
                    report.kept = report.kept.saturating_add(1);
                }
            }
            if self.attempted.len() < MOST_ATTEMPTED_HOSTS {
                self.attempted.insert(link.host().to_owned());
            }
        }
        report.failures = report.problems.len();
        report.problems.truncate(MOST_PROBLEMS);
        Ok(report)
    }

    fn find(&mut self, link: &Link, problems: &mut Vec<String>) -> Option<Vec<u8>> {
        let mut best = None;
        for cache in &mut self.sources {
            if self.cancelled.load(Ordering::Relaxed) {
                break;
            }
            let candidates = match cache.candidates(link, &self.cancelled, &mut self.bytes) {
                Ok(candidates) => candidates,
                Err(error) => {
                    problems.push(format!("{}: {error}", cache.source.path.display()));
                    continue;
                }
            };
            for candidate in candidates {
                if best
                    .as_ref()
                    .is_some_and(|(held, _)| *held >= candidate.quality)
                {
                    continue;
                }
                match cache.image(candidate.image) {
                    Ok(image) => {
                        if Some(candidate.quality) == youarel_core::icon_rules::quality(SIDE, SIDE)
                        {
                            return Some(image);
                        }
                        best = Some((candidate.quality, image));
                    }
                    Err(error) => problems.push(format!(
                        "{}: no usable cached image: {error}",
                        cache.source.path.display()
                    )),
                }
            }
        }
        best.map(|(_, image)| image)
    }
}

/// Fills every missing live host through bounded store ranges and local snapshots.
///
/// # Errors
///
/// Refuses cancellation or a store failure; individual cache failures stay in the report.
pub fn fill_all(
    store: &Store,
    now: u64,
    discovery: Discovery,
    cancelled: Arc<AtomicBool>,
) -> Result<Report> {
    let mut caches = Caches::new(discovery, cancelled);
    if !caches.available() {
        return caches.fill(store, now, &[]);
    }
    let mut after = None;
    let mut total = Report::default();
    loop {
        let batch = store.session(now, |session| session.icon_batch(after))?;
        let report = caches.fill(store, now, &batch.links)?;
        total.join(report);
        if !caches.available() {
            return Ok(total);
        }
        if !batch.more {
            return Ok(total);
        }
        after = batch.after;
    }
}
