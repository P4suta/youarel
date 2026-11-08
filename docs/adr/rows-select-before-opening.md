---
status: accepted
date: 2026-11-08
---
# Rows select before opening

## Context

The owner requests file-manager interaction across bookmark names, icons, and row backgrounds.
Opening on single activation conflicts with selecting bookmarks for organization.

## Decision

Single pointer activation selects any entry, and unmodified double-click opens a bookmark or folder once.
Keyboard and assistive activation remain immediate under [W3C keyboard guidance](https://www.w3.org/WAI/WCAG22/Understanding/keyboard.html).
Keep modifier selection and context actions.
Use contiguous 48-pixel targets, relative sizing, and title-focused content following [Apple's list guidance](https://developer.apple.com/design/human-interface-guidelines/lists-and-tables).
Hover inverts the row, selection adds an inset boundary, and keyboard focus adds a dashed boundary.
The owner's monochrome states replace Apple's system colors while keeping [distinct focus and selection](https://developer.apple.com/design/human-interface-guidelines/focus-and-selection).

## Alternatives

Opening on single activation is faster for launching, but makes pointer selection inconsistent.
Smaller rows increase density but reduce the owner's target-size preference.

## Consequences

Pointer opening requires a double-click.
Enter, Space, and context-menu Open provide alternatives without timed gestures.
Reconsider the density if enlarged names or pointer targets become difficult to use.

## Confirmation

Rendered checks cover every row region, modifiers, single opening, stable selection redraws, hover, focus, enlargement, and virtual scrolling.
