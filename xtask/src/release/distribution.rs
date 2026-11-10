use super::{DIST, candidate, required, runs, variable};
use crate::fs;
use anyhow::{Result, ensure};
use std::path::Path;
use youarel_core::update_rules::Version;

const SCHEMA: u8 = 1;
const RECORD: &str = "distribution.json";

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u8,
    source: String,
    version: String,
    candidate_run: String,
    distribution_run: String,
}

fn validate(record: &Record, source: &str, version: &str, run: &str) -> Result<()> {
    ensure!(
        record.schema == SCHEMA,
        "unknown distribution record format"
    );
    ensure!(
        candidate::valid_source(source) && record.source == source,
        "distribution source mismatch"
    );
    ensure!(
        Version::parse(version).is_some() && version != "0.0.0" && record.version == version,
        "distribution version mismatch"
    );
    ensure!(
        runs::valid_id(run) && record.distribution_run == run,
        "distribution run mismatch"
    );
    ensure!(
        runs::valid_id(&record.candidate_run) && record.candidate_run != run,
        "distribution must consume a separate candidate run"
    );
    Ok(())
}

pub(super) fn write(root: &Path) -> Result<()> {
    let record = Record {
        schema: SCHEMA,
        source: required("RELEASE_SOURCE_SHA")?,
        version: required("RELEASE_VERSION")?,
        candidate_run: required("RELEASE_CANDIDATE_RUN")?,
        distribution_run: required("RELEASE_BUILD_RUN")?,
    };
    validate(
        &record,
        &record.source,
        &record.version,
        &record.distribution_run,
    )?;
    if let Some(current) = variable("GITHUB_RUN_ID")? {
        ensure!(
            current == record.distribution_run,
            "distribution run differs from its controller"
        );
    }
    let bytes = serde_json::to_vec_pretty(&record)?;
    let path = root.join(DIST).join(RECORD);
    if fs::is_file(&path)? {
        ensure!(
            fs::read(&path)? == bytes,
            "the distribution already records another candidate"
        );
        return Ok(());
    }
    fs::write(&path, &bytes)
}

pub(super) fn candidate_run(root: &Path, source: &str, version: &str, run: &str) -> Result<String> {
    let record: Record = serde_json::from_slice(&fs::read(&root.join(DIST).join(RECORD))?)?;
    validate(&record, source, version, run)?;
    if let Some(selected) =
        variable("RELEASE_CANDIDATE_RUN")?.filter(|selected| !selected.is_empty())
    {
        ensure!(
            record.candidate_run == selected,
            "distribution selected another candidate run"
        );
    }
    Ok(record.candidate_run)
}

#[cfg(test)]
mod tests {
    use super::{Record, SCHEMA, validate};

    fn record() -> Record {
        Record {
            schema: SCHEMA,
            source: "a".repeat(40),
            version: "1.0.0".to_owned(),
            candidate_run: "1234".to_owned(),
            distribution_run: "1235".to_owned(),
        }
    }

    #[test]
    fn distribution_records_bind_the_original_candidate_to_the_signed_run() {
        let expected = record();
        validate(
            &expected,
            &expected.source,
            &expected.version,
            &expected.distribution_run,
        )
        .unwrap();
        for field in ["schema", "source", "version", "candidate", "distribution"] {
            let mut altered = expected.clone();
            match field {
                "schema" => altered.schema = 0,
                "source" => altered.source = "b".repeat(40),
                "version" => altered.version = "1.0.1".to_owned(),
                "candidate" => altered.candidate_run = altered.distribution_run.clone(),
                _ => altered.distribution_run = "1236".to_owned(),
            }
            validate(
                &altered,
                &expected.source,
                &expected.version,
                &expected.distribution_run,
            )
            .unwrap_err();
        }
        for invalid in ["", "0", "01234", "-1", "local"] {
            let mut altered = expected.clone();
            altered.candidate_run = invalid.to_owned();
            validate(
                &altered,
                &expected.source,
                &expected.version,
                &expected.distribution_run,
            )
            .unwrap_err();
        }
    }
}
