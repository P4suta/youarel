//! Replays the traces of `specs/ui.qnt` through `ui_rules::step` and compares the interface and the effect after every step.

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::path::Path;
use youarel_core::state::{Entry, ItemId};
use youarel_core::ui_rules::{Effect, Event, Field, Key, Mode, Row, Ui, step};

/// The directory of the committed traces, which `cargo xtask gen` writes and `cargo xtask proofs` compares.
const TRACES: &str = "../../specs/traces";
/// How many traces of the model the manifest commits.
const TRACE_COUNT: usize = 100;

/// The entries the model names, as the replay spells them.
const FOLDER: ItemId = ItemId([1; 16]);
const SHORTCUT: ItemId = ItemId([2; 16]);
const OTHER: ItemId = ItemId([3; 16]);
/// The link the model's offer to add stands for.
const ADD_LINK: &str = "l";

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

fn count(value: &Value) -> Result<usize> {
    let text = value
        .get("#bigint")
        .and_then(Value::as_str)
        .context("an ITF integer")?;
    text.parse().context("a count")
}

fn tag(value: &Value) -> Result<&str> {
    value
        .get("tag")
        .and_then(Value::as_str)
        .context("an ITF variant")
}

fn inner(value: &Value) -> Result<&Value> {
    value.get("value").context("a variant with a value")
}

fn text(value: &Value) -> Result<&str> {
    value.as_str().context("an ITF string")
}

fn boolean(value: &Value) -> Result<bool> {
    value.as_bool().context("an ITF boolean")
}

fn characters(value: &Value) -> Result<String> {
    let list = value.as_array().context("an ITF list")?;
    list.iter().map(text).collect()
}

fn item(value: &Value) -> Result<Entry> {
    Ok(match tag(value)? {
        "Folder" => Entry::Folder(FOLDER),
        "Shortcut" => Entry::Shortcut(SHORTCUT),
        "Other" => Entry::Shortcut(OTHER),
        other => bail!("unknown item {other}"),
    })
}

fn shortcut(value: &Value) -> Result<ItemId> {
    match item(value)? {
        Entry::Shortcut(id) => Ok(id),
        Entry::Folder(_) => bail!("the model names a folder where a shortcut belongs"),
    }
}

fn option(value: &Value) -> Result<Option<Entry>> {
    Ok(match tag(value)? {
        "None" => None,
        "Some" => Some(item(inner(value)?)?),
        other => bail!("unknown option {other}"),
    })
}

fn part(value: &Value) -> Result<Field> {
    Ok(match tag(value)? {
        "Title" => Field::Title,
        "Alias" => Field::Alias,
        "Name" => Field::Name,
        other => bail!("unknown field {other}"),
    })
}

fn mode(value: &Value) -> Result<Mode> {
    Ok(match tag(value)? {
        "Browse" => Mode::Browse,
        "Typing" => {
            let draft = inner(value)?;
            Mode::Typing {
                target: option(field(draft, "target")?)?,
                field: part(field(draft, "part")?)?,
                text: characters(field(draft, "text")?)?,
            }
        }
        "Confirm" => Mode::Confirm {
            target: item(inner(value)?)?,
        },
        other => bail!("unknown mode {other}"),
    })
}

const fn folder(inside: bool) -> Option<ItemId> {
    if inside { Some(FOLDER) } else { None }
}

/// The interface one state of a trace records.
fn ui(value: &Value) -> Result<Ui> {
    let record = variable(value, "ui")?;
    Ok(Ui {
        query: characters(field(record, "query")?)?,
        folder: folder(boolean(field(record, "inside")?)?),
        selected: count(field(record, "selected")?)?,
        mode: mode(field(record, "mode")?)?,
        carried: option(field(record, "carried")?)?,
    })
}

fn row(value: &Value) -> Result<Row> {
    Ok(match tag(value)? {
        "ParentRow" => Row::Parent(None),
        "EntryRow" => Row::Entry(item(inner(value)?)?),
        "AddRow" => Row::Add(ADD_LINK.to_owned()),
        other => bail!("unknown row {other}"),
    })
}

fn rows(value: &Value) -> Result<Vec<Row>> {
    let list = variable(value, "shown")?
        .as_array()
        .context("an ITF list")?;
    list.iter().map(row).collect()
}

