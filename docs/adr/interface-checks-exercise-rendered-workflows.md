---
status: proposed
date: 2026-11-08
---
# Interface checks exercise rendered workflows

## Context

Rust tests establish library behavior, but can't detect inaccessible names, keyboard traps, clipped forms, or CSS regressions.
The desktop embeds HTML whose rendered behavior needs direct checks.

## Decision

Run pinned Playwright and axe-core checks through the existing fast gate.
Exercise the production renderer with typed bookmark fixtures and a controlled command boundary.
Keep the native WebKit window test for the real command boundary.
Check keyboard workflows, dialogs, errors, contrast, target sizes, text enlargement, and narrow layouts.
Use the [Web Content Accessibility Guidelines 2.2](https://www.w3.org/TR/WCAG22/) and applicable [Digital Agency guidance](https://design.digital.go.jp/dads/components/) as acceptance criteria.

## Alternatives

Native window tests alone exercise the actual webview but provide limited layout and semantic diagnostics.
Browser checks alone can't establish native integration or screen-reader usability.

## Consequences

Development installs a locked test-only browser dependency.
No browser testing library joins the app.
Reconsider the fixture boundary if interface behavior moves into the renderer.

## Confirmation

The gate must reject inaccessible fixtures and broken interactions.
Production flows must pass browser checks and native window checks.
Direct review must confirm meaningful labels and task completion.
