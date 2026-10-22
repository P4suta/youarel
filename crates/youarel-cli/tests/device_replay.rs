//! Replays `specs/devices.qnt` traces through the production screen machine.
#![cfg(test)]

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::path::Path;
use youarel_core::device_rules::{Event, Role, Run, Stage};
use youarel_store::fs;

const TRACES: &str = "../../specs/traces";
const TRACE_COUNT: usize = 100;
const LONGEST_TRACE: u64 = 256 * 1024;

fn variable<'a>(state: &'a Value, name: &str) -> Result<&'a Value> {
    state
        .as_object()
        .context("an ITF state is an object")?
        .iter()
        .find_map(|(key, value)| (key.rsplit("::").next() == Some(name)).then_some(value))
        .with_context(|| format!("a trace state has no variable {name}"))
}

fn event(state: &Value) -> Result<Event> {
    Ok(match variable(state, "event")?.as_str() {
        Some("Poll") => Event::Poll,
        Some("Ready") => Event::Ready,
        Some("Peer") => Event::Peer,
        Some("Joined") => Event::Joined,
        Some("Synced") => Event::Synced,
        Some("Refuse") => Event::Refuse,
        Some("Cancel") => Event::Cancel,
        other => bail!("unknown screen event {other:?}"),
    })
}

fn stage(state: &Value) -> Result<Stage> {
    Ok(match variable(state, "stage")?.as_str() {
        Some("Offered") => Stage::Offered,
        Some("Waiting") => Stage::Waiting,
        Some("Confirming") => Stage::Confirming,
        Some("Syncing") => Stage::Syncing,
        Some("Done") => Stage::Done,
        Some("Expired") => Stage::Expired,
        Some("Refused") => Stage::Refused,
        Some("Cancelled") => Stage::Cancelled,
        other => bail!("unknown screen stage {other:?}"),
    })
}

fn replay(bytes: &[u8]) -> Result<usize> {
    let document: Value = serde_json::from_slice(bytes)?;
    let states = document
        .get("states")
        .and_then(Value::as_array)
        .context("a trace holds states")?;
    let first = states.first().context("a trace is nonempty")?;
    let role = if variable(first, "joining")?.as_bool().context("a role")? {
        Role::Join
    } else {
        Role::Invite
    };
    let mut run = Run::begin(role, 0);
    ensure!(run.stage() == stage(first)?, "the initial stage agrees");
    for state in states.iter().skip(1) {
        let now = variable(state, "now")?
            .get("#bigint")
            .and_then(Value::as_str)
            .context("an elapsed integer")?
            .parse()?;
        run = run.step(event(state)?, now);
        ensure!(run.stage() == stage(state)?, "the screen step disagrees");
    }
    Ok(states.len().saturating_sub(1))
}

#[test]
fn the_screen_machine_agrees_with_every_committed_model_step() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(TRACES);
    let mut steps: usize = 0;
    for index in 0..TRACE_COUNT {
        let path = directory.join(format!("devices.{index}.itf.json"));
        let bytes = fs::read_bounded(&path, LONGEST_TRACE).unwrap();
        steps = steps.checked_add(replay(&bytes).unwrap()).unwrap();
    }
    assert!(steps >= TRACE_COUNT);
}

#[test]
fn ignoring_the_deadline_disagrees_with_the_production_machine() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(TRACES)
        .join("devices_probe.itf.json");
    let error = replay(&fs::read_bounded(&path, LONGEST_TRACE).unwrap()).unwrap_err();
    assert!(error.to_string().contains("the screen step disagrees"));
}
