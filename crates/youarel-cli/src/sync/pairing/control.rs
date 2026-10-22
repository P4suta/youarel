use anyhow::{Context as _, Result};
use std::{
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    time::Duration,
};
use youarel_core::device_rules::{Event, Role, Run, Stage};

#[derive(Clone, Debug)]
pub struct Control {
    run: Arc<Mutex<Run>>,
    began: u64,
    elapsed: crate::clock::Stopwatch,
    wake: Arc<Condvar>,
}

impl Control {
    #[must_use]
    pub fn new(role: Role, now: u64) -> Self {
        Self {
            run: Arc::new(Mutex::new(Run::begin(role, now))),
            began: now,
            elapsed: crate::clock::Stopwatch::start(),
            wake: Arc::new(Condvar::new()),
        }
    }

    fn locked(&self) -> MutexGuard<'_, Run> {
        self.run.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[must_use]
    pub fn stage(&self) -> Stage {
        self.locked().stage()
    }

    pub fn event(&self, event: Event, now: u64) {
        let mut run = self.locked();
        *run = run.step(event, now);
        drop(run);
        self.wake.notify_all();
    }

    /// # Errors
    ///
    /// Reports poisoned progress coordination while waiting for cancellation or the next transport poll.
    pub fn wait(&self) -> Result<()> {
        let run = self.locked();
        if run.stage().terminal() {
            drop(run);
            return Ok(());
        }
        self.wake
            .wait_timeout(
                run,
                Duration::from_millis(youarel_core::device_rules::POLL_MILLIS),
            )
            .map(|(guard, _elapsed)| drop(guard))
            .map_err(|_poisoned| anyhow::anyhow!("The pairing progress lock failed."))
    }

    /// # Errors
    ///
    /// Reports an unrepresentable monotonic timestamp.
    pub fn refuse(&self) -> Result<()> {
        let now = self
            .began
            .checked_add(self.elapsed.elapsed()?)
            .context("The pairing clock overflowed.")?;
        self.event(Event::Refuse, now);
        Ok(())
    }

    pub(super) fn local<T>(
        &self,
        (stage, event): (Stage, Event),
        now: u64,
        work: impl FnOnce() -> Result<T>,
    ) -> Result<Option<T>> {
        let observed = self
            .began
            .checked_add(self.elapsed.elapsed()?)
            .context("The pairing clock overflowed.")?;
        let now = now.max(observed);
        let mut run = self.locked();
        *run = run.step(Event::Poll, now);
        if run.stage() != stage || run.stage().terminal() {
            return Ok(None);
        }
        match work() {
            Ok(value) => {
                *run = run.step(event, now);
                Ok(Some(value))
            }
            Err(error) => {
                *run = run.step(Event::Refuse, now);
                drop(run);
                Err(error)
            }
        }
    }
}
