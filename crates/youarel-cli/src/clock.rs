//! The one owner of the wall clock, which the shell reads once per action and passes to every decision as a value.

use anyhow::{Result, anyhow};

mod raw {
    #![expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "the shell's one clock owner reads the wall clock in a single statement"
    )]

    use std::time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH};

    pub(super) fn since_epoch() -> Result<Duration, SystemTimeError> {
        SystemTime::now().duration_since(UNIX_EPOCH)
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
