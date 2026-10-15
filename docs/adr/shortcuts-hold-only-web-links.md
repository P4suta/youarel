---
status: accepted
date: 2026-10-15
---
# Shortcuts hold only web links

## Context

A shortcut opens through the operating system's handler for its scheme.
A `file:`, `javascript:`, `data:`, or custom-scheme link can start another program or run code, and a synchronized shortcut can arrive from another device.
URL syntax is the WHATWG URL Standard, whose parsing and normalization are too large to reimplement safely.

## Decision

A shortcut holds only an `http` or `https` link with a non-empty host.
The `url` crate parses text by the WHATWG URL Standard and keeps its normalized serialization.
The pure core decides admission over the parts the parser reports, in [`link_rules::admit`](../../crates/youarel-core/src/link_rules.rs), and [`Link::parse`](../../crates/youarel-link/src/lib.rs) is the only way to make a link.
Every later path to the opener re-admits the link it opens, so a link from another device passes the same decision.

Rejected alternatives:

- An allowlist that people extend with their own schemes, because one approved scheme reopens the path to arbitrary programs.
- A parser of our own, because a divergence from the standard would make two browsers disagree about the same shortcut.
- Admission inside the adapter, because the core is where Kani proves the decision.

## Consequences

Kani proves the admission rule for every scheme of up to six bytes and every kind of host.
The `url` crate is a trust assumption, which the `link_parse` fuzz target checks for panics, admitted schemes, and a serialization that parses back to the same link.
Intranet shortcuts with other schemes, such as `smb:`, are out of scope.
