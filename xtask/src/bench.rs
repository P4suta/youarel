//! Reproducible native measurements in isolated stores, outside correctness gates.

mod data;
mod options;
pub use options::{Mode, Options};
mod build;
mod guard;
mod raw;
mod report;
pub use report::{Configuration, Measurement, Report, Status};

mod edits;
mod reads;
mod segments;

use crate::fs;
use anyhow::{Context as _, Result, ensure};
use data::{Generator, NOW, SITES};
use guard::{Available, Guard};
use raw::Timer;
use sha2::{Digest as _, Sha512_256};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicBool};
use youarel_core::bench_rules::Namespace;
use youarel_core::record_rules::Record;
use youarel_core::state::ItemId;
use youarel_store::{RecordSet, Store};

const GENERATION_BATCH: u64 = 512;
const BUDGET_CHECK_RECORDS: usize = 256;
const OUTPUT_DIRECTORY: &str = "target/bench";
const REPORT_FILE: &str = "report.json";
const LOCK_FILE: &str = "run.lock";
const ICON_SIDE: u32 = 64;

#[derive(Debug)]
struct Campaign {
    root: PathBuf,
    options: Options,
    guard: Guard,
    report: RefCell<Report>,
}

impl Campaign {
    fn new(
        root: &Path,
        options: Options,
        cancelled: Arc<AtomicBool>,
        available: Available,
    ) -> Self {
        Self {
            root: root.to_owned(),
            options,
            guard: Guard::new(root, cancelled, available),
            report: RefCell::new(Report::new(options)),
        }
    }

    fn store(&self, name: &str) -> Store {
        Store::new(self.root.join(name))
    }

    fn measure<T>(
        &self,
        mut measurement: Measurement,
        work: impl FnOnce(&mut Measurement) -> Result<T>,
    ) -> Result<T> {
        self.guard.check()?;
        let began = Timer::start();
        let result = work(&mut measurement);
        measurement.elapsed_ns = began.elapsed().as_nanos();
        measurement.completed = result.is_ok();
        self.report.borrow_mut().measurements.push(measurement);
        let value = result?;
        self.guard.check()?;
        Ok(value)
    }

    fn populate(&self) -> Result<Vec<ItemId>> {
        let mut generator = Generator::new(self.options.workload.seed());
        let mut fingerprint = Sha512_256::new();
        let folders = self.populate_folders(&mut generator, &mut fingerprint)?;
        let source = self.store("source");
        for first in
            (1..=self.options.workload.shortcuts()).step_by(usize::try_from(GENERATION_BATCH)?)
        {
            self.guard.check()?;
            let until = first
                .saturating_add(GENERATION_BATCH)
                .min(self.options.workload.shortcuts().saturating_add(1));
            let mut batch = Vec::new();
            for number in first..until {
                let folder = if folders.is_empty() || generator.pick(data::ROOT_WEIGHT)? == 0 {
                    None
                } else {
                    folders.get(generator.pick(folders.len())?).copied()
                };
                let record = generator.shortcut(Namespace::Shortcut, number, folder)?;
                fingerprint.update(data::bytes(&record)?);
                batch.push(record);
            }
            source.session(NOW, |session| {
                for record in batch {
                    session.merge_record(record)?;
                }
                Ok(())
            })?;
        }
        let image = crate::icons::png(ICON_SIDE)?;
        let icon = youarel_store::icons::IconId(Sha512_256::digest(&image).into());
        source.session(NOW, |session| {
            for site in 0..SITES {
                session.keep_icon(&data::site_host(site), icon, &image)?;
            }
            Ok(())
        })?;
        let mut report = self.report.borrow_mut();
        report.fixture_digest = crate::hex(&fingerprint.finalize());
        report.purged =
            self.options.workload.shortcuts() / youarel_core::bench_rules::PURGED_PERIOD;
        report.icon_sites = SITES;
        report.unique_icon_images = 1;
        Ok(folders)
    }

