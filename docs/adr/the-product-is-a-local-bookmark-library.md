---
status: accepted
date: 2026-11-08
amends: local-first-state-in-sealed-bundles.md, window-is-a-library-of-shortcuts.md
---
# The product is a local bookmark library

## Context

The product saves, edits, files, finds, and opens bookmarks.
Synchronization, recovery, import, export, alternate layouts, and themes add unwanted workflows.

## Decision

Present one bookmark list with collections, search, and editing.
Remove the retired controls, commands, background tasks, transports, and dependencies.
Preserve the database format and existing bookmarks.
Ignore retired preferences and leave their files untouched.
Superseded decisions cover [scheduling](window-syncs-on-its-own-schedule.md), [folder watching](window-watches-the-sync-folder.md), [publication](the-window-publishes-only-its-own-changes.md), [devices](devices-join-from-a-screen.md), [themes](themes-are-palettes-read-from-vs-code-themes.md), [imports](browser-bookmarks-are-read-from-their-files.md), and [browser icons](icons-are-read-from-the-browsers-caches.md).

The list follows [Apple's list guidance](https://developer.apple.com/design/human-interface-guidelines/lists-and-tables#Best-practices).
Owner policy selects black and white and excludes presentation customization.
Permanent-deletion confirmations describe local deletion and its undo limit, following [Apple's Alerts guidance](https://developer.apple.com/design/human-interface-guidelines/alerts#Content).

## Alternatives

Hiding controls would keep unwanted dependencies.
Replacing the database format would risk existing bookmarks.

## Consequences

Each installation keeps an independent library.
The indexed state and its checks support local edits and undo.
The release prohibition remains.

## Confirmation

The interfaces must reject retired commands and intents.
Existing bookmarks must survive edits and reopening despite retired configuration files.
The project checks must pass.
