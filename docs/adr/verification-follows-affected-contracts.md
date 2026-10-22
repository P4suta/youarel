---
status: accepted
date: 2026-10-22
---
# Verification follows affected contracts

## Status

Accepted.
This decision amends the execution policy of [proofs in containers](proofs-run-in-memory-limited-containers.md).
Their verifier semantics and exploration bounds remain required.

## Context

An ordinary edit needs evidence for its affected contracts and their dependencies.
Running proof and mutation campaigns before a push makes a finite edit wait on broad assurance work.
Running Linux workspace tests twice and an unconditional operating system matrix adds more duplication.
Workload timings measure a particular environment and do not establish correctness.

## Decision

The pre-push hook runs `just check`.
The explicit `just push-check` keeps the full campaign.
Hosted CI runs for pull requests and merge groups, cancels superseded runs, and offers a manual full-inventory run without a schedule.
One portable Linux gate always checks formatting, strict typing, lints, native workspace tests, structural policy, and dependencies.
That gate adds the Linux window's native build and tests when the changed inputs affect it.
Proofs, model checks, Miri, fuzz campaigns, mutation, and the full operating system matrix gate the exact release candidate.
An explicit full-inventory run can answer a development question without gating a commit, push, pull request, ready transition, or merge.
The release prohibition still controls signing and publication.

`cargo xtask ci plan BASE` exposes the selected work without executing it.
The plan records the resolved base, merge base, head, changed paths, and current verification inputs.
Git reports renames as a deletion and an addition.
Explicit source queries and temporary repositories clear the invoking hook's repository environment as [Git requires for foreign repositories](https://git-scm.com/docs/githooks#_description).
The ordinary staged-file and commit-message gates retain their hook index context.
Native tests compare the cleared variable inventory with `git rev-parse --local-env-vars` and query a real repository under an injected foreign context.
Missing bases, empty ranges, deleted inputs, unknown paths, dependency settings, and verifier changes keep the full input inventory.
Development selects only fast checks from that inventory; candidate verification selects its complete assurance work.
Each executor and the summary recompute the plan from the clean checkout and the event's base before accepting it.
Compiler caches may save build work; a previous proof receipt grants no credit.

The inventory includes every declared Kani obligation and model, every parser corpus, the Miri packages, and all mutation packages.
The release candidate and manual full run execute that complete inventory.
The planner omits the Linux window check only for a range containing documentation alone.
Every other source change conservatively includes the window, whose native dependencies differ from the workspace.
This avoids a second dependency graph that could silently omit a consumer.
An empty verifier inventory provides no evidence.
Native tests cover real Git ranges, renames, deleted inputs, missing bases, dirty checkouts, and stale or altered plans.
The core's source-bound Kani harnesses state candidate selection preservation, separation of development from campaigns, and rejection of incomplete required outcomes.
Harnesses and models accompany implementation changes; an unrun harness is reported as unrun.

Every selected verifier keeps its positive checks, nonempty work checks, and negative controls.
The always-running summary requires the fast gate and every selected job to succeed.
It rejects missing jobs, failed planning, altered plans, cancellation, unknown outcomes, and skipped required work.
A timeout is an inconclusive safety stop and supplies no passing evidence.
The final `required` check depends on that summary.

Large timing campaigns stay explicit observations.
Ordinary correctness gates use deterministic tests and operation counts where the implementation defines them.
No timing target or estimated campaign duration substitutes for a correctness verdict.

Rejected alternatives:

- A time budget that turns unfinished work into success, because it grants credit without evidence.
- A changed-line-only mutation filter, because edits to tests or dependencies can weaken coverage outside those lines.
- Reusing old receipts, because the earlier digest omitted model and verifier inputs and the project has no trusted receipt admission protocol.
- Selecting no work for an unknown path or an unavailable base, because an incomplete inventory cannot establish irrelevance.

## Consequences

Frontend edits avoid unrelated cryptographic proofs and workspace mutation campaigns.
Core and infrastructure changes keep conservative assurance inventories for explicit experiments and the release gate.
The development selector distinguishes documentation from source changes; it makes no finer dependency claim.
Full campaigns remain available for broader assurance and supported target checks.
This change makes no measured runtime claim and does not establish that an unrun campaign passes.
