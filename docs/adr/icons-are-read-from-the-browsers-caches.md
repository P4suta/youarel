---
status: superseded
date: 2026-10-21
superseded-by: the-product-is-a-local-bookmark-library.md
amends: icons-stay-on-the-device.md
---
# Browser caches supply icons

## Context

Locally cached browser icons could fill bookmark images without contacting their sites.

## Decision

The decision selected bounded discovery and SQLite Backup API snapshots of browser caches.
Queries matched admitted page hosts and preferred suitable raster images.
Every accepted image passed through normalization.
Workers supported cancellation outside the library lock.

## Alternatives

Independent copies of a database and its log would lack a consistent transaction boundary.
A handwritten SQLite reader would duplicate complex storage behavior.

## Consequences

Bundled SQLite added a native dependency and an explicit trust assumption.
Unsupported or unreadable caches could leave icons missing.

## Confirmation

The [archived record](https://github.com/P4suta/youarel/blob/d0fbc7afcb58bed077e9be67a956d9202e96df55/docs/adr/icons-are-read-from-the-browsers-caches.md) preserves the detailed decision and its verification claims.
