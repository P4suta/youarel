use super::raw::Timer;
use crate::fs;
use anyhow::{Result, bail};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use youarel_core::bench_rules::{Permission, Resources, Stop, permission};

pub(super) type Available = fn(&Path) -> Result<u64>;

#[derive(Debug)]
pub(super) struct Guard {
    root: PathBuf,
    files: Vec<PathBuf>,
    began: Timer,
    cancelled: Arc<AtomicBool>,
    available: Available,
    spooled: Cell<u64>,
    pub peak: Cell<u64>,
    pub stopped: Cell<Option<Stop>>,
    pub failure: RefCell<Option<String>>,
}

impl Guard {
    pub(super) fn new(root: &Path, cancelled: Arc<AtomicBool>, available: Available) -> Self {
        Self {
            root: root.to_owned(),
            files: ["source", "peer", "import", "remove"]
                .iter()
                .map(|name| root.join(name).join(youarel_store::DATABASE_FILE))
                .collect(),
            began: Timer::start(),
            cancelled,
            available,
            spooled: Cell::new(0),
            peak: Cell::new(0),
            stopped: Cell::new(None),
            failure: RefCell::new(None),
        }
    }

    fn facts(&self) -> Result<Resources> {
        let bytes = self
            .files
            .iter()
            .try_fold(self.spooled.get(), |bytes, path| {
                Ok::<_, anyhow::Error>(bytes.saturating_add(fs::file_bytes_if_present(path)?))
            })?;
        self.peak.set(self.peak.get().max(bytes));
        Ok(Resources {
            synthetic_bytes: bytes,
            available_bytes: (self.available)(&self.root)?,
            elapsed_millis: u64::try_from(self.began.elapsed().as_millis())?,
            cancelled: self.cancelled.load(Ordering::Relaxed),
        })
    }

    pub(super) fn check(&self) -> Result<()> {
        let result = self.facts().and_then(|facts| match permission(facts) {
            Permission::Continue => Ok(()),
            Permission::Stop(reason) => {
                self.stopped.set(Some(reason));
                bail!("benchmark stopped as unverified: {reason:?}")
            }
        });
        if let Err(error) = &result {
            *self.failure.borrow_mut() = Some(format!("{error:#}"));
        }
        result
    }

    pub(super) fn publication(&self) -> Result<(), youarel_store::PublicationError> {
        self.check()
            .map_err(|_stopped| youarel_store::PublicationError::Interrupted)
    }

    pub(super) fn spool(&self, count: usize) -> Result<()> {
        self.spooled.set(
            self.spooled
                .get()
                .checked_add(u64::try_from(count)?)
                .ok_or_else(|| anyhow::anyhow!("synthetic byte count overflow"))?,
        );
        self.check()
    }
}
