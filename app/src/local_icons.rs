//! Missing-icon work outside the library lock, with visible entries ahead of the background scan.

use crate::view::{ColumnEntry, EntryKind, RowView, View};
use anyhow::{Result, ensure};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use youarel_cli::browsers::cache::{self, Discovery};
use youarel_cli::icons::local::{Caches, Report};
use youarel_core::icon_rules::HOSTS_PER_TICK;
use youarel_core::state::ItemId;
use youarel_link::Link;
use youarel_store::{Store, StoreError};

const MOST_PENDING: usize = 256;

#[derive(Debug, Default)]
struct Work {
    caches: Option<Caches>,
    after: Option<ItemId>,
    done: bool,
}

#[derive(Debug)]
pub struct LocalIcons {
    store: Store,
    pending: Mutex<VecDeque<ItemId>>,
    work: Mutex<Work>,
    busy: AtomicBool,
    reset: AtomicBool,
    cancelled: Arc<AtomicBool>,
    discovery: Option<Discovery>,
    preferred: Mutex<Option<youarel_core::browser_rules::Browser>>,
}

struct Flight<'a>(&'a AtomicBool);

impl Drop for Flight<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl LocalIcons {
    #[must_use]
    pub fn new(store: Store) -> Self {
        Self {
            store,
            pending: Mutex::default(),
            work: Mutex::default(),
            busy: AtomicBool::new(false),
            reset: AtomicBool::new(false),
            cancelled: Arc::new(AtomicBool::new(false)),
            discovery: None,
            preferred: Mutex::default(),
        }
    }

    fn queue<'a>(&self, ids: impl Iterator<Item = &'a str>) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        for text in ids.take(MOST_PENDING) {
            let id = match youarel_store::codec::unhex(text) {
                Ok(bytes) => ItemId(bytes),
                Err(error) => {
                    eprintln!("youarel-launcher: cannot queue an icon: {error}");
                    continue;
                }
            };
            if pending.len() >= MOST_PENDING {
                break;
            }
            if !pending.contains(&id) {
                pending.push_back(id);
            }
        }
    }

    pub fn queue_rows(&self, rows: &[RowView]) {
        self.queue(rows.iter().filter_map(|row| match row {
            RowView::Shortcut { item, .. } if item.icon.is_none() => Some(item.id.as_str()),
            RowView::Folder { .. } | RowView::Shortcut { .. } | RowView::Gone => None,
        }));
    }

    pub fn queue_columns(&self, entries: &[ColumnEntry]) {
        self.queue(
            entries
                .iter()
                .filter(|entry| entry.kind == EntryKind::Shortcut && entry.icon.is_none())
                .map(|entry| entry.id.as_str()),
        );
    }

    pub fn queue_view(&self, view: &View) {
        self.queue_rows(&view.listing.rows);
        for column in &view.columns {
            self.queue_columns(&column.entries);
        }
        if let Some(item) = &view.preview {
            self.queue(std::iter::once(item.id.as_str()));
        }
    }

    pub fn restart(&self, browser: Option<youarel_core::browser_rules::Browser>) {
        *self
            .preferred
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = browser;
        self.reset.store(true, Ordering::Release);
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn reserve(&self) -> bool {
        self.busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    fn links(&self, now: u64, work: &mut Work) -> Result<Vec<Link>> {
        let ids: Vec<ItemId> = {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            let count = pending.len().min(HOSTS_PER_TICK);
            pending.drain(..count).collect()
        };
        if !ids.is_empty() {
            return Ok(self.store.session(now, |session| {
                let mut links = Vec::new();
                for id in ids {
                    let Some(item) = session.item(id)? else {
                        continue;
                    };
                    links.push(
                        Link::parse(&item.link.value)
                            .map_err(|_invalid| StoreError::Missing("an admitted link"))?,
                    );
                }
                Ok(links)
            })?);
        }
        if work.done {
            return Ok(Vec::new());
        }
        let batch = self
            .store
            .session(now, |session| session.icon_batch(work.after))?;
        work.after = batch.after;
        work.done = !batch.more;
        Ok(batch.links)
    }

    /// Runs one reserved batch, acquiring no library lock and releasing the reservation on every return.
    ///
    /// # Errors
    ///
    /// Refuses cancellation or a store failure; individual cache failures remain in its report.
    pub fn tick(&self, now: u64) -> Result<Report> {
        let _flight = Flight(&self.busy);
        ensure!(
            !self.cancelled.load(Ordering::Acquire),
            "the local icon lookup was cancelled"
        );
        let mut work = self.work.lock().unwrap_or_else(PoisonError::into_inner);
        if self.reset.swap(false, Ordering::AcqRel) {
            *work = Work::default();
        }
        if work.caches.is_none() {
            let preferred = *self
                .preferred
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let discovery = match &self.discovery {
                Some(discovery) => discovery.clone(),
                None => match preferred {
                    Some(browser) => cache::discover_browser(browser)?,
                    None => cache::discover()?,
                },
            };
            work.caches = Some(Caches::new(discovery, Arc::clone(&self.cancelled)));
        }
        let links = self.links(now, &mut work)?;
        let caches = work
            .caches
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the local icon reader is missing"))?;
        let report = caches.fill(&self.store, now, &links)?;
        if !caches.available() {
            work.done = true;
        }
        drop(work);
        Ok(report)
    }
}

#[cfg(test)]
mod tests;
