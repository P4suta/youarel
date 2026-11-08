---
status: amended
date: 2026-10-18
supersedes: window-launcher-renders-the-terminal-launchers-machine.md
amended-by: the-window-draws-a-page-of-the-library.md, the-window-behaves-like-the-finder.md, the-product-is-a-local-bookmark-library.md
---
# The window is a library of shortcuts

## Context

Saved research links needed folders, pointer interaction, and editing in an ordinary desktop window.

## Decision

The decision selected a Rust-owned library with views rendered by strict TypeScript.
Collections formed a nested tree.
Search covered bookmark fields, and each collection admitted one copy of a link.
The [local bookmark library decision](the-product-is-a-local-bookmark-library.md) selects one list and removes settings, alternate layouts, themes, and import or export.

## Alternatives

Extending the terminal popup's key machine would mix unrelated interaction models.
A front-end framework would add dependencies to a view renderer.

## Consequences

The desktop keeps collection navigation, selection, editing, undo, and opening.
The terminal keeps its own interface machine.

## Confirmation

The [archived record](https://github.com/P4suta/youarel/blob/d0fbc7afcb58bed077e9be67a956d9202e96df55/docs/adr/window-is-a-library-of-shortcuts.md) preserves the detailed decision and its verification claims.
