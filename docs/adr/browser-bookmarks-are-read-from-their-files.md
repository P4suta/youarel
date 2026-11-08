---
status: superseded
date: 2026-10-18
superseded-by: the-product-is-a-local-bookmark-library.md
---
# Browser profile files supply bookmarks

## Context

The desktop design offered direct import from browser profiles and supplied bookmark files.

## Decision

The decision proposed bounded readers for Chromium JSON, Firefox compressed backups, Safari property lists, and Netscape bookmark HTML.
Core rules selected profile paths and reader formats.
Imports merged folders by name and avoided duplicate links within a folder.

## Alternatives

Browser extensions would require separate distribution and maintenance.
Firefox's live database would add a database dependency for import.

## Consequences

Firefox imports could lag behind recent edits until its backup changed.
Safari access depended on existing operating-system permissions.

## Confirmation

The [archived record](https://github.com/P4suta/youarel/blob/d0fbc7afcb58bed077e9be67a956d9202e96df55/docs/adr/browser-bookmarks-are-read-from-their-files.md) preserves the detailed decision and its verification claims.