    fn populate_folders(
        &self,
        generator: &mut Generator,
        fingerprint: &mut Sha512_256,
    ) -> Result<Vec<ItemId>> {
        let mut folders = Vec::new();
        for first in
            (1..=self.options.workload.folders()).step_by(usize::try_from(GENERATION_BATCH)?)
        {
            self.guard.check()?;
            let until = first
                .saturating_add(GENERATION_BATCH)
                .min(self.options.workload.folders().saturating_add(1));
            let mut records = Vec::new();
            for number in first..until {
                let parent = if !folders.is_empty() && generator.pick(data::ROOT_WEIGHT)? == 0 {
                    folders.get(generator.pick(folders.len())?).copied()
                } else {
                    None
                };
                let record = generator.folder(Namespace::Folder, number, parent)?;
                fingerprint.update(data::bytes(&record)?);
                if let Record::Folder(id, _folder) = &record {
                    folders.push(*id);
                }
                records.push(record);
            }
            self.store("source").session(NOW, |session| {
                for record in records {
                    session.merge_record(record)?;
                }
                Ok(())
            })?;
        }
        Ok(folders)
    }

    fn fingerprint(&self, store: &Store) -> Result<[u8; 32]> {
        let mut fingerprint = Sha512_256::new();
        let mut records = 0usize;
        store.session(NOW, |session| {
            session.stream_records(RecordSet::Snapshot, |record| {
                records = records.saturating_add(1);
                if records.is_multiple_of(BUDGET_CHECK_RECORDS) {
                    self.guard.publication()?;
                }
                let bytes = data::bytes(&record).map_err(youarel_store::StoreError::Records)?;
                fingerprint.update(bytes);
                Ok(())
            })
        })?;
        Ok(fingerprint.finalize().into())
    }

    fn workloads(&self) -> Result<()> {
        let folders = self.measure(
            Measurement::operation(
                "generate",
                self.options
                    .workload
                    .shortcuts()
                    .saturating_add(self.options.workload.folders()),
            ),
            |metric| {
                let folders = self.populate()?;
                metric.rows = usize::try_from(
                    self.options
                        .workload
                        .shortcuts()
                        .saturating_add(self.options.workload.folders()),
                )?;
                Ok(folders)
            },
        )?;
        self.reads(&folders)?;
        self.sync_and_edits(&folders)?;
        self.removal()?;
        self.import()?;
        self.guard.check()
    }
}

fn execute(
    root: &Path,
    options: Options,
    cancelled: Arc<AtomicBool>,
    available: Available,
) -> Result<Report> {
    let workspace = fs::temporary_in(root)?;
    let campaign = Campaign::new(workspace.path(), options, cancelled, available);
    let began = Timer::start();
    let ran = campaign.workloads();
    let mut report = campaign.report.into_inner();
    report.finished(began);
    report.synthetic_peak_bytes = campaign.guard.peak.get();
    report.stop = campaign.guard.stopped.get().map(|stop| format!("{stop:?}"));
    if let Err(error) = ran {
        report.errors.push(format!("{error:#}"));
    }
    if let Some(reason) = campaign
        .guard
        .failure
        .into_inner()
        .filter(|reason| !report.errors.iter().any(|error| error.contains(reason)))
    {
        report.errors.push(reason);
    }
    if let Err(error) = workspace.close() {
        report
            .errors
            .push(format!("clean the synthetic workspace: {error}"));
    }
    report.status = if report.stop.is_some() {
        Status::StoppedUnverified
    } else if report.errors.is_empty() {
        Status::Completed
    } else {
        Status::Failed
    };
    Ok(report)
}

