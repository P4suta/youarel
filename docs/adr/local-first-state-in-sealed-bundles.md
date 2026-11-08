---
status: amended
date: 2026-10-15
amended-by: icons-stay-on-the-device.md, state-travels-as-sealed-segment-chains.md, the-product-is-a-local-bookmark-library.md
---
# Local-first state exchanged as sealed bundles

## Context

Cross-browser bookmarks needed local ownership without an authoritative server.

## Decision

The decision selected complete local state, last-writer-wins registers, hybrid logical clocks, tombstones, and deterministic folder resolution.
Sealed bundles carried state through optional transports.
The [local bookmark library decision](the-product-is-a-local-bookmark-library.md) removes synchronization while preserving the database representation.

## Alternatives

An authoritative server would make availability depend on that service.
A shared writable file would invite conflicting copies.

## Consequences

Local state remains independent of a service.
The active product has no bundle exchange or export.

## Confirmation

The [archived record](https://github.com/P4suta/youarel/blob/d0fbc7afcb58bed077e9be67a956d9202e96df55/docs/adr/local-first-state-in-sealed-bundles.md) preserves the detailed decision and its verification claims.
