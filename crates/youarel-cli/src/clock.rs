//! The one owner of wall and monotonic clocks, whose readings reach decisions as values.

use anyhow::{Result, anyhow};

mod raw {
    #![expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "the clock owner forwards wall and monotonic readings through these calls"
    )]

    use std::time::{Duration, Instant, SystemTime, SystemTimeError, UNIX_EPOCH};

    #[derive(Clone, Copy, Debug)]
    pub(super) struct Started(Instant);

    pub(super) fn start() -> Started {
        Started(Instant::now())
    }

    pub(super) fn elapsed(started: Started) -> Duration {
        started.0.elapsed()
    }

    pub(super) fn since_epoch() -> Result<Duration, SystemTimeError> {
        SystemTime::now().duration_since(UNIX_EPOCH)
    }
}

/// A process-local monotonic duration, independent of wall-clock corrections.
#[derive(Clone, Copy, Debug)]
pub struct Stopwatch(raw::Started);

impl Stopwatch {
    /// Starts measuring elapsed time at this instant.
    #[must_use]
    pub fn start() -> Self {
        Self(raw::start())
    }

    /// The elapsed milliseconds.
    ///
    /// # Errors
    ///
    /// Returns an error when the duration exceeds what 64-bit milliseconds hold.
    pub fn elapsed(self) -> Result<u64> {
        u64::try_from(raw::elapsed(self.0).as_millis())
            .map_err(|error| anyhow!("the duration exceeds 64-bit milliseconds: {error}"))
    }
}

/// The time in milliseconds since the Unix epoch.
///
/// # Errors
///
/// Returns an error when the clock reads before 1970 or beyond what milliseconds in 64 bits hold.
pub fn now() -> Result<u64> {
    let since =
        raw::since_epoch().map_err(|error| anyhow!("the clock reads before 1970: {error}"))?;
    u64::try_from(since.as_millis())
        .map_err(|error| anyhow!("the clock reads beyond 64-bit milliseconds: {error}"))
}

#[cfg(test)]
mod tests {
    use super::now;

    #[test]
    fn the_clock_reads_after_2020() {
        assert!(now().unwrap() > 1_577_836_800_000);
    }
}
