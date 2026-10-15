---
status: accepted
date: 2026-10-15
---
# A terminal interface before a graphical launcher

## Context

The intended experience is a launcher that opens on a key, finds a shortcut by a few letters, and opens it in the default browser.
A resident graphical launcher on three desktops needs a tray, a global hotkey, window management, and a separate distribution, which delays everything else.
The search, the folders, the storage, and the sync are the same whatever draws them.

## Decision

The first interface runs in a terminal with ratatui, launched as `youarel` without arguments.
Its behavior is a pure state machine in the core, `ui_rules::step`, and the shell around it owns the terminal, the opener, and the fetches, as the owner's sivi project does for its terminal interface.
A global hotkey comes from the terminal's own quick window.
A Tauri launcher follows the first usable release and drives the same state machine.
Mobile devices are out of scope.

Rejected alternatives:

- The graphical launcher first, because it multiplies the platform work before the core is settled.
- Flutter, because its desktop text input is weaker for Japanese input methods and it shares nothing with the Rust core.

## Consequences

The first release is one binary, which the existing command-line distribution signs and updates.
Icons wait for the graphical launcher, because a terminal cannot show them.
