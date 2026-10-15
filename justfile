set windows-shell := ["pwsh.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
    @just --list

# mise's paranoid mode loads this checkout's configuration only after a trust that records its current contents.
[private]
trust:
    mise trust --quiet {{quote(justfile_directory() / "mise.toml")}}

# Formatting, lints, the structural gates, the records, the tests, and the dependency checks.
check *args: trust
    mise x -- cargo xtask check {{args}}

# The quick staged-file checks the pre-commit hook runs.
check-staged: trust
    mise x -- cargo xtask check --staged

# Every check and every proof, which the pre-push hook runs.
push-check: trust
    mise x -- cargo xtask push-check

# The pure core keeps its boundary and every crate keeps its place.
structure: trust
    mise x -- cargo xtask structure

# Kani, Miri, fuzz corpus replays, mutation testing, and every other verifier of the obligation manifest.
proofs *args: trust
    mise x -- cargo xtask proofs {{args}}

# The proofs a hosted Linux runner repeats, without mutation testing.
proofs-ci: trust
    mise x -- cargo xtask proofs --ci

# Mutation testing of the pure crates alone.
mutants: trust
    mise x -- cargo xtask proofs --only mutants

# Fuzz one target beyond its corpus for a bounded number of runs.
fuzz target runs="20000": trust
    mise x -- cargo xtask fuzz {{quote(target)}} {{runs}}

# The generated files, such as the README, or `--check` to refuse drift.
gen *args: trust
    mise x -- cargo xtask gen {{args}}
