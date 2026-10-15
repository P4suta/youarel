//! A rule whose specification still runs it but no longer asserts anything, so every mutant of the rule survives.

/// Whether one more use fits below `limit`.
#[must_use]
pub const fn admits(limit: u32, used: u32) -> bool {
    used < limit
}

/// The specification of `admits` after someone removed its assertion.
#[cfg(test)]
fn spec_admits(limit: u32, used: u32) {
    let _ = admits(limit, used);
}

#[cfg(test)]
mod tests {
    const DOMAIN: u32 = 4;

    #[test]
    fn the_specification_runs_over_its_domain() {
        for limit in 0..DOMAIN {
            for used in 0..DOMAIN {
                super::spec_admits(limit, used);
            }
        }
    }
}
