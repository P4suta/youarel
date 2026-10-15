---
status: accepted
date: 2026-10-15
---
# Proofs run in memory-limited containers

## Context

Kani hands each harness to CBMC, whose memory grows with the harness's state space, and a harness that grows too far takes the workstation's memory with it.
The model checker behind Quint does the same with a model's reachable states.
The other checks, the build, the tests, and the launcher itself need the host's own operating system and run natively.

## Decision

On a workstation, `just proofs` runs Kani, Miri, the fuzz replays, and mutation testing in the image of `xtask/proofs/image`, limited to 8 GB of memory without swap and to four processors, through `cargo xtask contain`.
The container copies the checkout from a read-only mount, leaving out what Git ignores, so uncommitted edits run and nothing it writes reaches the checkout; named volumes keep the build outputs between runs.
The Quint step runs from the host in its own image, limited to 6 GB and two processors, offline, with the checkout read-only.
A harness that outgrows its limit fails the gate, which shrinks the harness rather than the host.
`just proofs-native` and the hosted runners run the same steps without the outer container.

Rejected alternatives:

- Development entirely inside a container, because the terminal, the browser, and the platform checks need the host's operating system.
- Running the proofs on the Linux host alone, because a workstation run then waits on another machine.

## Consequences

The proof image takes about 3 GB and is built once per change of its definition.
The limits make a harness's cost visible: the bounds in `verification/obligations.toml` are those that fit.
