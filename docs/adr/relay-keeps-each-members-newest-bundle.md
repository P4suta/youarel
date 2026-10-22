---
status: superseded
date: 2026-10-16
superseded-by: state-travels-as-sealed-segment-chains.md
---
# The relay keeps each member's newest bundle

## Context

[Local-first state exchanged as sealed bundles](local-first-state-in-sealed-bundles.md) makes the sealed bundle the one unit of exchange, and leaves the relay's storage open.
A relay could hold one sealed state per account, which each device fetches, merges, and writes back under a compare-and-swap of its version.
That would need a second message for the state alone, a version that the relay assigns, and a retry whenever two devices write at once.
The shared folder already avoids those conflicts, because each device writes only its own file.

## Decision

The relay keeps one slot per member, which holds that member's newest bundle, and hands every member's slot to any member that asks.
It verifies each bundle against the account's membership log and the sender's keys with the same code the devices run, keeps the log's operations that verify, and drops the slot of every device the log stops listing.
A member's request carries the device's signature, its clock, and a nonce, so the relay admits it once and only within two minutes of its time.
A pairing through the relay travels through a short-lived mailbox, whose nameplate becomes the code's number, and a device that waits for mail asks again after the relay holds its request for up to 25 seconds, rather than sleeping.

Rejected alternatives:

- One sealed state per account under a compare-and-swap, because it adds a message and a retry loop, and a slow device keeps losing the race.
- Long polls through a WebSocket for the mailbox, because a pairing exchanges four messages and needs nothing that stays open.

## Consequences

The relay is a shared folder that checks who writes, so every transport feeds the same import, and a device moves between them without a change of format.
The relay holds an account's bundles once per member, which costs space in proportion to the members, and the quota bounds the total.
An account that the relay deleted after its retention returns with the next bundle of any member, since a bundle carries the whole log.
The relay's decisions live in `crates/youarel-core/src/relay_rules.rs`, whose Kani harnesses prove the window, the replay check, the quotas, and the nameplates.
