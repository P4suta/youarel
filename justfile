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

# The explicit full campaign: every check, the Linux window, and every proof.
push-check: trust
    mise x -- cargo xtask push-check

# Show the required verification for a base-to-HEAD range without executing it.
ci-plan base: trust
    mise x -- cargo xtask ci plan {{quote(base)}}

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

# Reproducible native store and segment measurements; timings never gate correctness.
bench *args: trust
    mise x -- cargo run --release --package xtask -- bench {{args}}

# Kani, Quint, ProVerif, Miri, fuzz corpus replays, and mutation testing, in containers limited to 8 GB of memory and 4 processors at most.
proofs *args: trust
    mise x -- cargo xtask contain proofs {{args}}

# One rule module's Kani harnesses and rejecting probes, in the proof container.
proof-source source: trust
    mise x -- cargo xtask contain proof-source {{quote(source)}}

# The same proofs on this host without a container, as a hosted Linux runner runs them.
proofs-native *args: trust
    mise x -- cargo xtask proofs {{args}}

# The explicit proof-only inventory, without mutation testing.
proofs-ci: trust
    mise x -- cargo xtask proofs --ci

# Mutation testing of the pure crates alone, in the container.
mutants: trust
    mise x -- cargo xtask contain proofs --only mutants

# The relay's Worker: lint and build it, serve it with `wrangler dev --local` in a container, and run the relay's end-to-end test against it; nothing deploys it.
worker: trust
    mise x -- cargo xtask worker

# The window launcher on Linux: compile its script, then lint and test it in a container that holds WebKitGTK.
app-linux: trust
    mise x -- cargo xtask app-linux

# Fuzz one target beyond its corpus for a bounded number of runs, in the container.
fuzz target runs="20000": trust
    mise x -- cargo xtask contain fuzz {{quote(target)}} {{runs}}

# The generated files, such as the README, or `--check` to refuse drift.
gen *args: trust
    mise x -- cargo xtask gen {{args}}
