//! Required verification stays nonempty and preserves every affected contract.

/// A hosted verification job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Job {
    Check,
    Kani,
    Quint,
    Proverif,
    Miri,
    Fuzz,
    Mutants,
    App,
    Macos,
    Windows,
}

const CHECK_BIT: u16 = 1 << 0;
const KANI_BIT: u16 = 1 << 1;
const QUINT_BIT: u16 = 1 << 2;
const PROVERIF_BIT: u16 = 1 << 3;
const MIRI_BIT: u16 = 1 << 4;
const FUZZ_BIT: u16 = 1 << 5;
const MUTANTS_BIT: u16 = 1 << 6;
const APP_BIT: u16 = 1 << 7;
const MACOS_BIT: u16 = 1 << 8;
const WINDOWS_BIT: u16 = 1 << 9;

impl Job {
    const fn bit(self) -> u16 {
        match self {
            Self::Check => CHECK_BIT,
            Self::Kani => KANI_BIT,
            Self::Quint => QUINT_BIT,
            Self::Proverif => PROVERIF_BIT,
            Self::Miri => MIRI_BIT,
            Self::Fuzz => FUZZ_BIT,
            Self::Mutants => MUTANTS_BIT,
            Self::App => APP_BIT,
            Self::Macos => MACOS_BIT,
            Self::Windows => WINDOWS_BIT,
        }
    }
    pub const ALL: [Self; 10] = [
        Self::Check,
        Self::Kani,
        Self::Quint,
        Self::Proverif,
        Self::Miri,
        Self::Fuzz,
        Self::Mutants,
        Self::App,
        Self::Macos,
        Self::Windows,
    ];
}

const ALL_JOBS: u16 = (1 << Job::ALL.len()) - 1;

/// A set whose only admitted bits name known jobs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Selection(u16);

impl Selection {
    #[must_use]
    pub const fn all() -> Self {
        Self(ALL_JOBS)
    }

    #[must_use]
    pub const fn with(self, job: Job) -> Self {
        Self(self.0 | job.bit())
    }

    #[must_use]
    pub const fn includes(self, job: Job) -> bool {
        self.0 & job.bit() != 0
    }
}

/// Whether the adapter established a complete affected-input inventory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Confidence {
    Scoped,
    Conservative,
}

/// Heavy assurance applies to a candidate or an explicit experiment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Development,
    Candidate,
}

const FAST_JOBS: u16 = Job::Check.bit() | Job::App.bit();

/// A selected job must finish successfully; a safety stop provides no verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Success,
    Failure,
    Cancelled,
    Skipped,
    Incomplete,
}

#[must_use]
pub const fn select(affected: Selection, confidence: Confidence, stage: Stage) -> Selection {
    let selected = match confidence {
        Confidence::Scoped => affected.with(Job::Check),
        Confidence::Conservative => Selection::all(),
    };
    match stage {
        Stage::Development => Selection(selected.0 & FAST_JOBS),
        Stage::Candidate => selected,
    }
}

#[must_use]
pub const fn accepts(selected: Selection, job: Job, outcome: Outcome) -> bool {
    matches!(outcome, Outcome::Success)
        || (!selected.includes(job) && matches!(outcome, Outcome::Skipped))
}

/// Whether a byte preserves an output line and a quoted Docker mount field.
#[must_use]
pub const fn cache_field_byte(byte: u8) -> bool {
    !matches!(byte, b'\r' | b'\n' | b'"')
}

#[cfg(kani)]
mod proofs {
    use super::*;

    #[kani::proof]
    fn cache_fields_preserve_line_and_quote_boundaries() {
        let byte: u8 = kani::any();
        if cache_field_byte(byte) {
            assert_ne!(byte, b'\r');
            assert_ne!(byte, b'\n');
            assert_ne!(byte, b'"');
        } else {
            assert!(matches!(byte, b'\r' | b'\n' | b'"'));
        }
        kani::cover!(cache_field_byte(byte));
        kani::cover!(!cache_field_byte(byte));
    }

