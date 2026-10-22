use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use youarel_core::segment_rules::{
    Available, Cursor, Decision, Header, Kind, Sequence, Verdict, step,
};
use youarel_store::fs;

const TRACES: &str = "../../specs/traces";
const TRACE_COUNT: usize = 100;
const LONGEST_TRACE: u64 = 4 * 1024 * 1024;
const DIGEST_WEIGHTS: [u64; 3] = [1, 3, 9];
const MOST_WRITES: u64 = 2;

fn variable<'a>(state: &'a Value, name: &str) -> Result<&'a Value> {
    state
        .as_object()
        .context("an ITF state is an object")?
        .iter()
        .find_map(|(key, value)| (key.rsplit("::").next() == Some(name)).then_some(value))
        .with_context(|| format!("a trace state has no variable {name}"))
}

fn field<'a>(record: &'a Value, name: &str) -> Result<&'a Value> {
    record
        .get(name)
        .with_context(|| format!("a record has no field {name}"))
}

fn number(value: &Value) -> Result<u64> {
    value
        .get("#bigint")
        .and_then(Value::as_str)
        .context("an ITF integer")?
        .parse()
        .context("a nonnegative bounded model integer")
}

fn sequence(value: &Value) -> Result<Sequence> {
    Sequence::new(number(value)?).context("a positive sequence")
}

fn digest(value: u64) -> Result<[u8; 32]> {
    Ok([u8::try_from(value).context("a bounded digest equality class")?; 32])
}

fn cursor(value: &Value) -> Result<Option<Cursor>> {
    let position = number(field(value, "sequence")?)?;
    let hash = number(field(value, "digest")?)?;
    if position == 0 {
        ensure!(hash == 0, "an absent cursor has no digest");
        return Ok(None);
    }
    Ok(Some(Cursor::new(
        Sequence::new(position).context("a positive cursor")?,
        digest(hash)?,
    )))
}

fn body_digest(body: &Value) -> Result<[u8; 32]> {
    let pairs = body
        .get("#map")
        .and_then(Value::as_array)
        .context("the body's register map")?;
    ensure!(
        pairs.len() == DIGEST_WEIGHTS.len(),
        "a body holds the three writers"
    );
    let mut hash: u64 = 1;
    for (index, weight) in DIGEST_WEIGHTS.iter().enumerate() {
        let writer = u64::try_from(index)?
            .checked_add(1)
            .context("a writer index")?;
        let matching = pairs
            .iter()
            .filter_map(|pair| match pair.as_array().map(Vec::as_slice) {
                Some([key, value]) => Some((key, value)),
                Some(_) | None => None,
            })
            .filter(|(key, _)| number(key).is_ok_and(|key| key == writer))
            .collect::<Vec<_>>();
        let [(_, value)] = matching.as_slice() else {
            bail!("a body names each writer once");
        };
        let written = number(value)?;
        ensure!(
            written <= MOST_WRITES,
            "a model writer made at most two writes"
        );
        hash = hash
            .checked_add(
                weight
                    .checked_mul(written)
                    .context("a bounded body digit")?,
            )
            .context("a bounded body digest")?;
    }
    digest(hash)
}

fn header(value: &Value) -> Result<Header> {
    let kind = if field(value, "snapshot")?
        .as_bool()
        .context("a snapshot flag")?
    {
        Kind::Snapshot { part: 0, parts: 1 }
    } else {
        Kind::Delta
    };
    Header::new(
        sequence(field(value, "sequence")?)?,
        digest(number(field(value, "previous")?)?)?,
        kind,
    )
    .context("a model segment header")
}

fn available(value: &Value) -> Result<Available> {
    Ok(Available {
        first: sequence(field(value, "first")?)?,
        snapshot: Sequence::new(number(field(value, "snapshot")?)?),
    })
}

fn decision(value: &Value) -> Result<Decision> {
    let name = field(value, "verdict")?
        .as_str()
        .context("a verdict name")?;
    let jump = number(field(value, "jump")?)?;
    let verdict = match name {
        "Apply" => Verdict::Apply,
        "Reapply" => Verdict::Reapply,
        "Old" => Verdict::Old,
        "Gap" => Verdict::Gap,
        "Jump" => Verdict::Jump(Sequence::new(jump).context("a positive snapshot jump")?),
        "Lost" => Verdict::Lost,
        "Forked" => Verdict::Forked,
        other => bail!("unknown model verdict {other}"),
    };
    ensure!(name == "Jump" || jump == 0, "only Jump names a destination");
    Ok(Decision {
        cursor: cursor(field(value, "cursor")?)?,
        verdict,
    })
}

fn replay(bytes: &[u8]) -> Result<(usize, BTreeSet<String>)> {
    let document: Value = serde_json::from_slice(bytes)?;
    let states = document
        .get("states")
        .and_then(Value::as_array)
        .context("a trace holds states")?;
    let mut deliveries: usize = 0;
    let mut outcomes = BTreeSet::new();
    for (index, state) in states.iter().enumerate() {
        outcomes.insert(
            variable(state, "event")?
                .as_str()
                .context("an event name")?
                .to_owned(),
        );
        let receipt = variable(state, "receipt")?;
        if !field(receipt, "active")?
            .as_bool()
            .context("a receipt flag")?
        {
            continue;
        }
        let segment = field(receipt, "segment")?;
        let expected = decision(field(receipt, "decision")?)?;
        let actual = step(
            cursor(field(receipt, "before")?)?,
            header(segment)?,
            body_digest(field(segment, "body")?)?,
            available(field(receipt, "head")?)?,
        );
        ensure!(
            actual == expected,
            "state {index}: model {expected:?} differs from segment_rules::step {actual:?}"
        );
        outcomes.insert(
            field(field(receipt, "decision")?, "verdict")?
                .as_str()
                .context("a verdict")?
                .to_owned(),
        );
        deliveries = deliveries.saturating_add(1);
    }
    ensure!(deliveries > 0, "a model trace must exercise a delivery");
    Ok((deliveries, outcomes))
}

#[test]
fn every_quint_delivery_agrees_with_the_production_cursor_step() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(TRACES);
    let mut deliveries: usize = 0;
    let mut outcomes = BTreeSet::new();
    for index in 0..TRACE_COUNT {
        let path = directory.join(format!("segments.{index}.itf.json"));
        let (count, found) = fs::read_bounded(&path, LONGEST_TRACE)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| replay(&bytes))
            .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
        deliveries = deliveries.saturating_add(count);
        outcomes.extend(found);
    }
    assert!(deliveries >= TRACE_COUNT);
    for expected in [
        "Apply",
        "Reapply",
        "Jump",
        "Lost",
        "compacted",
        "crashed",
        "rotated",
    ] {
        assert!(
            outcomes.contains(expected),
            "the committed traces never witness {expected}: {outcomes:?}"
        );
    }
}

#[test]
fn the_probe_trace_disagrees_on_its_missing_predecessor_guard() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(TRACES)
        .join("segments_probe.itf.json");
    let error = replay(&fs::read_bounded(&path, LONGEST_TRACE).unwrap()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("differs from segment_rules::step"),
        "{error:#}"
    );
}
