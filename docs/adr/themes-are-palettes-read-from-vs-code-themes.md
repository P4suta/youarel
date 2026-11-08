---
status: superseded
date: 2026-10-18
superseded-by: the-product-is-a-local-bookmark-library.md
---
# Importable color palettes

## Context

The desktop design offered named palettes and imported VS Code color themes.

## Decision

The decision proposed twenty resolved color slots, sixteen bundled palettes, and bounded theme parsing.
Core rules admitted colors and supplied fallback slots.
Local settings held theme choices and imported palettes.

## Alternatives

Loading arbitrary stylesheets would exceed the page's content security policy.

## Consequences

Resolved imports would survive moving or editing the source theme file.

## Confirmation

The [archived record](https://github.com/P4suta/youarel/blob/d0fbc7afcb58bed077e9be67a956d9202e96df55/docs/adr/themes-are-palettes-read-from-vs-code-themes.md) preserves the detailed decision and its verification claims.
