---
status: superseded
date: 2026-10-21
superseded-by: the-product-is-a-local-bookmark-library.md
amends: passwordless-device-identity.md, state-travels-as-sealed-segment-chains.md
---
# Devices join from a screen

## Context

Pairing and recovery needed desktop flows that left bookmark operations available.

## Decision

The decision selected a Devices pane and bounded workers for pairing, naming, removal, and recovery.
Validated library access preceded protocol effects.
Cancellation and work identifiers controlled stale results.
Recovery material appeared once.
Device-name records used record version 2 and database format 4.

## Alternatives

Blocking pairing under the library lock would prevent ordinary operations.
Truncating membership identifiers would conflate identities.

## Consequences

The screen shared authenticated protocol operations with the command-line interface.
Historical epoch grants made retained snapshots recoverable.

## Confirmation

The [archived record](https://github.com/P4suta/youarel/blob/d0fbc7afcb58bed077e9be67a956d9202e96df55/docs/adr/devices-join-from-a-screen.md) preserves the detailed decision and its verification claims.
