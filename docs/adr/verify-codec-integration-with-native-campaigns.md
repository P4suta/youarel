---
status: accepted
date: 2026-11-11
---
# Verify codec integration with native campaigns

## Context

The device-record round-trip harness was inconclusive within four CPUs, 8 GiB, and five minutes per harness.
The [record codec](../../crates/youarel-core/src/record_rules.rs) uses proofs for primitive decisions and execution checks for complete records.

## Decision

Run Kani checks of name content through four UTF-8 bytes and of the byte limit over every `usize` value.
Existing wire and stamp proofs keep their input domains.

Exercise complete records through native codec tests, Miri, mutation testing, and corpus replay.
Retire the complete-record theorem.

## Alternatives

Retaining the complete-record theorem preserves the stronger formal claim but delays candidates until a tractable proof exists.
Verified function contracts could establish composition without expanding the complete codec.

## Consequences

The chosen boundary doesn't formally establish the complete codec's round trip for arbitrary membership identities and stamps.
Passing component proofs don't prove their composition.
Reconsider verified composition after a codec defect escapes execution checks.

## Confirmation

Require the admission, wire, and stamp proofs and their negative controls.
Require native codec cases, Miri, the full mutation inventory, and corpus replay before candidate preparation.
