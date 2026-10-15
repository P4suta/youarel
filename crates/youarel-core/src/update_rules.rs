//! Whether an offered release installs: forward past every version the device reached, and back only with consent.
//!
//! A device records its mark, the highest version it has run or installed, and the mark never falls.
//! A release replayed from the past then lies below the mark, so it cannot undo a fix without the person asking to go back.

use core::fmt;

/// The radix of a version number.
const DECIMAL: u8 = 10;

/// A release version: its major, minor, and patch numbers, which order in that sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// The version `text` spells as `major.minor.patch`, each a decimal number without a sign or a leading zero, or `None`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let version = Self {
            major: number(parts.next()?)?,
            minor: number(parts.next()?)?,
            patch: number(parts.next()?)?,
        };
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A decimal number without a sign or a leading zero that fits 32 bits.
fn number(text: &str) -> Option<u32> {
    match text.as_bytes() {
        [] | [b'0', _, ..] => None,
        digits => digits.iter().try_fold(0_u32, |value, symbol| {
            let digit = symbol.checked_sub(b'0').filter(|digit| *digit < DECIMAL)?;
            value
                .checked_mul(u32::from(DECIMAL))?
                .checked_add(u32::from(digit))
        }),
    }
}

/// Whether the person allowed an update to go back.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Consent {
    /// Forward only, which is the default.
    Forward,
    /// Back as well, which `--allow-downgrade` grants.
    Downgrade,
}

/// What an offered release comes to.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Verdict {
    /// Install it.
    Install,
    /// It is the running version already.
    Current,
    /// It lies below the mark, which the refusal names.
    Refuse { mark: Version },
}

/// The mark of a device that runs `running`, given the mark it recorded, if any.
#[must_use]
pub fn reached(mark: Option<Version>, running: Version) -> Version {
    mark.map_or(running, |mark| mark.max(running))
}

/// The verdict on `offered` for a device that runs `running` and recorded `mark`.
#[must_use]
pub fn decide(
    running: Version,
    mark: Option<Version>,
    offered: Version,
    consent: Consent,
) -> Verdict {
    let highest = reached(mark, running);
    if offered > running && offered >= highest {
        return Verdict::Install;
    }
    if offered == running {
        return Verdict::Current;
    }
    match consent {
        Consent::Downgrade => Verdict::Install,
        Consent::Forward => Verdict::Refuse { mark: highest },
    }
}

/// The properties Kani proves over every running version, mark, offer, and consent.
#[cfg(any(test, kani))]
mod proofs {
    use super::{Consent, Verdict, Version, decide, reached};

    /// What `decide` and `reached` promise.
    ///
    /// The mark covers the running and the recorded versions and is one of them; without consent nothing below it installs; a newer release always installs; the running version never reinstalls; consent installs any other release; and the mark after an install covers it.
    pub(super) fn spec(
        running: Version,
        mark: Option<Version>,
        offered: Version,
        consent: Consent,
    ) -> bool {
        let highest = reached(mark, running);
        let verdict = decide(running, mark, offered, consent);
        let covers = highest >= running
            && mark.is_none_or(|mark| highest >= mark)
            && (highest == running || Some(highest) == mark);
        let forward = consent == Consent::Downgrade
            || verdict != Verdict::Install
            || (offered > running && offered >= highest);
        let refusal = match verdict {
            Verdict::Refuse { mark: named } => {
                consent == Consent::Forward
                    && named == highest
                    && offered < highest
                    && offered != running
            }
            Verdict::Install | Verdict::Current => true,
        };
        let newer = !(offered > running && offered >= highest) || verdict == Verdict::Install;
        let current = (offered == running) == (verdict == Verdict::Current);
        let consented =
            consent == Consent::Forward || offered == running || verdict == Verdict::Install;
        let raised = reached(Some(highest), offered);
        let never_falls = raised >= highest && raised >= offered;
        covers && forward && refusal && newer && current && consented && never_falls
    }

