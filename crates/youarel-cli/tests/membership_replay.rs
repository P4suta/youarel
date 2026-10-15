//! Replays the traces of `specs/membership.qnt` through `membership_rules::resolve`, which must draw every device's members and epoch from its log in the order written and reversed.

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use youarel_core::membership_rules::{Action, Device, Digest, Operation, resolve};

/// The directory of the committed traces, which `cargo xtask gen` writes and `cargo xtask proofs` compares.
const TRACES: &str = "../../specs/traces";
/// How many traces of the model the manifest commits.
const TRACE_COUNT: usize = 100;
/// The digest the model names its genesis by.
const GENESIS_ID: u8 = 1;

/// The model's variables carry the module path as a prefix, which the replay drops.
fn variable<'a>(state: &'a Value, name: &str) -> Result<&'a Value> {
    state
        .as_object()
        .context("an ITF state is an object")?
        .iter()
        .find_map(|(key, value)| (key.rsplit("::").next() == Some(name)).then_some(value))
        .with_context(|| format!("the trace has no variable {name}"))
}

fn field<'a>(record: &'a Value, name: &str) -> Result<&'a Value> {
    record
        .get(name)
        .with_context(|| format!("the record has no field {name}"))
}

fn number(value: &Value) -> Result<u8> {
    let text = value
        .get("#bigint")
        .and_then(Value::as_str)
        .context("an ITF integer")?;
    text.parse().context("a small number")
}

fn epoch(value: &Value) -> Result<u32> {
    let text = value
        .get("#bigint")
        .and_then(Value::as_str)
        .context("an ITF integer")?;
    text.parse().context("an epoch")
}

fn elements<'a>(value: &'a Value, kind: &str) -> Result<&'a Vec<Value>> {
    value
        .get(kind)
        .and_then(Value::as_array)
        .with_context(|| format!("an ITF {kind}"))
}

/// The device the model names by `number`.
const fn device(number: u8) -> Device {
    [number; 32]
}

/// The digest the model names by `id`, whose order matches the model's order of ids.
const fn digest(id: u8) -> Digest {
    [id; 32]
}

fn action(value: &Value) -> Result<Action> {
    let tag = value
        .get("tag")
        .and_then(Value::as_str)
        .context("an ITF variant")?;
    let inner = || value.get("value").context("a variant with a value");
    Ok(match tag {
        "Genesis" => Action::Genesis,
        "Add" => Action::Add(device(number(inner()?)?)),
        "Remove" => Action::Remove(device(number(inner()?)?)),
        "Rotate" => Action::Rotate(epoch(inner()?)?),
        other => bail!("unknown action {other}"),
    })
}

fn operation(value: &Value) -> Result<Operation> {
    let parents = elements(field(value, "parents")?, "#set")?
        .iter()
        .map(|parent| number(parent).map(digest))
        .collect::<Result<Vec<Digest>>>()?;
    Ok(Operation {
        digest: digest(number(field(value, "id")?)?),
        parents,
        author: device(number(field(value, "author")?)?),
        action: action(field(value, "change")?)?,
    })
}

/// Replays one trace: in every state, each device's log resolves to the members and the epoch of its view.
fn replay(text: &str) -> Result<usize> {
    let document: Value = serde_json::from_str(text)?;
    let states = document
        .get("states")
        .and_then(Value::as_array)
        .context("a trace holds states")?;
    let mut replayed: usize = 0;
    for (index, state) in states.iter().enumerate() {
        let views = elements(variable(state, "views")?, "#map")?;
        for entry in elements(variable(state, "known")?, "#map")? {
            let [who, log] = entry.as_array().map(Vec::as_slice).unwrap_or_default() else {
                bail!("a map entry is a pair");
            };
            let view = views
                .iter()
                .find_map(|pair| match pair.as_array().map(Vec::as_slice) {
                    Some([key, value]) if key == who => Some(value),
                    Some(_) | None => None,
                })
                .context("every device has a view")?;
            let expected: BTreeSet<Device> = elements(field(view, "members")?, "#set")?
                .iter()
                .map(|member| number(member).map(device))
                .collect::<Result<_>>()?;
            let expected_epoch = epoch(field(view, "epoch")?)?;
            let mut operations = elements(log, "#set")?
                .iter()
                .map(operation)
                .collect::<Result<Vec<Operation>>>()?;
            for _ in 0..2 {
                let resolution = resolve(digest(GENESIS_ID), &operations);
                let members: BTreeSet<Device> = resolution.members.iter().copied().collect();
                ensure!(
                    members == expected && resolution.epoch == expected_epoch,
                    "state {index}: device {who} resolves {:?} at epoch {} but the model drew {:?} at epoch {expected_epoch}",
                    members.iter().map(|member| member[0]).collect::<Vec<u8>>(),
                    resolution.epoch,
                    expected.iter().map(|member| member[0]).collect::<Vec<u8>>()
                );
                operations.reverse();
            }
            replayed = replayed.saturating_add(1);
        }
    }
    Ok(replayed)
}

fn read(path: &Path) -> Result<String> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the replay test reads its committed fixtures directly"
    )]
    let text = std::fs::read_to_string(path);
    text.with_context(|| format!("read {}", path.display()))
}

#[test]
fn every_committed_trace_resolves_as_the_model_draws_it() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(TRACES);
    let mut total: usize = 0;
    for index in 0..TRACE_COUNT {
        let path = directory.join(format!("membership.{index}.itf.json"));
        let replayed = read(&path)
            .and_then(|text| replay(&text))
            .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
        total = total.saturating_add(replayed);
    }
    assert!(total > 0, "the traces hold states");
}

#[test]
fn the_probe_trace_breaks_the_rules_as_the_model_says() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(TRACES)
        .join("membership_probe.itf.json");
    replay(&read(&path).unwrap()).unwrap_err();
}
