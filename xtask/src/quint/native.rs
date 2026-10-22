//! Bounded trace generation with the pinned native CLI, without starting a proof campaign.

use super::{QUINT_VERSION, TRACES, probe_trace_name, settled, trace_name};
use crate::obligations::{self, Obligation, QuintModel};
use crate::{fs, tool};
use anyhow::{Context, Result, ensure};
use std::path::Path;
use std::process::Output;

fn run(root: &Path, executable: &Path, arguments: &[String]) -> Result<Output> {
    let mut command = tool::built(root, executable);
    command.args(arguments);
    tool::output(&mut command, "native Quint trace generation")
}

fn passing(root: &Path, executable: &Path, arguments: &[String]) -> Result<()> {
    let output = run(root, executable, arguments)?;
    ensure!(
        output.status.success(),
        "Quint failed with {}:\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn arguments(model: &QuintModel, output: &Path, probe: bool) -> Vec<String> {
    let module = if probe { &model.probe } else { &model.main };
    let name = if probe {
        probe_trace_name(module)
    } else {
        format!("{module}.{{seq}}.itf.json")
    };
    let mut arguments: Vec<String> = [
        "run",
        &model.spec,
        "--backend",
        "typescript",
        "--seed",
        &model.seed,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    arguments.extend([
        String::from("--main"),
        module.clone(),
        String::from("--max-samples"),
        if probe {
            super::PROBE_SAMPLES.to_owned()
        } else {
            model.traces.to_string()
        },
        String::from("--max-steps"),
        model.steps.to_string(),
        String::from("--mbt"),
        String::from("--out-itf"),
        output.join(name).to_string_lossy().into_owned(),
        String::from("--verbosity"),
        String::from("0"),
    ]);
    arguments.extend(if probe {
        [String::from("--invariant"), model.invariant.clone()]
    } else {
        [String::from("--n-traces"), model.traces.to_string()]
    });
    arguments
}

/// Generates one model's reproducible traces with the exact pinned CLI supplied by the caller.
///
/// # Errors
///
/// Refuses an unknown model, a different CLI version, failed model tests, or a missing trace or probe counterexample.
pub fn generate(root: &Path, name: &str, executable: &Path) -> Result<()> {
    let manifest = obligations::load(root)?;
    let model = manifest
        .obligation
        .iter()
        .find_map(|obligation| match obligation {
            Obligation::Quint(model) if model.main == name => Some(model),
            Obligation::Quint(_) | Obligation::Kani { .. } | Obligation::Proverif(_) => None,
        })
        .with_context(|| format!("no Quint obligation names model {name}"))?;
    let version = run(root, executable, &[String::from("--version")])?;
    ensure!(
        version.status.success()
            && String::from_utf8_lossy(&version.stdout).trim() == QUINT_VERSION,
        "native trace generation requires Quint {QUINT_VERSION}"
    );
    passing(
        root,
        executable,
        &[String::from("typecheck"), model.spec.clone()],
    )?;
    for module in [&model.main, &model.probe] {
        passing(
            root,
            executable,
            &[
                String::from("test"),
                model.spec.clone(),
                String::from("--backend"),
                String::from("typescript"),
                String::from("--main"),
                module.clone(),
            ],
        )?;
    }
    let output = tempfile::tempdir().context("create native Quint trace output")?;
    passing(root, executable, &arguments(model, output.path(), false))?;
    let probe = run(root, executable, &arguments(model, output.path(), true))?;
    ensure!(
        !probe.status.success(),
        "the probe {} found no counterexample",
        model.probe
    );
    fs::create_dir_all(&root.join(TRACES))?;
    for filename in (0..model.traces)
        .map(|index| trace_name(&model.main, index))
        .chain(core::iter::once(probe_trace_name(&model.probe)))
    {
        let trace = fs::read(&output.path().join(&filename))
            .with_context(|| format!("Quint wrote no trace {filename}"))?;
        fs::write(&root.join(TRACES).join(&filename), &settled(&trace)?)?;
    }
    eprintln!(
        "quint: generated traces of {name}; complete model checking remains a separate proof gate"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{arguments, generate};
    use crate::obligations::QuintModel;
    use std::path::Path;

    #[test]
    fn trace_generation_names_each_file_and_bounds_both_runs() {
        let model = QuintModel {
            spec: String::from("specs/sample.qnt"),
            main: String::from("sample"),
            probe: String::from("sample_probe"),
            invariant: String::from("safe"),
            witnesses: vec![String::from("applied")],
            seed: String::from("0x2a"),
            samples: 1000,
            steps: 12,
            distinct_states: None,
            traces: 3,
        };
        let output = Path::new("/traces");
        for probe in [false, true] {
            let planned = arguments(&model, output, probe);
            let path = output.join(if probe {
                "sample_probe.itf.json"
            } else {
                "sample.{seq}.itf.json"
            });
            assert!(
                planned
                    .windows(2)
                    .any(|pair| pair == ["--out-itf", path.to_string_lossy().as_ref()])
            );
            assert!(planned.windows(2).any(|pair| pair == ["--max-steps", "12"]));
            let samples = if probe { "1000" } else { "3" };
            assert!(
                planned
                    .windows(2)
                    .any(|pair| pair == ["--max-samples", samples])
            );
            let mode = if probe {
                ["--invariant", "safe"]
            } else {
                ["--n-traces", "3"]
            };
            assert!(planned.windows(2).any(|pair| pair == mode));
        }
    }

    #[test]
    fn an_unknown_model_starts_no_external_program() {
        let root = tempfile::tempdir().unwrap();
        let error = generate(root.path(), "missing", Path::new("not-a-program")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no Quint obligation names model missing")
        );
    }
}
