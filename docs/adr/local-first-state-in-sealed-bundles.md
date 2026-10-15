---
status: accepted
date: 2026-10-15
---
# Local-first state exchanged as sealed bundles

## Context

Browser bookmarks belong to one browser and one vendor's sync, so moving between browsers or machines means exporting and importing.
A sync server that holds the record of truth strands its data when people lose their identity, and leaves them stuck when it is down.
Some people want no server at all and would rather carry a file on a USB drive or send it through Signal.

## Decision

Every device holds the complete state, and no server holds the record of truth.
The state is a state-based CRDT: last-writer-wins registers per field under hybrid logical clocks, tombstones, and a folder tree resolved deterministically from the merged state.
The one unit of exchange is a sealed bundle: the signed device membership history, the encrypted state, and its encrypted icons.
A bundle arrives through any transport, and every transport feeds the same merge:

- none, for one device;
- a file carried by hand;
- a shared folder in which each device writes only its own file;
- an optional relay service that stores ciphertext.

The relay deletes an account after a long period without an authenticated request, and a returning device seeds a new account from its own state.
`youarel export` always writes the plain state.

Rejected alternatives:

- A server as the record of truth, because it strands orphaned data and makes the service a dependency.
- An operation log, because compaction across intermittent transports needs coordination that a state-based merge avoids.
- One shared file in a sync folder, because concurrent writers produce conflicted copies.

## Consequences

The merge must be commutative, associative, and idempotent, and the tree must stay acyclic under concurrent moves; both become verification obligations when the state lands.
A device that stays away longer than the tombstone horizon must pair again, so it never revives a deleted item.
A server can withhold updates but cannot lose data that a device still holds.
