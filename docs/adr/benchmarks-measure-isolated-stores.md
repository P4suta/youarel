---
status: accepted
date: 2026-10-21
amends: the-state-is-kept-in-an-indexed-store.md
---
# Benchmarks measure isolated stores

## Status

Accepted.

## Context

The library must expose the cost of opening, paging, searching, editing, and exchanging segments at a reproducible size.
The earlier redb spike measured three indexes rather than the library's complete workload.
Measurements must preserve the person's library and stay within the owner's storage and execution budget.
Host speed and filesystem caches affect timings, so elapsed time cannot decide correctness.

## Decision

`just bench` runs the release build of `cargo xtask bench` natively.
It accepts shortcut and folder counts and a ChaCha20 seed, and `--dry-run` prints its configuration without creating files or installing signal handlers.
Each measured campaign owns a temporary directory under `target/bench` and an exclusive file lock that refuses another campaign there.
No campaign opens the person's library, contacts a service, or publishes anything.

The generator mixes short, medium, and long titles, occasional long notes and aliases, earlier timestamps, root and nested folders, and exactly one purged shortcut per hundred generated shortcuts.
Identifiers retain their namespace and every counter bit independently of the timestamp.
Hosts share a normalized PNG, and the report states the number of hosts and distinct images so image storage costs remain explicit.
The workload streams bounded batches into the real store.
The canonical fixture digest and the seed identify the records independently of measured time.

The workload measures the first hydrated page after opening, first and random hydrated pages for every supported sort and scope, and one-, two-, and six-character searches.
It measures an add, a move, a folder removal, and a trusted-record import through store sessions.
Removal uses at most one thousand shortcuts, and import uses at most one hundred thousand; each also stays within the requested shortcut count.
The import measures record merging rather than HTML or browser parsing.
Two fresh identities pair through the real pairing API, then publish, seal, persist, authenticate, and apply a snapshot and a delta through the production segment APIs.
A streaming comparison requires the two resulting stores to agree.

`target/bench/report.json` records workload counts, the fixture digest, compiler, commit, dirty-worktree flag, executable and lockfile digests, target, build profile, active tuning parameters, operation counts, segment counts, bytes, and elapsed nanoseconds.
The report marks each measurement complete only after its operation succeeds.
Cancellation, a twelve-hour deadline, resource exhaustion, adapter failure, or failed cleanup leaves the campaign unverified or failed and returns an error.
The runner removes its temporary data before returning while retaining the report.

The [resource decision](../../crates/youarel-core/src/bench_rules.rs) checks a forty-GiB synthetic-data ceiling and a thirty-GiB free-space floor before work, between bounded batches, and while staging a snapshot.
It reserves another nine GiB before writes.
Pinned redb 4.4 can extend a file by up to two four-GiB regions at a growth boundary; the reserve covers that growth plus adapter overhead before the next observation.
Free-space observations cannot reserve space against unrelated processes.
The runner therefore checks the actual filesystem again at each boundary and stops early rather than planning from an initial free-space estimate.
Snapshot cancellation aborts its write transaction, retaining the original records without leaving reserved parts or consuming their sequences.

Keep `PAGE = 200`, `SORT_PREFIX = 32`, `TOKEN_PREFIX = 12`, `CANDIDATE_LIMIT = 2000`, `COMPACTION_RATIO = 2`, and `CACHE_BYTES = 256 MiB` until representative measurements justify a change.
Each report reads these values from production constants.
Keep the database handle scoped to a store session.
A handle kept open by the window would exclude the CLI or another process from redb, so reducing open cost must preserve that access contract.

Rejected alternatives:

- A timing threshold in `just check`, because host speed and cache state would change its verdict.
- Generating a full import during a small smoke run, because that silently enlarges the requested workload.
- Cloning the whole library to compare segment outcomes, because that adds another memory cost at the target scale.
- Changing indexes or cache sizes from a small fixture's timings, because that fixture supplies no evidence about million-record behavior.

## Consequences

The ordinary tests exercise a small campaign and its refusal and cleanup paths without asserting a duration.
The production resource decision has source-bound Kani harnesses and a negative probe; their proof campaign remains pending with the owner's verification window.
The owner has deferred million- and ten-million-shortcut campaigns, so this decision makes no latency claim at those sizes.
The report exposes incomplete measurements instead of treating a stopped campaign as evidence that the target scale works.
Large runs can stop before the nominal storage ceiling because the write reserve protects the free-space floor.
