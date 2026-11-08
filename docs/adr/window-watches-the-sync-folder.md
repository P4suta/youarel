---
status: superseded
date: 2026-10-19
superseded-by: the-product-is-a-local-bookmark-library.md
amends: window-syncs-on-its-own-schedule.md
amended-by: the-window-publishes-only-its-own-changes.md, the-product-is-a-local-bookmark-library.md
---
# The window watches the sync folder

## Context

Scheduled polling delayed delivery of bookmarks written by another device.

## Decision

The decision selected filesystem notifications through `notify`.
The core filtered relevant bundle changes and waited for the folder to settle.
Notifications requested merges without triggering publication or bypassing retry backoff.

## Alternatives

Faster polling would reread every bundle.
Publishing after each notification could create feedback between devices.

## Consequences

A failed watch fell back to the synchronization schedule.

## Confirmation

The [archived record](https://github.com/P4suta/youarel/blob/d0fbc7afcb58bed077e9be67a956d9202e96df55/docs/adr/window-watches-the-sync-folder.md) preserves the detailed decision and its verification claims.
