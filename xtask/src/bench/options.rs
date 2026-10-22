use anyhow::{Context as _, Result, bail};
use youarel_core::bench_rules::Workload;

const SHORTCUTS: u64 = 10_000;
const FOLDERS: u64 = 100;
const SEED: u64 = 0x5ca1e;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Measure,
    DryRun,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Options {
    pub workload: Workload,
    pub mode: Mode,
}

impl Options {
    /// # Errors
    ///
    /// Refuses unknown options, missing numbers, or counts beyond the supported workload.
    pub fn from_words(words: &[&str]) -> Result<Self> {
        let (mut shortcuts, mut folders, mut seed) = (SHORTCUTS, FOLDERS, SEED);
        let mut mode = Mode::Measure;
        let mut rest = words;
        while !rest.is_empty() {
            match rest {
                ["--dry-run", tail @ ..] => {
                    mode = Mode::DryRun;
                    rest = tail;
                }
                [
                    name @ ("--shortcuts" | "--folders" | "--seed"),
                    value,
                    tail @ ..,
                ] => {
                    let number = value
                        .parse::<u64>()
                        .with_context(|| format!("{name} needs an unsigned integer"))?;
                    match *name {
                        "--shortcuts" => shortcuts = number,
                        "--folders" => folders = number,
                        _ => seed = number,
                    }
                    rest = tail;
                }
                _ => bail!("usage: bench [--shortcuts N] [--folders F] [--seed S] [--dry-run]"),
            }
        }
        let workload = Workload::new(shortcuts, folders, seed)
            .map_err(|error| anyhow::anyhow!("invalid benchmark workload: {error:?}"))?;
        Ok(Self { workload, mode })
    }
}

#[cfg(test)]
mod tests {
    use super::{Mode, Options};
    #[test]
    fn counts_and_dry_runs_validate_before_any_store_opens() {
        let options = Options::from_words(&[
            "--shortcuts",
            "128",
            "--folders",
            "0",
            "--seed",
            "17",
            "--dry-run",
        ])
        .unwrap();
        assert_eq!(
            (
                options.workload.shortcuts(),
                options.workload.folders(),
                options.workload.seed(),
                options.mode
            ),
            (128, 0, 17, Mode::DryRun)
        );
        for words in [
            &["--shortcuts"][..],
            &["--shortcuts", "0"],
            &["--seed", "-1"],
            &["--folders", "100001"],
            &["--output", "private"],
        ] {
            Options::from_words(words).unwrap_err();
        }
    }
}
