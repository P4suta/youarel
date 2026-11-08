---
status: accepted
date: 2026-11-08
---
# Window commands are contextual

## Context

The desktop uses [folder navigation](folders-show-their-contents.md) and a name-focused list.
Permanent search and command controls compete with that list.

## Decision

Keep a static Name heading, fixed name order within ordinary folders, and a breadcrumb inside folders.
Use context menus, keyboard commands, and macOS menu-bar commands for creation and item actions.
Follow the [W3C menu keyboard pattern](https://www.w3.org/WAI/ARIA/apg/patterns/menubar/) and [Apple's contextual-menu guidance](https://developer.apple.com/design/human-interface-guidelines/context-menus).
New bookmarks use the current folder without a destination picker.
Retire desktop search and sort intents.

## Alternatives

Permanent command buttons improve discovery but repeat menu actions.
Full-library search helps large libraries but adds another navigation mode.

## Consequences

Empty folders must explain creation.
The content receives initial focus.
Keep keyboard context access and restore the invoking row after cancellation.
Reconsider permanent controls or search when observed tasks require them.

## Confirmation

The root has no search, sort, or duplicate command buttons.
Pointer, keyboard, and native menu commands create in the current folder.
Trash offers recovery and deletion without creation.
