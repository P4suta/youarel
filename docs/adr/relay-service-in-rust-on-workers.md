---
status: proposed
date: 2026-10-15
---
# The relay service in Rust on Cloudflare Workers

## Context

The optional relay stores ciphertext, verifies device signatures and the membership history, and notifies devices of new versions.
The clients are Rust, so the envelope codec and the membership verification could be one crate on both sides.
The owner's earlier relay in mylist is TypeScript because its client is Swift, which forces a differential fuzz between two decoders.

## Decision

The relay is a workers-rs Worker with one SQLite-backed Durable Object per account and one per pairing mailbox, and it shares the protocol crate with the clients.
D1 and KV are not used, for the isolation and consistency reasons of the mylist relay.

This record stays proposed until the relay work confirms that workers-rs supports SQLite storage and hibernatable WebSockets in Durable Objects, and that the cryptographic crates build for `wasm32`.
If either fails, the relay follows the mylist layout instead: plain TypeScript, conformance transcripts, and a differential fuzz against the Rust codec.

## Consequences

One proof of the envelope and the membership rules covers both the clients and the relay.
The relay depends on the maturity of workers-rs, which the confirmation above checks.
