---
status: proposed
date: 2026-11-11
amends: verification-follows-affected-contracts.md
---
# Hosted checks verify shipping behavior

## Context

The [first candidate](https://github.com/P4suta/youarel/actions/runs/42832812161) spent 95 minutes in verification without reaching native builds.
The owner excluded automatic Kani, Miri, and mutation campaigns.

## Decision

Hosted gates check lints, types, tests, dependencies, native behavior, and packages.
Run full campaigns locally before release, once for the checked source tree.

Build six native targets without release credentials, using [isolated compiler caches](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching#restrictions-for-accessing-a-cache).
Caches never replace checks.
Candidate preparation requires the tree reported by successful local checks.

Signing consumes one successful candidate without rebuilding.
Verify source, version, workflow, targets, provenance, and hashes before credentials.
Bind original candidate and distribution runs in the signed inventory.
Keep installation checks and publication protections.

## Alternatives

Repeating campaigns in CI duplicates local work.
Rebuilding discards verified artifact identity.

## Consequences

CI trusts the operator's local-check confirmation.
Unrun campaigns establish no verdict.
Reconsider automatic specialized checks after a defect escapes deterministic tests.

## Confirmation

Reject hosted campaigns, stale local source trees, incomplete runs, altered identities, and changed bytes.
Verify six builds and candidate reuse before signing.