    #[cfg(kani)]
    fn any_version() -> Version {
        Version {
            major: kani::any(),
            minor: kani::any(),
            patch: kani::any(),
        }
    }

    #[cfg(kani)]
    #[kani::proof]
    fn an_update_never_lands_below_the_mark_without_consent() {
        let running = any_version();
        let mark = if kani::any() {
            Some(any_version())
        } else {
            None
        };
        let offered = any_version();
        let consent = if kani::any() {
            Consent::Forward
        } else {
            Consent::Downgrade
        };
        kani::assert(
            spec(running, mark, offered, consent),
            "updates go forward past the mark",
        );
        kani::cover!(
            matches!(
                decide(running, mark, offered, consent),
                Verdict::Refuse { .. }
            ) && offered > running,
            "a replayed release above the running one is refused"
        );
    }

    /// A release below the mark that installs without consent would undo a fix; this probe claims one does and must fail.
    #[cfg(kani)]
    #[kani::proof]
    fn rejects_a_replayed_release() {
        let version = |minor| Version {
            major: 1,
            minor,
            patch: 0,
        };
        kani::assert(
            decide(version(0), Some(version(2)), version(1), Consent::Forward) == Verdict::Install,
            "NEGATIVE_UPDATE_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::proofs::spec;
    use super::{Consent, Verdict, Version, decide, reached};
    use alloc::string::ToString;

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn versions_parse_strictly_and_print_back() {
        assert_eq!(
            Version::parse("1.20.3"),
            Some(Version {
                major: 1,
                minor: 20,
                patch: 3
            })
        );
        assert_eq!(version("0.0.0").to_string(), "0.0.0");
        assert_eq!(version("4294967295.0.10").to_string(), "4294967295.0.10");
        for text in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.02.3",
            "+1.2.3",
            "1.2.-3",
            "1.2.x",
            "1..3",
            " 1.2.3",
            "4294967296.0.0",
            "1.:.3",
            "1.2./",
        ] {
            assert_eq!(Version::parse(text), None, "{text:?}");
        }
        assert!(version("0.10.0") > version("0.9.9"));
        assert!(version("2.0.0") > version("1.99.99"));
    }

    #[test]
    fn updates_go_forward_and_back_only_with_consent() {
        let (one, two, three) = (version("1.0.0"), version("2.0.0"), version("3.0.0"));
        assert_eq!(decide(one, None, two, Consent::Forward), Verdict::Install);
        assert_eq!(decide(two, None, two, Consent::Downgrade), Verdict::Current);
        assert_eq!(
            decide(two, None, one, Consent::Forward),
            Verdict::Refuse { mark: two }
        );
        assert_eq!(decide(two, None, one, Consent::Downgrade), Verdict::Install);
        assert_eq!(
            decide(one, Some(three), two, Consent::Forward),
            Verdict::Refuse { mark: three }
        );
        assert_eq!(
            decide(one, Some(three), three, Consent::Forward),
            Verdict::Install
        );
        assert_eq!(
            decide(two, Some(one), three, Consent::Forward),
            Verdict::Install
        );
        assert_eq!(reached(Some(one), two), two);
        assert_eq!(reached(Some(three), two), three);
        assert_eq!(reached(None, one), one);
    }

    #[test]
    fn the_specification_agrees_over_the_edges() {
        let versions = [
            "0.0.0",
            "0.0.1",
            "0.1.0",
            "1.0.0",
            "1.0.1",
            "4294967295.4294967295.4294967295",
        ]
        .map(version);
        for running in versions {
            for mark in versions.map(Some).into_iter().chain([None]) {
                for offered in versions {
                    for consent in [Consent::Forward, Consent::Downgrade] {
                        assert!(
                            spec(running, mark, offered, consent),
                            "{running} {mark:?} {offered} {consent:?}"
                        );
                    }
                }
            }
        }
    }
}
