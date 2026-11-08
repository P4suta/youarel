---
status: superseded
date: 2026-10-18
superseded-by: the-product-is-a-local-bookmark-library.md
amended-by: window-watches-the-sync-folder.md, the-window-publishes-only-its-own-changes.md, the-product-is-a-local-bookmark-library.md
---
# The window syncs on its own schedule

## Context

The desktop needed scheduled synchronization without blocking bookmark operations.

## Decision

The decision selected a timer, an edit-settling delay, and bounded retry backoff.
Settings held the transport target and last successful synchronization.
Work ran outside the library lock.

## Alternatives

Synchronizing after every intent would repeat network work during a burst of edits.

## Consequences

A running window synchronized in the background and reported failures.

## Confirmation

The [archived record](https://github.com/P4suta/youarel/blob/d0fbc7afcb58bed077e9be67a956d9202e96df55/docs/adr/window-syncs-on-its-own-schedule.md) preserves the detailed decision and its verification claims.
