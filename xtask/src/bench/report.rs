use super::{Options, raw::Timer};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Completed,
    StoppedUnverified,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Configuration {
    pub shortcuts: u64,
    pub folders: u64,
    pub seed: u64,
    pub imported: u64,
    pub removed: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Build {
    pub commit: String,
    pub dirty: bool,
    pub rustc: String,
    pub lock_digest: String,
    pub executable_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Parameters {
    pub page: usize,
    pub sort_prefix: usize,
    pub token_prefix: usize,
    pub candidate_limit: usize,
    pub compaction_ratio: usize,
    pub cache_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Measurement {
    pub name: String,
    pub scope: Option<String>,
    pub sort: Option<String>,
    pub offset: usize,
    pub rows: usize,
    pub operations: u64,
    pub elapsed_ns: u128,
    pub segments: usize,
    pub bytes: u64,
    pub completed: bool,
}

impl Measurement {
    pub(super) fn operation(name: &str, operations: u64) -> Self {
        Self {
            name: name.to_owned(),
            scope: None,
            sort: None,
            offset: 0,
            rows: 0,
            operations,
            elapsed_ns: 0,
            segments: 0,
            bytes: 0,
            completed: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Report {
    pub format: u16,
    pub status: Status,
    pub os: String,
    pub arch: String,
    pub build_profile: String,
    pub config: Configuration,
    pub build: Option<Build>,
    pub parameters: Parameters,
    pub fixture_digest: String,
    pub purged: u64,
    pub icon_sites: u64,
    pub unique_icon_images: usize,
    pub synthetic_peak_bytes: u64,
    pub elapsed_millis: u128,
    pub stop: Option<String>,
    pub errors: Vec<String>,
    pub measurements: Vec<Measurement>,
}

impl Report {
    pub(super) fn new(options: Options) -> Self {
        let workload = options.workload;
        Self {
            format: 1,
            status: Status::Running,
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            build_profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
            .to_owned(),
            config: Configuration {
                shortcuts: workload.shortcuts(),
                folders: workload.folders(),
                seed: workload.seed(),
                imported: workload.imported(),
                removed: workload.removed(),
            },
            build: None,
            parameters: Parameters {
                page: super::reads::PAGE,
                sort_prefix: youarel_core::index_rules::SORT_PREFIX,
                token_prefix: youarel_core::index_rules::TOKEN_PREFIX,
                candidate_limit: youarel_core::search_rules::CANDIDATE_LIMIT,
                compaction_ratio: youarel_core::segment_rules::COMPACTION_RATIO,
                cache_bytes: youarel_store::CACHE_BYTES,
            },
            fixture_digest: String::new(),
            purged: 0,
            icon_sites: 0,
            unique_icon_images: 0,
            synthetic_peak_bytes: 0,
            elapsed_millis: 0,
            stop: None,
            errors: Vec::new(),
            measurements: Vec::new(),
        }
    }

    pub(super) fn finished(&mut self, began: Timer) {
        self.elapsed_millis = began.elapsed().as_millis();
    }
}