    #[kani::proof]
    fn rejects_a_quoted_cache_field() {
        kani::assert(cache_field_byte(b'"'), "NEGATIVE_CACHE_FIELD_CONTROL");
    }

    #[kani::proof]
    fn selection_preserves_affected_jobs_and_requires_a_check() {
        let bits: u16 = kani::any();
        let affected = Selection(bits & ALL_JOBS);
        let selected = select(affected, Confidence::Scoped, Stage::Candidate);
        assert_eq!(selected.0 & affected.0, affected.0);
        assert!(selected.includes(Job::Check));
        assert_eq!(
            select(affected, Confidence::Conservative, Stage::Candidate),
            Selection::all()
        );
        kani::cover!(affected == Selection::default());
        kani::cover!(affected == Selection::all());
    }

    #[kani::proof]
    fn development_requires_fast_work_without_a_campaign() {
        let bits: u16 = kani::any();
        let affected = Selection(bits & ALL_JOBS);
        let selected = select(affected, Confidence::Scoped, Stage::Development);
        assert_eq!(selected.0 & !FAST_JOBS, 0);
        assert_eq!(selected.0 & affected.0 & FAST_JOBS, affected.0 & FAST_JOBS);
        assert!(selected.includes(Job::Check));
        assert_eq!(
            select(affected, Confidence::Conservative, Stage::Development).0,
            FAST_JOBS
        );
        kani::cover!(affected == Selection::all());
    }

    #[kani::proof]
    fn a_selected_job_only_accepts_success() {
        let job = kani::any::<u8>();
        kani::assume(usize::from(job) < Job::ALL.len());
        if let Some(&job) = Job::ALL.get(usize::from(job)) {
            let selected = Selection::default().with(job);
            assert!(accepts(selected, job, Outcome::Success));
            assert!(!accepts(selected, job, Outcome::Skipped));
            assert!(!accepts(selected, job, Outcome::Failure));
            assert!(!accepts(selected, job, Outcome::Cancelled));
            assert!(!accepts(selected, job, Outcome::Incomplete));
            kani::cover!(job == Job::Check);
            kani::cover!(job == Job::Windows);
        }
    }

    #[kani::proof]
    fn rejects_a_skipped_required_job() {
        kani::assert(
            accepts(Selection::all(), Job::Check, Outcome::Skipped),
            "NEGATIVE_VERIFICATION_CONTROL",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_work_does_not_require_unrelated_proofs() {
        let selected = select(
            Selection::default().with(Job::App),
            Confidence::Scoped,
            Stage::Development,
        );
        assert!(selected.includes(Job::Check));
        assert!(selected.includes(Job::App));
        assert!(!selected.includes(Job::Proverif));
        assert!(!selected.includes(Job::Mutants));
    }

    #[test]
    fn uncertainty_requires_the_complete_inventory() {
        assert_eq!(
            select(
                Selection::default(),
                Confidence::Conservative,
                Stage::Candidate
            ),
            Selection::all()
        );
    }

    #[test]
    fn ordinary_changes_never_require_a_heavy_campaign() {
        for confidence in [Confidence::Scoped, Confidence::Conservative] {
            let selected = select(Selection::all(), confidence, Stage::Development);
            assert!(selected.includes(Job::Check));
            assert!(selected.includes(Job::App));
            for job in [
                Job::Kani,
                Job::Quint,
                Job::Proverif,
                Job::Miri,
                Job::Fuzz,
                Job::Mutants,
                Job::Macos,
                Job::Windows,
            ] {
                assert!(!selected.includes(job));
            }
        }
    }

    #[test]
    fn only_unselected_jobs_may_skip() {
        let selected = select(Selection::default(), Confidence::Scoped, Stage::Development);
        assert!(!accepts(selected, Job::Check, Outcome::Skipped));
        for outcome in [Outcome::Failure, Outcome::Cancelled, Outcome::Incomplete] {
            assert!(!accepts(selected, Job::Check, outcome));
            assert!(!accepts(selected, Job::App, outcome));
        }
    }
}
