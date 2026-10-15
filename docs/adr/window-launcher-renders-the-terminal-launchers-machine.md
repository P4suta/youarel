---
status: proposed
date: 2026-10-16
---
# The window launcher renders the terminal launcher's machine

## Context

[A terminal interface before a graphical launcher](terminal-interface-before-a-graphical-launcher.md) promises a Tauri launcher that drives the same state machine, `ui_rules::step`.
A launcher that a global shortcut brings up must hide when the person turns away, toggle from the tray, and accept Japanese text through an input method.
A webview runs untrusted page titles through the same document that holds the launcher's commands.
Tauri's Linux webview needs the WebKitGTK development packages to build, which the Linux host lacks, and installing them needs privileges that a check must not take.

## Decision

The terminal launcher's effects and texts move into `youarel_cli::shell`, which both launchers call: it loads the rows, carries out an effect through the store and the browser, and names the texts of the header, the prompt, and each row.
The window launcher, `youarel-launcher`, holds the interface on the Rust side, maps each key press to the machine's key, and answers every input with the whole view, which the window draws as text and never as markup.
Its window reports keys by their DOM names, and typed text after any input method composes it, so the machine reads composed characters, not keystrokes.
The control key commands as it does in the terminal; control and V moves a carried entry and pastes text otherwise, and the command key of macOS keeps its text editing.

The window starts hidden, shows on `Alt+Space` or from the tray, hides when it loses the focus, and hides after it opens a shortcut.
A second start of the launcher hands `--toggle` to the running one, which `youarel toggle` calls, so a desktop that keeps global shortcuts to itself, such as Wayland's, binds a shortcut of its own.
On macOS the launcher stays out of the Dock.

The launcher lives in `app/`, a workspace of its own like the Worker's, so the webview's dependencies stay out of the other crates and their gates.
Its script is strict TypeScript that `tsc` compiles beside its source with no bundler and no runtime package, and a content security policy admits only the launcher's own files and its IPC.
Its icons come from a drawing in `cargo xtask gen`, which `gen --check` compares, so no binary in the repository lacks a source.
`just check` lints and tests the launcher on macOS and Windows, and `just app-linux` does so on Linux in a pinned container that holds WebKitGTK.
The launcher's lockfile accepts two advisories of GTK 3's bindings, with their reasons in `xtask/src/check.rs`, until Tauri leaves GTK 3.

Rejected alternatives:

- A second machine or key handling in TypeScript, because the window and the terminal would drift apart, and the machine's proofs would cover only one of them.
- A bundler and a front-end framework, because a launcher of one list needs neither, and each adds a dependency tree to the release.
- The window launcher inside the main workspace, because every check of every crate would then build the webview, and the Linux host could not build it at all.
- The command key as the control key on macOS, because it would take copy and paste away from the input field.

## Consequences

The window and the terminal behave alike, and a change to the machine reaches both.
Each input crosses the IPC boundary and reloads the store, which a launcher of a few thousand shortcuts affords.
Icons of shortcuts, the macOS passkeys of recovery, and the launcher's distribution and updates remain, and each needs its own record when it lands.
