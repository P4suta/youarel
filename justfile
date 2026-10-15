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

# The checks that depend on the operating system, on the Linux host over the current files, uncommitted edits included.
check-linux:
    domyjob run linux --wait -- /bin/bash -lc 'export MISE_TRUSTED_CONFIG_PATHS="$PWD" RUSTC_WRAPPER=; mise x -- cargo xtask check --platform'

# The checks that depend on the operating system, on the Windows host over the current files, uncommitted edits included.
check-win:
    domyjob run win --wait -- powershell.exe -NoProfile -Command '$env:MISE_TRUSTED_CONFIG_PATHS=(Get-Location).Path; $env:RUSTC_WRAPPER=""; mise x -- cargo xtask check --platform; exit $LASTEXITCODE'

# The checks that depend on the operating system, on every other host.
check-fleet: check-linux check-win

# The proofs a hosted Linux runner repeats, on the Linux host over the current files.
proofs-linux:
    domyjob run linux --wait -- /bin/bash -lc 'export MISE_TRUSTED_CONFIG_PATHS="$PWD" RUSTC_WRAPPER=; mise x -- just proofs-ci'

# The pure core keeps its boundary and every crate keeps its place.
structure: trust
    mise x -- cargo xtask structure

# Kani, Miri, fuzz corpus replays, and mutation testing, in a container limited to 8 GB of memory and 4 processors.
proofs *args: trust
    mise x -- cargo xtask contain proofs {{args}}

# The same proofs on this host without a container, as a hosted Linux runner runs them.
proofs-native *args: trust
    mise x -- cargo xtask proofs {{args}}

# The proofs a hosted Linux runner repeats, without mutation testing.
proofs-ci: trust
    mise x -- cargo xtask proofs --ci

# Mutation testing of the pure crates alone, in the container.
mutants: trust
    mise x -- cargo xtask contain proofs --only mutants

# Fuzz one target beyond its corpus for a bounded number of runs, in the container.
fuzz target runs="20000": trust
    mise x -- cargo xtask contain fuzz {{quote(target)}} {{runs}}

# The generated files, such as the README, or `--check` to refuse drift.
gen *args: trust
    mise x -- cargo xtask gen {{args}}
