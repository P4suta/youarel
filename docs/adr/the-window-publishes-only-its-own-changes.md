---
status: superseded
date: 2026-10-19
superseded-by: the-product-is-a-local-bookmark-library.md
amends: window-syncs-on-its-own-schedule.md, window-watches-the-sync-folder.md
---
# The window publishes only its own changes

## Context

Periodic publication rewrote encrypted bundles even while the library stayed idle.

## Decision

The decision proposed publication only for pending local writes, membership changes, or a missing target bundle.
The store recorded the last publication under a separate publication lock.
Periodic merges continued, and relay bundles renewed before retention expired.

## Alternatives

Comparing encrypted bytes would fail because each seal used a fresh nonce.
Longer intervals would merely spread redundant writes.

## Consequences

An idle shared folder would receive no publication.
Terminal edits and membership changes still needed discovery.

## Confirmation

The [archived record](https://github.com/P4suta/youarel/blob/d0fbc7afcb58bed077e9be67a956d9202e96df55/docs/adr/the-window-publishes-only-its-own-changes.md) preserves the detailed decision and its verification claims.
