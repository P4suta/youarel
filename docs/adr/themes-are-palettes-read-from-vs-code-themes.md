---
status: proposed
date: 2026-10-18
---
# Themes are palettes read from VS Code themes

## Context

The owner's design offers sixteen named color themes, a light and a dark choice that follow the system, and a button that reads a theme file.
People already keep the themes they like as VS Code color themes, which are JSON with comments and trailing commas, with workbench colors under `colors` and syntax colors under `tokenColors`.
A theme file comes from anywhere, and the page must draw it without a style sheet of its own, under a content security policy that admits no inline style sheet.

## Decision

A theme is a palette of twenty slots: the window's surfaces, text, borders, accent, and danger, and the parts of a link that the cards set as type.
`theme_rules` reads a color of three, four, six, or eight hexadecimal digits, and resolves a palette from the slots a theme declares, filling every other slot from a fallback of its own kind, light or dark, with Kani harnesses beside each.
The crate `youarel-theme` holds the sixteen bundled palettes and reads a VS Code theme within a bound on its size, mapping workbench colors to the surfaces and the colors of functions, keywords, numbers, strings, and comments to the parts of a link.
A theme that names no kind takes the kind its background's brightness says.

The window draws a palette by setting the style sheet's custom properties through the CSS object model, which the policy admits.
The settings keep the choice of a light and a dark theme, whether the window follows the system, and every imported palette, resolved and named, in `settings.json` beside the store.
An imported theme is never read again from its file, so a moved or edited file changes nothing.

Rejected alternatives:

- Loading a theme's file as a style sheet, because the policy forbids it and a file could then restyle anything.
- Keeping only the bundled themes, because the design reads a file and people keep their themes elsewhere.
- Syncing the imported themes inside the sealed state, because a palette is a preference of one device, as the layout is.

## Consequences

A theme that sets colors only for scopes youarel does not map shows its surfaces and youarel's fallbacks for the link's parts.
Another device of the account shows its own theme, and a person imports a theme on each device that should wear it.
The fuzz target `theme_parse` owns the reader, and `settings_decode` owns the settings that keep the imported palettes.
