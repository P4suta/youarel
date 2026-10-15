//! A sequence of named steps that runs every step and then reports each failure together.

use anyhow::{Result, ensure};

/// The outcome of the steps run so far.
#[derive(Debug, Default)]
pub struct Report {
    failures: Vec<String>,
}

impl Report {
    /// Runs one step, recording a failure without stopping the later steps.
    pub fn stage(&mut self, name: &str, run: impl FnOnce() -> Result<()>) {
        eprintln!("==> {name}");
        match run() {
            Ok(()) => {}
            Err(error) => {
                eprintln!("{name}: {error:#}");
                self.failures.push(name.to_owned());
            }
        }
    }

    /// The names of the steps that failed.
    #[must_use]
    pub fn failures(&self) -> &[String] {
        &self.failures
    }

    /// Succeeds when every step succeeded, and otherwise names each failed step.
    ///
    /// # Errors
    ///
    /// Returns an error listing the failed steps.
    pub fn finish(self, what: &str) -> Result<()> {
        ensure!(
            self.failures.is_empty(),
            "{what} failed: {}",
            self.failures.join(", ")
        );
        eprintln!("{what}: clean");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Report;
    use anyhow::anyhow;

    #[test]
    fn every_step_runs_and_each_failure_is_named() {
        let mut report = Report::default();
        let mut ran = Vec::new();
        report.stage("first", || {
            ran.push("first");
            Err(anyhow!("broken"))
        });
        report.stage("second", || {
            ran.push("second");
            Ok(())
        });
        assert_eq!(ran, ["first", "second"]);
        assert_eq!(report.failures(), ["first"]);
        let error = report.finish("steps").unwrap_err();
        assert_eq!(error.to_string(), "steps failed: first");
        Report::default().finish("steps").unwrap();
    }
}