/// Measures this host's real store and segment adapters without publishing or contacting a service.
///
/// # Errors
///
/// Refuses invalid inputs, another campaign, resource exhaustion, cancellation, or an incomplete workload, retaining its report.
pub fn run(root: &Path, options: Options) -> Result<()> {
    if options.mode == Mode::DryRun {
        let plan = serde_json::json!({ "config": Report::new(options).config,
            "synthetic_limit_bytes": youarel_core::bench_rules::MOST_BYTES, "free_floor_bytes": youarel_core::bench_rules::LEAST_FREE,
            "write_reserve_bytes": youarel_core::bench_rules::WRITE_RESERVE, "deadline_millis": youarel_core::bench_rules::MOST_MILLIS,
            "report": format!("{OUTPUT_DIRECTORY}/{REPORT_FILE}"), "network": false, "concurrent_campaigns": 1 });
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(());
    }
    let directory = root.join(OUTPUT_DIRECTORY);
    fs::create_dir_all(&directory)?;
    let _lock = fs::lock_run(&directory.join(LOCK_FILE))?;
    let build = build::identify(root)?;
    let mut report = execute(&directory, options, raw::interrupts()?, fs::available_space)?;
    report.build = Some(build);
    let bytes = serde_json::to_vec_pretty(&report).context("encode the benchmark report")?;
    fs::write(&directory.join(REPORT_FILE), &bytes)?;
    println!(
        "benchmark {:?}: {} measurements; {}",
        report.status,
        report.measurements.len(),
        directory.join(REPORT_FILE).display()
    );
    ensure!(
        report.status == Status::Completed,
        "{}",
        report.errors.join("\n")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Options, Report, Status, execute, run};
    use crate::fs;
    use anyhow::Result;
    use std::path::Path;
    use std::sync::{Arc, atomic::AtomicBool};

    fn ample(path: &Path) -> Result<u64> {
        anyhow::ensure!(
            fs::is_directory(path)?,
            "the synthetic workspace disappeared"
        );
        Ok(u64::MAX)
    }

    fn failed_space(_path: &Path) -> Result<u64> {
        anyhow::bail!("injected filesystem statistics failure")
    }

    #[test]
    fn a_small_campaign_measures_real_workloads_without_expanding_its_import() {
        let directory = tempfile::tempdir().unwrap();
        let options =
            Options::from_words(&["--shortcuts", "128", "--folders", "4", "--seed", "17"]).unwrap();
        let report = execute(
            directory.path(),
            options,
            Arc::new(AtomicBool::new(false)),
            ample,
        )
        .unwrap();
        assert_eq!(report.status, Status::Completed, "{:?}", report.errors);
        for name in [
            "generate",
            "open_to_first_page",
            "page_first",
            "page_random",
            "search_1",
            "search_2",
            "search_6",
            "add",
            "move",
            "remove_folder",
            "record_import",
            "snapshot_publish",
            "snapshot_apply",
            "delta_publish",
            "delta_apply",
        ] {
            assert!(
                report
                    .measurements
                    .iter()
                    .any(|measurement| measurement.name == name && measurement.completed),
                "{name}"
            );
        }
        assert_eq!(
            report
                .measurements
                .iter()
                .find(|measurement| measurement.name == "record_import")
                .unwrap()
                .operations,
            128
        );
        assert_eq!(
            report
                .measurements
                .iter()
                .find(|measurement| measurement.name == "remove_folder")
                .unwrap()
                .operations,
            128
        );
        assert_eq!(report.purged, 1);
        assert_ne!(report.fixture_digest, "");
        assert_eq!(
            fs::entries(directory.path()).unwrap(),
            Vec::<std::path::PathBuf>::new()
        );
        let encoded = serde_json::to_vec(&report).unwrap();
        assert_eq!(serde_json::from_slice::<Report>(&encoded).unwrap(), report);
    }

    #[test]
    fn cancellation_or_failed_statistics_never_claim_completed_measurements_and_clean_the_workspace()
     {
        let directory = tempfile::tempdir().unwrap();
        let options = Options::from_words(&["--shortcuts", "1", "--folders", "0"]).unwrap();
        let cancelled = execute(
            directory.path(),
            options,
            Arc::new(AtomicBool::new(true)),
            ample,
        )
        .unwrap();
        assert_eq!(cancelled.status, Status::StoppedUnverified);
        assert_eq!(cancelled.measurements.len(), 0);
        let failed = execute(
            directory.path(),
            options,
            Arc::new(AtomicBool::new(false)),
            failed_space,
        )
        .unwrap();
        assert_eq!(failed.status, Status::Failed);
        assert!(
            failed
                .errors
                .iter()
                .any(|error| error.contains("injected filesystem statistics failure"))
        );
        assert_eq!(
            fs::entries(directory.path()).unwrap(),
            Vec::<std::path::PathBuf>::new()
        );
        let dry = Options::from_words(&["--dry-run"]).unwrap();
        run(directory.path(), dry).unwrap();
        assert_eq!(
            fs::entries(directory.path()).unwrap(),
            Vec::<std::path::PathBuf>::new()
        );
    }
}
