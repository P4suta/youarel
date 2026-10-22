#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "native benchmark timing is a measured effect outside correctness decisions"
)]

use anyhow::{Context as _, Result};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub(super) struct Timer(Instant);

impl Timer {
    pub(super) fn start() -> Self {
        Self(Instant::now())
    }
    pub(super) fn elapsed(self) -> Duration {
        self.0.elapsed()
    }
}

pub(super) fn interrupts() -> Result<Arc<AtomicBool>> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let signalled = Arc::clone(&cancelled);
    ctrlc::try_set_handler(move || signalled.store(true, Ordering::Relaxed))
        .context("install the benchmark cancellation handler")?;
    Ok(cancelled)
}