fn letter(value: &Value) -> Result<char> {
    let mut chars = text(value)?.chars();
    match (chars.next(), chars.next()) {
        (Some(character), None) => Ok(character),
        _ => bail!("a key names one character"),
    }
}

fn key(value: &Value) -> Result<Key> {
    Ok(match tag(value)? {
        "Char" => Key::Char(letter(inner(value)?)?),
        "Control" => Key::Control(letter(inner(value)?)?),
        "Backspace" => Key::Backspace,
        "Enter" => Key::Enter,
        "Escape" => Key::Escape,
        "Up" => Key::Up,
        "Down" => Key::Down,
        "Left" => Key::Left,
        "Right" => Key::Right,
        "Tab" => Key::Tab,
        "ShiftUp" => Key::ShiftUp,
        "ShiftDown" => Key::ShiftDown,
        other => bail!("unknown key {other}"),
    })
}

fn event(value: &Value) -> Result<Event> {
    let read = variable(value, "event")?;
    Ok(match tag(read)? {
        "Press" => Event::Key(key(inner(read)?)?),
        "Paste" => Event::Paste(text(inner(read)?)?.to_owned()),
        "Refresh" => Event::Refresh,
        other => bail!("unknown event {other}"),
    })
}

fn effect(value: &Value) -> Result<Effect> {
    let asked = variable(value, "effect")?;
    Ok(match tag(asked)? {
        "Nothing" => Effect::None,
        "Open" => Effect::Open(shortcut(inner(asked)?)?),
        "Add" => Effect::Add(ADD_LINK.to_owned()),
        "Rename" => {
            let record = inner(asked)?;
            Effect::Rename {
                target: item(field(record, "target")?)?,
                text: characters(field(record, "text")?)?,
            }
        }
        "Realias" => {
            let record = inner(asked)?;
            Effect::Realias {
                id: shortcut(field(record, "id")?)?,
                alias: characters(field(record, "alias")?)?,
            }
        }
        "CreateFolder" => {
            let record = inner(asked)?;
            Effect::CreateFolder {
                name: characters(field(record, "name")?)?,
                parent: folder(boolean(field(record, "inside")?)?),
            }
        }
        "Remove" => Effect::Remove(item(inner(asked)?)?),
        "Move" => {
            let record = inner(asked)?;
            Effect::Move {
                entry: item(field(record, "entry")?)?,
                folder: folder(boolean(field(record, "inside")?)?),
            }
        }
        "Reorder" => {
            let record = inner(asked)?;
            Effect::Reorder {
                entry: item(field(record, "entry")?)?,
                neighbor: item(field(record, "neighbor")?)?,
                before: boolean(field(record, "before")?)?,
            }
        }
        "Quit" => Effect::Quit,
        other => bail!("unknown effect {other}"),
    })
}

/// Replays one trace: every step of the model agrees with `ui_rules::step` on the interface and the effect.
fn replay(text: &str) -> Result<usize> {
    let document: Value = serde_json::from_str(text)?;
    let states = document
        .get("states")
        .and_then(Value::as_array)
        .context("a trace holds states")?;
    let mut replayed: usize = 0;
    for (index, pair) in states.windows(2).enumerate() {
        let [before, after] = pair else {
            continue;
        };
        let read = event(after)?;
        let shown = rows(after)?;
        let expected_ui = ui(after)?;
        let expected_effect = effect(after)?;
        let (stepped, asked) = step(&ui(before)?, &shown, &read);
        ensure!(
            stepped == expected_ui,
            "step {index}: after {read:?} over {shown:?} the machine left {stepped:?} but the model wrote {expected_ui:?}"
        );
        ensure!(
            asked == expected_effect,
            "step {index}: after {read:?} over {shown:?} the machine asked {asked:?} but the model wrote {expected_effect:?}"
        );
        replayed = replayed.saturating_add(1);
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
fn every_committed_trace_replays_through_the_interface_machine() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(TRACES);
    let mut total: usize = 0;
    for index in 0..TRACE_COUNT {
        let path = directory.join(format!("ui.{index}.itf.json"));
        let replayed = read(&path)
            .and_then(|text| replay(&text))
            .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
        total = total.saturating_add(replayed);
    }
    assert!(total > 0, "the traces hold steps");
}

#[test]
fn the_probe_trace_breaks_the_machine_as_the_model_says() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(TRACES)
        .join("ui_probe.itf.json");
    replay(&read(&path).unwrap()).unwrap_err();
}
