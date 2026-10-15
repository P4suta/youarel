//! The `cargo xtask` entry point, which dispatches plain words to the tasks.

use std::path::Path;
use std::process::ExitCode;

const USAGE_EXIT_CODE: u8 = 2;

const USAGE: &str = "usage: cargo xtask check [--staged] | push-check | structure | commit-msg FILE | proofs [--ci] [--only kani|miri|fuzz|mutants] | fuzz TARGET RUNS | gen [--check] | ci check|test|proofs|gate NAME";

fn report(task: &str, result: anyhow::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{task}: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(root: &Path, words: &[&str]) -> ExitCode {
    match words {
        ["check"] => report(
            "check",
            xtask::check::run(root, xtask::check::Scope::Everything),
        ),
        ["check", "--staged"] => report(
            "check",
            xtask::check::run(root, xtask::check::Scope::Staged),
        ),
        ["push-check"] => report("push-check", xtask::check::push_check(root)),
        ["structure"] => report("structure", xtask::structure::check(root)),
        ["commit-msg", message] => report(
            "commit-msg",
            xtask::commit_msg::check(root, Path::new(message)),
        ),
        ["proofs", options @ ..] => xtask::proofs::Options::from_words(options)
            .map_or_else(usage, |options| {
                report("proofs", xtask::proofs::run(root, options))
            }),
        ["fuzz", target, runs] => report("fuzz", xtask::proofs::fuzz(root, target, runs)),
        ["gen"] => report("gen", xtask::check::generate(root)),
        ["gen", "--check"] => report("gen", xtask::check::generated(root)),
        ["ci", rest @ ..] if !rest.is_empty() => report("ci", xtask::ci::run(root, rest)),
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(USAGE_EXIT_CODE)
}

fn main() -> ExitCode {
    let root = match xtask::root() {
        Ok(root) => root,
        Err(error) => {
            eprintln!("xtask: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = arguments.iter().map(String::as_str).collect();
    dispatch(&root, &words)
}

#[cfg(test)]
mod tests {
    use super::{USAGE_EXIT_CODE, dispatch};
    use std::process::ExitCode;

    #[test]
    fn unknown_words_are_a_usage_error() {
        let root = tempfile::tempdir().unwrap();
        for words in [
            vec![],
            vec!["unknown"],
            vec!["check", "--everything"],
            vec!["proofs", "--only", "lean"],
            vec!["ci"],
        ] {
            assert_eq!(
                dispatch(root.path(), &words),
                ExitCode::from(USAGE_EXIT_CODE),
                "{words:?}"
            );
        }
    }

    #[test]
    fn a_failing_task_reports_failure() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(dispatch(root.path(), &["structure"]), ExitCode::FAILURE);
        assert_eq!(
            dispatch(root.path(), &["commit-msg", "missing-file"]),
            ExitCode::FAILURE
        );
    }
}
